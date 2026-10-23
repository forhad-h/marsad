//! Turns hook events into per-session state.

use crate::model::{Session, Status, Subagent};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

pub fn apply(sessions: &mut HashMap<String, Session>, ev: &Value) {
    let s = |k: &str| ev.get(k).and_then(Value::as_str);
    let (Some(id), Some(event)) = (s("session_id"), s("hook_event_name")) else { return };
    let ts = ev["ts"].as_f64().unwrap_or_else(crate::now);
    let session = sessions.entry(id.to_string()).or_insert_with(|| Session::new(id, ts));
    session.last_event = session.last_event.max(ts);

    if let Some(cwd) = s("cwd") {
        // Subagents can run elsewhere; keep the session's own directory once known.
        if session.cwd.is_empty() || ev.get("agent_id").is_none() {
            session.cwd = cwd.to_string();
        }
    }
    if let Some(p) = s("transcript_path").filter(|_| ev.get("agent_id").is_none()) {
        session.transcript_path = Some(PathBuf::from(p));
    }
    if let Some(m) = s("permission_mode") {
        session.permission_mode = Some(m.to_string());
    }
    if let Some(t) = s("session_title") {
        session.title = Some(t.to_string());
    }
    if let Some(pid) = ev["pid"].as_u64() {
        session.pid = Some(pid as u32);
    }
    if let Some(model) = model_name(&ev["model"]) {
        session.model = Some(model);
    }

    let summary = s("summary").unwrap_or("");
    let tool = s("tool_name").unwrap_or("");
    let tool_detail = || if summary.is_empty() { tool.to_string() } else { format!("{tool}: {summary}") };

    match event {
        "SessionStart" => {
            session.set_status(Status::Idle, ts);
            session.detail.clear();
        }
        "UserPromptSubmit" => {
            session.set_status(Status::Active, ts);
            session.detail = s("prompt").map(|p| format!("› {p}")).unwrap_or_default();
        }
        "PreToolUse" if tool == "AskUserQuestion" => {
            session.set_status(Status::Question, ts);
            session.detail = summary.to_string();
        }
        "PreToolUse" => {
            // A subagent's tool call shouldn't clear a pending prompt from the main agent.
            if !session.status.needs_user() || ev.get("agent_id").is_none() {
                session.set_status(Status::Active, ts);
                session.detail = tool_detail();
            }
        }
        "PostToolUse" | "PostToolUseFailure" => {
            if !session.status.needs_user() || ev.get("agent_id").is_none() {
                session.set_status(Status::Active, ts);
            }
        }
        // Claude Code also asks permission to show a question; it is still a question.
        "PermissionRequest" if tool == "AskUserQuestion" => {
            session.set_status(Status::Question, ts);
            session.detail = summary.to_string();
        }
        "PermissionRequest" => {
            session.set_status(Status::Approve, ts);
            session.detail = tool_detail();
        }
        "Notification" => match s("notification_type").unwrap_or("") {
            "permission_prompt" if session.status == Status::Question => {}
            "permission_prompt" => {
                session.set_status(Status::Approve, ts);
                if session.detail.is_empty() {
                    session.detail = s("message").unwrap_or("").to_string();
                }
            }
            "elicitation_dialog" | "elicitation_url_dialog" | "agent_needs_input" => {
                session.set_status(Status::Question, ts);
                session.detail = s("message").unwrap_or("").to_string();
            }
            "idle_prompt" => session.set_status(Status::Idle, ts),
            _ => {}
        },
        "Elicitation" => {
            session.set_status(Status::Question, ts);
            session.detail = s("message").unwrap_or("MCP input requested").to_string();
        }
        "Stop" => {
            session.set_status(Status::Idle, ts);
            session.detail.clear();
        }
        "StopFailure" => {
            session.set_status(Status::Error, ts);
            session.detail = s("error").or(s("message")).unwrap_or("API error").to_string();
        }
        "SubagentStart" => {
            if let Some(aid) = s("agent_id") {
                session.subagents.entry(aid.to_string()).or_insert_with(|| Subagent {
                    agent_type: s("agent_type").unwrap_or("agent").to_string(),
                    running: true,
                    ..Default::default()
                });
            }
            if !session.status.needs_user() {
                session.set_status(Status::Active, ts);
            }
        }
        "SubagentStop" => {
            if let Some(a) = s("agent_id").and_then(|aid| session.subagents.get_mut(aid)) {
                a.running = false;
            }
        }
        "PreCompact" => {
            session.set_status(Status::Active, ts);
            session.detail = "compacting context".into();
        }
        "SessionEnd" => {
            session.set_status(Status::Ended, ts);
            session.detail = s("reason").unwrap_or("").to_string();
        }
        _ => {}
    }
}

fn model_name(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => ["display_name", "id"]
            .iter()
            .find_map(|k| o.get(*k).and_then(Value::as_str))
            .map(str::to_string),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(events: &[Value]) -> Session {
        let mut map = HashMap::new();
        for (i, e) in events.iter().enumerate() {
            let mut e = e.clone();
            e["session_id"] = json!("s1");
            e["ts"] = json!(i as f64);
            apply(&mut map, &e);
        }
        map.remove("s1").unwrap()
    }

    #[test]
    fn status_transitions() {
        let s = run(&[
            json!({"hook_event_name": "SessionStart", "cwd": "/p/web", "pid": 42}),
            json!({"hook_event_name": "UserPromptSubmit", "prompt": "fix it"}),
            json!({"hook_event_name": "PreToolUse", "tool_name": "Bash", "summary": "npm test"}),
            json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash", "summary": "npm test"}),
        ]);
        assert_eq!(s.status, Status::Approve);
        assert_eq!(s.detail, "Bash: npm test");
        assert_eq!(s.project(), "web");
        assert_eq!(s.pid, Some(42));
    }

    #[test]
    fn question_then_answer_then_idle() {
        let ask = json!({"hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion", "summary": "Which?"});
        assert_eq!(run(std::slice::from_ref(&ask)).status, Status::Question);
        let done = run(&[ask, json!({"hook_event_name": "PostToolUse"}), json!({"hook_event_name": "Stop"})]);
        assert_eq!(done.status, Status::Idle);
    }

    #[test]
    fn question_is_not_reported_as_approval() {
        let ask = json!({"hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion", "summary": "Which?"});
        let perm = json!({"hook_event_name": "PermissionRequest", "tool_name": "AskUserQuestion", "summary": "Which?"});
        let note = json!({"hook_event_name": "Notification", "notification_type": "permission_prompt", "message": "Claude needs your permission"});
        let s = run(&[ask, perm.clone(), note.clone()]);
        assert_eq!((s.status, s.detail.as_str()), (Status::Question, "Which?"));
        // A question's permission request alone, without its PreToolUse, still reads as a question.
        assert_eq!(run(std::slice::from_ref(&perm)).status, Status::Question);
        // A permission notification for anything else is an approval.
        assert_eq!(run(&[note]).status, Status::Approve);
    }

    #[test]
    fn later_event_supplies_a_missing_pid() {
        let s = run(&[
            json!({"hook_event_name": "SessionStart"}),
            json!({"hook_event_name": "PreToolUse", "tool_name": "Read", "pid": 7}),
        ]);
        assert_eq!(s.pid, Some(7));
    }

    #[test]
    fn subagent_tool_does_not_hide_approval() {
        let s = run(&[
            json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash"}),
            json!({"hook_event_name": "PreToolUse", "tool_name": "Read", "agent_id": "a1"}),
        ]);
        assert_eq!(s.status, Status::Approve);
    }

    #[test]
    fn subagents_tracked() {
        let s = run(&[
            json!({"hook_event_name": "SubagentStart", "agent_id": "a1", "agent_type": "Explore"}),
            json!({"hook_event_name": "SubagentStop", "agent_id": "a1"}),
            json!({"hook_event_name": "SessionEnd", "reason": "prompt_input_exit"}),
        ]);
        assert!(!s.subagents["a1"].running);
        assert_eq!(s.subagents["a1"].agent_type, "Explore");
        assert_eq!(s.status, Status::Ended);
    }
}
