//! Token usage and skill attribution from Claude Code transcript JSONL files.
//!
//! Per-skill tokens use window attribution: once a skill is invoked (via the Skill tool
//! or a typed `/skill` command), every following assistant turn is charged to it until
//! the next real user prompt or the next skill.

use crate::jsonl::JsonlTail;
use crate::model::{SkillRun, SkillStat, TokenUsage, TranscriptStats, Via};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// Built-in slash commands that are not skills.
const BUILTIN_COMMANDS: &[&str] = &[
    "add-dir", "agents", "artifacts", "clear", "compact", "config", "context", "cost", "doctor",
    "effort", "exit", "export", "fast", "help", "hooks", "ide", "init", "login", "logout", "mcp",
    "memory", "model", "permissions", "plan", "plugin", "plugins", "release-notes", "reload-plugins", "resume",
    "rewind", "status", "statusline", "terminal-setup", "theme", "usage", "vim",
];

pub struct TranscriptReader {
    tail: JsonlTail,
    /// message.id -> (usage, index of the skill run it belongs to, local day). Streamed messages repeat their id.
    messages: HashMap<String, (TokenUsage, Option<usize>, Option<String>)>,
    /// (name, timestamp, via) per invocation, in transcript order.
    runs: Vec<(String, Option<String>, Via)>,
    current_run: Option<usize>,
    /// Agent/Task tool_use id -> skill whose window it ran in, for linking subagents to skills.
    launches: HashMap<String, Option<String>>,
    /// True for a subagent's own transcript, where every entry is marked `isSidechain`.
    sidechain: bool,
    context_used: Option<u64>,
    context_at: Option<f64>,
    model: Option<String>,
    title: Option<String>,
    cwd: Option<String>,
    first_at: Option<String>,
    last_at: Option<String>,
}

/// Tokens and assistant turns for one local calendar day.
#[derive(Clone, Copy, Debug, Default)]
pub struct DayUsage {
    pub tokens: TokenUsage,
    pub turns: u32,
}

/// Local calendar day (YYYY-MM-DD) of an ISO-8601 timestamp.
pub fn local_day(iso: &str) -> Option<String> {
    let t = chrono::DateTime::parse_from_rfc3339(iso).ok()?;
    Some(t.with_timezone(&chrono::Local).format(crate::model::DAY_FMT).to_string())
}

impl TranscriptReader {
    pub fn new(path: PathBuf) -> Self {
        Self {
            tail: JsonlTail::new(path),
            messages: HashMap::new(),
            runs: Vec::new(),
            current_run: None,
            launches: HashMap::new(),
            sidechain: false,
            context_used: None,
            context_at: None,
            model: None,
            title: None,
            cwd: None,
            first_at: None,
            last_at: None,
        }
    }

    /// Reader for a subagent transcript, whose entries all count toward its own context.
    pub fn subagent(path: PathBuf) -> Self {
        Self { sidechain: true, ..Self::new(path) }
    }

    /// Reads newly appended lines. Returns true if anything was read.
    pub fn poll(&mut self) -> bool {
        let entries = self.tail.read_new();
        for e in &entries {
            self.ingest(e);
        }
        !entries.is_empty()
    }

    fn ingest(&mut self, e: &Value) {
        if let Some(ts) = e["timestamp"].as_str() {
            if self.first_at.is_none() {
                self.first_at = Some(ts.to_string());
            }
            self.last_at = Some(ts.to_string());
        }
        if self.cwd.is_none() {
            self.cwd = e["cwd"].as_str().map(str::to_string);
        }
        match e["type"].as_str() {
            Some("assistant") => self.assistant(e),
            Some("user") => self.user(e),
            Some("ai-title") => {
                if let Some(t) = e["aiTitle"].as_str() {
                    self.title = Some(t.to_string());
                }
            }
            _ => {}
        }
    }

    fn assistant(&mut self, e: &Value) {
        let msg = &e["message"];
        if let Some(m) = msg["model"].as_str().filter(|m| !m.starts_with('<')) {
            self.model = Some(m.to_string());
        }
        let usage = parse_usage(&msg["usage"]);
        let prompt = usage.input + usage.cache_read + usage.cache_write;
        if prompt > 0 && e["isSidechain"].as_bool().unwrap_or(false) == self.sidechain {
            self.context_used = Some(prompt);
            self.context_at = e["timestamp"]
                .as_str()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.timestamp_millis() as f64 / 1000.0);
        }
        let id = msg["id"].as_str().or(e["uuid"].as_str()).unwrap_or_default().to_string();
        let run = self.current_run;
        let day = e["timestamp"].as_str().and_then(local_day);
        self.messages
            .entry(id)
            .and_modify(|(u, _, _)| *u = usage)
            .or_insert((usage, run, day));

        for block in msg["content"].as_array().into_iter().flatten() {
            if block["type"] != "tool_use" {
                continue;
            }
            match block["name"].as_str() {
                Some("Skill") => {
                    if let Some(name) = block["input"]["skill"].as_str() {
                        self.open_skill(name, e, Via::Tool);
                    }
                }
                Some("Agent" | "Task") => {
                    if let Some(id) = block["id"].as_str() {
                        let skill = self.current_run.map(|i| self.runs[i].0.clone());
                        self.launches.insert(id.to_string(), skill);
                    }
                }
                _ => {}
            }
        }
    }

    fn user(&mut self, e: &Value) {
        if e["isMeta"] == true {
            return;
        }
        let content = &e["message"]["content"];
        let text = match content {
            Value::String(s) => s.as_str(),
            Value::Array(blocks) => {
                if blocks.iter().any(|b| b["type"] == "tool_result") {
                    return;
                }
                blocks.iter().find_map(|b| b["text"].as_str()).unwrap_or("")
            }
            _ => return,
        };
        // A background agent finishing is reported as a user entry, but the skill keeps running.
        if e["origin"]["kind"] == "task-notification" || text.starts_with("<task-notification>") {
            return;
        }
        // A new real prompt closes the skill window; a typed /skill opens a new one.
        self.current_run = None;
        if let Some(cmd) = slash_command(text)
            && !BUILTIN_COMMANDS.contains(&cmd) {
                self.open_skill(cmd, e, Via::Command);
            }
    }

    fn open_skill(&mut self, name: &str, entry: &Value, via: Via) {
        let at = entry["timestamp"].as_str().map(str::to_string);
        self.runs.push((name.to_string(), at, via));
        self.current_run = Some(self.runs.len() - 1);
    }

    pub fn stats(&self) -> TranscriptStats {
        let mut tokens = TokenUsage::default();
        let mut runs: Vec<SkillRun> = self
            .runs
            .iter()
            .map(|(name, at, via)| SkillRun {
                name: name.clone(),
                at: at.clone(),
                via: *via,
                tokens: TokenUsage::default(),
                turns: 0,
            })
            .collect();
        for (usage, run, _) in self.messages.values() {
            tokens += *usage;
            if let Some(r) = run.and_then(|i| runs.get_mut(i)) {
                r.tokens += *usage;
                r.turns += 1;
            }
        }
        let mut skills: BTreeMap<String, SkillStat> = BTreeMap::new();
        for r in &runs {
            let stat = skills.entry(r.name.clone()).or_default();
            stat.calls += 1;
            stat.tokens += r.tokens;
        }
        TranscriptStats {
            tokens,
            skills,
            runs,
            launches: self.launches.clone(),
            context_used: self.context_used,
            context_at: self.context_at,
            model: self.model.clone(),
            title: self.title.clone(),
            started: self.first_at.clone(),
        }
    }

    /// Usage bucketed by the local day each API call happened.
    pub fn daily(&self) -> BTreeMap<String, DayUsage> {
        let mut out: BTreeMap<String, DayUsage> = BTreeMap::new();
        for (usage, _, day) in self.messages.values() {
            let Some(day) = day else { continue };
            let d = out.entry(day.clone()).or_default();
            d.tokens += *usage;
            d.turns += 1;
        }
        out
    }

    pub fn cwd(&self) -> Option<&str> {
        self.cwd.as_deref()
    }

    /// First and last entry timestamps (ISO-8601).
    pub fn span(&self) -> (Option<&str>, Option<&str>) {
        (self.first_at.as_deref(), self.last_at.as_deref())
    }
}

fn parse_usage(u: &Value) -> TokenUsage {
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    TokenUsage {
        input: n("input_tokens"),
        output: n("output_tokens"),
        cache_read: n("cache_read_input_tokens"),
        cache_write: n("cache_creation_input_tokens"),
    }
}

/// Extracts `foo` from a prompt containing `<command-name>/foo</command-name>`.
fn slash_command(text: &str) -> Option<&str> {
    let start = text.find("<command-name>/")? + "<command-name>/".len();
    let end = text[start..].find("</command-name>")? + start;
    Some(text[start..end].trim())
}

/// Subagent transcripts live at `<transcript without .jsonl>/subagents/agent-<id>.jsonl`.
pub fn subagent_dir(transcript: &std::path::Path) -> PathBuf {
    transcript.with_extension("").join("subagents")
}

/// `(agent_id, path)` of every `agent-<id>.jsonl` in a subagents directory.
pub fn subagent_files(dir: &std::path::Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_prefix("agent-")?.strip_suffix(".jsonl")?.to_string();
            Some((id, e.path()))
        })
        .collect()
}

/// What Claude Code records about a subagent in its `.meta.json`.
#[derive(Debug, Default, PartialEq)]
pub struct SubagentMeta {
    pub agent_type: Option<String>,
    /// The short task description passed to the Agent tool.
    pub description: Option<String>,
    /// Id of the Agent tool call that launched it, in the parent's transcript.
    pub tool_use_id: Option<String>,
}

/// Reads a subagent's `.meta.json`, if present.
pub fn subagent_meta(dir: &std::path::Path, agent_id: &str) -> Option<SubagentMeta> {
    let text = std::fs::read_to_string(dir.join(format!("agent-{agent_id}.meta.json"))).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let s = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    Some(SubagentMeta { agent_type: s("agentType"), description: s("description"), tool_use_id: s("toolUseId") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn reader_for(lines: &[&str]) -> TranscriptReader {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("marsad-tr-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        let mut r = TranscriptReader::new(path);
        r.poll();
        std::fs::remove_dir_all(dir).ok();
        r
    }

    #[test]
    fn dedupes_streamed_messages_and_attributes_skills() {
        let r = reader_for(&[
            r#"{"type":"user","message":{"content":"hello"}}"#,
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5-5","usage":{"input_tokens":10,"output_tokens":5},"content":[{"type":"text","text":"hi"}]}}"#,
            r#"{"type":"assistant","message":{"id":"m1","usage":{"input_tokens":10,"output_tokens":7},"content":[{"type":"tool_use","name":"Skill","input":{"skill":"ce-plan"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"Launching skill"}]}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":[{"type":"text","text":"Base directory for this skill"}]}}"#,
            r#"{"type":"assistant","message":{"id":"m2","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":1000}}}"#,
            r#"{"type":"user","message":{"content":"next prompt"}}"#,
            r#"{"type":"assistant","message":{"id":"m3","usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ]);
        let s = r.stats();
        assert_eq!(s.tokens.input, 111);
        assert_eq!(s.tokens.output, 58);
        assert_eq!(s.tokens.cache_read, 1000);
        let plan = &s.skills["ce-plan"];
        assert_eq!(plan.calls, 1);
        assert_eq!(plan.tokens.total(), 1150);
        assert_eq!(s.model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(s.runs.len(), 1);
        assert_eq!((s.runs[0].turns, s.runs[0].via), (1, Via::Tool));
        assert_eq!(s.context_used, Some(1));
    }

    #[test]
    fn typed_slash_skill_opens_window_but_builtins_do_not() {
        let r = reader_for(&[
            r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}"#,
            r#"{"type":"user","message":{"content":"<command-message>review</command-message><command-name>/code-review</command-name>"}}"#,
            r#"{"type":"assistant","message":{"id":"m1","usage":{"output_tokens":9}}}"#,
        ]);
        let s = r.stats();
        assert!(!s.skills.contains_key("clear"));
        assert_eq!(s.skills["code-review"].tokens.output, 9);
    }

    #[test]
    fn records_which_skill_launched_each_agent() {
        let r = reader_for(&[
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t0","name":"Agent","input":{}}]}}"#,
            r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","name":"Skill","input":{"skill":"ce-work"}}]}}"#,
            r#"{"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{}}]}}"#,
        ]);
        let s = r.stats();
        assert_eq!(s.launches["t0"], None);
        assert_eq!(s.launches["t1"].as_deref(), Some("ce-work"));
    }

    #[test]
    fn background_agent_notification_keeps_skill_window_open() {
        let r = reader_for(&[
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","name":"Skill","input":{"skill":"ce-code-review"}}]}}"#,
            r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>\n<status>completed</status>"}}"#,
            r#"{"type":"assistant","message":{"id":"m2","usage":{"output_tokens":4},"content":[{"type":"tool_use","id":"t1","name":"Agent","input":{}}]}}"#,
        ]);
        let s = r.stats();
        assert_eq!(s.launches["t1"].as_deref(), Some("ce-code-review"));
        assert_eq!(s.skills["ce-code-review"].tokens.output, 4);
    }

    #[test]
    fn subagent_reader_counts_sidechain_context() {
        let lines = [r#"{"type":"assistant","isSidechain":true,"message":{"id":"m1","usage":{"input_tokens":7,"cache_read_input_tokens":3}}}"#];
        assert_eq!(reader_for(&lines).stats().context_used, None);

        let dir = std::env::temp_dir().join(format!("marsad-sub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent-a1.jsonl");
        std::fs::write(&path, format!("{}\n", lines[0])).unwrap();
        std::fs::write(dir.join("agent-a1.meta.json"), r#"{"agentType":"Explore","description":"Map code","toolUseId":"t1"}"#).unwrap();
        let mut r = TranscriptReader::subagent(path);
        r.poll();
        let meta = subagent_meta(&dir, "a1");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(r.stats().context_used, Some(10));
        let meta = meta.unwrap();
        assert_eq!(meta.agent_type.as_deref(), Some("Explore"));
        assert_eq!(meta.description.as_deref(), Some("Map code"));
        assert_eq!(meta.tool_use_id.as_deref(), Some("t1"));
    }
}
