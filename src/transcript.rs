//! Token usage and skill attribution from Claude Code transcript JSONL files.
//!
//! Per-skill tokens use window attribution: once a skill is invoked (via the Skill tool
//! or a typed `/skill` command), every following assistant turn is charged to it until
//! the next real user prompt or the next skill.
//!
//! The reader also keeps a timeline of what happened: prompts, skills, subagent launches
//! and their reports, questions and answers, and the reply that ended each turn.

use crate::jsonl::JsonlTail;
use crate::model::{Event, EventKind, SkillRun, SkillStat, TokenUsage, TranscriptStats, Via};
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

/// Longest prompt, reply or report kept in the timeline, in characters.
const MAX_TEXT: usize = 4000;

/// One API call's usage, and where it belongs.
struct Message {
    usage: TokenUsage,
    /// Skill run whose window it ran in.
    run: Option<usize>,
    /// Timeline index of the prompt that started its turn.
    turn: Option<usize>,
    day: Option<String>,
}

pub struct TranscriptReader {
    tail: JsonlTail,
    /// message.id -> its usage. Streamed messages repeat their id.
    messages: HashMap<String, Message>,
    /// (name, timestamp, via) per invocation, in transcript order.
    runs: Vec<(String, Option<String>, Via)>,
    current_run: Option<usize>,
    /// Agent/Task tool_use id -> skill whose window it ran in, for linking subagents to skills.
    launches: HashMap<String, Option<String>>,
    /// True for a subagent's own transcript, where every entry is marked `isSidechain`.
    sidechain: bool,
    timeline: Vec<Event>,
    /// Timeline index of the current turn's prompt.
    turn: Option<usize>,
    /// Timeline index and message id of the latest reply, while nothing has followed it.
    reply: Option<(usize, String)>,
    /// Agent/AskUserQuestion tool_use id -> timeline indices waiting for its result.
    pending: HashMap<String, Vec<usize>>,
    context_used: Option<u64>,
    context_at: Option<f64>,
    model: Option<String>,
    title: Option<String>,
    cwd: Option<String>,
    first_at: Option<String>,
    last_at: Option<String>,
    /// Seconds spent working: finished turns plus turns closed without a recorded duration.
    busy_secs: f64,
    /// Part of `busy_secs` estimated since the last recorded turn duration, which replaces it.
    estimated_secs: f64,
    /// Unix start and latest entry time of the turn in progress.
    open_turn: Option<(f64, f64)>,
    turn_ended_at: Option<f64>,
    /// Bash tool_use id -> its command, until its result says whether it went to the background.
    bash: HashMap<String, String>,
    /// (task id, unix launch time, command) of background shells not yet reported finished.
    background: Vec<(String, f64, String)>,
}

/// Tokens and assistant turns for one local calendar day.
#[derive(Clone, Copy, Debug, Default)]
pub struct DayUsage {
    pub tokens: TokenUsage,
    pub turns: u32,
}

/// Unix time of an entry's timestamp.
fn unix_ts(e: &Value) -> Option<f64> {
    let t = chrono::DateTime::parse_from_rfc3339(e["timestamp"].as_str()?).ok()?;
    Some(t.timestamp_millis() as f64 / 1000.0)
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
            timeline: Vec::new(),
            turn: None,
            reply: None,
            pending: HashMap::new(),
            context_used: None,
            context_at: None,
            model: None,
            title: None,
            cwd: None,
            first_at: None,
            last_at: None,
            busy_secs: 0.0,
            estimated_secs: 0.0,
            open_turn: None,
            turn_ended_at: None,
            bash: HashMap::new(),
            background: Vec::new(),
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
            Some("system") if e["subtype"] == "turn_duration" => {
                // Claude Code's own measure of the turn replaces anything estimated for it.
                let ms = e["durationMs"].as_f64().unwrap_or(0.0);
                self.busy_secs += ms / 1000.0 - self.estimated_secs;
                self.estimated_secs = 0.0;
                self.open_turn = None;
                self.turn_ended_at = unix_ts(e);
            }
            // A background shell finishing mid-turn is recorded only as a queued command.
            Some("queue-operation") => self.close_task(e["content"].as_str().unwrap_or("")),
            Some("attachment") if e["attachment"]["type"] == "queued_command" => {
                self.close_task(e["attachment"]["prompt"].as_str().unwrap_or(""))
            }
            _ => {}
        }
        if let (Some((_, last)), Some(t)) = (&mut self.open_turn, unix_ts(e)) {
            *last = t;
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
        let (run, turn) = (self.current_run, self.turn);
        let day = e["timestamp"].as_str().and_then(local_day);
        self.messages
            .entry(id.clone())
            .and_modify(|m| m.usage = usage)
            .or_insert(Message { usage, run, turn, day });

        // Text that ends a turn is Claude's reply; text before a tool call is narration.
        let ends_turn = msg["stop_reason"] != "tool_use";
        for block in msg["content"].as_array().into_iter().flatten() {
            match block["type"].as_str() {
                Some("text") if ends_turn => {
                    if let Some(text) = block["text"].as_str().map(str::trim).filter(|t| !t.is_empty()) {
                        self.add_reply(text, &id, e);
                    }
                }
                Some("tool_use") => self.tool_use(block, e),
                _ => {}
            }
        }
    }

    fn tool_use(&mut self, block: &Value, e: &Value) {
        let input = &block["input"];
        let id = block["id"].as_str().unwrap_or_default();
        match block["name"].as_str() {
            Some("Skill") => {
                if let Some(name) = input["skill"].as_str() {
                    self.open_skill(name, e, Via::Tool);
                }
            }
            Some("Agent" | "Task") => {
                let skill = self.current_run.map(|i| self.runs[i].0.clone());
                self.launches.insert(id.to_string(), skill);
                let s = |k: &str| input[k].as_str().unwrap_or_default().to_string();
                let agent_type = Some(s("subagent_type")).filter(|t| !t.is_empty()).unwrap_or_else(|| "agent".into());
                let kind = EventKind::Agent { agent_type, description: s("description"), tool_use_id: id.to_string(), result: None };
                let i = self.push(kind, e);
                self.pending.entry(id.to_string()).or_default().push(i);
            }
            Some("Bash") => {
                let label = crate::hook::truncate(input["command"].as_str().unwrap_or_default().lines().next().unwrap_or(""), 60);
                self.bash.insert(id.to_string(), label);
            }
            Some("AskUserQuestion") => {
                for q in input["questions"].as_array().into_iter().flatten() {
                    let Some(question) = q["question"].as_str() else { continue };
                    let i = self.push(EventKind::Question { question: question.to_string(), answer: None }, e);
                    self.pending.entry(id.to_string()).or_default().push(i);
                }
            }
            _ => {}
        }
    }

    fn push(&mut self, kind: EventKind, e: &Value) -> usize {
        let at = e["timestamp"].as_str().map(str::to_string);
        self.timeline.push(Event { at, kind, tokens: TokenUsage::default() });
        self.reply = None;
        self.timeline.len() - 1
    }

    /// Adds reply text; a reply split over several entries, or replaced by a later message
    /// before anything else happened, stays one event.
    fn add_reply(&mut self, text: &str, msg_id: &str, e: &Value) {
        if let Some((i, id)) = &self.reply
            && let EventKind::Response(prev) = &mut self.timeline[*i].kind {
                if id == msg_id {
                    if !prev.ends_with(text) {
                        *prev = clip(&format!("{prev}\n\n{text}"));
                    }
                } else {
                    *prev = clip(text);
                    self.reply = Some((*i, msg_id.to_string()));
                }
                return;
            }
        let i = self.push(EventKind::Response(clip(text)), e);
        self.reply = Some((i, msg_id.to_string()));
    }

    /// Fills in a subagent's report or a question's answer.
    fn tool_result(&mut self, block: &Value, e: &Value) {
        let Some(indices) = block["tool_use_id"].as_str().and_then(|id| self.pending.get(id)).cloned() else { return };
        let text = result_text(&block["content"]);
        // A background agent reports later, as a message from its agent id or in a task notification.
        if let Some(rest) = text.strip_prefix("Async agent launched") {
            if let Some(agent_id) = rest.split("agentId: ").nth(1).and_then(|r| r.split_whitespace().next()) {
                self.pending.insert(agent_id.to_string(), indices);
            }
            return;
        }
        let declined = block["is_error"] == true;
        for i in indices {
            match &mut self.timeline[i].kind {
                EventKind::Agent { result, .. } => *result = Some(clip(&text)),
                EventKind::Question { question, answer } => {
                    let given = e["toolUseResult"]["answers"][question.as_str()].as_str();
                    // Declining a question can come with a note for Claude.
                    let note = text.split("the user said:").nth(1).and_then(|n| n.lines().map(str::trim).find(|l| !l.is_empty()));
                    *answer = Some(match (given, note) {
                        (Some(a), _) => a.to_string(),
                        (None, Some(note)) if declined => format!("(declined) {note}"),
                        (None, _) if declined => "(declined)".into(),
                        (None, _) => clip(&text),
                    });
                }
                _ => {}
            }
        }
    }

    /// Sets a subagent's report; `weak` reports (notification summaries) don't replace one already in.
    fn set_report(&mut self, key: &str, report: &str, weak: bool) {
        let Some(indices) = self.pending.get(key) else { return };
        for &i in indices {
            if let EventKind::Agent { result, .. } = &mut self.timeline[i].kind
                && !(weak && result.is_some()) {
                    *result = Some(clip(report));
                }
        }
    }

    /// A finished background task's notification names it by task id.
    fn close_task(&mut self, text: &str) {
        if let (Some(id), Some(_)) = (tag(text, "task-id"), tag(text, "status")) {
            self.background.retain(|(task, _, _)| task != id);
        }
    }

    /// A background agent finishing is announced in a `<task-notification>` naming its tool call.
    fn task_notification(&mut self, text: &str) {
        self.close_task(text);
        let Some(id) = tag(text, "tool-use-id") else { return };
        let report = tag(text, "result").or_else(|| tag(text, "summary")).unwrap_or_default();
        // The full report may come separately, as a hand-back message.
        if !report.starts_with("This agent's report was delivered") {
            self.set_report(id, report, true);
        }
    }

    /// A subagent's hand-back message: its final report, framed and indented by the harness.
    fn hand_back(&mut self, e: &Value) {
        let origin = &e["origin"];
        let (Some(from), Some(body)) = (origin["from"].as_str(), origin["body"].as_str()) else { return };
        let report = body.split_once("The report follows:").map_or(body, |(_, r)| r);
        let report: Vec<&str> = report.lines().map(|l| l.strip_prefix("  ").unwrap_or(l)).collect();
        self.set_report(from, report.join("\n").trim(), false);
    }

    fn user(&mut self, e: &Value) {
        // A message another agent sent this one, such as a subagent's report: not a prompt.
        if e["origin"]["kind"] == "peer" {
            self.hand_back(e);
            return;
        }
        if e["isMeta"] == true {
            return;
        }
        let content = &e["message"]["content"];
        let text = match content {
            Value::String(s) => s.as_str(),
            Value::Array(blocks) => {
                if blocks.iter().any(|b| b["type"] == "tool_result") {
                    // A Bash command Claude ran in the background, or one moved there on timeout.
                    let tool = blocks.iter().find_map(|b| b["tool_use_id"].as_str()).unwrap_or_default();
                    if let (Some(label), Some(task)) = (self.bash.remove(tool), e["toolUseResult"]["backgroundTaskId"].as_str()) {
                        self.background.push((task.to_string(), unix_ts(e).unwrap_or(0.0), label));
                    }
                    for b in blocks.iter().filter(|b| b["type"] == "tool_result") {
                        self.tool_result(b, e);
                    }
                    return;
                }
                blocks.iter().find_map(|b| b["text"].as_str()).unwrap_or("")
            }
            _ => return,
        };
        // A background agent finishing is reported as a user entry, but the skill keeps running.
        if e["origin"]["kind"] == "task-notification" || text.starts_with("<task-notification>") {
            self.task_notification(text);
            return;
        }
        // An interrupted turn usually gets its duration recorded next; until then, estimate it.
        if text.starts_with("[Request interrupted") {
            self.close_turn();
            self.turn_ended_at = unix_ts(e);
        }
        // Output of a local command, or an interruption marker: not something the user typed.
        let Some(prompt) = prompt_text(text) else { return };
        // A new real prompt closes the skill window and starts a turn; a typed /skill opens a new window.
        self.current_run = None;
        self.turn = Some(self.push(EventKind::Prompt(clip(&prompt)), e));
        // A message sent mid-turn: estimate the turn so far, until its duration is recorded.
        // Otherwise the last turn is over and nothing will replace its estimate any more.
        if self.open_turn.is_some() {
            self.close_turn();
        } else {
            self.estimated_secs = 0.0;
        }
        self.open_turn = unix_ts(e).map(|t| (t, t));
        if let Some(cmd) = slash_command(text)
            && !BUILTIN_COMMANDS.contains(&cmd) {
                self.open_skill(cmd, e, Via::Command);
            }
    }

    /// Counts the open turn up to its last entry, as an estimate a recorded duration replaces.
    fn close_turn(&mut self) {
        if let Some((start, last)) = self.open_turn.take() {
            self.busy_secs += last - start;
            self.estimated_secs += last - start;
        }
    }

    fn open_skill(&mut self, name: &str, entry: &Value, via: Via) {
        let at = entry["timestamp"].as_str().map(str::to_string);
        self.runs.push((name.to_string(), at, via));
        self.current_run = Some(self.runs.len() - 1);
        self.push(EventKind::Skill(self.runs.len() - 1), entry);
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
        let mut timeline = self.timeline.clone();
        for m in self.messages.values() {
            tokens += m.usage;
            if let Some(r) = m.run.and_then(|i| runs.get_mut(i)) {
                r.tokens += m.usage;
                r.turns += 1;
            }
            if let Some(ev) = m.turn.and_then(|i| timeline.get_mut(i)) {
                ev.tokens += m.usage;
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
            timeline,
            launches: self.launches.clone(),
            context_used: self.context_used,
            context_at: self.context_at,
            model: self.model.clone(),
            title: self.title.clone(),
            started: self.first_at.clone(),
            busy_secs: self.busy_secs,
            turn_started: self.open_turn.map(|(start, _)| start),
            turn_ended_at: self.turn_ended_at,
            background: self.background.iter().map(|(_, at, label)| (*at, label.clone())).collect(),
        }
    }

    /// Usage bucketed by the local day each API call happened.
    pub fn daily(&self) -> BTreeMap<String, DayUsage> {
        let mut out: BTreeMap<String, DayUsage> = BTreeMap::new();
        for m in self.messages.values() {
            let Some(day) = &m.day else { continue };
            let d = out.entry(day.clone()).or_default();
            d.tokens += m.usage;
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

/// What the user typed, as they typed it: `/cmd args` for slash commands and `! cmd` for
/// shell commands. None for command output, caveats and interruption markers.
fn prompt_text(text: &str) -> Option<String> {
    let text = text.trim();
    if let Some(cmd) = slash_command(text) {
        let args = tag(text, "command-args").unwrap_or_default();
        return Some(format!("/{cmd} {args}").trim_end().to_string());
    }
    if let Some(cmd) = tag(text, "bash-input") {
        return Some(format!("! {cmd}"));
    }
    let skip = ["<local-command-", "<bash-stdout>", "<bash-stderr>", "[Request interrupted"];
    if text.is_empty() || skip.iter().any(|p| text.starts_with(p)) {
        return None;
    }
    Some(text.to_string())
}

/// Trimmed contents of the first `<name>…</name>` in `text`.
fn tag<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&format!("</{name}>"))? + start;
    Some(text[start..end].trim())
}

/// A tool result's text: a plain string, or its first text block.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.trim().to_string(),
        Value::Array(blocks) => blocks.iter().find_map(|b| b["text"].as_str()).unwrap_or_default().trim().to_string(),
        _ => String::new(),
    }
}

fn clip(text: &str) -> String {
    crate::hook::truncate(text, MAX_TEXT)
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

    const BASH: &str = r#"{"type":"assistant","timestamp":"2026-10-11T10:00:00Z","message":{"id":"m1","stop_reason":"tool_use","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo build --release\nmore","run_in_background":true}}]}}"#;
    const LAUNCHED: &str = r#"{"type":"user","timestamp":"2026-10-11T10:00:01Z","toolUseResult":{"backgroundTaskId":"b1"},"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Command running in background with ID: b1."}]}}"#;

    fn notification(status: &str) -> String {
        format!("<task-notification>\n<task-id>b1</task-id>\n<tool-use-id>t1</tool-use-id>\n<status>{status}</status>\n<summary>done</summary>\n</task-notification>")
    }

    #[test]
    fn background_shells_stay_open_until_notified() {
        let launched = unix_ts(&serde_json::json!({"timestamp": "2026-10-11T10:00:01Z"})).unwrap();
        assert_eq!(reader_for(&[BASH, LAUNCHED]).stats().background, [(launched, "cargo build --release".to_string())]);
        for status in ["completed", "failed", "killed"] {
            let done = serde_json::json!({"type": "user", "timestamp": "2026-10-11T10:05:00Z", "origin": {"kind": "task-notification"}, "message": {"content": notification(status)}}).to_string();
            assert!(reader_for(&[BASH, LAUNCHED, &done]).stats().background.is_empty(), "{status} closes it");
        }
        // A notification absorbed mid-turn is only recorded as a queued command and a queue operation.
        let queued = serde_json::json!({"type": "attachment", "attachment": {"type": "queued_command", "prompt": notification("completed")}}).to_string();
        let op = serde_json::json!({"type": "queue-operation", "operation": "enqueue", "content": notification("completed")}).to_string();
        assert!(reader_for(&[BASH, LAUNCHED, &queued]).stats().background.is_empty());
        assert!(reader_for(&[BASH, LAUNCHED, &op, &queued]).stats().background.is_empty());
        // Unrelated notifications change nothing.
        let other = op.replace("<task-id>b1", "<task-id>zz");
        assert_eq!(reader_for(&[BASH, LAUNCHED, &other]).stats().background.len(), 1);
    }

    #[test]
    fn timed_out_commands_moved_to_the_background_are_tracked() {
        let fg = BASH.replace(r#","run_in_background":true"#, "");
        let moved = LAUNCHED.replace(r#"{"backgroundTaskId":"b1"}"#, r#"{"backgroundTaskId":"b1","timedOutAfterMs":120000}"#);
        assert_eq!(reader_for(&[&fg, &moved]).stats().background.len(), 1);
        // A foreground command that finishes normally opens nothing.
        let finished = LAUNCHED.replace(r#"{"backgroundTaskId":"b1"}"#, r#"{"stdout":"ok"}"#);
        assert!(reader_for(&[&fg, &finished]).stats().background.is_empty());
        // A background agent is not a shell.
        let agent = r#"{"type":"user","timestamp":"2026-10-11T10:00:01Z","toolUseResult":{"agentId":"a1"},"message":{"content":[{"type":"tool_result","tool_use_id":"t9","content":"Async agent launched successfully. agentId: a1"}]}}"#;
        assert!(reader_for(&[agent]).stats().background.is_empty());
    }

    #[test]
    fn turn_end_is_recorded() {
        let at = |iso: &str| chrono::DateTime::parse_from_rfc3339(iso).unwrap().timestamp() as f64;
        let ended = reader_for(&[
            r#"{"type":"user","timestamp":"2026-10-11T10:00:00Z","message":{"content":"go"}}"#,
            r#"{"type":"system","subtype":"turn_duration","durationMs":5000,"timestamp":"2026-10-11T10:00:05Z"}"#,
        ]);
        assert_eq!(ended.stats().turn_ended_at, Some(at("2026-10-11T10:00:05Z")));
        let interrupted = reader_for(&[
            r#"{"type":"user","timestamp":"2026-10-11T10:00:00Z","message":{"content":"go"}}"#,
            r#"{"type":"user","timestamp":"2026-10-11T10:00:09Z","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
        ]);
        assert_eq!(interrupted.stats().turn_ended_at, Some(at("2026-10-11T10:00:09Z")));
        assert_eq!(reader_for(&[r#"{"type":"user","timestamp":"2026-10-11T10:00:00Z","message":{"content":"go"}}"#]).stats().turn_ended_at, None);
    }

    #[test]
    fn busy_time_skips_idle_gaps() {
        let r = reader_for(&[
            // A finished turn: Claude Code's recorded duration counts, not the gap after it.
            r#"{"type":"user","timestamp":"2026-10-11T10:00:00Z","message":{"content":"first"}}"#,
            r#"{"type":"assistant","timestamp":"2026-10-11T10:00:30Z","message":{"id":"m1"}}"#,
            r#"{"type":"system","subtype":"turn_duration","durationMs":40000,"timestamp":"2026-10-11T10:00:40Z"}"#,
            // An interrupted turn with no recorded duration counts up to its last entry.
            r#"{"type":"user","timestamp":"2026-11-11T09:00:00Z","message":{"content":"a month later"}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-11T09:00:20Z","message":{"id":"m2"}}"#,
            r#"{"type":"user","timestamp":"2026-11-11T09:00:21Z","message":{"content":"[Request interrupted by user]"}}"#,
            // An interrupted turn whose duration is recorded uses that duration.
            r#"{"type":"user","timestamp":"2026-11-11T09:30:00Z","message":{"content":"try again"}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-11T09:30:05Z","message":{"id":"m4"}}"#,
            r#"{"type":"user","timestamp":"2026-11-11T09:30:06Z","message":{"content":"[Request interrupted by user]"}}"#,
            r#"{"type":"system","subtype":"turn_duration","durationMs":7000,"timestamp":"2026-11-11T09:30:06Z"}"#,
            // A message sent mid-turn: the turn's recorded duration replaces the estimate.
            r#"{"type":"user","timestamp":"2026-11-11T10:00:00Z","message":{"content":"go"}}"#,
            r#"{"type":"assistant","timestamp":"2026-11-11T10:00:10Z","message":{"id":"m3"}}"#,
            r#"{"type":"user","timestamp":"2026-11-11T10:00:15Z","message":{"content":"also this"}}"#,
            r#"{"type":"system","subtype":"turn_duration","durationMs":60000,"timestamp":"2026-11-11T10:01:00Z"}"#,
            r#"{"type":"user","timestamp":"2026-11-11T11:00:00Z","message":{"content":"still running"}}"#,
        ]);
        let s = r.stats();
        assert_eq!(s.busy_secs, 40.0 + 20.0 + 7.0 + 60.0);
        let open = chrono::DateTime::parse_from_rfc3339("2026-11-11T11:00:00Z").unwrap().timestamp() as f64;
        assert_eq!(s.turn_started, Some(open));
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
    fn timeline_records_prompts_skills_agents_questions_and_replies() {
        let r = reader_for(&[
            r#"{"type":"user","timestamp":"2026-10-11T10:00:00Z","message":{"content":"add dark mode"}}"#,
            r#"{"type":"assistant","message":{"id":"m1","stop_reason":"tool_use","usage":{"output_tokens":3},"content":[{"type":"text","text":"Let me look."}]}}"#,
            r#"{"type":"assistant","message":{"id":"m1","stop_reason":"tool_use","usage":{"output_tokens":3},"content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"subagent_type":"Explore","description":"Map styles"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"Styles live in theme.css"}]}]}}"#,
            r#"{"type":"assistant","message":{"id":"m2","stop_reason":"tool_use","usage":{"output_tokens":2},"content":[{"type":"tool_use","id":"t2","name":"AskUserQuestion","input":{"questions":[{"question":"Which default?"}]}}]}}"#,
            r#"{"type":"user","toolUseResult":{"answers":{"Which default?":"Dark"}},"message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"answered"}]}}"#,
            r#"{"type":"assistant","message":{"id":"m3","stop_reason":"end_turn","usage":{"output_tokens":5},"content":[{"type":"text","text":"Done: dark by default."}]}}"#,
            r#"{"type":"user","message":{"content":"<local-command-stdout>ok</local-command-stdout>"}}"#,
            r#"{"type":"user","message":{"content":"<command-message>review</command-message><command-name>/code-review</command-name><command-args>high</command-args>"}}"#,
        ]);
        let t = r.stats().timeline;
        let kinds: Vec<_> = t.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            [
                EventKind::Prompt("add dark mode".into()),
                EventKind::Agent {
                    agent_type: "Explore".into(),
                    description: "Map styles".into(),
                    tool_use_id: "t1".into(),
                    result: Some("Styles live in theme.css".into()),
                },
                EventKind::Question { question: "Which default?".into(), answer: Some("Dark".into()) },
                EventKind::Response("Done: dark by default.".into()),
                EventKind::Prompt("/code-review high".into()),
                EventKind::Skill(0),
            ]
        );
        assert_eq!(t[0].at.as_deref(), Some("2026-10-11T10:00:00Z"));
        assert_eq!(t[0].tokens.output, 10, "the prompt carries its whole turn's tokens");
    }

    #[test]
    fn background_agent_report_comes_from_task_notification() {
        let r = reader_for(&[
            r#"{"type":"assistant","message":{"id":"m1","stop_reason":"tool_use","content":[{"type":"tool_use","id":"t1","name":"Agent","input":{"description":"Review"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Async agent launched successfully.\nagentId: a1 (internal ID)"}]}}"#,
            r#"{"type":"assistant","message":{"id":"m2","stop_reason":"tool_use","content":[{"type":"tool_use","id":"t2","name":"Agent","input":{"description":"Map"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"Async agent launched successfully.\nagentId: a2 (internal ID)"}]}}"#,
            r#"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"a1","body":"[Subagent hand-back] The report follows:\n  No bugs.\n  \n  1. **Done**"},"message":{"content":"<agent-message from=\"a1\">…</agent-message>"}}"#,
            r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>\n<tool-use-id>t1</tool-use-id>\n<result>This agent's report was delivered to you as a message from \"a1\".</result>\n</task-notification>"}}"#,
            r#"{"type":"user","origin":{"kind":"task-notification"},"message":{"content":"<task-notification>\n<tool-use-id>t2</tool-use-id>\n<summary>Agent \"Map\" finished</summary>\n</task-notification>"}}"#,
        ]);
        let reports: Vec<_> = r
            .stats()
            .timeline
            .into_iter()
            .map(|e| match e.kind {
                EventKind::Agent { result, .. } => result,
                other => panic!("unexpected event {other:?}"),
            })
            .collect();
        assert_eq!(reports, [Some("No bugs.\n\n1. **Done**".to_string()), Some("Agent \"Map\" finished".to_string())]);
    }

    #[test]
    fn declined_question_keeps_the_users_note() {
        let r = reader_for(&[
            r#"{"type":"assistant","message":{"id":"m1","stop_reason":"tool_use","content":[{"type":"tool_use","id":"t1","name":"AskUserQuestion","input":{"questions":[{"question":"Which stack?"}]}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"The user doesn't want to proceed. To tell you how to proceed, the user said:\nUse Rust.\nMore boilerplate"}]}}"#,
        ]);
        let t = r.stats().timeline;
        assert_eq!(t[0].kind, EventKind::Question { question: "Which stack?".into(), answer: Some("(declined) Use Rust.".into()) });
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
