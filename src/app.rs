mod badge_state;

pub use badge_state::{Announcement, BadgeState};
use crate::events;
use crate::jsonl::JsonlTail;
use crate::model::{EventKind, RateWindow, Session, SortKey, Status, Subagent, TimelineEntry};
use crate::share_view::{Action, ShareView};
use crate::transcript::{self, TranscriptReader};
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind};
use ratatui::crossterm::execute;
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
    /// The shared AGENT-BOARD.md of the selected session's repo.
    Board,
    /// The rank, the achievements and the progress to each next level.
    Badges,
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
    /// A success to show in the footer; the next key clears it.
    pub info: Option<String>,
    /// The launch command to copy by hand when the clipboard couldn't be reached; the next key clears it.
    pub copy_note: Option<CopyNote>,
    pub board: BoardState,
    pub badges: BadgeState,
    /// A rank-up or new achievement level for the footer.
    pub announcement: Announcement,
    /// The share flow, drawn over the list or the badge view; it gets every key first.
    pub share: Option<ShareView>,
    alerts: crate::alerts::Detector,
    update_rx: std::sync::mpsc::Receiver<crate::update::Notice>,
    log: JsonlTail,
    readers: HashMap<PathBuf, TranscriptReader>,
    sys: System,
    last_slow_refresh: Instant,
    pub counter: Counter,
}

/// Shown above the footer when `c` couldn't put the launch command on a clipboard it can confirm.
pub struct CopyNote {
    pub headline: &'static str,
    pub command: String,
    pub path: String,
}

/// The board view's state: which board, its last good parse, and when the file was read.
#[derive(Default)]
pub struct BoardState {
    /// The session whose repo the board belongs to.
    pub session: String,
    /// The cwd the path was resolved from, and when.
    cwd: String,
    resolved: Option<Instant>,
    pub path: Option<PathBuf>,
    /// Modified time and length of the file last parsed, so it is re-read only when it changes.
    key: Option<(std::time::SystemTime, u64)>,
    pub board: Option<crate::board::Board>,
    pub truncated: bool,
    /// No board file exists yet.
    pub missing: bool,
    /// The last read failed; `board` still holds the last good parse, if any.
    pub read_error: Option<String>,
    pub scroll: usize,
    /// The furthest the view can scroll, set while drawing.
    pub max_scroll: std::cell::Cell<usize>,
}

/// How long a resolved board path is reused before git is asked again (a repo may be created meanwhile).
const BOARD_RESOLVE: Duration = Duration::from_secs(30);

impl BoardState {
    /// Re-resolves the board path when the session moved or the cached one is old, then re-reads
    /// the file if it changed.
    fn refresh(&mut self, cwd: &str) -> Result<(), &'static str> {
        if self.path.is_none() || self.cwd != cwd || self.resolved.is_none_or(|t| t.elapsed() >= BOARD_RESOLVE) {
            self.cwd = cwd.to_string();
            self.resolved = Some(Instant::now());
            let path = match crate::board::resolve(cwd) {
                crate::board::Root::Board(path) => path,
                crate::board::Root::NoCwd => return Err("this session has no working directory yet"),
                crate::board::Root::Gone => return Err("this session's directory no longer exists"),
            };
            if self.path.as_ref() != Some(&path) {
                self.path = Some(path);
                self.key = None;
                self.board = None;
                self.scroll = 0;
            }
        }
        self.reload();
        Ok(())
    }

    fn reload(&mut self) {
        let Some(path) = &self.path else { return };
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (self.missing, self.board, self.key, self.read_error) = (true, None, None, None);
                return;
            }
            Err(e) => {
                self.read_error = Some(e.to_string());
                return;
            }
        };
        self.missing = false;
        let key = (meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.len());
        if self.key == Some(key) && self.read_error.is_none() {
            return;
        }
        // Agents edit the board in place, so a read can land mid-edit; keep the last good parse then.
        match crate::board::read(path) {
            Ok((text, truncated)) => {
                self.board = Some(crate::board::parse(&text));
                self.truncated = truncated;
                self.key = Some(key);
                self.read_error = None;
            }
            Err(e) => self.read_error = Some(e.to_string()),
        }
    }

    fn scroll(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta).min(self.max_scroll.get());
    }
}

/// What the run loop does after a key.
#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Stay,
    Quit,
    /// Suspend the terminal and edit the post in `$EDITOR`.
    EditPost(String),
}

/// Indexed (session, agent) key -> tokens.
type DayTotals = HashMap<(String, String), i64>;

/// Today's and all-time tokens for the header: the indexed counter, topped up with live usage
/// the background sync hasn't indexed yet.
#[derive(Default)]
pub struct Counter {
    conn: Option<rusqlite::Connection>,
    loaded: Option<Instant>,
    /// Local day the indexed figures below are for.
    day: String,
    indexed: DayTotals,
    all_time: i64,
    /// Live readers' tokens per (session, agent), with the day they were counted for.
    live: HashMap<(String, String), (String, i64)>,
}

/// How often the header re-reads the indexed counter.
const COUNTER_RELOAD: Duration = Duration::from_secs(10);

impl Counter {
    fn reload_if_due(&mut self) {
        let today = crate::store::day_n_ago(1);
        if self.loaded.is_some_and(|t| t.elapsed() < COUNTER_RELOAD) && self.day == today {
            return;
        }
        self.loaded = Some(Instant::now());
        if self.conn.is_none() {
            self.conn = crate::store::open_readonly().ok();
        }
        let Some(conn) = &self.conn else { return };
        if let (Ok(rows), Ok(all)) = (crate::store::day_rows(conn, &today), crate::store::lifetime(conn)) {
            self.indexed = rows;
            self.all_time = all.total;
            self.day = today;
        }
    }

    fn set_live(&mut self, session: &str, agent: &str, reader: &TranscriptReader) {
        let today = crate::store::day_n_ago(1);
        let tokens = reader.daily().get(&today).map_or(0, |d| d.tokens.total() as i64);
        self.live.insert((session.to_string(), agent.to_string()), (today, tokens));
    }

    /// (today, all time).
    pub fn totals(&self) -> (i64, i64) {
        merge_live(&self.indexed, self.all_time, &self.day, &self.live)
    }
}

/// Adds live usage that is ahead of the index, per (session, agent), to both totals.
fn merge_live(indexed: &DayTotals, all_time: i64, day: &str, live: &HashMap<(String, String), (String, i64)>) -> (i64, i64) {
    let extra: i64 = live
        .iter()
        .filter(|(_, (d, _))| d == day)
        .map(|(key, (_, tokens))| (tokens - indexed.get(key).copied().unwrap_or(0)).max(0))
        .sum();
    (indexed.values().sum::<i64>() + extra, all_time + extra)
}

impl App {
    fn new() -> Self {
        let dir = crate::data_dir();
        // `marsad update` only swaps the binary, so refresh the rules this build ships.
        let _ = crate::coordination::ensure_protocol();
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
            info: None,
            copy_note: None,
            board: BoardState::default(),
            badges: BadgeState::default(),
            announcement: Announcement::default(),
            share: None,
            alerts: crate::alerts::Detector::default(),
            update_rx: crate::update::background(),
            log: JsonlTail::new(dir.join("events.jsonl")),
            readers: HashMap::new(),
            sys: System::new(),
            last_slow_refresh: Instant::now() - Duration::from_secs(60),
            counter: Counter::default(),
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

    /// Opens the board of the selected session's repo, or says why there is none.
    fn open_board(&mut self) {
        let Some((id, cwd)) = self.selected_session().map(|s| (s.id.clone(), s.cwd.clone())) else { return };
        self.selected = Some(id.clone());
        let mut board = BoardState { session: id, ..BoardState::default() };
        match board.refresh(&cwd) {
            Ok(()) => {
                self.board = board;
                self.view = View::Board;
            }
            Err(why) => self.error = Some(why.into()),
        }
    }

    fn refresh_board(&mut self) {
        // A session that moved somewhere without a board keeps showing the last one.
        if let Some(cwd) = self.sessions.get(&self.board.session).map(|s| s.cwd.clone()) {
            let _ = self.board.refresh(&cwd);
        }
    }

    /// Copies the command that starts a session with the coordination rules, refreshing the rules first.
    fn copy_launch_command(&mut self) {
        let path = match crate::coordination::ensure_protocol() {
            Ok(path) => path,
            Err(e) => {
                self.error = Some(format!("couldn't write the coordination rules: {e:#}"));
                return;
            }
        };
        let command = crate::coordination::launch_command(&path);
        let headline = match crate::clipboard::copy(&command) {
            crate::clipboard::Copied::Native(_) => {
                self.info = Some("copied the coordination command · it applies to new conversations ".into());
                return;
            }
            crate::clipboard::Copied::Terminal => "Sent to your terminal's clipboard, if it allows that. To copy it by hand:",
            crate::clipboard::Copied::Failed => "Couldn't reach a clipboard. Copy this command by hand:",
        };
        let path = crate::coordination::quote_arg(&path.to_string_lossy());
        self.copy_note = Some(CopyNote { headline, command, path });
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
        if self.usage.tick() {
            self.after_sync();
        }
        if self.badges.due {
            self.badges.due = false;
            self.load_badges();
        }
        // The clipboard runs here, on the main thread, between frames: OSC 52 writes to the terminal.
        if let Some(share) = &mut self.share
            && let Some(text) = share.tick()
        {
            share.copied(crate::clipboard::copy(&text));
        }
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
            self.counter.reload_if_due();
            if self.view == View::Board {
                self.refresh_board();
            }
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

            // Keyed like the indexer: the transcript's file name and the agent id.
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let reader = self.readers.entry(path.clone()).or_insert_with(|| TranscriptReader::new(path.clone()));
            if reader.poll() {
                self.counter.set_live(&stem, crate::model::MAIN_AGENT, reader);
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
                    self.counter.set_live(&stem, &agent_id, reader);
                    agent.stats = reader.stats();
                }
            }
        }
    }
}

impl App {
    /// Handles one key press. The share view takes every key first, so `q` there cancels the share
    /// rather than quitting; a drawn announcement takes Esc next; then the footer notes clear.
    pub fn on_key(&mut self, code: KeyCode) -> Flow {
        if let Some(share) = &mut self.share {
            match share.on_key(code) {
                Action::Close => self.share = None,
                Action::EditPost(text) => return Flow::EditPost(text),
                Action::None => {}
            }
            return Flow::Stay;
        }
        if code == KeyCode::Esc && self.announcement.dismiss(self.view, false) {
            return Flow::Stay;
        }
        (self.error, self.info, self.copy_note) = (None, None, None);
        match (self.view, code) {
            (_, KeyCode::Char('q')) => return Flow::Quit,
            (View::Usage, code) => {
                if !self.usage.on_key(code) {
                    self.view = View::List;
                }
            }
            (View::List, KeyCode::Char('u')) => {
                self.view = View::Usage;
                self.usage.reload();
            }
            (View::List, KeyCode::Esc) => return Flow::Quit,
            (View::List, KeyCode::Down | KeyCode::Char('j')) => self.move_selection(1),
            (View::List, KeyCode::Up | KeyCode::Char('k')) => self.move_selection(-1),
            (View::List, KeyCode::Home | KeyCode::Char('g')) => self.move_selection(isize::MIN / 2),
            (View::List, KeyCode::End | KeyCode::Char('G')) => self.move_selection(isize::MAX / 2),
            (View::List, KeyCode::Char('a')) => self.show_ended = !self.show_ended,
            (View::List, KeyCode::Char('m')) => {
                self.alerts_on = !self.alerts_on;
                if let Err(e) = crate::settings::set_alerts(self.alerts_on) {
                    self.error = Some(format!("couldn't save alerts setting: {e}"));
                }
            }
            (View::List | View::History | View::Agents, KeyCode::Char('b')) => self.open_board(),
            (View::List, KeyCode::Char('B')) => {
                self.view = View::Badges;
                self.badges.due = true;
            }
            (View::Badges, KeyCode::Esc | KeyCode::Char('B') | KeyCode::Left | KeyCode::Char('h')) => self.view = View::List,
            (View::List | View::Badges, KeyCode::Char('p')) => self.start_share(),
            (View::List | View::Board, KeyCode::Char('c')) => self.copy_launch_command(),
            (View::Board, KeyCode::Esc | KeyCode::Char('b') | KeyCode::Left | KeyCode::Char('h')) => self.view = View::List,
            (View::Board, KeyCode::Down | KeyCode::Char('j')) => self.board.scroll(1),
            (View::Board, KeyCode::Up | KeyCode::Char('k')) => self.board.scroll(-1),
            (View::Board, KeyCode::PageDown | KeyCode::Char(' ')) => self.board.scroll(10),
            (View::Board, KeyCode::PageUp) => self.board.scroll(-10),
            (View::Board, KeyCode::Home | KeyCode::Char('g')) => self.board.scroll(isize::MIN / 2),
            (View::Board, KeyCode::End | KeyCode::Char('G')) => self.board.scroll(isize::MAX / 2),
            (View::List, KeyCode::Char('s')) => self.set_sort(self.sort.next(), false),
            (View::List, KeyCode::Char('S')) => self.set_sort(self.sort, !self.sort_reversed),
            (View::List, KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')) => self.open(View::History),
            (View::History, KeyCode::Char('f')) => {
                self.history_filter = self.history_filter.next();
                self.history_scroll = 0;
                self.detail_scroll = 0;
            }
            (View::History, KeyCode::Char('J')) => self.scroll_detail(3),
            (View::History, KeyCode::Char('K')) => self.scroll_detail(-3),
            (View::List | View::History, KeyCode::Tab) => self.open(View::Agents),
            (View::Agents, KeyCode::Tab) => self.open(View::History),
            (View::History | View::Agents, KeyCode::Esc | KeyCode::Enter | KeyCode::Left | KeyCode::Char('h')) => {
                self.view = View::List
            }
            (View::History | View::Agents, KeyCode::Down | KeyCode::Char('j')) => self.scroll(1),
            (View::History | View::Agents, KeyCode::Up | KeyCode::Char('k')) => self.scroll(-1),
            (View::History | View::Agents, KeyCode::PageDown | KeyCode::Char(' ')) => self.scroll(10),
            (View::History | View::Agents, KeyCode::PageUp) => self.scroll(-10),
            (View::History | View::Agents, KeyCode::Home | KeyCode::Char('g')) => self.scroll(isize::MIN / 2),
            (View::History | View::Agents, KeyCode::End | KeyCode::Char('G')) => self.scroll(isize::MAX / 2),
            _ => {}
        }
        Flow::Stay
    }
}

pub fn run() -> anyhow::Result<()> {
    let mut app = App::new();
    app.refresh();
    let mut terminal = ratatui::init();
    // A pasted newline must not end the line being edited.
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let result = (|| -> anyhow::Result<()> {
        loop {
            terminal.draw(|f| crate::ui::draw(f, &app))?;
            app.after_draw();
            if event::poll(Duration::from_millis(250))? {
                match event::read()? {
                    Event::Paste(text) => {
                        if let Some(share) = &mut app.share {
                            share.on_paste(&text);
                        }
                    }
                    Event::Key(key) if key.kind == KeyEventKind::Press => match app.on_key(key.code) {
                        Flow::Quit => return Ok(()),
                        Flow::EditPost(text) => {
                            let _ = execute!(std::io::stdout(), DisableBracketedPaste);
                            ratatui::restore();
                            let edited = crate::share_view::edit_with(&crate::share_view::editor(), &text);
                            terminal = ratatui::init();
                            let _ = execute!(std::io::stdout(), EnableBracketedPaste);
                            terminal.clear()?;
                            if let Some(share) = &mut app.share {
                                share.edited(edited);
                            }
                        }
                        Flow::Stay => {}
                    },
                    _ => {}
                }
            }
            app.tick = app.tick.wrapping_add(1);
            app.refresh();
        }
    })();
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str, a: &str) -> (String, String) {
        (s.to_string(), a.to_string())
    }

    #[test]
    fn live_usage_tops_up_the_indexed_counter() {
        let day = "2026-10-23";
        let indexed: DayTotals = HashMap::from([(key("s1", "main"), 1_000), (key("s2", "main"), 500)]);
        let live = HashMap::from([
            // Covers AE3: ahead of the index by 200.
            (key("s1", "main"), (day.to_string(), 1_200)),
            // Not indexed yet at all.
            (key("s3", "a1"), (day.to_string(), 50)),
            // Behind the index: the index wins.
            (key("s2", "main"), (day.to_string(), 400)),
            // Yesterday's figure from a reader that hasn't polled since midnight.
            (key("s4", "main"), ("2026-10-22".to_string(), 999)),
        ]);
        assert_eq!(merge_live(&indexed, 10_000, day, &live), (1_500 + 250, 10_000 + 250));
        assert_eq!(merge_live(&DayTotals::new(), 0, day, &HashMap::new()), (0, 0));
    }

    const ONE_AGENT: &str = "## Agents\n| agent | task | branch/worktree | status | started | last seen |\n|---|---|---|---|---|---|\n| agent-a | fix | a | active | 2026-10-23 10:00 | 2026-10-23 10:00 |\n";

    fn agents(state: &BoardState) -> Vec<String> {
        state.board.as_ref().map(|b| b.agents.iter().map(|a| a.name.clone()).collect()).unwrap_or_default()
    }

    #[test]
    fn the_board_view_follows_the_file() {
        let root = std::env::temp_dir().join(format!("marsad-app-board-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cwd = root.to_str().unwrap();
        let mut state = BoardState::default();

        state.refresh(cwd).unwrap();
        let path = state.path.clone().unwrap();
        assert!(state.missing && state.board.is_none(), "no file yet");

        std::fs::write(&path, ONE_AGENT).unwrap();
        state.refresh(cwd).unwrap();
        assert!(!state.missing);
        assert_eq!(agents(&state), ["agent-a"]);

        std::fs::write(&path, format!("{ONE_AGENT}| agent-bb | docs | b | active | 2026-10-23 10:00 | 2026-10-23 10:00 |\n")).unwrap();
        state.refresh(cwd).unwrap();
        assert_eq!(agents(&state), ["agent-a", "agent-bb"], "a changed file is re-read");

        // Same modified time and length: the parse is reused, not re-read.
        state.board = None;
        state.refresh(cwd).unwrap();
        assert!(state.board.is_none());
        state.key = None;
        state.refresh(cwd).unwrap();
        assert_eq!(agents(&state).len(), 2);

        // A failed read keeps the last good parse.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        state.refresh(cwd).unwrap();
        assert!(state.read_error.is_some());
        assert_eq!(agents(&state).len(), 2);

        std::fs::remove_dir(&path).unwrap();
        state.refresh(cwd).unwrap();
        assert!(state.missing && state.board.is_none() && state.read_error.is_none(), "deleted again");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn the_board_view_resets_when_the_session_moves() {
        let root = std::env::temp_dir().join(format!("marsad-app-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join(crate::board::FILE), ONE_AGENT).unwrap();
        let mut state = BoardState::default();
        state.refresh(a.to_str().unwrap()).unwrap();
        state.max_scroll.set(50);
        state.scroll(10);
        assert_eq!((state.scroll, agents(&state).len()), (10, 1));
        state.refresh(b.to_str().unwrap()).unwrap();
        assert_eq!(state.scroll, 0);
        assert!(state.missing && state.board.is_none());

        assert_eq!(state.refresh(""), Err("this session has no working directory yet"));
        assert_eq!(state.refresh(root.join("gone").to_str().unwrap()), Err("this session's directory no longer exists"));
        std::fs::remove_dir_all(root).ok();
    }

    /// An app that touches nothing on disk: no event log, settings, database or update check.
    fn blank() -> App {
        App {
            sessions: HashMap::new(),
            selected: None,
            show_ended: false,
            sort: SortKey::default(),
            sort_reversed: false,
            view: View::List,
            history_scroll: 0,
            history_filter: HistoryFilter::default(),
            detail_scroll: 0,
            compact_window: None,
            tick: 0,
            hooks_installed: true,
            usage: crate::usage::Usage::new(),
            update: None,
            alerts_on: false,
            error: None,
            info: None,
            copy_note: None,
            board: BoardState::default(),
            badges: BadgeState::default(),
            announcement: Announcement::default(),
            share: None,
            alerts: crate::alerts::Detector::default(),
            update_rx: std::sync::mpsc::channel().1,
            log: JsonlTail::new(PathBuf::new()),
            readers: HashMap::new(),
            sys: System::new(),
            last_slow_refresh: Instant::now(),
            counter: Counter::default(),
        }
    }

    fn levels(rank: i64) -> crate::badges::Badges {
        let mut stats = crate::badges::Stats::default();
        stats.0.insert(crate::badges::RANK_KEY, rank);
        crate::badges::badges(&stats)
    }

    #[test]
    fn b_opens_the_badge_view_and_esc_returns() {
        let mut app = blank();
        assert_eq!(app.on_key(KeyCode::Char('B')), Flow::Stay);
        assert!(app.view == View::Badges && app.badges.due, "the badges load before the next frame, not on the key");
        assert_eq!(app.on_key(KeyCode::Esc), Flow::Stay);
        assert!(app.view == View::List);
        assert_eq!(app.on_key(KeyCode::Char('q')), Flow::Quit, "q in the list still quits");
    }

    #[test]
    fn q_in_the_share_view_cancels_the_share_and_leaves_marsad_running() {
        let mut app = blank();
        let dir = std::env::temp_dir().join(format!("marsad-app-share-{}", std::process::id()));
        app.share = Some(ShareView::new(crate::share_view::fake_deps(dir.join("c.png"))));
        let start = Instant::now();
        while app.share.as_ref().unwrap().stage != crate::share_view::Stage::Preview {
            app.share.as_mut().unwrap().tick();
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.on_key(KeyCode::Char('e')), Flow::Stay);
        assert_eq!(app.on_key(KeyCode::Char('q')), Flow::Stay, "q types into the line");
        assert!(app.share.is_some());
        assert_eq!(app.on_key(KeyCode::Esc), Flow::Stay, "Esc leaves the line editor");
        assert_eq!(app.on_key(KeyCode::Char('q')), Flow::Stay, "q cancels the share");
        assert!(app.share.is_none() && app.view == View::List);
        assert!(!dir.exists(), "nothing written");
    }

    #[test]
    fn esc_dismisses_a_drawn_announcement_before_it_quits() {
        let mut app = blank();
        app.announcement.offer("new: bronze streak".into(), levels(0));
        assert_eq!(app.on_key(KeyCode::Char('j')), Flow::Stay);
        assert!(app.announcement.text(View::List, false).is_some(), "a key press doesn't clear it");
        // Not drawn yet: Esc is the list's own Esc.
        let mut undrawn = blank();
        undrawn.announcement.offer("new: bronze streak".into(), levels(0));
        assert_eq!(undrawn.on_key(KeyCode::Esc), Flow::Quit);
        app.announcement.drawn(Instant::now(), View::List, false);
        assert_eq!(app.on_key(KeyCode::Esc), Flow::Stay);
        assert_eq!(app.announcement.text(View::List, false), None);
        assert_eq!(app.on_key(KeyCode::Esc), Flow::Quit);
    }

    #[test]
    fn board_scrolling_is_clamped() {
        let mut state = BoardState::default();
        state.max_scroll.set(7);
        state.scroll(isize::MAX / 2);
        assert_eq!(state.scroll, 7);
        state.scroll(isize::MIN / 2);
        assert_eq!(state.scroll, 0);
    }
}
