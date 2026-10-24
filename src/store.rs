//! Usage history in SQLite (`~/.claude/marsad/usage.db`).
//!
//! Built from Claude Code transcripts, re-reading a file only when its size or mtime
//! changes. Tokens are bucketed by the local day each API call happened; skill runs by
//! the day they started. Rows outlive their transcripts (Claude Code deletes those after
//! `cleanupPeriodDays`). Session and skill detail is kept for `marsad retention` days (30 by
//! default); `token_days` is the permanent counter and is never pruned. Versions before 0.4
//! folded pruned days into one `lifetime` row; that sum lives on in `legacy_totals`.

use crate::model::{DAY_FMT, MAIN_AGENT};
use crate::transcript::{self, TranscriptReader};
use anyhow::Context;
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;


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
CREATE TABLE IF NOT EXISTS token_days (   -- permanent counter, never pruned; all time = this + legacy_totals
    session_id  TEXT,
    agent_id    TEXT,
    day         TEXT,       -- local date YYYY-MM-DD
    input       INTEGER,
    output      INTEGER,
    cache_read  INTEGER,
    cache_write INTEGER,
    total       INTEGER,
    turns       INTEGER,
    PRIMARY KEY (session_id, agent_id, day)
);
CREATE TABLE IF NOT EXISTS legacy_totals (  -- days pruned by versions before 0.4, per install that pruned them
    source      TEXT PRIMARY KEY,   -- install id (meta.install_id of that machine)
    since       TEXT,       -- oldest pruned day
    through     TEXT,       -- days in [since, through) are only in this sum and never enter token_days
    input       INTEGER,
    output      INTEGER,
    cache_read  INTEGER,
    cache_write INTEGER,
    total       INTEGER
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);  -- install_id, schema, restored:<source>
CREATE TABLE IF NOT EXISTS lifetime (      -- frozen pre-0.4 data, copied into legacy_totals; no longer written
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    since       TEXT,
    through     TEXT,
    input       INTEGER,
    output      INTEGER,
    cache_read  INTEGER,
    cache_write INTEGER,
    total       INTEGER
);
CREATE TABLE IF NOT EXISTS skills_seen (  -- every skill ever run, never pruned
    skill       TEXT PRIMARY KEY,
    first_day   TEXT        -- local date YYYY-MM-DD of its earliest known run
);
CREATE TABLE IF NOT EXISTS badge_best (   -- highest value ever computed per badge stat, never lowered
    key         TEXT PRIMARY KEY,   -- e.g. 'rank', 'streak', 'skills', 'subagents', 'busiest_day'
    value       INTEGER
);
CREATE INDEX IF NOT EXISTS usage_daily_day ON usage_daily(day);
CREATE INDEX IF NOT EXISTS token_days_day ON token_days(day);
CREATE INDEX IF NOT EXISTS skill_runs_day ON skill_runs(day);
";

pub fn db_path() -> PathBuf {
    crate::data_dir().join("usage.db")
}

pub fn open() -> anyhow::Result<Connection> {
    open_at(&db_path())
}

pub(crate) fn open_at(path: &Path) -> anyhow::Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    prepare(&mut conn)?;
    Ok(conn)
}

/// Schema version this build writes; older databases are migrated on open.
const SCHEMA_VERSION: i64 = 2;

fn schema_version(conn: &Connection) -> i64 {
    conn.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| r.get::<_, String>(0))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Creates missing tables and migrates older data, one version step at a time.
fn prepare(conn: &mut Connection) -> anyhow::Result<()> {
    conn.execute_batch(SCHEMA)?;
    if schema_version(conn) >= SCHEMA_VERSION {
        return Ok(());
    }
    // Immediate takes the write lock before re-reading the version, so two first openers can't
    // both migrate: the second waits on the busy timeout, then sees it done.
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version = schema_version(&tx);
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    tx.execute("INSERT OR IGNORE INTO meta VALUES ('install_id', ?1)", [new_install_id()])?;
    // Version 1 only: copying usage_daily again would count days inside legacy ranges twice.
    if version < 1 {
        // The old all-time sum keeps its pruned range, so those days are never counted again.
        tx.execute(
            "INSERT OR IGNORE INTO legacy_totals
             SELECT (SELECT value FROM meta WHERE key = 'install_id'), since, through,
                    input, output, cache_read, cache_write, total
             FROM lifetime WHERE id = 1 AND since IS NOT NULL",
            [],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO token_days
             SELECT session_id, agent_id, day, input, output, cache_read, cache_write, total, turns FROM usage_daily",
            [],
        )?;
    }
    if version < 2 {
        backfill_skills_seen(&tx)?;
    }
    // Re-read every transcript still on disk once, so days the old retention skipped are counted
    // and skills older than the retention reach skills_seen.
    tx.execute("DELETE FROM files", [])?;
    tx.execute("INSERT OR REPLACE INTO meta VALUES ('schema', ?1)", [SCHEMA_VERSION.to_string()])?;
    tx.commit()?;
    Ok(())
}

/// Adds every skill in `skill_runs` to `skills_seen`, keeping each one's earliest day.
pub(crate) fn backfill_skills_seen(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO skills_seen SELECT skill, MIN(day) FROM skill_runs WHERE skill IS NOT NULL AND day IS NOT NULL
         GROUP BY skill ON CONFLICT(skill) DO UPDATE SET first_day = MIN(first_day, excluded.first_day)",
        [],
    )?;
    Ok(())
}

fn new_install_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}", nanos, std::process::id())
}

/// This machine's id, which tags the legacy totals it pruned and its exports.
pub fn install_id(conn: &Connection) -> anyhow::Result<String> {
    Ok(conn.query_row("SELECT value FROM meta WHERE key = 'install_id'", [], |r| r.get(0))?)
}

/// Legacy rows whose days stay out of the day counter: this install's own, and those restored
/// with `marsad import --restore`. Another machine's merged legacy sum covers only its sessions,
/// so it never blocks this machine's days.
pub(crate) const BLOCKING_LEGACY: &str = "l.since IS NOT NULL AND l.through IS NOT NULL
    AND (l.source = (SELECT value FROM meta WHERE key = 'install_id')
         OR EXISTS (SELECT 1 FROM meta WHERE key = 'restored:' || l.source))";

/// `[since, through)` day ranges that only legacy totals count.
pub fn legacy_ranges(conn: &Connection) -> anyhow::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(&format!("SELECT l.since, l.through FROM legacy_totals l WHERE {BLOCKING_LEGACY}"))?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(crate) fn in_legacy(ranges: &[(String, String)], day: &str) -> bool {
    ranges.iter().any(|(since, through)| day >= since.as_str() && day < through.as_str())
}

/// Read-only handle for user/Claude-supplied SQL.
pub fn open_readonly() -> anyhow::Result<Connection> {
    open_readonly_at(&db_path())
}

fn open_readonly_at(path: &Path) -> anyhow::Result<Connection> {
    let readonly = || Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY);
    let mut conn = readonly().context("no usage history yet; run `marsad sync` first")?;
    // A database from an older version is migrated before its first read.
    if schema_version(&conn) < SCHEMA_VERSION {
        drop(conn);
        drop(open_at(path)?);
        conn = readonly()?;
    }
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

/// Days of history to keep unless `marsad retention` says otherwise.
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

/// Days of history to keep; None keeps everything.
pub fn retention() -> Option<u32> {
    match crate::settings::retention_days() {
        None => Some(DEFAULT_RETENTION_DAYS),
        Some(0) => None,
        Some(n) => Some(n as u32),
    }
}

pub fn set_retention(days: Option<u32>) -> anyhow::Result<()> {
    // 0 = forever.
    crate::settings::set_retention_days(days.unwrap_or(0) as u64)
}

/// "forever" or "the last N days", for messages.
pub fn retention_label(days: Option<u32>) -> String {
    match days {
        None => "forever".into(),
        Some(n) => format!("the last {n} days"),
    }
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
            Some(dir) => transcript::subagent_meta(dir, &self.agent_id)
                .and_then(|m| m.agent_type)
                .unwrap_or_else(|| "agent".into()),
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
    /// Rows removed by the retention setting.
    pub pruned: usize,
}

/// Indexes new or changed transcripts under `~/.claude/projects`, then prunes detail rows
/// older than the retention (unless it is forever). The permanent counter is never pruned.
pub fn sync(conn: &mut Connection) -> anyhow::Result<SyncReport> {
    sync_dir(conn, &crate::claude_dir().join("projects"), retention())
}

pub fn sync_dir(conn: &mut Connection, projects: &Path, retention: Option<u32>) -> anyhow::Result<SyncReport> {
    let cutoff = retention.map(day_n_ago);
    let legacy = legacy_ranges(conn)?;
    let mut report = SyncReport::default();

    for unit in units(projects) {
        let Some((size, mtime)) = file_sig(&unit.path) else { continue };
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
        // A file that yields no usage although the counter has rows for it was unreadable or
        // emptied; replacing would erase permanent tokens, so keep them and retry next sync.
        if reader.daily().is_empty() && has_token_days(conn, &unit)? {
            report.failed += 1;
            continue;
        }
        // One bad file must not block indexing of the rest.
        match write_unit(conn, &unit, &reader, cutoff.as_deref(), &legacy, (size, mtime)) {
            Ok(()) => report.updated += 1,
            Err(_) => report.failed += 1,
        }
    }

    if let Some(cutoff) = &cutoff {
        report.pruned = prune(conn, cutoff)?;
    }
    Ok(report)
}

fn has_token_days(conn: &Connection, unit: &Unit) -> anyhow::Result<bool> {
    let mut stmt = conn.prepare_cached("SELECT 1 FROM token_days WHERE session_id = ?1 AND agent_id = ?2 LIMIT 1")?;
    Ok(stmt.exists(params![unit.session_id, unit.agent_id])?)
}

/// Replaces one file's rows and records its signature, in a single transaction. Detail rows
/// start at `cutoff`; counter rows cover every day outside the `legacy` ranges.
fn write_unit(
    conn: &mut Connection,
    unit: &Unit,
    reader: &TranscriptReader,
    cutoff: Option<&str>,
    legacy: &[(String, String)],
    (size, mtime): (i64, f64),
) -> anyhow::Result<()> {
    let stats = reader.stats();
    let agent_type = unit.agent_type();
    let tx = conn.transaction()?;
    let key = params![unit.session_id, unit.agent_id];
    tx.execute("DELETE FROM usage_daily WHERE session_id = ?1 AND agent_id = ?2", key)?;
    tx.execute("DELETE FROM skill_runs WHERE session_id = ?1 AND agent_id = ?2", key)?;
    // Replaced rather than merged, so usage re-bucketed after a timezone change isn't counted
    // twice. Days inside a legacy range never have rows here, so none are lost.
    tx.execute("DELETE FROM token_days WHERE session_id = ?1 AND agent_id = ?2", key)?;

    for (day, d) in &reader.daily() {
        let t = d.tokens;
        let row = params![
            unit.session_id, unit.agent_id, day,
            t.input as i64, t.output as i64, t.cache_read as i64, t.cache_write as i64, t.total() as i64, d.turns
        ];
        if !in_legacy(legacy, day) {
            tx.execute("INSERT INTO token_days VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)", row)?;
        }
        if cutoff.is_none_or(|c| day.as_str() >= c) {
            tx.execute(
                "INSERT INTO usage_daily VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    unit.session_id, unit.agent_id, agent_type, day,
                    t.input as i64, t.output as i64, t.cache_read as i64, t.cache_write as i64, t.total() as i64, d.turns
                ],
            )?;
        }
    }
    for (seq, run) in stats.runs.iter().enumerate() {
        let day = run.at.as_deref().and_then(transcript::local_day);
        // Seen before the retention check, so a re-read recovers skills older than the cutoff.
        if let Some(day) = &day {
            tx.execute(
                "INSERT INTO skills_seen VALUES (?1, ?2)
                 ON CONFLICT(skill) DO UPDATE SET first_day = MIN(first_day, excluded.first_day)",
                params![run.name, day],
            )?;
        }
        if day.as_deref().is_none_or(|d| cutoff.is_some_and(|c| d < c)) {
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

/// Deletes session and skill detail older than the retention window; returns how many rows
/// went. The permanent counter (`token_days`) keeps those days.
fn prune(conn: &mut Connection, cutoff: &str) -> anyhow::Result<usize> {
    let tx = conn.transaction()?;
    let mut pruned = tx.execute("DELETE FROM usage_daily WHERE day < ?1", [cutoff])?;
    pruned += tx.execute("DELETE FROM skill_runs WHERE day < ?1", [cutoff])?;
    tx.execute(
        "DELETE FROM sessions WHERE session_id NOT IN (SELECT session_id FROM usage_daily)
           AND session_id NOT IN (SELECT session_id FROM skill_runs)",
        [],
    )?;
    tx.commit()?;
    Ok(pruned)
}

// ---------- queries shared by the TUI and `marsad report` ----------

/// Highest value ever stored per badge stat.
#[allow(dead_code)] // used by the badge model (U2)
pub fn badge_best(conn: &Connection) -> anyhow::Result<std::collections::HashMap<String, i64>> {
    let mut stmt = conn.prepare("SELECT key, value FROM badge_best")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Raises one stat's best value to `value`; a lower value leaves it as it was.
#[allow(dead_code)] // used by the badge model (U2)
pub fn raise_best(conn: &Connection, key: &str, value: i64) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO badge_best VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
        params![key, value],
    )?;
    Ok(())
}

/// All-time token totals: the permanent counter plus totals pruned by earlier versions.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct Lifetime {
    /// Oldest local day counted; None when there is no usage yet.
    pub since: Option<String>,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub total: i64,
}

pub fn lifetime(conn: &Connection) -> anyhow::Result<Lifetime> {
    Ok(conn.query_row(
        "SELECT (SELECT MIN(d) FROM (SELECT MIN(since) AS d FROM legacy_totals UNION ALL SELECT MIN(day) FROM token_days)),
                COALESCE(l.input, 0) + COALESCE(u.input, 0), COALESCE(l.output, 0) + COALESCE(u.output, 0),
                COALESCE(l.cache_read, 0) + COALESCE(u.cache_read, 0),
                COALESCE(l.cache_write, 0) + COALESCE(u.cache_write, 0), COALESCE(l.total, 0) + COALESCE(u.total, 0)
         FROM (SELECT SUM(input) input, SUM(output) output, SUM(cache_read) cache_read,
                      SUM(cache_write) cache_write, SUM(total) total FROM token_days) u,
              (SELECT SUM(input) input, SUM(output) output, SUM(cache_read) cache_read,
                      SUM(cache_write) cache_write, SUM(total) total FROM legacy_totals) l",
        [],
        |r| {
            Ok(Lifetime {
                since: r.get(0)?,
                input: r.get(1)?,
                output: r.get(2)?,
                cache_read: r.get(3)?,
                cache_write: r.get(4)?,
                total: r.get(5)?,
            })
        },
    )?)
}

/// Tokens over one stretch of days, with the per-day average over its calendar days.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct PeriodTotal {
    pub period: &'static str,
    pub days: i64,
    pub total: i64,
    pub daily_average: i64,
}

/// Totals for today, the last 7 and 30 days, and all time (which starts at the first counted day).
pub fn periods(conn: &Connection, today: &str) -> anyhow::Result<Vec<PeriodTotal>> {
    let parse = |d: &str| chrono::NaiveDate::parse_from_str(d, DAY_FMT);
    let today_date = parse(today)?;
    let since_day = |n: u64| (today_date - chrono::Days::new(n - 1)).format(DAY_FMT).to_string();
    let sum = |from: &str| -> anyhow::Result<i64> {
        Ok(conn.query_row(
            "SELECT COALESCE(SUM(total), 0) FROM token_days WHERE day >= ?1 AND day <= ?2",
            [from, today],
            |r| r.get(0),
        )?)
    };
    let all = lifetime(conn)?;
    let all_days = all.since.as_deref().and_then(|d| parse(d).ok()).map_or(0, |d| (today_date - d).num_days() + 1);
    let mut out = Vec::new();
    for (period, days, total) in [
        ("today", 1, sum(today)?),
        ("7d", 7, sum(&since_day(7))?),
        ("30d", 30, sum(&since_day(30))?),
        ("all", all_days.max(0), all.total),
    ] {
        let daily_average = if days > 0 { total / days } else { 0 };
        out.push(PeriodTotal { period, days, total, daily_average });
    }
    Ok(out)
}

/// Tokens indexed for `day`, per (session, agent), for overlaying live usage.
pub fn day_rows(conn: &Connection, day: &str) -> anyhow::Result<std::collections::HashMap<(String, String), i64>> {
    let mut stmt = conn.prepare("SELECT session_id, agent_id, total FROM token_days WHERE day = ?1")?;
    let rows = stmt.query_map([day], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

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

/// An inclusive range of local days; None on either side is open-ended.
#[derive(Clone, Copy, Default)]
pub struct Days<'a> {
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
}

impl<'a> Days<'a> {
    pub fn since(from: Option<&'a str>) -> Self {
        Self { from, to: None }
    }

    pub fn on(day: &'a str) -> Self {
        Self { from: Some(day), to: Some(day) }
    }
}

/// Per-day totals, newest first.
pub fn days(conn: &Connection, range: Days) -> anyhow::Result<Vec<DayRow>> {
    let mut stmt = conn.prepare(
        "SELECT day, SUM(total) FROM token_days
         WHERE (?1 IS NULL OR day >= ?1) AND (?2 IS NULL OR day <= ?2)
         GROUP BY day ORDER BY day DESC",
    )?;
    let rows = stmt.query_map([range.from, range.to], |r| Ok(DayRow { day: r.get(0)?, total: r.get(1)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Oldest day with any usage.
pub fn first_day(conn: &Connection) -> anyhow::Result<Option<String>> {
    Ok(conn.query_row("SELECT MIN(day) FROM token_days", [], |r| r.get(0))?)
}

/// Number of sessions with usage in the range.
pub fn session_count(conn: &Connection, range: Days) -> anyhow::Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(DISTINCT session_id) FROM usage_daily
         WHERE (?1 IS NULL OR day >= ?1) AND (?2 IS NULL OR day <= ?2)",
        [range.from, range.to],
        |r| r.get(0),
    )?)
}

/// Sessions ranked by tokens within the range.
pub fn sessions_in(conn: &Connection, range: Days) -> anyhow::Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT u.session_id, COALESCE(s.project, ''), COALESCE(s.title, ''), COALESCE(s.model, ''),
                SUM(u.total), SUM(u.output), SUM(u.turns),
                (SELECT COUNT(*) FROM skill_runs r WHERE r.session_id = u.session_id
                   AND (?1 IS NULL OR r.day >= ?1) AND (?2 IS NULL OR r.day <= ?2))
         FROM usage_daily u LEFT JOIN sessions s USING (session_id)
         WHERE (?1 IS NULL OR u.day >= ?1) AND (?2 IS NULL OR u.day <= ?2)
         GROUP BY u.session_id ORDER BY SUM(u.total) DESC",
    )?;
    let rows = stmt.query_map([range.from, range.to], |r| {
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

pub fn skills_in(conn: &Connection, range: Days) -> anyhow::Result<Vec<SkillRow>> {
    let mut stmt = conn.prepare(
        "SELECT skill, COUNT(*), SUM(total), SUM(output), COUNT(DISTINCT session_id)
         FROM skill_runs WHERE (?1 IS NULL OR day >= ?1) AND (?2 IS NULL OR day <= ?2)
         GROUP BY skill ORDER BY SUM(total) DESC",
    )?;
    let rows = stmt.query_map([range.from, range.to], |r| {
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
        prepare(&mut conn).unwrap();
        let r = sync_dir(&mut conn, &root.join("projects"), Some(30)).unwrap();
        assert_eq!((r.scanned, r.updated), (2, 2));

        let days = days(&conn, Days::default()).unwrap();
        assert_eq!(days.len(), 2, "the day counter keeps 40-day-old usage");
        assert_eq!((days[0].total, days[1].total), (10 + 5 + 20 + 100 + 7, 999));
        assert_eq!(total_tokens(&conn), 142, "detail older than the retention is not kept");
        let sessions = sessions_in(&conn, Days::default()).unwrap();
        assert_eq!((sessions[0].project.as_str(), sessions[0].skill_runs), ("web", 1));
        let skills = skills_in(&conn, Days::on(&days[0].day)).unwrap();
        assert_eq!((skills[0].skill.as_str(), skills[0].total), ("ce-plan", 120));
        let agents = query(&conn, "SELECT agent_type FROM usage_daily ORDER BY agent_type", &[]).unwrap();
        assert_eq!(agents.rows, vec![vec![json!("Explore")], vec![json!("main")]]);

        let again = sync_dir(&mut conn, &root.join("projects"), Some(30)).unwrap();
        assert_eq!(again.updated, 0, "unchanged files are skipped");
        std::fs::remove_dir_all(root).ok();
    }

    fn total_tokens(conn: &Connection) -> i64 {
        conn.query_row("SELECT COALESCE(SUM(total), 0) FROM usage_daily", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn sync_handles_corrupt_appended_and_rewritten_transcripts() {
        let root = std::env::temp_dir().join(format!("marsad-store-inc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("projects").join("-p-api");
        std::fs::create_dir_all(&proj).unwrap();
        let file = proj.join("s2.jsonl");
        let ts = chrono::Utc::now().to_rfc3339();
        let msg = |id: &str, out: u64| {
            format!(r#"{{"type":"assistant","timestamp":"{ts}","message":{{"id":"{id}","usage":{{"output_tokens":{out}}}}}}}"#)
        };
        let mut conn = Connection::open_in_memory().unwrap();
        prepare(&mut conn).unwrap();
        let projects = root.join("projects");

        // Corrupt and partial lines are skipped; valid ones still count.
        std::fs::write(&file, format!("{}\nnot json at all\n{{\"type\":\n{}\n", msg("a", 10), msg("b", 5))).unwrap();
        let r = sync_dir(&mut conn, &projects, None).unwrap();
        assert_eq!((r.updated, r.failed), (1, 0));
        assert_eq!(total_tokens(&conn), 15);

        // Appending re-indexes the file without double counting.
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{}", msg("c", 100)).unwrap();
        drop(f);
        assert_eq!(sync_dir(&mut conn, &projects, None).unwrap().updated, 1);
        assert_eq!(total_tokens(&conn), 115);

        // A rewritten, shorter file replaces the old rows rather than adding to them.
        std::fs::write(&file, format!("{}\n", msg("z", 7))).unwrap();
        assert_eq!(sync_dir(&mut conn, &projects, None).unwrap().updated, 1);
        assert_eq!(total_tokens(&conn), 7);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn unreadable_or_emptied_transcript_keeps_counted_tokens() {
        let root = std::env::temp_dir().join(format!("marsad-store-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("projects").join("-p-gone");
        std::fs::create_dir_all(&proj).unwrap();
        let file = proj.join("s5.jsonl");
        let ts = chrono::Utc::now().to_rfc3339();
        std::fs::write(&file, format!(r#"{{"type":"assistant","timestamp":"{ts}","message":{{"id":"a","usage":{{"output_tokens":30}}}}}}"#) + "\n").unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        prepare(&mut conn).unwrap();
        let projects = root.join("projects");
        sync_dir(&mut conn, &projects, None).unwrap();
        let counted = |conn: &Connection| days(conn, Days::default()).unwrap().iter().map(|d| d.total).sum::<i64>();
        assert_eq!(counted(&conn), 30);

        // The file changed but the reader saw nothing: the counter keeps its rows and the file
        // is retried rather than recorded as indexed.
        std::fs::write(&file, "").unwrap();
        let r = sync_dir(&mut conn, &projects, None).unwrap();
        assert_eq!((r.updated, r.failed), (0, 1));
        assert_eq!(counted(&conn), 30);
        assert_eq!(sync_dir(&mut conn, &projects, None).unwrap().failed, 1, "retried on the next sync");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn without_retention_history_outlives_deleted_transcripts() {
        let root = std::env::temp_dir().join(format!("marsad-store-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("projects").join("-p-old");
        std::fs::create_dir_all(&proj).unwrap();
        let file = proj.join("s3.jsonl");
        let old = (chrono::Utc::now() - chrono::Duration::days(200)).to_rfc3339();
        let now = chrono::Utc::now().to_rfc3339();
        let msg = |ts: &str, id: &str, out: u64| {
            format!(r#"{{"type":"assistant","timestamp":"{ts}","message":{{"id":"{id}","usage":{{"output_tokens":{out}}}}}}}"#)
        };
        std::fs::write(&file, format!("{}\n{}\n", msg(&old, "a", 40), msg(&now, "b", 2))).unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        prepare(&mut conn).unwrap();
        let projects = root.join("projects");

        sync_dir(&mut conn, &projects, None).unwrap();
        assert_eq!(total_tokens(&conn), 42, "200-day-old usage is kept");
        assert_eq!(days(&conn, Days::since(Some(&day_n_ago(7)))).unwrap().len(), 1);
        assert_eq!(session_count(&conn, Days::default()).unwrap(), 1);

        // Claude Code deleting the transcript doesn't remove its history.
        std::fs::remove_file(&file).unwrap();
        sync_dir(&mut conn, &projects, None).unwrap();
        assert_eq!(total_tokens(&conn), 42);

        // Setting a retention prunes on the next sync, but the all-time total keeps the tokens.
        sync_dir(&mut conn, &projects, Some(30)).unwrap();
        assert_eq!(total_tokens(&conn), 2);
        let all = lifetime(&conn).unwrap();
        assert_eq!((all.total, all.output), (42, 42));
        assert!(all.since.as_deref().is_some_and(|d| d < day_n_ago(100).as_str()), "since the pruned day");

        // Re-indexing and re-pruning never counts a pruned day twice.
        sync_dir(&mut conn, &projects, Some(30)).unwrap();
        sync_dir(&mut conn, &projects, Some(7)).unwrap();
        assert_eq!(lifetime(&conn).unwrap().total, 42);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn growing_the_retention_brings_detail_back_without_double_counting() {
        let root = std::env::temp_dir().join(format!("marsad-store-grow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("projects").join("-p-grow");
        std::fs::create_dir_all(&proj).unwrap();
        let file = proj.join("s4.jsonl");
        let msg = |days_ago: i64, id: &str, out: u64| {
            let ts = (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339();
            format!(r#"{{"type":"assistant","timestamp":"{ts}","message":{{"id":"{id}","usage":{{"output_tokens":{out}}}}}}}"#)
        };
        std::fs::write(&file, format!("{}\n{}\n", msg(20, "a", 5), msg(0, "b", 2))).unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        prepare(&mut conn).unwrap();
        let projects = root.join("projects");

        assert_eq!(lifetime(&conn).unwrap(), Lifetime::default(), "empty history");
        sync_dir(&mut conn, &projects, Some(30)).unwrap();
        sync_dir(&mut conn, &projects, Some(7)).unwrap();
        assert_eq!((total_tokens(&conn), lifetime(&conn).unwrap().total), (2, 7));

        // The transcript changes and the retention goes back to 30: the 20-day-old turn's detail
        // returns, and the all-time total still counts it once.
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{}", msg(0, "c", 1)).unwrap();
        drop(f);
        sync_dir(&mut conn, &projects, Some(30)).unwrap();
        assert_eq!((total_tokens(&conn), lifetime(&conn).unwrap().total), (8, 8));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn periods_total_and_average_over_calendar_days() {
        let mut conn = Connection::open_in_memory().unwrap();
        prepare(&mut conn).unwrap();
        conn.execute_batch(
            "INSERT INTO token_days VALUES ('a', 'main', '2026-10-23', 0, 0, 0, 0, 10, 1);
             INSERT INTO token_days VALUES ('b', 'main', '2026-10-20', 0, 0, 0, 0, 20, 1);
             INSERT INTO token_days VALUES ('c', 'main', '2026-09-12', 0, 0, 0, 0, 60, 1);",
        )
        .unwrap();
        let got: Vec<(&str, i64, i64, i64)> =
            periods(&conn, "2026-10-23").unwrap().iter().map(|p| (p.period, p.days, p.total, p.daily_average)).collect();
        assert_eq!(got, [("today", 1, 10, 10), ("7d", 7, 30, 4), ("30d", 30, 30, 1), ("all", 42, 90, 2)]);

        let mut empty = Connection::open_in_memory().unwrap();
        prepare(&mut empty).unwrap();
        let all = periods(&empty, "2026-10-23").unwrap();
        assert!(all.iter().all(|p| p.total == 0 && p.daily_average == 0));
        assert_eq!(all[3].days, 0, "no usage, no start day");
    }

    /// A database as versions before 0.4 left it: pruned days in `lifetime`, recent ones in detail.
    fn pre_upgrade_db(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_daily (session_id TEXT, agent_id TEXT, agent_type TEXT, day TEXT, input INTEGER,
                 output INTEGER, cache_read INTEGER, cache_write INTEGER, total INTEGER, turns INTEGER,
                 PRIMARY KEY (session_id, agent_id, day));
             CREATE TABLE files (path TEXT PRIMARY KEY, size INTEGER, mtime REAL);
             CREATE TABLE lifetime (id INTEGER PRIMARY KEY CHECK (id = 1), since TEXT, through TEXT, input INTEGER,
                 output INTEGER, cache_read INTEGER, cache_write INTEGER, total INTEGER);
             INSERT INTO usage_daily VALUES ('s1', 'main', 'main', '2026-10-20', 0, 7, 0, 0, 7, 2);
             INSERT INTO files VALUES ('/gone.jsonl', 1, 1.0);
             INSERT INTO lifetime VALUES (1, '2026-08-01', '2026-09-23', 0, 50, 0, 0, 50);",
        )
        .unwrap();
    }

    #[test]
    fn upgrade_keeps_the_all_time_total() {
        let root = std::env::temp_dir().join(format!("marsad-upgrade-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let db = root.join("usage.db");
        pre_upgrade_db(&db);

        // Covers AE2. Read-only first, as `marsad total --no-sync` would.
        assert_eq!(lifetime(&open_readonly_at(&db).unwrap()).unwrap().total, 57);
        let conn = open_at(&db).unwrap();
        let all = lifetime(&conn).unwrap();
        assert_eq!((all.total, all.since.as_deref()), (57, Some("2026-08-01")), "migrated once");
        let legacy: i64 = conn.query_row("SELECT COUNT(*) FROM legacy_totals", [], |r| r.get(0)).unwrap();
        assert_eq!(legacy, 1);
        assert_eq!(legacy_ranges(&conn).unwrap(), [("2026-08-01".to_string(), "2026-09-23".to_string())]);
        let files: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(files, 0, "transcripts are re-read once");
        let id = install_id(&conn).unwrap();
        drop(conn);
        assert_eq!(install_id(&open_at(&db).unwrap()).unwrap(), id, "the install id is stable");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn days_inside_a_legacy_sum_are_not_counted_again() {
        let root = std::env::temp_dir().join(format!("marsad-store-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("projects").join("-p-legacy");
        std::fs::create_dir_all(&proj).unwrap();
        let msg = |days_ago: i64, id: &str, out: u64| {
            let ts = (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339();
            format!(r#"{{"type":"assistant","timestamp":"{ts}","message":{{"id":"{id}","usage":{{"output_tokens":{out}}}}}}}"#)
        };
        // Days 60 and 0 are free; day 20 lies inside a legacy range from day 40 to day 10.
        std::fs::write(proj.join("s5.jsonl"), format!("{}\n{}\n{}\n", msg(60, "a", 1), msg(20, "b", 10), msg(0, "c", 100))).unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        prepare(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO legacy_totals SELECT value, ?1, ?2, 0, 10, 0, 0, 10 FROM meta WHERE key = 'install_id'",
            [day_n_ago(41), day_n_ago(11)],
        )
        .unwrap();
        sync_dir(&mut conn, &root.join("projects"), None).unwrap();
        let counted: Vec<i64> = days(&conn, Days::default()).unwrap().iter().map(|d| d.total).collect();
        assert_eq!(counted, [100, 1], "the day before the legacy range is counted; the one inside is not");
        assert_eq!(lifetime(&conn).unwrap().total, 111);
        std::fs::remove_dir_all(root).ok();
    }

    /// A version-1 database: a legacy range holding a usage_daily day with no counter row (kept
    /// forever), and skill runs indexed before `skills_seen` existed.
    fn v1_db(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO meta VALUES ('install_id', 'me');
             INSERT INTO meta VALUES ('schema', '1');
             INSERT INTO legacy_totals VALUES ('me', '2026-08-01', '2026-09-23', 0, 50, 0, 0, 50);
             INSERT INTO usage_daily VALUES ('s1', 'main', 'main', '2026-08-10', 0, 9, 0, 0, 9, 1);
             INSERT INTO token_days VALUES ('s1', 'main', '2026-10-20', 0, 7, 0, 0, 7, 2);
             INSERT INTO skill_runs VALUES ('s1', 'main', 'main', 0, 'ce-plan', 'tool', 't', '2026-10-20', 1, 0, 1, 0, 0, 1);
             INSERT INTO skill_runs VALUES ('s1', 'main', 'main', 1, 'ce-plan', 'tool', 't', '2026-10-02', 1, 0, 1, 0, 0, 1);
             INSERT INTO skill_runs VALUES ('s2', 'main', 'main', 0, 'ce-work', 'command', 't', '2026-10-21', 1, 0, 1, 0, 0, 1);
             INSERT INTO files VALUES ('/a.jsonl', 1, 1.0);",
        )
        .unwrap();
    }

    fn skills_seen(conn: &Connection) -> Vec<Value> {
        query(conn, "SELECT skill, first_day FROM skills_seen ORDER BY skill", &[]).unwrap().rows.into_iter().map(Value::from).collect()
    }

    #[test]
    fn v1_database_migrates_to_v2_once() {
        let root = std::env::temp_dir().join(format!("marsad-v2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let db = root.join("usage.db");
        v1_db(&db);

        let conn = open_at(&db).unwrap();
        assert_eq!(schema_version(&conn), 2);
        assert_eq!(skills_seen(&conn), [json!(["ce-plan", "2026-10-02"]), json!(["ce-work", "2026-10-21"])], "earliest day per skill");
        let files: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(files, 0, "transcripts are re-read once");
        // The v1 copy must not run again: the legacy-range usage_daily day stays out of the counter.
        let counted = query(&conn, "SELECT day, total FROM token_days", &[]).unwrap().rows;
        assert_eq!(counted, vec![vec![json!("2026-10-20"), json!(7)]]);
        assert_eq!(lifetime(&conn).unwrap().total, 57);
        drop(conn);
        assert_eq!(skills_seen(&open_at(&db).unwrap()).len(), 2, "reopening changes nothing");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn skills_seen_outlives_retention_and_reindexing() {
        let root = std::env::temp_dir().join(format!("marsad-store-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("projects").join("-p-skills");
        std::fs::create_dir_all(&proj).unwrap();
        let file = proj.join("s6.jsonl");
        let skill = |days_ago: i64, id: &str, name: &str| {
            let ts = (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339();
            format!(r#"{{"type":"assistant","timestamp":"{ts}","message":{{"id":"{id}","usage":{{"output_tokens":1}},"content":[{{"type":"tool_use","name":"Skill","input":{{"skill":"{name}"}}}}]}}}}"#)
        };
        std::fs::write(&file, format!("{}\n{}\n{}\n", skill(40, "a", "old-skill"), skill(20, "b", "mid-skill"), skill(0, "c", "new-skill"))).unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        prepare(&mut conn).unwrap();
        let projects = root.join("projects");
        let runs = |conn: &Connection| -> i64 { conn.query_row("SELECT COUNT(*) FROM skill_runs", [], |r| r.get(0)).unwrap() };

        sync_dir(&mut conn, &projects, Some(30)).unwrap();
        assert_eq!(runs(&conn), 2, "the 40-day-old run is past the retention");
        let seen = skills_seen(&conn);
        assert_eq!(seen.len(), 3, "but its skill is still seen: {seen:?}");
        assert_eq!(seen[2], json!(["old-skill", transcript::local_day(&(chrono::Utc::now() - chrono::Duration::days(40)).to_rfc3339()).unwrap()]));

        // Re-reading the same transcript never double counts or moves a first day.
        let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "{}", skill(0, "d", "new-skill")).unwrap();
        drop(f);
        assert_eq!(sync_dir(&mut conn, &projects, Some(30)).unwrap().updated, 1);
        assert_eq!(skills_seen(&conn), seen);

        // Pruning skill runs leaves the seen skills.
        sync_dir(&mut conn, &projects, Some(7)).unwrap();
        assert_eq!(runs(&conn), 2, "only today's two runs are kept");
        assert_eq!(skills_seen(&conn), seen);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn best_values_never_go_down() {
        let mut conn = Connection::open_in_memory().unwrap();
        prepare(&mut conn).unwrap();
        assert!(badge_best(&conn).unwrap().is_empty());
        raise_best(&conn, "streak", 7).unwrap();
        raise_best(&conn, "streak", 3).unwrap();
        raise_best(&conn, "rank", 10).unwrap();
        raise_best(&conn, "rank", 12).unwrap();
        let best = badge_best(&conn).unwrap();
        assert_eq!((best["streak"], best["rank"], best.len()), (7, 12, 2), "lower keeps the old value; higher replaces it");
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
