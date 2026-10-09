//! Token usage and skill attribution from Claude Code transcript JSONL files.
//!
//! Per-skill tokens use window attribution: once a skill is invoked (via the Skill tool
//! or a typed `/skill` command), every following assistant turn is charged to it until
//! the next real user prompt or the next skill.

use crate::jsonl::JsonlTail;
use crate::model::{SkillStat, TokenUsage, TranscriptStats};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// Built-in slash commands that are not skills.
const BUILTIN_COMMANDS: &[&str] = &[
    "add-dir", "agents", "artifacts", "clear", "compact", "config", "context", "cost", "doctor",
    "effort", "exit", "export", "fast", "help", "hooks", "ide", "init", "login", "logout", "mcp",
    "memory", "model", "permissions", "plan", "plugin", "plugins", "release-notes", "resume",
    "rewind", "status", "statusline", "terminal-setup", "theme", "usage", "vim",
];

pub struct TranscriptReader {
    tail: JsonlTail,
    /// message.id -> (usage, skill window it ran in). Streamed messages repeat their id.
    messages: HashMap<String, (TokenUsage, Option<String>)>,
    skill_calls: BTreeMap<String, u32>,
    current_skill: Option<String>,
    model: Option<String>,
    title: Option<String>,
}

impl TranscriptReader {
    pub fn new(path: PathBuf) -> Self {
        Self {
            tail: JsonlTail::new(path),
            messages: HashMap::new(),
            skill_calls: BTreeMap::new(),
            current_skill: None,
            model: None,
            title: None,
        }
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
        let id = msg["id"].as_str().or(e["uuid"].as_str()).unwrap_or_default().to_string();
        let skill = self.current_skill.clone();
        self.messages
            .entry(id)
            .and_modify(|(u, _)| *u = usage)
            .or_insert((usage, skill));

        for block in msg["content"].as_array().into_iter().flatten() {
            if block["type"] == "tool_use" && block["name"] == "Skill" {
                if let Some(name) = block["input"]["skill"].as_str() {
                    self.open_skill(name);
                }
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
        // A new real prompt closes the skill window; a typed /skill opens a new one.
        self.current_skill = None;
        if let Some(cmd) = slash_command(text) {
            if !BUILTIN_COMMANDS.contains(&cmd) {
                self.open_skill(cmd);
            }
        }
    }

    fn open_skill(&mut self, name: &str) {
        *self.skill_calls.entry(name.to_string()).or_default() += 1;
        self.current_skill = Some(name.to_string());
    }

    pub fn stats(&self) -> TranscriptStats {
        let mut tokens = TokenUsage::default();
        let mut skills: BTreeMap<String, SkillStat> = self
            .skill_calls
            .iter()
            .map(|(k, &calls)| (k.clone(), SkillStat { calls, tokens: TokenUsage::default() }))
            .collect();
        for (usage, skill) in self.messages.values() {
            tokens += *usage;
            if let Some(s) = skill {
                skills.entry(s.clone()).or_default().tokens += *usage;
            }
        }
        TranscriptStats { tokens, skills, model: self.model.clone(), title: self.title.clone() }
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

/// Reads `agentType` from a subagent's `.meta.json`, if present.
pub fn subagent_type(dir: &std::path::Path, agent_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(format!("agent-{agent_id}.meta.json"))).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v["agentType"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn reader_for(lines: &[&str]) -> TranscriptReader {
        let dir = std::env::temp_dir().join(format!("marsad-tr-{}-{}", std::process::id(), lines.len()));
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
}
