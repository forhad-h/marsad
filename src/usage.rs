//! State for the Usage view, plus the background sync that keeps the history current.

use crate::store::{self, DayRow, RunRow, SessionRow, SkillRow};
use ratatui::crossterm::event::KeyCode;
use rusqlite::Connection;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

const SYNC_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Days,
    Sessions,
    Skills,
}

impl Pane {
    fn next(self) -> Self {
        match self {
            Pane::Days => Pane::Sessions,
            Pane::Sessions => Pane::Skills,
            Pane::Skills => Pane::Days,
        }
    }

    fn prev(self) -> Self {
        self.next().next()
    }
}

/// How far back the Usage view looks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Period {
    Week,
    Month,
    Quarter,
    Year,
    All,
}

impl Period {
    const ORDER: [Period; 5] = [Period::Week, Period::Month, Period::Quarter, Period::Year, Period::All];

    pub fn days(self) -> Option<u32> {
        match self {
            Period::Week => Some(7),
            Period::Month => Some(30),
            Period::Quarter => Some(90),
            Period::Year => Some(365),
            Period::All => None,
        }
    }

    pub fn label(self) -> String {
        match self.days() {
            Some(n) => format!("last {n} days"),
            None => "all time".into(),
        }
    }

    fn shift(self, delta: isize) -> Self {
        let i = Self::ORDER.iter().position(|p| *p == self).unwrap_or(0) as isize;
        Self::ORDER[(i + delta).rem_euclid(Self::ORDER.len() as isize) as usize]
    }
}

/// Headline numbers for the selected period.
#[derive(Default, Debug)]
pub struct Summary {
    pub total: i64,
    pub sessions: i64,
    /// Calendar days covered: the period, or less when history starts later.
    pub span_days: i64,
    pub busiest: Option<(String, i64)>,
}

impl Summary {
    pub fn daily_average(&self) -> i64 {
        if self.span_days > 0 { self.total / self.span_days } else { 0 }
    }
}

/// One session's skill runs and per-day totals.
pub struct Drill {
    pub session: SessionRow,
    pub runs: Vec<RunRow>,
    pub days: Vec<(String, i64)>,
    pub scroll: usize,
}

pub struct Usage {
    pub period: Period,
    pub summary: Summary,
    /// Days of the period with usage, newest first.
    pub days: Vec<DayRow>,
    /// Selected local day; None = the whole period.
    pub day: Option<String>,
    pub sessions: Vec<SessionRow>,
    pub skills: Vec<SkillRow>,
    pub focus: Pane,
    pub session_idx: usize,
    pub skill_idx: usize,
    pub drill: Option<Drill>,
    pub syncing: bool,
    pub error: Option<String>,
    conn: Option<Connection>,
    /// Delivers whether a background sync changed any rows.
    rx: Option<Receiver<Result<bool, String>>>,
    last_sync: Option<Instant>,
}

impl Usage {
    pub fn new() -> Self {
        Self {
            period: Period::Month,
            summary: Summary::default(),
            days: Vec::new(),
            day: None,
            sessions: Vec::new(),
            skills: Vec::new(),
            focus: Pane::Days,
            session_idx: 0,
            skill_idx: 0,
            drill: None,
            syncing: false,
            error: None,
            conn: None,
            rx: None,
            last_sync: None,
        }
    }

    /// Called every frame: collects a finished sync and starts the next one when due.
    pub fn tick(&mut self) {
        let finished = match self.rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(res)) => Some(res),
            // The sync thread died without reporting (e.g. a panic): surface it and allow a retry.
            Some(Err(TryRecvError::Disconnected)) => Some(Err("sync stopped unexpectedly".into())),
            Some(Err(TryRecvError::Empty)) | None => None,
        };
        if let Some(res) = finished {
            self.rx = None;
            self.syncing = false;
            match res {
                Ok(changed) => {
                    self.error = None;
                    if changed {
                        self.reload();
                    }
                }
                Err(e) => self.error = Some(e),
            }
        }
        if self.rx.is_none() && self.last_sync.is_none_or(|t| t.elapsed() >= SYNC_EVERY) {
            self.start_sync();
        }
    }

    pub fn start_sync(&mut self) {
        if self.rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let res = store::open_and_sync().map(|r| r.updated + r.pruned > 0).map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
        self.rx = Some(rx);
        self.syncing = true;
        self.last_sync = Some(Instant::now());
    }

    pub fn reload(&mut self) {
        if self.conn.is_none() {
            match store::open() {
                Ok(c) => self.conn = Some(c),
                Err(e) => {
                    self.error = Some(e.to_string());
                    return;
                }
            }
        }
        let conn = self.conn.as_ref().expect("opened above");
        let start = self.period.days().map(store::day_n_ago);
        let result = (|| -> anyhow::Result<()> {
            let period = store::Days::since(start.as_deref());
            self.days = store::days(conn, period)?;
            if self.day.as_ref().is_some_and(|d| !self.days.iter().any(|r| &r.day == d)) {
                self.day = None;
            }
            self.summary = summarize(&self.days, start.as_deref(), store::first_day(conn)?.as_deref());
            self.summary.sessions = store::session_count(conn, period)?;
            let range = self.day.as_deref().map_or(period, store::Days::on);
            self.sessions = store::sessions_in(conn, range)?;
            self.skills = store::skills_in(conn, range)?;
            if let Some(drill) = self.drill.as_mut() {
                drill.runs = store::session_runs(conn, &drill.session.session_id)?;
                drill.days = store::session_days(conn, &drill.session.session_id)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.error = Some(e.to_string());
        }
        self.session_idx = self.session_idx.min(self.sessions.len().saturating_sub(1));
        self.skill_idx = self.skill_idx.min(self.skills.len().saturating_sub(1));
    }

    fn set_period(&mut self, period: Period) {
        self.period = period;
        self.day = None;
        self.session_idx = 0;
        self.skill_idx = 0;
        self.reload();
    }

    /// Index of the selected day in the day list, where 0 is the whole period.
    pub fn day_index(&self) -> usize {
        match &self.day {
            None => 0,
            Some(d) => self.days.iter().position(|r| &r.day == d).map_or(0, |i| i + 1),
        }
    }

    fn move_day(&mut self, delta: isize) {
        let next = (self.day_index() as isize + delta).clamp(0, self.days.len() as isize) as usize;
        self.day = if next == 0 { None } else { Some(self.days[next - 1].day.clone()) };
        self.session_idx = 0;
        self.skill_idx = 0;
        self.reload();
    }

    fn open_drill(&mut self) {
        let Some(session) = self.sessions.get(self.session_idx).cloned() else { return };
        self.drill = Some(Drill { session, runs: Vec::new(), days: Vec::new(), scroll: 0 });
        self.reload();
    }

    /// Handles a key; returns false when the view should close.
    pub fn on_key(&mut self, code: KeyCode) -> bool {
        if let Some(drill) = self.drill.as_mut() {
            let max = drill.runs.len().saturating_sub(1);
            match code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Left | KeyCode::Char('h') => self.drill = None,
                KeyCode::Down | KeyCode::Char('j') => drill.scroll = (drill.scroll + 1).min(max),
                KeyCode::Up | KeyCode::Char('k') => drill.scroll = drill.scroll.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => drill.scroll = (drill.scroll + 10).min(max),
                KeyCode::PageUp => drill.scroll = drill.scroll.saturating_sub(10),
                KeyCode::Home | KeyCode::Char('g') => drill.scroll = 0,
                KeyCode::End | KeyCode::Char('G') => drill.scroll = max,
                _ => {}
            }
            return true;
        }
        let step = |idx: &mut usize, len: usize, delta: isize| {
            *idx = (*idx as isize + delta).clamp(0, len.saturating_sub(1) as isize) as usize;
        };
        match code {
            KeyCode::Esc | KeyCode::Char('u') => return false,
            KeyCode::Char('r') => self.start_sync(),
            KeyCode::Char('p') => self.set_period(self.period.shift(1)),
            KeyCode::Char('P') => self.set_period(self.period.shift(-1)),
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => self.focus = self.focus.next(),
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => self.focus = self.focus.prev(),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Up | KeyCode::Char('k') => {
                let delta = if matches!(code, KeyCode::Down | KeyCode::Char('j')) { 1 } else { -1 };
                match self.focus {
                    Pane::Days => self.move_day(delta),
                    Pane::Sessions => step(&mut self.session_idx, self.sessions.len(), delta),
                    Pane::Skills => step(&mut self.skill_idx, self.skills.len(), delta),
                }
            }
            KeyCode::Enter => match self.focus {
                Pane::Days => self.focus = Pane::Sessions,
                Pane::Sessions => self.open_drill(),
                Pane::Skills => {}
            },
            _ => {}
        }
        true
    }
}

/// Totals and busiest day for `days` (newest first); the span runs from the period start, or
/// the first recorded day if later, through today.
fn summarize(days: &[DayRow], start: Option<&str>, first: Option<&str>) -> Summary {
    let parse = |d: &str| chrono::NaiveDate::parse_from_str(d, crate::model::DAY_FMT).ok();
    let today = chrono::Local::now().date_naive();
    let from = [start, first].into_iter().flatten().filter_map(parse).max();
    Summary {
        total: days.iter().map(|d| d.total).sum(),
        sessions: 0,
        span_days: from.map_or(0, |f| (today - f).num_days() + 1).max(0),
        busiest: days.iter().max_by_key(|d| d.total).map(|d| (d.day.clone(), d.total)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage_with(rows: &str) -> Usage {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(store::SCHEMA).unwrap();
        conn.execute_batch(rows).unwrap();
        let mut u = Usage::new();
        u.conn = Some(conn);
        u.reload();
        u
    }

    #[test]
    fn navigation_on_empty_history_is_safe() {
        let mut u = usage_with("");
        assert!(u.error.is_none());
        for code in [KeyCode::Down, KeyCode::Up, KeyCode::Tab, KeyCode::Down, KeyCode::Enter, KeyCode::Tab, KeyCode::Down] {
            assert!(u.on_key(code));
        }
        assert_eq!(u.day, None);
        assert!(u.drill.is_none(), "nothing to drill into");
        assert!(!u.on_key(KeyCode::Esc), "esc leaves the view");
    }

    #[test]
    fn days_filter_sessions_and_enter_drills_in() {
        let (d1, d2) = (store::day_n_ago(2), store::day_n_ago(3));
        let mut u = usage_with(&format!(
            "INSERT INTO sessions (session_id, project) VALUES ('s1', 'web'), ('s2', 'api');
             INSERT INTO usage_daily VALUES ('s1', 'main', 'main', '{d1}', 0, 0, 0, 0, 500, 3);
             INSERT INTO usage_daily VALUES ('s2', 'main', 'main', '{d2}', 0, 0, 0, 0, 100, 1);
             INSERT INTO skill_runs VALUES ('s1', 'main', 'main', 0, 'ce-plan', 'tool', '{d1}T10:00:00Z', '{d1}', 1, 0, 0, 0, 0, 400);"
        ));
        // "All days" first: both sessions, biggest first.
        assert_eq!(u.days.len(), 2);
        assert_eq!(u.sessions.iter().map(|s| s.project.as_str()).collect::<Vec<_>>(), ["web", "api"]);

        // Down selects the newest day; further moves clamp at the oldest.
        u.on_key(KeyCode::Down);
        assert_eq!(u.day.as_deref(), Some(d1.as_str()));
        assert_eq!(u.sessions.len(), 1);
        assert_eq!(u.skills[0].skill, "ce-plan");
        for _ in 0..5 {
            u.on_key(KeyCode::Down);
        }
        assert_eq!(u.day.as_deref(), Some(d2.as_str()));
        assert_eq!(u.sessions[0].project, "api");
        u.on_key(KeyCode::Up);
        u.on_key(KeyCode::Up);
        assert_eq!(u.day, None, "back to all days");

        // Enter on Days jumps to Sessions; Enter there opens the drill-down.
        u.on_key(KeyCode::Enter);
        assert!(u.focus == Pane::Sessions);
        u.on_key(KeyCode::Enter);
        let drill = u.drill.as_ref().expect("drill opened");
        assert_eq!(drill.session.session_id, "s1");
        assert_eq!(drill.runs.len(), 1);
        assert_eq!(drill.days, vec![(d1.clone(), 500)]);

        // Inside the drill, Esc closes it but stays in the view.
        assert!(u.on_key(KeyCode::Esc));
        assert!(u.drill.is_none());
        u.on_key(KeyCode::BackTab);
        assert!(u.focus == Pane::Days);
        assert!(!u.on_key(KeyCode::Char('u')));
    }

    #[test]
    fn period_limits_days_and_summary() {
        let today = store::day_n_ago(1);
        let old = store::day_n_ago(100);
        let mut u = usage_with(&format!(
            "INSERT INTO usage_daily VALUES ('s1', 'main', 'main', '{today}', 0, 0, 0, 0, 300, 1);
             INSERT INTO usage_daily VALUES ('s2', 'main', 'main', '{old}', 0, 0, 0, 0, 900, 1);"
        ));
        assert_eq!(u.period, Period::Month);
        assert_eq!(u.days.len(), 1, "100-day-old usage is outside the last 30 days");
        assert_eq!((u.summary.total, u.summary.sessions, u.summary.span_days), (300, 1, 30));
        assert_eq!(u.summary.daily_average(), 10);

        u.on_key(KeyCode::Char('p'));
        u.on_key(KeyCode::Char('p'));
        assert_eq!(u.period, Period::Year);
        assert_eq!(u.days.len(), 2);
        assert_eq!(u.sessions.len(), 2);
        assert_eq!(u.summary.busiest, Some((old.clone(), 900)));

        u.on_key(KeyCode::Char('p'));
        assert_eq!(u.period, Period::All);
        assert_eq!(u.summary.span_days, 100, "all time starts at the first recorded day");
        u.on_key(KeyCode::Char('p'));
        assert_eq!(u.period, Period::Week, "wraps around");
        u.on_key(KeyCode::Char('P'));
        assert_eq!(u.period, Period::All);
    }
}
