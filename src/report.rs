//! `marsad report` and `marsad sql`: usage history for humans and for Claude.

use crate::store::{self, QueryResult};
use clap::{Args, ValueEnum};
use serde_json::{Map, Value};

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Table,
    Json,
    Csv,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum By {
    Day,
    Session,
    Skill,
    Project,
    Model,
    Agent,
}

#[derive(Args)]
pub struct ReportArgs {
    /// What to group by.
    #[arg(long, value_enum, default_value = "session")]
    by: By,
    /// Only the last N days (1 = today).
    #[arg(long, conflicts_with_all = ["date", "from"])]
    days: Option<u32>,
    /// A single local day, YYYY-MM-DD.
    #[arg(long)]
    date: Option<String>,
    /// Start of a day range, YYYY-MM-DD (inclusive).
    #[arg(long)]
    from: Option<String>,
    /// End of a day range, YYYY-MM-DD (inclusive).
    #[arg(long)]
    to: Option<String>,
    /// Only this session (id or id prefix).
    #[arg(long)]
    session: Option<String>,
    /// Only sessions whose project name contains this text.
    #[arg(long)]
    project: Option<String>,
    /// Maximum rows.
    #[arg(long, default_value_t = 25)]
    limit: u32,
    #[arg(long, value_enum, default_value = "table")]
    format: Format,
    /// Don't index new transcript data first.
    #[arg(long)]
    no_sync: bool,
}

#[derive(Args)]
pub struct SqlArgs {
    /// A read-only SQL query against the usage database (see `marsad schema`).
    query: String,
    #[arg(long, value_enum, default_value = "table")]
    format: Format,
    /// Don't index new transcript data first.
    #[arg(long)]
    no_sync: bool,
}

pub fn sync_now() -> anyhow::Result<()> {
    let r = store::open_and_sync()?;
    eprintln!("Indexed {} changed transcript(s) of {} from the last {} days.", r.updated, r.scanned, store::RETENTION_DAYS);
    if r.failed > 0 {
        eprintln!("{} transcript(s) could not be indexed; they will be retried on the next sync.", r.failed);
    }
    Ok(())
}

fn ensure_synced(skip: bool) -> anyhow::Result<()> {
    if !skip {
        store::open_and_sync()?;
    }
    Ok(())
}

pub fn report(a: ReportArgs) -> anyhow::Result<()> {
    ensure_synced(a.no_sync)?;
    let (from, to) = match (&a.date, a.days) {
        (Some(d), _) => (Some(d.clone()), Some(d.clone())),
        (None, Some(n)) => (Some(store::day_n_ago(n)), None),
        _ => (a.from.clone(), a.to.clone()),
    };
    let conn = store::open_readonly()?;
    let result = store::query(&conn, &report_sql(a.by), &[&from, &to, &a.session, &a.project, &a.limit])?;
    print(&result, a.format);
    Ok(())
}

/// The grouped usage query for `--by`. Parameters: ?1 from day, ?2 to day,
/// ?3 session id prefix, ?4 project substring (each NULL = no filter), ?5 row limit.
fn report_sql(by: By) -> String {
    let filter = |t: &str| {
        format!(
            "(?1 IS NULL OR {t}.day >= ?1) AND (?2 IS NULL OR {t}.day <= ?2)
             AND (?3 IS NULL OR {t}.session_id LIKE ?3 || '%')
             AND (?4 IS NULL OR s.project LIKE '%' || ?4 || '%')"
        )
    };
    let usage_cols = "SUM(u.total) AS total, SUM(u.input) AS input, SUM(u.output) AS output,
                      SUM(u.cache_read) AS cache_read, SUM(u.cache_write) AS cache_write, SUM(u.turns) AS turns";
    match by {
        By::Day => format!(
            "SELECT u.day, {usage_cols}, COUNT(DISTINCT u.session_id) AS sessions
             FROM usage_daily u LEFT JOIN sessions s USING (session_id) WHERE {}
             GROUP BY u.day ORDER BY u.day DESC LIMIT ?5",
            filter("u")
        ),
        By::Session => format!(
            "SELECT u.session_id, s.project, s.title, s.model, MIN(u.day) AS first_day, MAX(u.day) AS last_day,
                    {usage_cols},
                    (SELECT COUNT(*) FROM skill_runs r WHERE r.session_id = u.session_id
                       AND (?1 IS NULL OR r.day >= ?1) AND (?2 IS NULL OR r.day <= ?2)) AS skill_runs
             FROM usage_daily u LEFT JOIN sessions s USING (session_id) WHERE {}
             GROUP BY u.session_id ORDER BY total DESC LIMIT ?5",
            filter("u")
        ),
        By::Skill => format!(
            "SELECT r.skill, COUNT(*) AS calls, SUM(r.total) AS approx_total, SUM(r.output) AS output,
                    SUM(r.turns) AS turns, COUNT(DISTINCT r.session_id) AS sessions
             FROM skill_runs r LEFT JOIN sessions s USING (session_id) WHERE {}
             GROUP BY r.skill ORDER BY approx_total DESC LIMIT ?5",
            filter("r")
        ),
        By::Project => format!(
            "SELECT COALESCE(s.project, '?') AS project, {usage_cols}, COUNT(DISTINCT u.session_id) AS sessions
             FROM usage_daily u LEFT JOIN sessions s USING (session_id) WHERE {}
             GROUP BY project ORDER BY total DESC LIMIT ?5",
            filter("u")
        ),
        By::Model => format!(
            "SELECT COALESCE(s.model, '?') AS model, {usage_cols}, COUNT(DISTINCT u.session_id) AS sessions
             FROM usage_daily u LEFT JOIN sessions s USING (session_id) WHERE {}
             GROUP BY model ORDER BY total DESC LIMIT ?5",
            filter("u")
        ),
        By::Agent => format!(
            "SELECT u.agent_type, {usage_cols}, COUNT(DISTINCT u.session_id) AS sessions
             FROM usage_daily u LEFT JOIN sessions s USING (session_id) WHERE {}
             GROUP BY u.agent_type ORDER BY total DESC LIMIT ?5",
            filter("u")
        ),
    }
}

pub fn sql(a: SqlArgs) -> anyhow::Result<()> {
    ensure_synced(a.no_sync)?;
    let conn = store::open_readonly()?;
    let result = store::query(&conn, &a.query, &[])?;
    print(&result, a.format);
    Ok(())
}

pub fn schema() {
    println!("-- {}", store::db_path().display());
    println!("-- Local-date usage for the last {} days. Older rows are deleted automatically.", store::RETENTION_DAYS);
    println!("{}", store::SCHEMA.trim());
}

pub fn print(r: &QueryResult, format: Format) {
    match format {
        Format::Json => {
            let rows: Vec<Value> = r
                .rows
                .iter()
                .map(|row| Value::Object(r.columns.iter().cloned().zip(row.iter().cloned()).collect::<Map<_, _>>()))
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows).unwrap_or_default());
        }
        Format::Csv => {
            println!("{}", r.columns.join(","));
            for row in &r.rows {
                let cells: Vec<String> = row.iter().map(|v| csv_cell(&cell(v))).collect();
                println!("{}", cells.join(","));
            }
        }
        Format::Table => print_table(r),
    }
}

fn cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// Integers get thousands separators in tables (JSON/CSV stay raw for machines).
fn human(v: &Value) -> String {
    match v.as_i64() {
        Some(n) => {
            let s = n.abs().to_string();
            let mut out = String::new();
            for (i, c) in s.chars().enumerate() {
                if i > 0 && (s.len() - i) % 3 == 0 {
                    out.push(',');
                }
                out.push(c);
            }
            if n < 0 { format!("-{out}") } else { out }
        }
        None => crate::hook::truncate(&cell(v), 48),
    }
}

fn print_table(r: &QueryResult) {
    if r.rows.is_empty() {
        println!("(no rows)");
        return;
    }
    let cells: Vec<Vec<String>> = r.rows.iter().map(|row| row.iter().map(human).collect()).collect();
    let widths: Vec<usize> = (0..r.columns.len())
        .map(|i| cells.iter().map(|row| row[i].chars().count()).chain([r.columns[i].len()]).max().unwrap_or(0))
        .collect();
    let numeric: Vec<bool> = (0..r.columns.len()).map(|i| r.rows.iter().all(|row| row[i].is_number() || row[i].is_null())).collect();
    let line = |vals: Vec<String>| {
        let parts: Vec<String> = vals
            .iter()
            .enumerate()
            .map(|(i, v)| if numeric[i] { format!("{v:>w$}", w = widths[i]) } else { format!("{v:<w$}", w = widths[i]) })
            .collect();
        println!("| {} |", parts.join(" | "));
    };
    line(r.columns.clone());
    println!("|{}|", widths.iter().map(|w| "-".repeat(w + 2)).collect::<Vec<_>>().join("|"));
    for row in cells {
        line(row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn every_report_grouping_runs_with_and_without_filters() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(store::SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO sessions (session_id, project, model) VALUES ('abc', 'web', 'claude-opus-5-5');
             INSERT INTO usage_daily VALUES ('abc', 'main', 'main', '2026-10-09', 1, 2, 3, 4, 10, 1);
             INSERT INTO skill_runs VALUES ('abc', 'main', 'main', 0, 'ce-plan', 'tool', 't', '2026-10-09', 1, 1, 1, 1, 1, 4);",
        )
        .unwrap();
        let none: Option<String> = None;
        let some = |v: &str| Some(v.to_string());
        for by in [By::Day, By::Session, By::Skill, By::Project, By::Model, By::Agent] {
            let sql = report_sql(by);
            let all = store::query(&conn, &sql, &[&none, &none, &none, &none, &25]).unwrap();
            assert_eq!(all.rows.len(), 1, "unfiltered");
            let hit = store::query(&conn, &sql, &[&some("2026-10-01"), &some("2026-10-09"), &some("ab"), &some("we"), &25]).unwrap();
            assert_eq!(hit.rows.len(), 1, "matching filters");
            let miss = store::query(&conn, &sql, &[&some("2026-10-10"), &none, &none, &none, &25]).unwrap();
            assert!(miss.rows.is_empty(), "date filter excludes");
        }
    }
}
