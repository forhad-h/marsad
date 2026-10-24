use std::collections::{BTreeMap, HashMap};
use std::ops::AddAssign;
use std::path::PathBuf;

/// Ordered by urgency: sessions that need the user sort first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Approve,
    Question,
    Error,
    Active,
    Idle,
    Ended,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Approve => "approve",
            Status::Question => "question",
            Status::Error => "error",
            Status::Active => "active",
            Status::Idle => "idle",
            Status::Ended => "ended",
        }
    }

    pub fn needs_user(self) -> bool {
        matches!(self, Status::Approve | Status::Question)
    }
}

/// Why a session last went idle; only a finished turn plays the done sound.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IdleCause {
    /// Session start or an idle notification.
    #[default]
    Other,
    /// Claude finished its turn (Stop) with no background shell left running.
    Finished,
    /// The user pressed Esc or interrupted the turn, which fires no hook.
    Interrupted,
}

/// How long a turn end with no later hook waits for a late Stop before it counts as an interrupt.
const INTERRUPT_GRACE_SECS: f64 = 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

impl AddAssign for TokenUsage {
    fn add_assign(&mut self, o: Self) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }
}

#[derive(Clone, Debug, Default)]
pub struct SkillStat {
    pub calls: u32,
    /// Tokens of assistant turns that ran inside this skill's window (approximate).
    pub tokens: TokenUsage,
}

/// Agent id/type used for a session's main agent (as opposed to a subagent).
pub const MAIN_AGENT: &str = "main";

/// Date format for local calendar days, e.g. `2026-10-09`.
pub const DAY_FMT: &str = "%Y-%m-%d";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// Claude invoked the Skill tool.
    Tool,
    /// The user typed `/skill`.
    Command,
}

impl Via {
    /// Inverse of `as_str`; anything unrecognized is treated as a tool call.
    pub fn parse(s: &str) -> Self {
        if s == "command" { Via::Command } else { Via::Tool }
    }

    /// Stored form in the usage database.
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Tool => "tool",
            Via::Command => "command",
        }
    }
}

/// One skill invocation and the tokens of the turns that ran inside its window.
#[derive(Clone, Debug)]
pub struct SkillRun {
    pub name: String,
    /// ISO-8601 UTC timestamp from the transcript.
    pub at: Option<String>,
    pub via: Via,
    pub tokens: TokenUsage,
    pub turns: u32,
}

/// What happened at one point of a session, for the history timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    /// Something the user typed: a prompt, a `/command` or a `!` shell command.
    Prompt(String),
    /// A skill invocation; index into `TranscriptStats::runs`.
    Skill(usize),
    /// A subagent launch, with its final report once it has returned.
    Agent { agent_type: String, description: String, tool_use_id: String, result: Option<String> },
    /// One AskUserQuestion question, with the user's answer once given.
    Question { question: String, answer: Option<String> },
    /// Claude's reply that ended a turn.
    Response(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// ISO-8601 UTC timestamp from the transcript.
    pub at: Option<String>,
    pub kind: EventKind,
    /// Tokens of the turn a prompt started; zero for other events.
    pub tokens: TokenUsage,
}

/// Token and skill figures derived from one transcript file.
#[derive(Clone, Debug, Default)]
pub struct TranscriptStats {
    pub tokens: TokenUsage,
    pub skills: BTreeMap<String, SkillStat>,
    /// Skill invocations in transcript order.
    pub runs: Vec<SkillRun>,
    /// Prompts, skills, subagents, questions and replies in transcript order.
    pub timeline: Vec<Event>,
    /// Agent/Task tool_use id -> skill whose window launched it (None outside any skill).
    pub launches: HashMap<String, Option<String>>,
    /// Prompt size of the latest API call (input + cache read + cache write).
    pub context_used: Option<u64>,
    /// Unix time of the API call `context_used` came from.
    pub context_at: Option<f64>,
    pub model: Option<String>,
    pub title: Option<String>,
    /// ISO-8601 timestamp of the first transcript entry.
    pub started: Option<String>,
    /// Seconds of finished work, idle time between turns excluded.
    pub busy_secs: f64,
    /// Unix time the turn in progress started, if one is open.
    pub turn_started: Option<f64>,
    /// Unix time the latest turn ended, interrupted or not.
    pub turn_ended_at: Option<f64>,
    /// Background shells still running, oldest first, as (unix launch time, command).
    pub background: Vec<(f64, String)>,
}

#[derive(Clone, Copy, Debug)]
pub struct RateWindow {
    pub used_pct: f64,
    pub resets_at: Option<f64>,
}

/// What the status line last reported for a session (exact context window and rate limits).
#[derive(Clone, Debug, Default)]
pub struct StatusSnapshot {
    pub ts: f64,
    pub window: Option<u64>,
    pub used: Option<u64>,
    pub model: Option<String>,
    pub five_hour: Option<RateWindow>,
    pub seven_day: Option<RateWindow>,
}

#[derive(Clone, Copy, Debug)]
pub struct ContextInfo {
    pub used: u64,
    pub window: u64,
    /// Token count at which auto-compact triggers.
    pub limit: u64,
    /// True when the window size is guessed from the model id rather than reported.
    pub estimated: bool,
}

impl ContextInfo {
    pub fn left(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }

    pub fn used_ratio(&self) -> f64 {
        if self.limit == 0 { 0.0 } else { self.used as f64 / self.limit as f64 }
    }
}

/// How full a gauge is: green below 60%, yellow below 85%, red above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Critical,
}

impl Level {
    pub fn of(ratio: f64) -> Self {
        match ratio {
            r if r >= 0.85 => Level::Critical,
            r if r >= 0.6 => Level::Warn,
            _ => Level::Ok,
        }
    }
}

/// Compact token count: `950`, `12.3k`, `1.2M`.
pub fn fmt_tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// Local wall-clock time a rate-limit window resets: `16:30` today,
/// `Tue 09:00` within the week, `Oct 21 09:00` beyond.
pub fn fmt_reset(ts: f64) -> String {
    use chrono::{Local, TimeZone};
    let Some(t) = Local.timestamp_opt(ts as i64, 0).single() else { return "—".into() };
    let days = (t.date_naive() - Local::now().date_naive()).num_days();
    match days {
        ..=0 => t.format("%H:%M"),
        1..7 => t.format("%a %H:%M"),
        _ => t.format("%b %d %H:%M"),
    }
    .to_string()
}

/// The user's auto-compact window, from `CLAUDE_CODE_AUTO_COMPACT_WINDOW` or the
/// `autoCompactWindow` setting. Accepts `200000`, `500k`, `1M` or a bare `200` (thousands).
pub fn auto_compact_window() -> Option<u64> {
    let parse = |v: &str| -> Option<u64> {
        let v = v.trim().to_ascii_lowercase();
        let n = if let Some(k) = v.strip_suffix('k') {
            k.parse::<f64>().ok()? * 1e3
        } else if let Some(m) = v.strip_suffix('m') {
            m.parse::<f64>().ok()? * 1e6
        } else {
            let n: f64 = v.parse().ok()?;
            if n <= 1000.0 { n * 1e3 } else { n }
        };
        Some((n as u64).clamp(100_000, 1_000_000))
    };
    if let Some(v) = std::env::var("CLAUDE_CODE_AUTO_COMPACT_WINDOW").ok().and_then(|v| parse(&v)) {
        return Some(v);
    }
    let text = std::fs::read_to_string(crate::claude_dir().join("settings.json")).ok()?;
    let settings: serde_json::Value = serde_json::from_str(&text).ok()?;
    match &settings["autoCompactWindow"] {
        serde_json::Value::Number(n) => n.as_u64().and_then(|n| parse(&n.to_string())),
        serde_json::Value::String(s) => parse(s),
        _ => None,
    }
}

/// Context fill relative to the auto-compact point, given the prompt size of the latest
/// API call. `reported_window` is the exact size from the status line, when known.
pub fn context_info(used: u64, model: Option<&str>, reported_window: Option<u64>, compact_window: Option<u64>) -> ContextInfo {
    let window = reported_window.unwrap_or_else(|| model_window(model.unwrap_or(""), used));
    let limit = match compact_window {
        Some(w) => w.min(window),
        None if window >= 1_000_000 => NATIVE_1M_COMPACT_AT,
        None => window,
    };
    ContextInfo { used, window, limit, estimated: reported_window.is_none() }
}

/// Native 1M windows compact early, at about this many tokens.
const NATIVE_1M_COMPACT_AT: u64 = 967_000;

/// Context window size implied by a model id, per the Claude Code model docs.
pub fn model_window(model: &str, used: u64) -> u64 {
    let m = model.to_ascii_lowercase();
    let version = |family: &str| -> Option<(u32, u32)> {
        let rest = &m[m.find(family)? + family.len()..];
        let mut nums = rest.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty() && p.len() <= 2);
        Some((nums.next()?.parse().ok()?, nums.next().and_then(|n| n.parse().ok()).unwrap_or(0)))
    };
    let native_1m = m.contains("[1m]")
        || m.contains("fable")
        || version("sonnet-").is_some_and(|(maj, _)| maj >= 5)
        || version("haiku-").is_some_and(|(maj, _)| maj >= 5)
        || version("opus-").is_some_and(|(maj, min)| maj >= 5 || (maj == 4 && min >= 7));
    if native_1m || used > 200_000 { 1_000_000 } else { 200_000 }
}

#[derive(Clone, Debug, Default)]
pub struct Subagent {
    pub agent_type: String,
    /// The task description it was launched with.
    pub description: String,
    /// Id of the Agent tool call that launched it.
    pub tool_use_id: Option<String>,
    /// Whether its `.meta.json` has been read; it can appear after the transcript does.
    pub meta_loaded: bool,
    pub running: bool,
    pub stats: TranscriptStats,
}

impl Subagent {
    /// Context fill relative to the auto-compact point, from its own transcript.
    pub fn context(&self, compact_window: Option<u64>) -> Option<ContextInfo> {
        let used = self.stats.context_used?;
        Some(context_info(used, self.stats.model.as_deref(), None, compact_window))
    }
}

/// One event of a session's history, with the agent and transcript it came from.
pub struct TimelineEntry<'a> {
    pub agent: &'a str,
    /// The agent's transcript figures, which a `Skill` event indexes into.
    pub stats: &'a TranscriptStats,
    pub event: &'a Event,
}

impl TimelineEntry<'_> {
    pub fn skill_run(&self) -> Option<&SkillRun> {
        match self.event.kind {
            EventKind::Skill(i) => self.stats.runs.get(i),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub transcript_path: Option<PathBuf>,
    pub status: Status,
    pub status_since: f64,
    /// When marsad first saw the session; a stable tie-breaker for sorting.
    pub started: f64,
    pub last_event: f64,
    /// Time of the latest hook event from the main agent (not a subagent).
    pub last_hook: f64,
    pub idle_cause: IdleCause,
    /// True after the main agent's Stop or SessionStart until its next hook: the turn closed with a
    /// hook, so a transcript turn end landing just after it is not an interrupt.
    pub stopped: bool,
    /// Time of the latest SessionStart from a new process (not a compaction); background shells
    /// launched before it belonged to an earlier process and never report finishing.
    pub run_started: f64,
    /// What the session is doing right now, e.g. "Bash: cargo test".
    pub detail: String,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub title: Option<String>,
    pub pid: Option<u32>,
    pub subagents: BTreeMap<String, Subagent>,
    pub stats: TranscriptStats,
    pub snapshot: Option<StatusSnapshot>,
}

impl Session {
    pub fn new(id: &str, ts: f64) -> Self {
        Self {
            id: id.to_string(),
            cwd: String::new(),
            transcript_path: None,
            status: Status::Idle,
            status_since: ts,
            started: ts,
            last_event: ts,
            last_hook: ts,
            idle_cause: IdleCause::Other,
            stopped: false,
            run_started: 0.0,
            detail: String::new(),
            permission_mode: None,
            model: None,
            title: None,
            pid: None,
            subagents: BTreeMap::new(),
            stats: TranscriptStats::default(),
            snapshot: None,
        }
    }

    pub fn set_status(&mut self, status: Status, ts: f64) {
        if self.status != status {
            self.status = status;
            self.status_since = ts;
        }
    }

    pub fn set_idle(&mut self, cause: IdleCause, ts: f64) {
        self.set_status(Status::Idle, ts);
        self.idle_cause = cause;
    }

    /// Corrects the hook-driven status with what only the transcript knows: a turn that ended
    /// without a Stop was interrupted, and a background shell still running keeps the session active.
    pub fn reconcile(&mut self, now: f64) {
        if self.status == Status::Ended {
            return;
        }
        let running = self.shells().next().is_some();
        // Only a status entered before the turn ended can be its interruption: the transcript's turn
        // end lands just after Stop, so later background-agent work or approvals are not undone.
        let interrupted = !self.stopped && self.stats.turn_ended_at.is_some_and(|t| {
            t > self.last_hook && self.status_since <= t && now - t >= INTERRUPT_GRACE_SECS
        });
        // A session kept active by a background shell has no hook newer than its last turn end.
        let waiting = self.status.needs_user() || (self.status == Status::Active && !running);
        if interrupted && waiting {
            self.set_idle(IdleCause::Interrupted, now);
            self.detail.clear();
        }
        if self.status == Status::Idle && running {
            let shells: Vec<&str> = self.shells().collect();
            let more = if shells.len() > 1 { format!(" +{}", shells.len() - 1) } else { String::new() };
            self.detail = format!("background: {}{more}", shells[0]);
            self.set_status(Status::Active, now);
        }
    }

    /// Background shells this process launched that are still running, oldest first.
    fn shells(&self) -> impl Iterator<Item = &str> {
        self.stats.background.iter().filter(|(launched, _)| *launched >= self.run_started).map(|(_, cmd)| cmd.as_str())
    }

    /// Time spent working across the whole session, including earlier runs of a resumed one.
    /// Idle time between turns doesn't count; the turn in progress counts while it runs.
    pub fn running_secs(&self, now: f64) -> f64 {
        let live = match (self.status, self.stats.turn_started) {
            (Status::Idle | Status::Ended, _) | (_, None) => 0.0,
            (_, Some(start)) => (now - start).max(0.0),
        };
        self.stats.busy_secs + live
    }

    pub fn project(&self) -> String {
        std::path::Path::new(&self.cwd)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.id.chars().take(8).collect())
    }

    /// Main agent plus all subagents.
    pub fn total_tokens(&self) -> TokenUsage {
        let mut t = self.stats.tokens;
        for a in self.subagents.values() {
            t += a.stats.tokens;
        }
        t
    }

    /// Context fill relative to the auto-compact point. `compact_window` is the
    /// user's `autoCompactWindow` setting, if any.
    pub fn context(&self, compact_window: Option<u64>) -> Option<ContextInfo> {
        let snap = self.snapshot.as_ref();
        // Prefer whichever source reported most recently.
        let from_snapshot = snap.and_then(|s| s.used.map(|u| (u, s.ts)));
        let from_transcript = self.stats.context_used.map(|u| (u, self.stats.context_at.unwrap_or(0.0)));
        let used = match (from_transcript, from_snapshot) {
            (Some(t), Some(s)) => if s.1 > t.1 { s.0 } else { t.0 },
            (t, s) => t.or(s)?.0,
        };
        let model = snap.and_then(|s| s.model.clone()).or(self.model.clone()).or(self.stats.model.clone());
        Some(context_info(used, model.as_deref(), snap.and_then(|s| s.window), compact_window))
    }

    /// The session's history, oldest first, each event with the agent that produced it.
    /// Subagents contribute their skills and questions; their prompts and replies are
    /// already on the main agent's launch event.
    pub fn timeline(&self) -> Vec<TimelineEntry<'_>> {
        let mut list: Vec<TimelineEntry> = self
            .stats
            .timeline
            .iter()
            .map(|event| TimelineEntry { agent: MAIN_AGENT, stats: &self.stats, event })
            .collect();
        for a in self.subagents.values() {
            list.extend(
                a.stats
                    .timeline
                    .iter()
                    .filter(|e| matches!(e.kind, EventKind::Skill(_) | EventKind::Question { .. }))
                    .map(|event| TimelineEntry { agent: &a.agent_type, stats: &a.stats, event }),
            );
        }
        list.sort_by(|a, b| a.event.at.cmp(&b.event.at));
        list
    }

    /// The subagent an Agent tool call launched.
    pub fn subagent_for(&self, tool_use_id: &str) -> Option<&Subagent> {
        self.subagents.values().find(|a| a.tool_use_id.as_deref() == Some(tool_use_id))
    }

    /// Subagents oldest first, each with the skill whose window launched it. Nested
    /// subagents are launched from another subagent's transcript, so all are searched.
    pub fn subagents_by_start(&self) -> Vec<(&Subagent, Option<&str>)> {
        let mut list: Vec<(&Subagent, Option<&str>)> = self
            .subagents
            .values()
            .map(|a| {
                let skill = a.tool_use_id.as_deref().and_then(|id| {
                    std::iter::once(&self.stats)
                        .chain(self.subagents.values().map(|o| &o.stats))
                        .find_map(|st| st.launches.get(id))
                        .and_then(|s| s.as_deref())
                });
                (a, skill)
            })
            .collect();
        // Not-yet-written transcripts have no start time; keep them last.
        list.sort_by(|a, b| match (&a.0.stats.started, &b.0.stats.started) {
            (Some(x), Some(y)) => x.cmp(y),
            (x, y) => y.is_some().cmp(&x.is_some()),
        });
        list
    }

    /// Skills across the main agent and its subagents, by name.
    pub fn all_skills(&self) -> BTreeMap<String, SkillStat> {
        let mut out = self.stats.skills.clone();
        for a in self.subagents.values() {
            for (name, s) in &a.stats.skills {
                let e = out.entry(name.clone()).or_default();
                e.calls += s.calls;
                e.tokens += s.tokens;
            }
        }
        out
    }
}

/// Column the live session list is sorted by; `s` cycles through them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    /// Most urgent first, then most recent activity.
    #[default]
    Status,
    /// Alphabetical; stays put while sessions work, which keeps selection steady.
    Project,
    /// Least context left first.
    Context,
    /// Least time worked first.
    Age,
    /// Most tokens first.
    Tokens,
}

impl SortKey {
    pub fn next(self) -> Self {
        match self {
            SortKey::Status => SortKey::Project,
            SortKey::Project => SortKey::Context,
            SortKey::Context => SortKey::Age,
            SortKey::Age => SortKey::Tokens,
            SortKey::Tokens => SortKey::Status,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Status => "status",
            SortKey::Project => "project",
            SortKey::Context => "context",
            SortKey::Age => "time",
            SortKey::Tokens => "tokens",
        }
    }
}

/// Sorts sessions by `key` (reversed if asked). Sessions without a context reading
/// always go last; ties fall back to first-seen order so rows don't shuffle.
pub fn sort_sessions(list: &mut [&Session], key: SortKey, reversed: bool, compact_window: Option<u64>) {
    let left = |s: &Session| s.context(compact_window).map(|c| c.left());
    let now = crate::now();
    list.sort_by(|a, b| {
        let ord = match key {
            SortKey::Status => a.status.cmp(&b.status).then(b.last_event.total_cmp(&a.last_event)),
            SortKey::Project => a.project().to_lowercase().cmp(&b.project().to_lowercase()),
            SortKey::Context => match (left(a), left(b)) {
                (Some(x), Some(y)) => x.cmp(&y),
                (x, y) => x.is_none().cmp(&y.is_none()),
            },
            SortKey::Age => a.running_secs(now).total_cmp(&b.running_secs(now)),
            SortKey::Tokens => b.total_tokens().total().cmp(&a.total_tokens().total()),
        };
        let unknown_last = key == SortKey::Context && left(a).is_none() != left(b).is_none();
        let ord = if reversed && !unknown_last { ord.reverse() } else { ord };
        ord.then(a.started.total_cmp(&b.started)).then_with(|| a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_windows() {
        assert_eq!(model_window("claude-opus-5-5", 0), 1_000_000);
        assert_eq!(model_window("claude-opus-4-7", 0), 1_000_000);
        assert_eq!(model_window("claude-opus-4-6", 0), 200_000);
        assert_eq!(model_window("claude-opus-4-6[1m]", 0), 1_000_000);
        assert_eq!(model_window("claude-sonnet-4-5-20250929", 0), 200_000);
        assert_eq!(model_window("claude-sonnet-4-5-20250929", 250_000), 1_000_000);
        assert_eq!(model_window("claude-fable-5-1", 0), 1_000_000);
        assert_eq!(model_window("claude-haiku-4-5", 0), 200_000);
    }

    #[test]
    fn compact_limit() {
        let mut s = Session::new("s", 0.0);
        s.stats.context_used = Some(500_000);
        s.model = Some("claude-opus-5-5".into());
        let c = s.context(None).unwrap();
        assert_eq!((c.window, c.limit, c.left()), (1_000_000, 967_000, 467_000));
        assert!(c.estimated);
        assert_eq!(s.context(Some(600_000)).unwrap().limit, 600_000);
        s.snapshot = Some(StatusSnapshot { window: Some(200_000), ..Default::default() });
        let c = s.context(None).unwrap();
        assert_eq!((c.limit, c.left(), c.estimated), (200_000, 0, false));
    }

    #[test]
    fn sorting() {
        let mk = |id: &str, cwd: &str, status: Status, started: f64, used: Option<u64>| {
            let mut s = Session::new(id, started);
            s.cwd = cwd.into();
            s.status = status;
            s.model = Some("claude-haiku-4-5".into());
            s.stats.context_used = used;
            s
        };
        let a = mk("a", "/x/beta", Status::Idle, 1.0, Some(50_000));
        let b = mk("b", "/x/Alpha", Status::Approve, 2.0, None);
        let c = mk("c", "/x/gamma", Status::Active, 3.0, Some(150_000));
        let order = |key, reversed| {
            let mut list = vec![&a, &b, &c];
            sort_sessions(&mut list, key, reversed, None);
            list.iter().map(|s| s.id.as_str()).collect::<String>()
        };
        assert_eq!(order(SortKey::Status, false), "bca");
        assert_eq!(order(SortKey::Project, false), "bac", "case-insensitive");
        assert_eq!(order(SortKey::Project, true), "cab");
        assert_eq!(order(SortKey::Context, false), "cab", "least left first, unknown last");
        assert_eq!(order(SortKey::Context, true), "acb", "unknown stays last when reversed");
        assert_eq!(order(SortKey::Tokens, false), "abc", "ties keep first-seen order");
        assert_eq!(order(SortKey::Age, false), "abc", "ties keep first-seen order");
    }

    #[test]
    fn running_time_counts_only_work() {
        let mut s = Session::new("s", 0.0);
        s.stats.busy_secs = 100.0;
        s.stats.turn_started = Some(1_000.0);
        s.set_status(Status::Active, 1_000.0);
        assert_eq!(s.running_secs(1_030.0), 130.0, "the turn in progress counts live");
        s.set_status(Status::Approve, 1_030.0);
        assert_eq!(s.running_secs(1_050.0), 150.0);
        for idle in [Status::Idle, Status::Ended] {
            s.set_status(idle, 1_060.0);
            assert_eq!(s.running_secs(9_999.0), 100.0, "{idle:?} adds nothing");
        }
    }

    /// A session in `status` since the hook at `hook`, whose transcript recorded a turn end at `ended`.
    fn reconciling(status: Status, hook: f64, ended: Option<f64>, background: &[&str]) -> Session {
        let mut s = Session::new("s", 0.0);
        s.set_status(status, hook);
        s.last_hook = hook;
        s.stats.turn_ended_at = ended;
        s.stats.background = background.iter().map(|b| (hook, b.to_string())).collect();
        s
    }

    #[test]
    fn interrupts_go_idle_after_a_grace_period() {
        for status in [Status::Question, Status::Approve, Status::Active] {
            let mut s = reconciling(status, 10.0, Some(12.0), &[]);
            s.reconcile(13.0);
            assert_eq!(s.status, status, "{status:?}: within the grace period, Stop may still arrive");
            s.reconcile(15.0);
            assert_eq!((s.status, s.idle_cause), (Status::Idle, IdleCause::Interrupted), "{status:?}");
        }
        // A turn end from before the latest hook belongs to an earlier turn.
        let mut s = reconciling(Status::Active, 13.0, Some(12.0), &[]);
        s.reconcile(20.0);
        assert_eq!(s.status, Status::Active);
        // Ended sessions are left alone.
        let mut s = reconciling(Status::Ended, 10.0, Some(12.0), &["cargo build"]);
        s.reconcile(20.0);
        assert_eq!(s.status, Status::Ended);
    }

    #[test]
    fn background_shells_keep_a_session_active() {
        let mut s = reconciling(Status::Idle, 10.0, Some(10.0), &["cargo build", "npm test"]);
        s.idle_cause = IdleCause::Finished;
        s.reconcile(11.0);
        assert_eq!((s.status, s.detail.as_str()), (Status::Active, "background: cargo build +1"));
        // It stays active while they run, even though the turn ended without a later hook.
        s.reconcile(30.0);
        assert_eq!(s.status, Status::Active);
        // Escaping a question while a shell runs still leaves the question, then shows the shell.
        let mut s = reconciling(Status::Question, 10.0, Some(12.0), &["cargo build"]);
        s.reconcile(15.0);
        assert_eq!(s.status, Status::Active);
        assert_eq!(s.idle_cause, IdleCause::Interrupted);
    }

    #[test]
    fn subagents_link_to_launching_skill() {
        let mut s = Session::new("s", 0.0);
        s.stats.launches.insert("t1".into(), Some("ce-work".into()));
        s.stats.launches.insert("t2".into(), None);
        let agent = |tool: &str, started: Option<&str>| {
            let mut a = Subagent { tool_use_id: Some(tool.into()), ..Default::default() };
            a.stats.started = started.map(str::to_string);
            a
        };
        s.subagents.insert("a".into(), agent("t2", Some("2026-10-11T10:05:00Z")));
        s.subagents.insert("b".into(), agent("t1", Some("2026-10-11T10:00:00Z")));
        s.subagents.insert("c".into(), agent("t3", None));
        // A nested subagent, launched from inside subagent "b" while it ran ce-debug.
        s.subagents.get_mut("b").unwrap().stats.launches.insert("t3".into(), Some("ce-debug".into()));

        let list: Vec<_> = s.subagents_by_start().into_iter().map(|(a, k)| (a.tool_use_id.clone().unwrap(), k)).collect();
        assert_eq!(
            list,
            [("t1".to_string(), Some("ce-work")), ("t2".to_string(), None), ("t3".to_string(), Some("ce-debug"))]
        );
    }

    #[test]
    fn subagent_context_uses_its_own_model() {
        let mut a = Subagent::default();
        assert!(a.context(None).is_none());
        a.stats.context_used = Some(50_000);
        a.stats.model = Some("claude-haiku-4-5".into());
        let ctx = a.context(None).unwrap();
        assert_eq!((ctx.limit, ctx.left()), (200_000, 150_000));
    }
}
