use crate::events;
use crate::jsonl::JsonlTail;
use crate::model::{Session, Status, Subagent};
use crate::transcript::{self, TranscriptReader};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

/// Ended sessions stay visible this long unless `a` is toggled.
const ENDED_GRACE_SECS: f64 = 10.0 * 60.0;
/// Sessions with no known process and no events for this long are treated as gone.
const STALE_SECS: f64 = 12.0 * 3600.0;

pub struct App {
    pub sessions: HashMap<String, Session>,
    pub selected: Option<String>,
    pub show_ended: bool,
    pub tick: u64,
    pub hooks_installed: bool,
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
            tick: 0,
            hooks_installed,
            log: JsonlTail::new(dir.join("events.jsonl")),
            readers: HashMap::new(),
            sys: System::new(),
            last_slow_refresh: Instant::now() - Duration::from_secs(60),
        }
    }

    /// Sessions to show, most urgent first.
    pub fn visible(&self) -> Vec<&Session> {
        let now = crate::now();
        let mut list: Vec<&Session> = self
            .sessions
            .values()
            .filter(|s| self.show_ended || s.status != Status::Ended || now - s.status_since < ENDED_GRACE_SECS)
            .collect();
        list.sort_by(|a, b| {
            a.status.cmp(&b.status).then(b.last_event.total_cmp(&a.last_event))
        });
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

    fn move_selection(&mut self, delta: isize) {
        let ids: Vec<String> = self.visible().iter().map(|s| s.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let cur = self.selected_index().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, ids.len() as isize - 1) as usize;
        self.selected = Some(ids[next].clone());
    }

    fn refresh(&mut self) {
        for ev in self.log.read_new() {
            events::apply(&mut self.sessions, &ev);
        }
        if self.last_slow_refresh.elapsed() >= Duration::from_secs(1) {
            self.last_slow_refresh = Instant::now();
            self.check_processes();
            self.refresh_transcripts();
        }
    }

    /// Marks sessions whose Claude Code process is gone (crash, closed terminal) as ended.
    fn check_processes(&mut self) {
        let now = crate::now();
        let mut pids: Vec<Pid> = self
            .sessions
            .values()
            .filter(|s| s.status != Status::Ended)
            .filter_map(|s| s.pid.map(Pid::from_u32))
            .collect();
        pids.sort();
        pids.dedup();
        self.sys.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, ProcessRefreshKind::nothing());
        for s in self.sessions.values_mut().filter(|s| s.status != Status::Ended) {
            let gone = match s.pid {
                Some(pid) => self.sys.process(Pid::from_u32(pid)).is_none(),
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
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let file = entry.path();
                let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
                let Some(agent_id) = name.strip_prefix("agent-").and_then(|n| n.strip_suffix(".jsonl")) else {
                    continue;
                };
                let agent = session.subagents.entry(agent_id.to_string()).or_insert_with(|| Subagent {
                    agent_type: String::new(),
                    running: false,
                    stats: Default::default(),
                });
                if agent.agent_type.is_empty() || agent.agent_type == "agent" {
                    agent.agent_type = transcript::subagent_type(&dir, agent_id).unwrap_or_else(|| "agent".into());
                }
                let reader = self.readers.entry(file.clone()).or_insert_with(|| TranscriptReader::new(file));
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
            if event::poll(Duration::from_millis(250))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                            KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
                            KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
                            KeyCode::Home | KeyCode::Char('g') => app.move_selection(isize::MIN / 2),
                            KeyCode::End | KeyCode::Char('G') => app.move_selection(isize::MAX / 2),
                            KeyCode::Char('a') => app.show_ended = !app.show_ended,
                            _ => {}
                        }
                    }
                }
            }
            app.tick = app.tick.wrapping_add(1);
            app.refresh();
        }
    })();
    ratatui::restore();
    result
}
