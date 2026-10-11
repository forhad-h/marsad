//! `marsad statusline`: Claude Code's status line command.
//! Saves the exact context-window and rate-limit figures for the dashboard, then prints
//! either the user's original status line (if marsad wrapped one) or a compact default.

use crate::model::{Level, RateWindow, StatusSnapshot, auto_compact_window, context_info, fmt_tokens};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn status_dir() -> PathBuf {
    crate::data_dir().join("status")
}

/// Where `marsad install` keeps a pre-existing statusLine setting it replaced.
pub fn original_path() -> PathBuf {
    crate::data_dir().join("statusline-original.json")
}

pub fn run() -> anyhow::Result<()> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let raw: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let _ = save(&raw);

    if let Some(cmd) = original_command() {
        let out = run_original(&cmd, &input).unwrap_or_default();
        std::io::stdout().write_all(&out)?;
        return Ok(());
    }
    if let Some(snap) = parse(&raw) {
        println!("{}", default_line(&snap));
    }
    Ok(())
}

fn save(raw: &Value) -> anyhow::Result<()> {
    let Some(id) = raw["session_id"].as_str().filter(|id| !id.contains(['/', '\\', '.'])) else {
        return Ok(());
    };
    let dir = status_dir();
    std::fs::create_dir_all(&dir)?;
    let slim = json!({
        "ts": crate::now(),
        "model": raw["model"],
        "context_window": raw["context_window"],
        "rate_limits": raw["rate_limits"],
    });
    // Write then rename so the dashboard never reads a half-written file.
    let tmp = dir.join(format!("{id}.json.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&slim)?)?;
    std::fs::rename(tmp, dir.join(format!("{id}.json")))?;
    Ok(())
}

pub fn load(path: &Path) -> Option<StatusSnapshot> {
    let v: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let mut snap = parse(&v)?;
    snap.ts = v["ts"].as_f64().unwrap_or(0.0);
    Some(snap)
}

fn parse(v: &Value) -> Option<StatusSnapshot> {
    let cw = &v["context_window"];
    let window = cw["context_window_size"].as_u64();
    let cu = &cw["current_usage"];
    let used = if cu.is_object() {
        let n = |k: &str| cu[k].as_u64().unwrap_or(0);
        Some(n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens"))
    } else {
        cw["used_percentage"].as_f64().zip(window).map(|(p, w)| (p / 100.0 * w as f64) as u64)
    };
    let rate = |k: &str| {
        let r = &v["rate_limits"][k];
        r["used_percentage"].as_f64().map(|p| RateWindow { used_pct: p, resets_at: r["resets_at"].as_f64() })
    };
    let model = v["model"]["id"].as_str().or(v["model"]["display_name"].as_str()).map(str::to_string);
    if window.is_none() && model.is_none() {
        return None;
    }
    Some(StatusSnapshot {
        ts: 0.0,
        window,
        used,
        model,
        five_hour: rate("five_hour"),
        seven_day: rate("seven_day"),
    })
}

fn original_command() -> Option<String> {
    let v: Value = serde_json::from_slice(&std::fs::read(original_path()).ok()?).ok()?;
    v["command"].as_str().map(str::to_string)
}

/// A wrapped status line that takes longer than this is killed rather than stalling ours.
const ORIGINAL_TIMEOUT: Duration = Duration::from_secs(5);

fn run_original(cmd: &str, input: &str) -> Option<Vec<u8>> {
    let mut command = if cfg!(windows) { Command::new("cmd") } else { Command::new("sh") };
    command.args(if cfg!(windows) { ["/C", cmd] } else { ["-c", cmd] });
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().ok()?;
    // A command that never reads stdin closes the pipe early; its output is still wanted.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + ORIGINAL_TIMEOUT;
    while child.try_wait().ok()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    reader.join().ok()
}

fn default_line(snap: &StatusSnapshot) -> String {
    const DIM: &str = "\x1b[2m";
    const RESET: &str = "\x1b[0m";
    let color = |ratio: f64| match Level::of(ratio) {
        Level::Critical => "\x1b[31m",
        Level::Warn => "\x1b[33m",
        Level::Ok => "\x1b[32m",
    };
    let mut parts = Vec::new();
    if let Some(used) = snap.used {
        let ctx = context_info(used, snap.model.as_deref(), snap.window, auto_compact_window());
        parts.push(format!(
            "{}ctx {:.0}%{RESET} {DIM}· {} left{RESET}",
            color(ctx.used_ratio()),
            ctx.used_ratio() * 100.0,
            fmt_tokens(ctx.left())
        ));
    }
    for (label, w) in [("5h", snap.five_hour), ("7d", snap.seven_day)] {
        if let Some(w) = w {
            let reset = w.resets_at.map(|t| format!(" {DIM}↻{}{RESET}", crate::model::fmt_reset(t))).unwrap_or_default();
            parts.push(format!("{}{label} {:.0}%{RESET}{reset}", color(w.used_pct / 100.0), w.used_pct));
        }
    }
    parts.join(&format!(" {DIM}·{RESET} "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn wrapped_command_output_is_forwarded() {
        assert_eq!(run_original("cat", "hello").as_deref(), Some(&b"hello"[..]));
        // Never reads stdin and exits immediately: its output must still come through.
        assert_eq!(run_original("echo static", &"x".repeat(1 << 20)).as_deref(), Some(&b"static\n"[..]));
        assert_eq!(run_original("exit 3", "").as_deref(), Some(&b""[..]));
    }

    #[test]
    fn default_line_without_rate_limits_shows_only_context() {
        let snap = StatusSnapshot { window: Some(200_000), used: Some(180_000), ..Default::default() };
        let line = default_line(&snap);
        assert!(line.contains("ctx 90%") && line.contains("\x1b[31m"), "{line}");
        assert!(!line.contains("5h") && !line.contains("7d"));
    }

    #[test]
    fn default_line_shows_rate_limit_reset_times() {
        let now = crate::now();
        let snap = StatusSnapshot {
            five_hour: Some(RateWindow { used_pct: 42.0, resets_at: Some(now + 2.0 * 3600.0) }),
            seven_day: Some(RateWindow { used_pct: 18.0, resets_at: Some(now + 3.0 * 86400.0) }),
            ..Default::default()
        };
        let line = default_line(&snap);
        let five = crate::model::fmt_reset(now + 2.0 * 3600.0);
        let seven = crate::model::fmt_reset(now + 3.0 * 86400.0);
        assert!(line.contains(&format!("↻{five}")) && line.contains(&format!("↻{seven}")), "{line}");
        assert!(seven.contains(' '), "a reset days away names the day: {seven}");
    }

    #[test]
    fn parses_statusline_input() {
        let v = json!({
            "model": {"id": "claude-opus-5-5", "display_name": "Opus 5.5"},
            "context_window": {
                "context_window_size": 1000000,
                "used_percentage": 8,
                "current_usage": {"input_tokens": 10, "cache_creation_input_tokens": 90, "cache_read_input_tokens": 79900, "output_tokens": 5}
            },
            "rate_limits": {"five_hour": {"used_percentage": 23.5, "resets_at": 1738425600}}
        });
        let s = parse(&v).unwrap();
        assert_eq!((s.window, s.used), (Some(1_000_000), Some(80_000)));
        assert_eq!(s.five_hour.unwrap().used_pct, 23.5);
        assert!(s.seven_day.is_none());
        assert_eq!(s.model.as_deref(), Some("claude-opus-5-5"));
    }
}
