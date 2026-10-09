//! State for the 30-day Usage view, plus the background sync that keeps the history current.

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

/// One session's skill runs and per-day totals.
pub struct Drill {
    pub session: SessionRow,
    pub runs: Vec<RunRow>,
    pub days: Vec<(String, i64)>,
    pub scroll: usize,
}

pub struct Usage {
    pub days: Vec<DayRow>,
    /// Selected local day; None = all retained days.
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
        let result = (|| -> anyhow::Result<()> {
            self.days = store::days(conn)?;
            if self.day.as_ref().is_some_and(|d| !self.days.iter().any(|r| &r.day == d)) {
                self.day = None;
            }
            self.sessions = store::sessions_on(conn, self.day.as_deref())?;
            self.skills = store::skills_on(conn, self.day.as_deref())?;
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

    /// Index of the selected day in the day list, where 0 is "all days".
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
