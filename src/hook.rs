//! `marsad hook`: called by Claude Code for every registered event.
//! Keeps only the fields the dashboard needs and appends one JSON line.

use serde_json::{Map, Value, json};
use std::fs::OpenOptions;
use std::io::{Read, Write};

/// Rotate the log once it grows past this size.
const MAX_LOG_BYTES: u64 = 20 * 1024 * 1024;

const KEEP: &[&str] = &[
    "session_id",
    "hook_event_name",
    "cwd",
    "transcript_path",
    "permission_mode",
    "agent_id",
    "agent_type",
    "source",
    "reason",
    "model",
    "session_title",
    "tool_name",
    "notification_type",
];

pub fn run() -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let raw: Value = serde_json::from_str(&input)?;
    let event = trim(&raw);

    let dir = crate::data_dir();
    std::fs::create_dir_all(&dir)?;
    let log = dir.join("events.jsonl");
    if std::fs::metadata(&log).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&log, dir.join("events.1.jsonl"));
    }
    let mut line = serde_json::to_vec(&event)?;
    line.push(b'\n');
    // Single write on an O_APPEND handle so concurrent hooks don't interleave.
    OpenOptions::new().create(true).append(true).open(log)?.write_all(&line)?;
    Ok(())
}

fn trim(raw: &Value) -> Value {
    let mut out = Map::new();
    out.insert("ts".into(), json!(crate::now()));
    for key in KEEP {
        if let Some(v) = raw.get(*key).filter(|v| !v.is_null()) {
            out.insert((*key).into(), v.clone());
        }
    }
    for key in ["message", "prompt", "error"] {
        if let Some(s) = raw.get(key).and_then(Value::as_str) {
            out.insert(key.into(), json!(truncate(s, 160)));
        }
    }
    if let Some(tool) = raw.get("tool_name").and_then(Value::as_str) {
        let summary = tool_summary(tool, raw.get("tool_input").unwrap_or(&Value::Null));
        if !summary.is_empty() {
            out.insert("summary".into(), json!(summary));
        }
    }
    let event = raw.get("hook_event_name").and_then(Value::as_str).unwrap_or("");
    if matches!(event, "SessionStart" | "UserPromptSubmit")
        && let Some(pid) = claude_pid() {
            out.insert("pid".into(), json!(pid));
        }
    Value::Object(out)
}

/// A short human description of what a tool call is doing.
pub fn tool_summary(tool: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let text = match tool {
        "Bash" => s("command").lines().next().unwrap_or("").to_string(),
        "Read" | "Edit" | "Write" | "MultiEdit" => file_name(s("file_path")),
        "NotebookEdit" => file_name(s("notebook_path")),
        "Grep" | "Glob" => s("pattern").to_string(),
        "Skill" => s("skill").to_string(),
        "Agent" | "Task" => format!("{} {}", s("subagent_type"), s("description")).trim().to_string(),
        "WebFetch" => s("url").to_string(),
        "WebSearch" => s("query").to_string(),
        "AskUserQuestion" => input
            .pointer("/questions/0/question")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    truncate(&text, 120)
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

/// The Claude Code process that ran this hook: the first ancestor that isn't a shell.
fn claude_pid() -> Option<u32> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    let mut pid = Pid::from_u32(std::process::id());
    for _ in 0..6 {
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing());
        let parent = sys.process(pid)?.parent()?;
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[parent]), true, ProcessRefreshKind::nothing());
        let name = sys.process(parent)?.name().to_string_lossy().to_lowercase();
        let name = name.trim_start_matches('-').trim_end_matches(".exe");
        if !matches!(name, "sh" | "bash" | "zsh" | "dash" | "fish" | "cmd" | "powershell" | "pwsh") {
            return Some(parent.as_u32());
        }
        pid = parent;
    }
    None
}
