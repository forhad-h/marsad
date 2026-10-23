//! `marsad context`: how full a session's context is, so a session can decide to hand off before
//! auto-compact. With `--hook` it is also the UserPromptSubmit hook that tells a session, once per
//! crossing, that its context passed the warning threshold.

use crate::model::{ContextInfo, Session, auto_compact_window, context_info, fmt_tokens};
use crate::report::Format;
use clap::Args;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Args)]
pub struct ContextArgs {
    /// Session id or id prefix (default: the session this command runs in).
    #[arg(long)]
    session: Option<String>,
    /// Set the automatic handoff warning: a percent of the auto-compact point, or `off`.
    #[arg(long)]
    warn: Option<String>,
    #[arg(long, value_enum, default_value = "table")]
    format: Format,
    /// Hook entrypoint called by Claude Code on UserPromptSubmit (reads hook JSON on stdin).
    #[arg(long, hide = true)]
    hook: bool,
}

pub fn run(a: ContextArgs) -> anyhow::Result<()> {
    if a.hook {
        // Never fail or print anything but the note: a hook must not disturb the session.
        if let Some(out) = hook() {
            println!("{out}");
        }
        return Ok(());
    }
    if let Some(warn) = a.warn {
        let pct = match warn.as_str() {
            "off" | "0" => None,
            n => Some(n.trim_end_matches('%').parse::<u32>().ok().filter(|p| (1..=100).contains(p)).ok_or_else(|| {
                anyhow::anyhow!("expected a percent from 1 to 100, or `off`, got {n:?}")
            })?),
        };
        crate::settings::set_context_warning(pct)?;
        println!("{}", warning_label(pct));
        return Ok(());
    }

    let sessions = crate::live::load_sessions(true);
    let env_id = std::env::var("CLAUDE_CODE_SESSION_ID").ok();
    let session = find_session(&sessions, a.session.as_deref(), env_id.as_deref(), &ancestors())?;
    let ctx = session.context(auto_compact_window());
    let warn = crate::settings::context_warning();
    match a.format {
        Format::Json => {
            let out = json!({
                "session_id": session.id,
                "project": session.project(),
                "tokens_total": session.total_tokens().total(),
                "context_used": ctx.map(|c| c.used),
                "context_limit": ctx.map(|c| c.limit),
                "context_left": ctx.map(|c| c.left()),
                "context_pct": ctx.map(|c| (c.used_ratio() * 1000.0).round() / 10.0),
                "context_estimated": ctx.map(|c| c.estimated),
                "warn_pct": warn,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        _ => {
            let id: String = session.id.chars().take(8).collect();
            println!("Session {id} ({})", session.project());
            match ctx {
                Some(c) => println!(
                    "Context: {} of {} before auto-compact ({:.0}% full), {} left{}",
                    fmt_tokens(c.used),
                    fmt_tokens(c.limit),
                    c.used_ratio() * 100.0,
                    fmt_tokens(c.left()),
                    if c.estimated { " (window estimated)" } else { "" }
                ),
                None => println!("Context: unknown (no API call recorded yet)"),
            }
            println!("Tokens used: {} (main agent and subagents)", fmt_tokens(session.total_tokens().total()));
            println!("{}", warning_label(warn));
        }
    }
    Ok(())
}

fn warning_label(pct: Option<u32>) -> String {
    match pct {
        Some(p) => format!("Handoff warning at {p}% of the auto-compact point (`marsad context --warn <pct|off>`)."),
        None => "Handoff warning is off (`marsad context --warn 80` turns it on).".into(),
    }
}

/// This process and its ancestors, nearest first.
fn ancestors() -> Vec<u32> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    let mut out = vec![std::process::id()];
    let mut pid = Pid::from_u32(std::process::id());
    for _ in 0..12 {
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing());
        let Some(parent) = sys.process(pid).and_then(|p| p.parent()) else { break };
        out.push(parent.as_u32());
        pid = parent;
    }
    if let Some(claude) = std::env::var("CLAUDE_PID").ok().and_then(|p| p.parse().ok()) {
        out.push(claude);
    }
    out
}

/// The session to report: an explicit id, then Claude Code's session id variable, then the
/// session whose Claude Code process is an ancestor of this one.
fn find_session<'a>(
    sessions: &'a HashMap<String, Session>,
    explicit: Option<&str>,
    env_id: Option<&str>,
    ancestors: &[u32],
) -> anyhow::Result<&'a Session> {
    let by_prefix = |id: &str| {
        let mut hits: Vec<&Session> = sessions.values().filter(|s| s.id.starts_with(id)).collect();
        hits.sort_by(|a, b| b.last_event.total_cmp(&a.last_event));
        hits.first().copied()
    };
    if let Some(id) = explicit {
        return by_prefix(id).ok_or_else(|| anyhow::anyhow!("no session matches {id:?}"));
    }
    if let Some(s) = env_id.filter(|id| !id.is_empty()).and_then(by_prefix) {
        return Ok(s);
    }
    // The nearest ancestor wins, so a nested `claude` reports itself rather than its parent.
    for pid in ancestors {
        let mut hits: Vec<&Session> = sessions.values().filter(|s| s.pid == Some(*pid)).collect();
        hits.sort_by(|a, b| b.last_event.total_cmp(&a.last_event));
        if let Some(s) = hits.first() {
            return Ok(s);
        }
    }
    anyhow::bail!("not running inside a Claude Code session marsad knows; pass --session <id>")
}

// ---------- the UserPromptSubmit hook ----------

/// Markers for sessions already warned since they last crossed the threshold.
fn warned_dir() -> PathBuf {
    crate::data_dir().join("context-warned")
}

/// Markers older than this belong to sessions long gone.
const MARKER_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(30 * 86_400);

/// Reads the hook input and returns the JSON to print when the session newly crossed the threshold.
fn hook() -> Option<String> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).ok()?;
    let raw: Value = serde_json::from_str(&input).ok()?;
    let threshold = crate::settings::context_warning()?;
    let id = raw["session_id"].as_str().filter(|id| !id.is_empty() && !id.contains(['/', '\\', '.']))?;
    let snapshot = crate::statusline::load(&crate::statusline::status_dir().join(format!("{id}.json")));
    let tail = raw["transcript_path"].as_str().and_then(|p| last_context(Path::new(p)));
    let ctx = fresher_context(snapshot.as_ref(), tail.as_ref())?;
    let pct = ctx.used_ratio() * 100.0;

    let dir = warned_dir();
    let marker = dir.join(id);
    let (emit, warned) = decide(pct, threshold, marker.exists());
    if warned {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&marker, b"").ok()?;
    } else {
        let _ = std::fs::remove_file(&marker);
    }
    clean_markers(&dir);
    emit.then(|| {
        json!({
            "hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": note(pct),
            }
        })
        .to_string()
    })
}

/// (whether to warn now, whether the session counts as warned afterwards). Dropping below the
/// threshold, for example after a compact or `/clear`, re-arms the warning.
fn decide(pct: f64, threshold: u32, warned: bool) -> (bool, bool) {
    let over = pct >= threshold as f64;
    (over && !warned, over)
}

fn note(pct: f64) -> String {
    format!(
        "[marsad] This session's context is {pct:.0}% full; Claude Code auto-compacts at 100%. \
         Consider writing a handoff prompt for a fresh session now, before continuing."
    )
}

fn clean_markers(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > MARKER_MAX_AGE);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Context fill from the latest main-agent API call in the transcript, found by reading only its tail.
struct TailContext {
    used: u64,
    at: f64,
    model: Option<String>,
}

const TAIL_BYTES: u64 = 512 * 1024;

fn last_context(path: &Path) -> Option<TailContext> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    // The first line may be cut in half when reading from the middle of the file.
    let lines = text.lines().skip(usize::from(start > 0));
    lines.collect::<Vec<_>>().into_iter().rev().find_map(|line| {
        let e: Value = serde_json::from_str(line).ok()?;
        if e["type"] != "assistant" || e["isSidechain"].as_bool().unwrap_or(false) {
            return None;
        }
        let u = crate::transcript::parse_usage(&e["message"]["usage"]);
        let used = u.input + u.cache_read + u.cache_write;
        (used > 0).then(|| TailContext {
            used,
            at: crate::transcript::unix_ts(&e).unwrap_or(0.0),
            model: e["message"]["model"].as_str().map(str::to_string),
        })
    })
}

/// Whichever of the status line snapshot and the transcript reported more recently, as
/// `Session::context` decides for the dashboard.
fn fresher_context(snap: Option<&crate::model::StatusSnapshot>, tail: Option<&TailContext>) -> Option<ContextInfo> {
    let from_snap = snap.and_then(|s| s.used.map(|u| (u, s.ts)));
    let used = match (tail.map(|t| (t.used, t.at)), from_snap) {
        (Some(t), Some(s)) => if s.1 > t.1 { s.0 } else { t.0 },
        (t, s) => t.or(s)?.0,
    };
    let model = snap.and_then(|s| s.model.clone()).or_else(|| tail.and_then(|t| t.model.clone()));
    Some(context_info(used, model.as_deref(), snap.and_then(|s| s.window), auto_compact_window()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StatusSnapshot;

    #[test]
    fn warns_once_per_crossing() {
        // Covers AE4.
        let mut warned = false;
        let mut notes = Vec::new();
        for pct in [78.0, 83.0, 90.0, 20.0, 85.0] {
            let (emit, now) = decide(pct, 80, warned);
            warned = now;
            if emit {
                notes.push(pct);
            }
        }
        assert_eq!(notes, [83.0, 85.0]);
    }

    fn session(id: &str, pid: Option<u32>, last: f64) -> Session {
        let mut s = Session::new(id, 0.0);
        s.pid = pid;
        s.last_event = last;
        s
    }

    #[test]
    fn finds_the_session_it_runs_in() {
        let sessions: HashMap<String, Session> = [
            session("aaaa1111", Some(42), 1.0),
            session("bbbb2222", Some(7), 2.0),
            session("bbbb3333", None, 3.0),
        ]
        .into_iter()
        .map(|s| (s.id.clone(), s))
        .collect();
        let find = |explicit, env, pids: &[u32]| find_session(&sessions, explicit, env, pids).map(|s| s.id.clone());
        assert_eq!(find(None, None, &[100, 42, 1]).unwrap(), "aaaa1111", "by an ancestor pid");
        assert_eq!(find(Some("bbbb2"), None, &[42]).unwrap(), "bbbb2222", "an explicit id wins");
        assert_eq!(find(None, Some("bbbb3333"), &[42]).unwrap(), "bbbb3333", "then Claude Code's session id");
        assert_eq!(find(Some("bbbb"), None, &[]).unwrap(), "bbbb3333", "a prefix picks the latest");
        let err = find(Some("zzz"), None, &[]).unwrap_err().to_string();
        assert!(err.contains("zzz"), "{err}");
        assert!(find(None, None, &[1, 2]).is_err());
    }

    #[test]
    fn hook_reads_the_fresher_context_source() {
        let snap = |used, ts| StatusSnapshot { ts, used: Some(used), window: Some(200_000), ..Default::default() };
        let tail = |used, at| TailContext { used, at, model: None };
        let pct = |c: Option<ContextInfo>| (c.unwrap().used_ratio() * 100.0).round();
        // An older snapshot above the threshold loses to a newer, post-compact transcript entry.
        assert_eq!(pct(fresher_context(Some(&snap(170_000, 10.0)), Some(&tail(40_000, 20.0)))), 20.0);
        assert_eq!(pct(fresher_context(Some(&snap(170_000, 30.0)), Some(&tail(40_000, 20.0)))), 85.0);
        assert!(fresher_context(None, None).is_none());
    }

    #[test]
    fn transcript_tail_finds_the_latest_main_agent_call() {
        let dir = std::env::temp_dir().join(format!("marsad-context-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("t.jsonl");
        let call = |used: u64, side: bool| {
            format!(
                r#"{{"type":"assistant","isSidechain":{side},"timestamp":"2026-10-23T10:00:00Z","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":{used}}}}}}}"#
            )
        };
        // Padding pushes the first call out of the tail window.
        let padding = format!("{{\"type\":\"user\",\"message\":{{\"content\":\"{}\"}}}}", "x".repeat(TAIL_BYTES as usize));
        let text = [call(1, false), padding, call(150_000, false), call(9, true), "{\"type\":".into()].join("\n");
        std::fs::write(&file, text).unwrap();
        let got = last_context(&file).unwrap();
        assert_eq!((got.used, got.model.as_deref()), (150_000, Some("claude-opus-5-5")));
        assert!(last_context(&dir.join("missing.jsonl")).is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
