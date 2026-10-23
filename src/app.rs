use crate::events;
use crate::jsonl::JsonlTail;
use crate::model::{EventKind, RateWindow, Session, SortKey, Status, Subagent, TimelineEntry};
use crate::transcript::{self, TranscriptReader};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Marks sessions whose Claude Code process is gone (crash, closed terminal) as ended.
pub fn mark_exited(sessions: &mut HashMap<String, Session>, sys: &mut System) {
    let now = crate::now();
    let mut pids: Vec<Pid> = sessions
        .values()
        .filter(|s| s.status != Status::Ended)
        .filter_map(|s| s.pid.map(Pid::from_u32))
        .collect();
    pids.sort();
    pids.dedup();
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, ProcessRefreshKind::nothing());
    for s in sessions.values_mut().filter(|s| s.status != Status::Ended) {
        let gone = match s.pid {
            Some(pid) => sys.process(Pid::from_u32(pid)).is_none(),
            None => now - s.last_event > STALE_SECS,
        };
        if gone {
            s.set_status(Status::Ended, now);
            s.detail = "process exited".into();
            for a in s.subagents.values_mut() {
                a.running = false;
            }
        }
    }
}

/// Attaches the latest status line report (exact context window, rate limits) to each session.
pub fn load_snapshots(sessions: &mut HashMap<String, Session>) {
    let dir = crate::statusline::status_dir();
    for s in sessions.values_mut().filter(|s| s.status != Status::Ended || s.snapshot.is_none()) {
        if let Some(snap) = crate::statusline::load(&dir.join(format!("{}.json", s.id))) {
            s.snapshot = Some(snap);
        }
    }
}

/// Account-wide 5-hour and 7-day usage, from the most recent status line report.
pub fn latest_rate_limits(sessions: &HashMap<String, Session>) -> Option<(Option<RateWindow>, Option<RateWindow>)> {
    sessions
        .values()
        .filter_map(|s| s.snapshot.as_ref())
        .filter(|snap| snap.five_hour.is_some() || snap.seven_day.is_some())
        .max_by(|a, b| a.ts.total_cmp(&b.ts))
        .map(|snap| (snap.five_hour, snap.seven_day))
}

/// Ended sessions stay visible this long unless `a` is toggled.
const ENDED_GRACE_SECS: f64 = 10.0 * 60.0;
/// Sessions with no known process and no events for this long are treated as gone.
const STALE_SECS: f64 = 12.0 * 3600.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    History,
    /// One session's subagents: launching skill, tokens used, context left.
    Agents,
    Usage,
}

/// Which events the history shows; `f` cycles through them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryFilter {
    #[default]
    All,
    Prompts,
    Replies,
    Skills,
    Agents,
    Questions,
}

impl HistoryFilter {
    pub fn next(self) -> Self {
        match self {
            HistoryFilter::All => HistoryFilter::Prompts,
            HistoryFilter::Prompts => HistoryFilter::Replies,
            HistoryFilter::Replies => HistoryFilter::Skills,
            HistoryFilter::Skills => HistoryFilter::Agents,
            HistoryFilter::Agents => HistoryFilter::Questions,
            HistoryFilter::Questions => HistoryFilter::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            HistoryFilter::All => "all",
            HistoryFilter::Prompts => "prompts",
            HistoryFilter::Replies => "replies",
            HistoryFilter::Skills => "skills",
            HistoryFilter::Agents => "subagents",
            HistoryFilter::Questions => "questions",
        }
    }

    pub fn matches(self, kind: &EventKind) -> bool {
        match self {
            HistoryFilter::All => true,
            HistoryFilter::Prompts => matches!(kind, EventKind::Prompt(_)),
            HistoryFilter::Replies => matches!(kind, EventKind::Response(_)),
            HistoryFilter::Skills => matches!(kind, EventKind::Skill(_)),
            HistoryFilter::Agents => matches!(kind, EventKind::Agent { .. }),
            HistoryFilter::Questions => matches!(kind, EventKind::Question { .. }),
        }
    }
}

pub struct App {
    pub sessions: HashMap<String, Session>,
    pub selected: Option<String>,
    pub show_ended: bool,
    pub sort: SortKey,
    pub sort_reversed: bool,
    pub view: View,
    pub history_scroll: usize,
    pub history_filter: HistoryFilter,
    /// Lines scrolled in the selected history event's detail pane.
    pub detail_scroll: u16,
    pub compact_window: Option<u64>,
    pub tick: u64,
    pub hooks_installed: bool,
    pub usage: crate::usage::Usage,
    pub update: Option<crate::update::Notice>,
    /// Sounds and notifications for sessions that need the user or finish; `m` toggles.
    pub alerts_on: bool,
    /// A failure to show in the footer, such as settings that couldn't be saved; the next key clears it.
    pub error: Option<String>,
    alerts: crate::alerts::Detector,
    update_rx: std::sync::mpsc::Receiver<crate::update::Notice>,
    log: JsonlTail,
    readers: HashMap<PathBuf, TranscriptReader>,
    sys: System,
    last_slow_refresh: Instant,
}

impl App {
    fn new() -> Self {
        let dir = crate::data_dir();
        let mut sessions = HashMap::new();
        // Replay the rotated log first so long-running sessions keep their history.
        for ev in JsonlTail::new(dir.join("events.1.jsonl")).read_new() {
            events::apply(&mut sessions, &ev);
        }
        let hooks_installed = std::fs::read_to_string(crate::claude_dir().join("settings.json"))
            .is_ok_and(|s| s.contains("marsad") && s.contains(" hook"));
        Self {
            sessions,
            selected: None,
            show_ended: false,
            sort: SortKey::default(),
            sort_reversed: false,
            view: View::List,
            history_scroll: 0,
            history_filter: HistoryFilter::default(),
            detail_scroll: 0,
            compact_window: crate::model::auto_compact_window(),
            tick: 0,
            hooks_installed,
            usage: crate::usage::Usage::new(),
            update: None,
            alerts_on: crate::settings::alerts(),
            error: None,
            alerts: crate::alerts::Detector::default(),
            update_rx: crate::update::background(),
            log: JsonlTail::new(dir.join("events.jsonl")),
            readers: HashMap::new(),
            sys: System::new(),
            last_slow_refresh: Instant::now() - Duration::from_secs(60),
        }
    }

    /// Sessions to show, in the chosen sort order (most urgent first by default).
    pub fn visible(&self) -> Vec<&Session> {
        let now = crate::now();
        let mut list: Vec<&Session> = self
            .sessions
            .values()
            .filter(|s| self.show_ended || s.status != Status::Ended || now - s.status_since < ENDED_GRACE_SECS)
            .collect();
        crate::model::sort_sessions(&mut list, self.sort, self.sort_reversed, self.compact_window);
        list
    }

    pub fn selected_session(&self) -> Option<&Session> {
        let visible = self.visible();
        self.selected
            .as_ref()
            .and_then(|id| visible.iter().find(|s| &s.id == id).copied())
            .or_else(|| visible.first().copied())
    }

    pub fn selected_index(&self) -> Option<usize> {
        let sel = self.selected_session()?;
        self.visible().iter().position(|s| s.id == sel.id)
    }

    /// Account-wide 5-hour and 7-day usage, from the most recent status line report.
    pub fn rate_limits(&self) -> Option<(Option<RateWindow>, Option<RateWindow>)> {
        latest_rate_limits(&self.sessions)
    }

    fn move_selection(&mut self, delta: isize) {
        let ids: Vec<String> = self.visible().iter().map(|s| s.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let cur = self.selected_index().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, ids.len() as isize - 1) as usize;
        self.selected = Some(ids[next].clone());
    }

    /// Opens a full-screen view of the highlighted session, scrolled to the top.
    fn open(&mut self, view: View) {
        if let Some(id) = self.selected_session().map(|s| s.id.clone()) {
            self.selected = Some(id);
            self.view = view;
            self.history_scroll = 0;
            self.detail_scroll = 0;
        }
    }

    /// Changes the sort order while keeping the highlighted session selected.
    fn set_sort(&mut self, key: SortKey, reversed: bool) {
        self.selected = self.selected_session().map(|s| s.id.clone());
        self.sort = key;
        self.sort_reversed = reversed;
    }

    /// The selected session's history events that pass the filter.
    pub fn history<'a>(&self, s: &'a Session) -> Vec<TimelineEntry<'a>> {
        let mut list = s.timeline();
        list.retain(|e| self.history_filter.matches(&e.event.kind));
        list
    }

    /// Scrolls the detail pane, stopping near the end of the selected event's text.
    fn scroll_detail(&mut self, delta: i32) {
        let lines = self.selected_session().map_or(0, |s| {
            let events = self.history(s);
            events.get(self.history_scroll.min(events.len().saturating_sub(1))).map_or(0, |e| match &e.event.kind {
                EventKind::Prompt(t) | EventKind::Response(t) => t.lines().count(),
                EventKind::Agent { result, .. } => result.as_deref().map_or(0, |r| r.lines().count()) + 6,
                EventKind::Question { question, answer } => question.lines().count() + answer.as_deref().map_or(1, |a| a.lines().count()) + 3,
                EventKind::Skill(_) => 0,
            })
        });
        self.detail_scroll = (self.detail_scroll as i32 + delta).clamp(0, lines as i32) as u16;
    }

    fn scroll(&mut self, delta: isize) {
        let len = match (self.view, self.selected_session()) {
            (View::Agents, Some(s)) => s.subagents.len() + 1,
            (_, Some(s)) => self.history(s).len(),
            _ => 0,
        };
        let max = len.saturating_sub(1) as isize;
        self.history_scroll = (self.history_scroll as isize + delta).clamp(0, max) as usize;
        self.detail_scroll = 0;
    }

    fn refresh(&mut self) {
        self.usage.tick();
        if let Ok(notice) = self.update_rx.try_recv() {
            self.update = Some(notice);
        }
        for ev in self.log.read_new() {
            events::apply(&mut self.sessions, &ev);
        }
        if self.last_slow_refresh.elapsed() >= Duration::from_secs(1) {
            self.last_slow_refresh = Instant::now();
            self.check_processes();
            self.refresh_transcripts();
            self.refresh_snapshots();
        }
        // Every refresh, so a Stop that leaves a background shell never shows as idle.
        let now = crate::now();
        for s in self.sessions.values_mut() {
            s.reconcile(now);
        }
        let alerts = self.alerts.observe(self.sessions.values());
        if self.alerts_on {
            alerts.fire();
        }
    }

    /// Marks sessions whose Claude Code process is gone (crash, closed terminal) as ended.
    fn check_processes(&mut self) {
        mark_exited(&mut self.sessions, &mut self.sys);
    }

    fn refresh_snapshots(&mut self) {
        load_snapshots(&mut self.sessions);
    }

    fn refresh_transcripts(&mut self) {
        let ids: Vec<String> = self.visible().iter().map(|s| s.id.clone()).collect();
        for id in ids {
            let Some(session) = self.sessions.get_mut(&id) else { continue };
            let Some(path) = session.transcript_path.clone() else { continue };

            let reader = self.readers.entry(path.clone()).or_insert_with(|| TranscriptReader::new(path.clone()));
            if reader.poll() {
                session.stats = reader.stats();
                if session.model.is_none() {
                    session.model = session.stats.model.clone();
                }
            }

            let dir = transcript::subagent_dir(&path);
            for (agent_id, file) in transcript::subagent_files(&dir) {
                let agent = session.subagents.entry(agent_id.clone()).or_insert_with(Subagent::default);
                if !agent.meta_loaded
                    && let Some(meta) = transcript::subagent_meta(&dir, &agent_id) {
                        agent.meta_loaded = true;
                        if let Some(t) = meta.agent_type {
                            agent.agent_type = t;
                        }
                        agent.description = meta.description.unwrap_or_default();
                        agent.tool_use_id = meta.tool_use_id;
                    }
                if agent.agent_type.is_empty() {
                    agent.agent_type = "agent".into();
                }
                let reader = self.readers.entry(file.clone()).or_insert_with(|| TranscriptReader::subagent(file));
                if reader.poll() {
                    agent.stats = reader.stats();
                }
            }
        }
    }
}

pub fn run() -> anyhow::Result<()> {
    let mut app = App::new();
    app.refresh();
    let mut terminal = ratatui::init();
    let result = (|| -> anyhow::Result<()> {
        loop {
            terminal.draw(|f| crate::ui::draw(f, &app))?;
            if event::poll(Duration::from_millis(250))?
                && let Event::Key(key) = event::read()?
                    && key.kind == KeyEventKind::Press {
                        app.error = None;
                        match (app.view, key.code) {
                            (_, KeyCode::Char('q')) => return Ok(()),
                            (View::Usage, code) => {
                                if !app.usage.on_key(code) {
                                    app.view = View::List;
                                }
                            }
                            (View::List, KeyCode::Char('u')) => {
                                app.view = View::Usage;
                                app.usage.reload();
                            }
                            (View::List, KeyCode::Esc) => return Ok(()),
                            (View::List, KeyCode::Down | KeyCode::Char('j')) => app.move_selection(1),
                            (View::List, KeyCode::Up | KeyCode::Char('k')) => app.move_selection(-1),
                            (View::List, KeyCode::Home | KeyCode::Char('g')) => app.move_selection(isize::MIN / 2),
                            (View::List, KeyCode::End | KeyCode::Char('G')) => app.move_selection(isize::MAX / 2),
                            (View::List, KeyCode::Char('a')) => app.show_ended = !app.show_ended,
                            (View::List, KeyCode::Char('m')) => {
                                app.alerts_on = !app.alerts_on;
                                if let Err(e) = crate::settings::set_alerts(app.alerts_on) {
                                    app.error = Some(format!("couldn't save alerts setting: {e}"));
                                }
                            }
                            (View::List, KeyCode::Char('s')) => app.set_sort(app.sort.next(), false),
                            (View::List, KeyCode::Char('S')) => app.set_sort(app.sort, !app.sort_reversed),
                            (View::List, KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')) => app.open(View::History),
                            (View::History, KeyCode::Char('f')) => {
                                app.history_filter = app.history_filter.next();
                                app.history_scroll = 0;
                                app.detail_scroll = 0;
                            }
                            (View::History, KeyCode::Char('J')) => app.scroll_detail(3),
                            (View::History, KeyCode::Char('K')) => app.scroll_detail(-3),
                            (View::List | View::History, KeyCode::Tab) => app.open(View::Agents),
                            (View::Agents, KeyCode::Tab) => app.open(View::History),
                            (View::History | View::Agents, KeyCode::Esc | KeyCode::Enter | KeyCode::Left | KeyCode::Char('h')) => {
                                app.view = View::List
                            }
                            (View::History | View::Agents, KeyCode::Down | KeyCode::Char('j')) => app.scroll(1),
                            (View::History | View::Agents, KeyCode::Up | KeyCode::Char('k')) => app.scroll(-1),
                            (View::History | View::Agents, KeyCode::PageDown | KeyCode::Char(' ')) => app.scroll(10),
                            (View::History | View::Agents, KeyCode::PageUp) => app.scroll(-10),
                            (View::History | View::Agents, KeyCode::Home | KeyCode::Char('g')) => app.scroll(isize::MIN / 2),
                            (View::History | View::Agents, KeyCode::End | KeyCode::Char('G')) => app.scroll(isize::MAX / 2),
                            _ => {}
                        }
                    }
            app.tick = app.tick.wrapping_add(1);
            app.refresh();
        }
    })();
    ratatui::restore();
    result
}
