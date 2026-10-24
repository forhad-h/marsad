//! `marsad export` and `marsad import`: move or back up all marsad data as one JSON file.
//!
//! Import merges instead of replacing, so importing the same file twice, or this machine's own
//! export, never counts tokens twice: token rows keep the larger figure per key, legacy totals
//! merge per source install, and days inside any legacy range never enter the day counter.

use crate::store;
use anyhow::{Context, bail};
use rusqlite::{Connection, params_from_iter, types::Value as SqlValue};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

/// Format version written by this build; import refuses newer ones.
const FORMAT: u64 = 1;

/// One exported table: its columns, primary key, and the upsert that merges an imported row.
struct Table {
    name: &'static str,
    columns: &'static [&'static str],
    key: &'static [&'static str],
    /// `ON CONFLICT` clause applied when the key already exists.
    merge: &'static str,
}

/// In merge order: legacy ranges first, so they can block day-counter rows in the same file.
const TABLES: &[Table] = &[
    Table {
        name: "legacy_totals",
        columns: &["source", "since", "through", "input", "output", "cache_read", "cache_write", "total"],
        key: &["source"],
        merge: "DO UPDATE SET since = MIN(since, excluded.since), through = MAX(through, excluded.through),
            input = MAX(input, excluded.input), output = MAX(output, excluded.output),
            cache_read = MAX(cache_read, excluded.cache_read), cache_write = MAX(cache_write, excluded.cache_write),
            total = MAX(total, excluded.total)",
    },
    Table {
        name: "token_days",
        columns: &["session_id", "agent_id", "day", "input", "output", "cache_read", "cache_write", "total", "turns"],
        key: &["session_id", "agent_id", "day"],
        merge: "DO UPDATE SET input = MAX(input, excluded.input), output = MAX(output, excluded.output),
            cache_read = MAX(cache_read, excluded.cache_read), cache_write = MAX(cache_write, excluded.cache_write),
            total = MAX(total, excluded.total), turns = MAX(turns, excluded.turns)",
    },
    Table {
        name: "sessions",
        columns: &["session_id", "project", "cwd", "title", "model", "first_at", "last_at", "transcript_path"],
        key: &["session_id"],
        merge: "DO UPDATE SET project = COALESCE(project, excluded.project), cwd = COALESCE(cwd, excluded.cwd),
            title = COALESCE(title, excluded.title), model = COALESCE(model, excluded.model),
            first_at = COALESCE(first_at, excluded.first_at), last_at = COALESCE(last_at, excluded.last_at),
            transcript_path = COALESCE(transcript_path, excluded.transcript_path)",
    },
    Table {
        name: "usage_daily",
        columns: &[
            "session_id", "agent_id", "agent_type", "day", "input", "output", "cache_read", "cache_write", "total", "turns",
        ],
        key: &["session_id", "agent_id", "day"],
        merge: "DO UPDATE SET input = MAX(input, excluded.input), output = MAX(output, excluded.output),
            cache_read = MAX(cache_read, excluded.cache_read), cache_write = MAX(cache_write, excluded.cache_write),
            total = MAX(total, excluded.total), turns = MAX(turns, excluded.turns)",
    },
    Table {
        name: "skill_runs",
        columns: &[
            "session_id", "agent_id", "agent_type", "seq", "skill", "via", "at", "day", "turns", "input", "output",
            "cache_read", "cache_write", "total",
        ],
        key: &["session_id", "agent_id", "seq"],
        merge: "DO UPDATE SET agent_type = excluded.agent_type, skill = excluded.skill, via = excluded.via,
            at = excluded.at, day = excluded.day, turns = excluded.turns, input = excluded.input,
            output = excluded.output, cache_read = excluded.cache_read, cache_write = excluded.cache_write,
            total = excluded.total WHERE excluded.total > skill_runs.total",
    },
];

/// Columns that must hold a number for a row to be importable (the rest may be null).
fn numeric(column: &str) -> bool {
    matches!(column, "input" | "output" | "cache_read" | "cache_write" | "total" | "turns" | "seq")
}

/// The whole export as JSON.
fn export_value(conn: &Connection, settings: Map<String, Value>) -> anyhow::Result<Value> {
    let mut out = Map::new();
    out.insert("marsad_export".into(), json!(FORMAT));
    out.insert("exported_at".into(), json!(chrono::Utc::now().to_rfc3339()));
    out.insert("source".into(), json!(store::install_id(conn)?));
    out.insert("settings".into(), Value::Object(settings));
    for t in TABLES {
        let sql = format!("SELECT {} FROM {}", t.columns.join(", "), t.name);
        let rows = store::query(conn, &sql, &[])?;
        let list: Vec<Value> = rows
            .rows
            .into_iter()
            .map(|r| Value::Object(rows.columns.iter().cloned().zip(r).collect()))
            .collect();
        out.insert(t.name.into(), Value::Array(list));
    }
    Ok(Value::Object(out))
}

/// What one table's import did.
#[derive(Debug, Default, PartialEq)]
struct Merged {
    added: usize,
    known: usize,
    skipped: usize,
}

/// Merges an export into `conn`. A table that can't be read is reported and the others continue.
fn import_value(conn: &mut Connection, export: &Value, restore: bool) -> anyhow::Result<Vec<(&'static str, Result<Merged, String>)>> {
    match export["marsad_export"].as_u64() {
        Some(FORMAT) => {}
        Some(v) => bail!("this file is export format {v}; this marsad reads format {FORMAT}. Update marsad first"),
        None => bail!("not a marsad export file"),
    }
    let mut results = Vec::new();
    for t in TABLES {
        let rows = export.get(t.name).and_then(Value::as_array);
        let res = match rows {
            None => Err("missing from the file".to_string()),
            Some(rows) => merge_table(conn, t, rows, restore).map_err(|e| e.to_string()),
        };
        results.push((t.name, res));
    }
    Ok(results)
}

/// `restore` treats the file as this machine's own earlier data, so its legacy ranges block the
/// day counter here too.
fn merge_table(conn: &mut Connection, t: &Table, rows: &[Value], restore: bool) -> anyhow::Result<Merged> {
    let tx = conn.transaction()?;
    let count = |tx: &rusqlite::Transaction| -> anyhow::Result<usize> {
        Ok(tx.query_row(&format!("SELECT COUNT(*) FROM {}", t.name), [], |r| r.get::<_, i64>(0))? as usize)
    };
    let before = count(&tx)?;
    let placeholders = vec!["?"; t.columns.len()].join(", ");
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({placeholders}) ON CONFLICT({}) {}",
        t.name,
        t.columns.join(", "),
        t.key.join(", "),
        t.merge
    );
    let ranges = store::legacy_ranges(&tx)?;
    let mut merged = Merged::default();
    {
        let mut stmt = tx.prepare(&sql)?;
        for row in rows {
            let Some(values) = row_values(t, row) else {
                merged.skipped += 1;
                continue;
            };
            // Days a legacy total already counts never enter the day counter.
            if t.name == "token_days"
                && let Some(SqlValue::Text(day)) = t.columns.iter().position(|c| *c == "day").and_then(|i| values.get(i))
                && store::in_legacy(&ranges, day) {
                    merged.known += 1;
                    continue;
                }
            stmt.execute(params_from_iter(values))?;
        }
    }
    if t.name == "legacy_totals" && restore {
        // This machine may have indexed copies of the same transcripts, so the restored days
        // must not count in both the legacy sum and the day counter.
        for source in rows.iter().filter_map(|r| r["source"].as_str()) {
            tx.execute("INSERT OR IGNORE INTO meta VALUES ('restored:' || ?1, '1')", [source])?;
        }
        tx.execute(
            &format!(
                "DELETE FROM token_days WHERE EXISTS (SELECT 1 FROM legacy_totals l
                   WHERE {} AND token_days.day >= l.since AND token_days.day < l.through)",
                store::BLOCKING_LEGACY
            ),
            [],
        )?;
    }
    merged.added = count(&tx)?.saturating_sub(before);
    merged.known = rows.len() - merged.skipped - merged.added;
    tx.commit()?;
    Ok(merged)
}

/// A row's values in column order, or None when a key is missing or a number isn't one.
fn row_values(t: &Table, row: &Value) -> Option<Vec<SqlValue>> {
    t.columns
        .iter()
        .map(|c| match row.get(*c).unwrap_or(&Value::Null) {
            Value::Null if t.key.contains(c) || numeric(c) => None,
            Value::Null => Some(SqlValue::Null),
            Value::Number(n) if numeric(c) => n.as_i64().map(SqlValue::Integer),
            Value::String(s) if !numeric(c) => Some(SqlValue::Text(s.clone())),
            _ => None,
        })
        .collect()
}

pub fn export(file: Option<PathBuf>) -> anyhow::Result<()> {
    let mut conn = store::open()?;
    store::sync(&mut conn)?;
    let value = export_value(&conn, crate::settings::portable())?;
    let path = file.unwrap_or_else(|| PathBuf::from(format!("marsad-export-{}.json", store::day_n_ago(1))));
    std::fs::write(&path, serde_json::to_vec_pretty(&value)?).with_context(|| format!("writing {}", path.display()))?;
    let days = value["token_days"].as_array().map_or(0, Vec::len);
    println!("Exported marsad data to {} ({days} session-days).", path.display());
    println!("On the other machine: `marsad import {}`", path.display());
    Ok(())
}

pub fn import(file: &Path, restore: bool) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let value: Value = serde_json::from_str(&text).context("not a marsad export file (invalid JSON)")?;
    let mut conn = store::open()?;
    let before = store::lifetime(&conn)?.total;
    for (table, res) in import_value(&mut conn, &value, restore)? {
        match res {
            Ok(m) => {
                let skipped = if m.skipped > 0 { format!(", {} unreadable skipped", m.skipped) } else { String::new() };
                println!("{table}: {} added, {} already known{skipped}", m.added, m.known);
            }
            Err(e) => println!("{table}: not imported ({e})"),
        }
    }
    match value["settings"].as_object() {
        Some(settings) => {
            let applied = crate::settings::merge_missing(settings)?;
            if applied.is_empty() {
                println!("settings: kept this machine's");
            } else {
                println!("settings: applied {}", applied.join(", "));
            }
        }
        None => println!("settings: none in the file"),
    }
    let after = store::lifetime(&conn)?.total;
    println!(
        "All-time tokens: {} (+{})",
        crate::model::fmt_tokens(after.max(0) as u64),
        crate::model::fmt_tokens((after - before).max(0) as u64)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(rows: &str) -> Connection {
        let root = std::env::temp_dir().join(format!("marsad-portable-{}-{}", std::process::id(), rand_suffix()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("usage.db");
        drop(store::open_at(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(rows).unwrap();
        conn
    }

    fn rand_suffix() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }

    fn all_time(conn: &Connection) -> i64 {
        store::lifetime(conn).unwrap().total
    }

    const MACHINE_A: &str = "
        INSERT INTO legacy_totals
          SELECT value, '2026-07-01', '2026-08-01', 0, 0, 0, 0, 1000 FROM meta WHERE key = 'install_id';
        INSERT INTO token_days VALUES ('sa', 'main', '2026-09-01', 0, 10, 0, 0, 10, 1);
        INSERT INTO usage_daily VALUES ('sa', 'main', 'main', '2026-09-01', 0, 10, 0, 0, 10, 1);
        INSERT INTO sessions (session_id, project) VALUES ('sa', 'web');
        INSERT INTO skill_runs VALUES ('sa', 'main', 'main', 0, 'ce-plan', 'tool', 't', '2026-09-01', 1, 0, 5, 0, 0, 5);";

    #[test]
    fn importing_twice_counts_once() {
        let a = db(MACHINE_A);
        let export = export_value(&a, Map::new()).unwrap();
        let mut b = db("INSERT INTO token_days VALUES ('sb', 'main', '2026-09-02', 0, 7, 0, 0, 7, 1);");
        assert_eq!(all_time(&b), 7);

        // Covers AE5.
        let first = import_value(&mut b, &export, false).unwrap();
        assert_eq!(all_time(&b), 7 + 10 + 1000);
        assert!(first.iter().all(|(_, r)| r.as_ref().is_ok_and(|m| m.skipped == 0)));
        let again = import_value(&mut b, &export, false).unwrap();
        assert_eq!(all_time(&b), 7 + 10 + 1000, "a second import adds nothing");
        assert!(again.iter().all(|(_, r)| r.as_ref().is_ok_and(|m| m.added == 0)));
        let legacy: i64 = b.query_row("SELECT COUNT(*) FROM legacy_totals", [], |r| r.get(0)).unwrap();
        assert_eq!(legacy, 1);
    }

    #[test]
    fn importing_this_machines_own_export_changes_nothing() {
        let mut a = db(MACHINE_A);
        let before = all_time(&a);
        let export = export_value(&a, Map::new()).unwrap();
        import_value(&mut a, &export, false).unwrap();
        assert_eq!(all_time(&a), before);
    }

    #[test]
    fn merging_another_machine_keeps_this_machines_days_in_its_pruned_range() {
        let a = db(MACHINE_A);
        let export = export_value(&a, Map::new()).unwrap();
        // B's own session on a day A pruned before 0.4: A's legacy sum doesn't include it.
        let mut b = db("INSERT INTO token_days VALUES ('own', 'main', '2026-07-15', 0, 300, 0, 0, 300, 1);");
        import_value(&mut b, &export, false).unwrap();
        assert_eq!(all_time(&b), 1000 + 10 + 300);
        assert!(store::legacy_ranges(&b).unwrap().is_empty(), "B keeps indexing its own days in A's range");
    }

    #[test]
    fn restore_after_reindexing_copied_transcripts_counts_legacy_days_once() {
        let a = db(MACHINE_A);
        let export = export_value(&a, Map::new()).unwrap();
        // B indexed a copy of a transcript whose 2026-07-15 usage is inside A's legacy sum.
        let mut b = db("INSERT INTO token_days VALUES ('old', 'main', '2026-07-15', 0, 300, 0, 0, 300, 1);");
        import_value(&mut b, &export, true).unwrap();
        assert_eq!(all_time(&b), 1000 + 10, "the copied day is counted by the legacy sum only");
        let ranges = store::legacy_ranges(&b).unwrap();
        assert_eq!(ranges, [("2026-07-01".to_string(), "2026-08-01".to_string())], "re-indexing stays blocked");
        import_value(&mut b, &export, false).unwrap();
        assert_eq!(all_time(&b), 1000 + 10, "a later plain import of the same file changes nothing");
    }

    #[test]
    fn bad_files_and_rows_are_reported() {
        let mut b = db("");
        let newer = json!({ "marsad_export": 2 });
        assert!(import_value(&mut b, &newer, false).unwrap_err().to_string().contains("format 2"));
        assert!(import_value(&mut b, &json!({ "hello": 1 }), false).is_err());

        let partial = json!({
            "marsad_export": 1,
            "token_days": [
                { "session_id": "s", "agent_id": "main", "day": "2026-09-01", "input": 0, "output": 1,
                  "cache_read": 0, "cache_write": 0, "total": 1, "turns": 1 },
                { "session_id": "s", "agent_id": "main", "day": "2026-09-02", "input": 0, "output": 1 },
            ],
        });
        let res = import_value(&mut b, &partial, false).unwrap();
        let days = res.iter().find(|(t, _)| *t == "token_days").unwrap();
        assert_eq!(days.1, Ok(Merged { added: 1, known: 0, skipped: 1 }));
        let sessions = res.iter().find(|(t, _)| *t == "sessions").unwrap();
        assert!(sessions.1.is_err(), "a missing table is reported, not fatal");
        assert_eq!(all_time(&b), 1);
    }
}
