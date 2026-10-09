use std::collections::BTreeMap;
use std::ops::AddAssign;
use std::path::PathBuf;

/// Ordered by urgency: sessions that need the user sort first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    Permission,
    Question,
    Error,
    Active,
    Idle,
    Ended,
}

impl Status {
    pub fn needs_user(self) -> bool {
        matches!(self, Status::Permission | Status::Question)
    }
}

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

/// Token and skill figures derived from one transcript file.
#[derive(Clone, Debug, Default)]
pub struct TranscriptStats {
    pub tokens: TokenUsage,
    pub skills: BTreeMap<String, SkillStat>,
    pub model: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Subagent {
    pub agent_type: String,
    pub running: bool,
    pub stats: TranscriptStats,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    pub transcript_path: Option<PathBuf>,
    pub status: Status,
    pub status_since: f64,
    pub last_event: f64,
    /// What the session is doing right now, e.g. "Bash: cargo test".
    pub detail: String,
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    pub title: Option<String>,
    pub pid: Option<u32>,
    pub subagents: BTreeMap<String, Subagent>,
    pub stats: TranscriptStats,
}

impl Session {
    pub fn new(id: &str, ts: f64) -> Self {
        Self {
            id: id.to_string(),
            cwd: String::new(),
            transcript_path: None,
            status: Status::Idle,
            status_since: ts,
            last_event: ts,
            detail: String::new(),
            permission_mode: None,
            model: None,
            title: None,
            pid: None,
            subagents: BTreeMap::new(),
            stats: TranscriptStats::default(),
        }
    }

    pub fn set_status(&mut self, status: Status, ts: f64) {
        if self.status != status {
            self.status = status;
            self.status_since = ts;
        }
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
