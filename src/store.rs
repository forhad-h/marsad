//! 30-day usage history in SQLite (`~/.claude/marsad/usage.db`).
//!
//! Built from Claude Code transcripts, re-reading a file only when its size or mtime
//! changes. Tokens are bucketed by the local day each API call happened; skill runs by
//! the day they started. Rows older than `RETENTION_DAYS` are deleted on every sync.

use crate::model::{DAY_FMT, MAIN_AGENT};
use crate::transcript::{self, TranscriptReader};
use anyhow::Context;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const RETENTION_DAYS: i64 = 30;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    session_id      TEXT PRIMARY KEY,
    project         TEXT,   -- basename of cwd
    cwd             TEXT,
    title           TEXT,   -- AI-generated session title
    model           TEXT,   -- last model used by the main agent
    first_at        TEXT,   -- ISO-8601 UTC of first transcript entry
    last_at         TEXT,   -- ISO-8601 UTC of last transcript entry
    transcript_path TEXT
);
CREATE TABLE IF NOT EXISTS usage_daily (
    session_id  TEXT,
    agent_id    TEXT,       -- 'main' or the subagent id
    agent_type  TEXT,       -- 'main' or the subagent type, e.g. 'Explore'
    day         TEXT,       -- local date YYYY-MM-DD
    input       INTEGER,
    output      INTEGER,
    cache_read  INTEGER,
    cache_write INTEGER,
    total       INTEGER,    -- input + output + cache_read + cache_write
    turns       INTEGER,    -- API calls
    PRIMARY KEY (session_id, agent_id, day)
);
CREATE TABLE IF NOT EXISTS skill_runs (
    session_id  TEXT,
    agent_id    TEXT,
    agent_type  TEXT,
    seq         INTEGER,    -- invocation order within the agent's transcript
    skill       TEXT,
    via         TEXT,       -- 'tool' (Claude invoked it) or 'command' (user typed /skill)
    at          TEXT,       -- ISO-8601 UTC
    day         TEXT,       -- local date YYYY-MM-DD the run started
    turns       INTEGER,
    input       INTEGER,
    output      INTEGER,
    cache_read  INTEGER,
    cache_write INTEGER,
    total       INTEGER,    -- approximate: tokens of turns inside the skill's window
    PRIMARY KEY (session_id, agent_id, seq)
);
CREATE TABLE IF NOT EXISTS files (path TEXT PRIMARY KEY, size INTEGER, mtime REAL);
CREATE INDEX IF NOT EXISTS usage_daily_day ON usage_daily(day);
CREATE INDEX IF NOT EXISTS skill_runs_day ON skill_runs(day);
";

pub fn db_path() -> PathBuf {
    crate::data_dir().join("usage.db")
}

pub fn open() -> anyhow::Result<Connection> {
    open_at(&db_path())
}

fn open_at(path: &Path) -> anyhow::Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// Read-only handle for user/Claude-supplied SQL.
pub fn open_readonly() -> anyhow::Result<Connection> {
    open_readonly_at(&db_path())
}

fn open_readonly_at(path: &Path) -> anyhow::Result<Connection> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .context("no usage history yet; run `marsad sync` first")?;
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.pragma_update(None, "query_only", true)?;
    Ok(conn)
}

/// Opens the database and indexes changed transcripts.
pub fn open_and_sync() -> anyhow::Result<SyncReport> {
    sync(&mut open()?)
}

/// The local day `n` days back, counting today as day 1 (`n` = 0 also means today).
pub fn day_n_ago(n: u32) -> String {
    (chrono::Local::now().date_naive() - chrono::Days::new(n.saturating_sub(1) as u64))
        .format(DAY_FMT)
        .to_string()
}

/// Oldest local day kept.
pub fn cutoff_day() -> String {
    day_n_ago(RETENTION_DAYS as u32)
}

/// One transcript file: a session's main agent or one of its subagents.
struct Unit {
    path: PathBuf,
    session_id: String,
    agent_id: String,
    /// The subagents directory holding this agent's `.meta.json`; None for the main agent.
    meta_dir: Option<PathBuf>,
}

impl Unit {
    /// Reads the subagent type lazily, so unchanged files cost no extra file read.
    fn agent_type(&self) -> String {
        match &self.meta_dir {
            None => MAIN_AGENT.into(),
            Some(dir) => transcript::subagent_type(dir, &self.agent_id).unwrap_or_else(|| "agent".into()),
        }
    }
}

fn units(projects: &Path) -> Vec<Unit> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(projects) else { return out };
    for proj in dirs.flatten().map(|d| d.path()).filter(|p| p.is_dir()) {
        let Ok(entries) = std::fs::read_dir(&proj) else { continue };
        for path in entries.flatten().map(|e| e.path()) {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            if path.extension().is_some_and(|e| e == "jsonl") {
                out.push(Unit { path, session_id: stem, agent_id: MAIN_AGENT.into(), meta_dir: None });
            } else if path.is_dir() {
                let sub = path.join("subagents");
                for (id, file) in transcript::subagent_files(&sub) {
                    out.push(Unit { path: file, session_id: stem.clone(), agent_id: id, meta_dir: Some(sub.clone()) });
                }
            }
        }
    }
    out
}

fn file_sig(path: &Path) -> Option<(i64, f64)> {
    let m = std::fs::metadata(path).ok()?;
    let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
    Some((m.len() as i64, mtime))
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub scanned: usize,
    pub updated: usize,
    /// Changed files that could not be indexed; they are retried on the next sync.
    pub failed: usize,
    /// Rows removed by the retention window.
    pub pruned: usize,
}

/// Indexes new or changed transcripts under `~/.claude/projects`, then prunes old rows.
pub fn sync(conn: &mut Connection) -> anyhow::Result<SyncReport> {
    sync_dir(conn, &crate::claude_dir().join("projects"))
}

pub fn sync_dir(conn: &mut Connection, projects: &Path) -> anyhow::Result<SyncReport> {
    let cutoff = cutoff_day();
    let cutoff_time = crate::now() - RETENTION_DAYS as f64 * 86400.0;
    let mut report = SyncReport::default();

    for unit in units(projects) {
        let Some((size, mtime)) = file_sig(&unit.path) else { continue };
        if mtime < cutoff_time {
            continue;
        }
        report.scanned += 1;
        let path_str = unit.path.to_string_lossy().into_owned();
        let known: Option<(i64, f64)> = conn
            .prepare_cached("SELECT size, mtime FROM files WHERE path = ?1")?
            .query_row([&path_str], |r| Ok((r.get(0)?, r.get(1)?)))
            .ok();
        if known == Some((size, mtime)) {
            continue;
        }
        let mut reader = TranscriptReader::new(unit.path.clone());
        reader.poll();
        // One bad file must not block indexing of the rest.
        match write_unit(conn, &unit, &reader, &cutoff, (size, mtime)) {
            Ok(()) => report.updated += 1,
            Err(_) => report.failed += 1,
        }
    }

    report.pruned = prune(conn, &cutoff, cutoff_time)?;
    Ok(report)
}

/// Replaces one file's rows and records its signature, in a single transaction.
fn write_unit(
    conn: &mut Connection,
    unit: &Unit,
    reader: &TranscriptReader,
    cutoff: &str,
    (size, mtime): (i64, f64),
) -> anyhow::Result<()> {
    let stats = reader.stats();
    let agent_type = unit.agent_type();
    let tx = conn.transaction()?;
    let key = params![unit.session_id, unit.agent_id];
    tx.execute("DELETE FROM usage_daily WHERE session_id = ?1 AND agent_id = ?2", key)?;
    tx.execute("DELETE FROM skill_runs WHERE session_id = ?1 AND agent_id = ?2", key)?;

    for (day, d) in reader.daily().iter().filter(|(day, _)| day.as_str() >= cutoff) {
        let t = d.tokens;
        tx.execute(
            "INSERT INTO usage_daily VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                unit.session_id, unit.agent_id, agent_type, day,
                t.input as i64, t.output as i64, t.cache_read as i64, t.cache_write as i64, t.total() as i64, d.turns
            ],
        )?;
    }
    for (seq, run) in stats.runs.iter().enumerate() {
        let day = run.at.as_deref().and_then(transcript::local_day);
        if day.as_deref().is_none_or(|d| d < cutoff) {
            continue;
        }
        let t = run.tokens;
        tx.execute(
            "INSERT INTO skill_runs VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                unit.session_id, unit.agent_id, agent_type, seq as i64, run.name, run.via.as_str(), run.at, day,
                run.turns, t.input as i64, t.output as i64, t.cache_read as i64, t.cache_write as i64, t.total() as i64
            ],
        )?;
    }

    if unit.meta_dir.is_none() {
        let cwd = reader.cwd();
        let project = cwd.and_then(|c| Path::new(c).file_name()).map(|n| n.to_string_lossy().into_owned());
        let (first, last) = reader.span();
        tx.execute(
            "INSERT INTO sessions VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(session_id) DO UPDATE SET project = excluded.project, cwd = excluded.cwd,
               title = excluded.title, model = excluded.model, first_at = excluded.first_at,
               last_at = excluded.last_at, transcript_path = excluded.transcript_path",
            params![unit.session_id, project, cwd, stats.title, stats.model, first, last, unit.path.to_string_lossy()],
        )?;
    } else {
        tx.execute("INSERT OR IGNORE INTO sessions (session_id) VALUES (?1)", [&unit.session_id])?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO files (path, size, mtime) VALUES (?1, ?2, ?3)",
        params![unit.path.to_string_lossy(), size, mtime],
    )?;
    tx.commit()?;
    Ok(())
}

/// Deletes rows older than the retention window; returns how many usage rows went.
fn prune(conn: &Connection, cutoff: &str, cutoff_time: f64) -> anyhow::Result<usize> {
    let mut pruned = conn.execute("DELETE FROM usage_daily WHERE day < ?1", [cutoff])?;
    pruned += conn.execute("DELETE FROM skill_runs WHERE day < ?1", [cutoff])?;
    conn.execute(
        "DELETE FROM sessions WHERE session_id NOT IN (SELECT session_id FROM usage_daily)
           AND session_id NOT IN (SELECT session_id FROM skill_runs)",
        [],
    )?;
    conn.execute("DELETE FROM files WHERE mtime < ?1", [cutoff_time])?;
    Ok(pruned)
}

// ---------- queries shared by the TUI and `marsad report` ----------

#[derive(Clone, Debug)]
pub struct DayRow {
    pub day: String,
    pub total: i64,
}

#[derive(Clone, Debug)]
pub struct SessionRow {
    pub session_id: String,
    pub project: String,
    pub title: String,
    pub model: String,
    pub total: i64,
    pub output: i64,
    pub turns: i64,
    pub skill_runs: i64,
}

#[derive(Clone, Debug)]
pub struct SkillRow {
    pub skill: String,
    pub calls: i64,
    pub total: i64,
    pub output: i64,
    pub sessions: i64,
}

#[derive(Clone, Debug)]
pub struct RunRow {
    pub at: String,
    pub agent_type: String,
    pub skill: String,
    pub via: crate::model::Via,
    pub turns: i64,
    pub output: i64,
    pub total: i64,
}

pub fn days(conn: &Connection) -> anyhow::Result<Vec<DayRow>> {
    let mut stmt = conn.prepare(
        "SELECT day, SUM(total) FROM usage_daily GROUP BY day ORDER BY day DESC",
    )?;
    let rows = stmt.query_map([], |r| Ok(DayRow { day: r.get(0)?, total: r.get(1)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Sessions ranked by tokens, for one day or (None) the whole retention window.
pub fn sessions_on(conn: &Connection, day: Option<&str>) -> anyhow::Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT u.session_id, COALESCE(s.project, ''), COALESCE(s.title, ''), COALESCE(s.model, ''),
                SUM(u.total), SUM(u.output), SUM(u.turns),
                (SELECT COUNT(*) FROM skill_runs r WHERE r.session_id = u.session_id AND (?1 IS NULL OR r.day = ?1))
         FROM usage_daily u LEFT JOIN sessions s USING (session_id)
         WHERE ?1 IS NULL OR u.day = ?1
         GROUP BY u.session_id ORDER BY SUM(u.total) DESC",
    )?;
    let rows = stmt.query_map([day], |r| {
        Ok(SessionRow {
            session_id: r.get(0)?,
            project: r.get(1)?,
            title: r.get(2)?,
            model: r.get(3)?,
            total: r.get(4)?,
            output: r.get(5)?,
            turns: r.get(6)?,
            skill_runs: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn skills_on(conn: &Connection, day: Option<&str>) -> anyhow::Result<Vec<SkillRow>> {
    let mut stmt = conn.prepare(
        "SELECT skill, COUNT(*), SUM(total), SUM(output), COUNT(DISTINCT session_id)
         FROM skill_runs WHERE ?1 IS NULL OR day = ?1
         GROUP BY skill ORDER BY SUM(total) DESC",
    )?;
    let rows = stmt.query_map([day], |r| {
        Ok(SkillRow { skill: r.get(0)?, calls: r.get(1)?, total: r.get(2)?, output: r.get(3)?, sessions: r.get(4)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn session_runs(conn: &Connection, session_id: &str) -> anyhow::Result<Vec<RunRow>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(at, ''), agent_type, skill, via, turns, output, total
         FROM skill_runs WHERE session_id = ?1 ORDER BY at",
    )?;
    let rows = stmt.query_map([session_id], |r| {
        Ok(RunRow {
            at: r.get(0)?,
            agent_type: r.get(1)?,
            skill: r.get(2)?,
            via: crate::model::Via::parse(&r.get::<_, String>(3)?),
            turns: r.get(4)?,
            output: r.get(5)?,
            total: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Per-day totals for one session.
pub fn session_days(conn: &Connection, session_id: &str) -> anyhow::Result<Vec<(String, i64)>> {
    let mut stmt =
        conn.prepare("SELECT day, SUM(total) FROM usage_daily WHERE session_id = ?1 GROUP BY day ORDER BY day")?;
    let rows = stmt.query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ---------- generic query output for `marsad report` / `marsad sql` ----------

pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
}

pub fn query(conn: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> anyhow::Result<QueryResult> {
    let mut stmt = conn.prepare(sql)?;
    // The open flag and query_only don't stop everything (e.g. VACUUM INTO creates a file first).
    if !stmt.readonly() {
        anyhow::bail!("only read-only statements are allowed");
    }
    let columns: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
    let n = columns.len();
    let rows = stmt.query_map(args, |r| {
        (0..n)
            .map(|i| {
                Ok(match r.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(v) => json!(v),
                    rusqlite::types::ValueRef::Real(v) => json!(v),
                    rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                    rusqlite::types::ValueRef::Blob(b) => json!(format!("<{} bytes>", b.len())),
                })
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    Ok(QueryResult { columns, rows: rows.collect::<Result<_, _>>()? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sync_indexes_prunes_and_skips_unchanged() {
        let root = std::env::temp_dir().join(format!("marsad-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("projects").join("-p-web");
        std::fs::create_dir_all(proj.join("s1").join("subagents")).unwrap();
        let now = chrono::Utc::now();
        let ts = now.to_rfc3339();
        let old = (now - chrono::Duration::days(40)).to_rfc3339();
        let mut f = std::fs::File::create(proj.join("s1.jsonl")).unwrap();
        for line in [
            format!(r#"{{"type":"user","timestamp":"{old}","cwd":"/p/web","message":{{"content":"old"}}}}"#),
            format!(r#"{{"type":"assistant","timestamp":"{old}","message":{{"id":"m0","usage":{{"output_tokens":999}}}}}}"#),
            format!(r#"{{"type":"user","timestamp":"{ts}","message":{{"content":"hi"}}}}"#),
            format!(r#"{{"type":"assistant","timestamp":"{ts}","message":{{"id":"m1","model":"claude-opus-5-5","usage":{{"input_tokens":10,"output_tokens":5}},"content":[{{"type":"tool_use","name":"Skill","input":{{"skill":"ce-plan"}}}}]}}}}"#),
            format!(r#"{{"type":"assistant","timestamp":"{ts}","message":{{"id":"m2","usage":{{"output_tokens":20,"cache_read_input_tokens":100}}}}}}"#),
        ] {
            writeln!(f, "{line}").unwrap();
        }
        let mut sub = std::fs::File::create(proj.join("s1/subagents/agent-a1.jsonl")).unwrap();
        writeln!(sub, r#"{{"type":"assistant","timestamp":"{ts}","message":{{"id":"x","usage":{{"output_tokens":7}}}}}}"#).unwrap();
        std::fs::write(proj.join("s1/subagents/agent-a1.meta.json"), r#"{"agentType":"Explore"}"#).unwrap();

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let r = sync_dir(&mut conn, &root.join("projects")).unwrap();
        assert_eq!((r.scanned, r.updated), (2, 2));

        let days = days(&conn).unwrap();
        assert_eq!(days.len(), 1, "40-day-old usage is pruned");
        assert_eq!(days[0].total, 10 + 5 + 20 + 100 + 7);
        let sessions = sessions_on(&conn, None).unwrap();
        assert_eq!((sessions[0].project.as_str(), sessions[0].skill_runs), ("web", 1));
        let skills = skills_on(&conn, Some(&days[0].day)).unwrap();
        assert_eq!((skills[0].skill.as_str(), skills[0].total), ("ce-plan", 120));
        let agents = query(&conn, "SELECT agent_type FROM usage_daily ORDER BY agent_type", &[]).unwrap();
        assert_eq!(agents.rows, vec![vec![json!("Explore")], vec![json!("main")]]);

        let again = sync_dir(&mut conn, &root.join("projects")).unwrap();
        assert_eq!(again.updated, 0, "unchanged files are skipped");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn readonly_connection_rejects_every_write() {
        let root = std::env::temp_dir().join(format!("marsad-ro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let db = root.join("usage.db");
        assert!(open_readonly_at(&db).is_err(), "no database yet");
        open_at(&db).unwrap().execute("INSERT INTO files VALUES ('a', 1, 1.0)", []).unwrap();

        let conn = open_readonly_at(&db).unwrap();
        assert_eq!(query(&conn, "SELECT COUNT(*) FROM files", &[]).unwrap().rows, vec![vec![json!(1)]]);
        let copy = root.join("copy.db");
        for sql in [
            "DELETE FROM files".to_string(),
            "DROP TABLE files".to_string(),
            "INSERT INTO files VALUES ('b', 1, 1.0)".to_string(),
            format!("VACUUM INTO '{}'", copy.display()),
        ] {
            assert!(query(&conn, &sql, &[]).is_err(), "{sql} must be rejected");
        }
        assert!(!copy.exists(), "VACUUM INTO must not create a file");
        assert_eq!(query(&conn, "SELECT COUNT(*) FROM files", &[]).unwrap().rows, vec![vec![json!(1)]]);
        std::fs::remove_dir_all(root).ok();
    }
}
