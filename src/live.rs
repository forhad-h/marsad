//! `marsad status`: the dashboard's live view (status, context left, usage limits) as
//! a table or JSON, so Claude can read it too.

use crate::events;
use crate::jsonl::JsonlTail;
use crate::model::{RateWindow, Session, Status};
use crate::report::Format;
use crate::store::QueryResult;
use crate::transcript::TranscriptReader;
use clap::Args;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Args)]
pub struct StatusArgs {
    /// Also list sessions that have ended.
    #[arg(long)]
    all: bool,
    #[arg(long, value_enum, default_value = "table")]
    format: Format,
}

/// Rebuilds session state the same way the dashboard does: replay the hook log, drop
/// sessions whose process is gone, attach status line reports, then read transcripts.
fn load_sessions(include_ended: bool) -> HashMap<String, Session> {
    let dir = crate::data_dir();
    let mut sessions = HashMap::new();
    for log in ["events.1.jsonl", "events.jsonl"] {
        for ev in JsonlTail::new(dir.join(log)).read_new() {
            events::apply(&mut sessions, &ev);
        }
    }
    crate::app::mark_exited(&mut sessions, &mut sysinfo::System::new());
    sessions.retain(|_, s| include_ended || s.status != Status::Ended);
    crate::app::load_snapshots(&mut sessions);
    for s in sessions.values_mut() {
        if let Some(path) = s.transcript_path.clone() {
            let mut reader = TranscriptReader::new(path);
            reader.poll();
            s.stats = reader.stats();
        }
    }
    sessions
}

fn rate_json(w: Option<RateWindow>) -> Value {
    w.map_or(Value::Null, |w| json!({ "used_pct": w.used_pct, "resets_at": w.resets_at }))
}

pub fn status(a: StatusArgs) -> anyhow::Result<()> {
    let sessions = load_sessions(a.all);
    let compact_window = crate::model::auto_compact_window();
    let now = crate::now();

    let mut list: Vec<&Session> = sessions.values().collect();
    list.sort_by(|x, y| x.status.cmp(&y.status).then(y.last_event.total_cmp(&x.last_event)));

    let columns = [
        "session_id", "project", "status", "status_secs", "activity", "model", "context_used",
        "context_limit", "context_left", "context_pct", "context_estimated", "tokens_total", "title",
    ];
    let rows = list
        .iter()
        .map(|s| {
            let ctx = s.context(compact_window);
            let model = s.snapshot.as_ref().and_then(|x| x.model.clone()).or(s.model.clone()).or(s.stats.model.clone());
            vec![
                json!(s.id),
                json!(s.project()),
                json!(s.status.as_str()),
                json!((now - s.status_since).max(0.0).round() as i64),
                json!(s.detail),
                json!(model),
                json!(ctx.map(|c| c.used)),
                json!(ctx.map(|c| c.limit)),
                json!(ctx.map(|c| c.left())),
                json!(ctx.map(|c| (c.used_ratio() * 1000.0).round() / 10.0)),
                json!(ctx.map(|c| c.estimated)),
                json!(s.total_tokens().total()),
                json!(s.title.clone().or(s.stats.title.clone())),
            ]
        })
        .collect();
    let table = QueryResult { columns: columns.iter().map(|c| c.to_string()).collect(), rows };
    let limits = crate::app::latest_rate_limits(&sessions);

    match a.format {
        Format::Json => {
            let rows: Vec<Value> = table
                .rows
                .iter()
                .map(|r| Value::Object(table.columns.iter().cloned().zip(r.iter().cloned()).collect()))
                .collect();
            let out = json!({
                "sessions": rows,
                "rate_limits": limits.map(|(five, seven)| json!({
                    "five_hour": rate_json(five),
                    "seven_day": rate_json(seven),
                })),
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        format => {
            crate::report::print(&table, format);
            if let Some((five, seven)) = limits {
                let fmt = |label: &str, w: Option<RateWindow>| {
                    w.map(|w| match w.resets_at {
                        Some(r) => {
                            let mins = ((r - now).max(0.0) / 60.0) as i64;
                            format!("{label} {:.0}% (resets in {}h{:02}m)", w.used_pct, mins / 60, mins % 60)
                        }
                        None => format!("{label} {:.0}%", w.used_pct),
                    })
                };
                let parts: Vec<String> = [fmt("5h", five), fmt("7d", seven)].into_iter().flatten().collect();
                eprintln!("Usage limits: {}", parts.join(", "));
            }
        }
    }
    Ok(())
}
