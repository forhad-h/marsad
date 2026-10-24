//! Badges: a rank from all-time tokens and bronze/silver/gold achievements, computed from the
//! permanent counters and kept at their best value so pruning never revokes them.

use crate::model::{DAY_FMT, MAIN_AGENT};
use crate::report::Format;
use crate::store;
use clap::Args;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

/// One rank on the craft ladder.
pub struct Tier {
    pub name: &'static str,
    /// All-time tokens needed.
    pub min: i64,
}

/// Ranks from all-time tokens, cache reads included (the dashboard header's figure).
pub const TIERS: [Tier; 7] = [
    Tier { name: "Novice", min: 0 },
    Tier { name: "Apprentice", min: 1_000_000 },
    Tier { name: "Practitioner", min: 10_000_000 },
    Tier { name: "Specialist", min: 50_000_000 },
    Tier { name: "Expert", min: 250_000_000 },
    Tier { name: "Master", min: 1_000_000_000 },
    Tier { name: "Grandmaster", min: 5_000_000_000 },
];

pub const LEVELS: [&str; 3] = ["bronze", "silver", "gold"];

/// One achievement and the values its bronze, silver and gold levels need.
pub struct Achievement {
    /// Stat key, also the `badge_best` key.
    pub key: &'static str,
    pub name: &'static str,
    /// Short name for announcements ("gold streak").
    pub short: &'static str,
    pub unit: &'static str,
    pub levels: [i64; 3],
}

pub const ACHIEVEMENTS: [Achievement; 4] = [
    Achievement { key: "streak", name: "Longest streak", short: "streak", unit: "days", levels: [3, 7, 30] },
    Achievement { key: "skills", name: "Distinct skills", short: "skills", unit: "skills", levels: [3, 10, 25] },
    Achievement { key: "subagents", name: "Subagents run", short: "subagents", unit: "subagents", levels: [10, 100, 1000] },
    Achievement {
        key: "busiest_day",
        name: "Busiest day",
        short: "busiest day",
        unit: "tokens",
        levels: [10_000_000, 50_000_000, 200_000_000],
    },
];

pub const RANK_KEY: &str = "rank";

/// The value behind every badge, keyed like `badge_best`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats(pub BTreeMap<&'static str, i64>);

impl Stats {
    pub fn get(&self, key: &str) -> i64 {
        self.0.get(key).copied().unwrap_or(0)
    }
}

/// Each stat from its permanent source, with `rank_total` as the rank figure (`store::lifetime`
/// in the CLI, the header's live total in the dashboard).
pub fn compute(conn: &Connection, rank_total: i64) -> anyhow::Result<Stats> {
    let mut days = conn.prepare("SELECT day, SUM(total) FROM token_days GROUP BY day HAVING SUM(total) > 0 ORDER BY day")?;
    let days: Vec<(String, i64)> = days.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let subagents: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT DISTINCT session_id, agent_id FROM token_days WHERE agent_id != ?1)",
        [MAIN_AGENT],
        |r| r.get(0),
    )?;
    let skills: i64 = conn.query_row("SELECT COUNT(*) FROM skills_seen", [], |r| r.get(0))?;
    let mut stats = BTreeMap::new();
    stats.insert(RANK_KEY, rank_total.max(0));
    stats.insert("streak", longest_streak(days.iter().map(|(d, _)| d.as_str())));
    stats.insert("skills", skills);
    stats.insert("subagents", subagents);
    stats.insert("busiest_day", days.iter().map(|(_, t)| *t).max().unwrap_or(0));
    Ok(Stats(stats))
}

/// Longest run of consecutive calendar days in a sorted list of days.
fn longest_streak<'a>(days: impl Iterator<Item = &'a str>) -> i64 {
    let (mut best, mut run) = (0, 0);
    let mut prev: Option<chrono::NaiveDate> = None;
    for day in days {
        let Ok(d) = chrono::NaiveDate::parse_from_str(day, DAY_FMT) else { continue };
        run = if prev.is_some_and(|p| d.signed_duration_since(p).num_days() == 1) { run + 1 } else { 1 };
        best = best.max(run);
        prev = Some(d);
    }
    best
}

/// Raises the stored best values to the stats computed now (on a writable connection, after a
/// sync); returns the shown values.
pub fn record(conn: &Connection, rank_total: i64) -> anyhow::Result<Stats> {
    let now = compute(conn, rank_total)?;
    for (key, value) in &now.0 {
        raise_best(conn, key, *value)?;
    }
    shown_from(conn, now)
}

/// Highest value ever stored per badge stat.
fn badge_best(conn: &Connection) -> anyhow::Result<std::collections::HashMap<String, i64>> {
    let mut stmt = conn.prepare("SELECT key, value FROM badge_best")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Raises one stat's best value to `value`; a lower value leaves it as it was.
fn raise_best(conn: &Connection, key: &str, value: i64) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO badge_best VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// The shown values: each computed stat, never lower than its stored best. Writes nothing.
pub fn shown(conn: &Connection, rank_total: i64) -> anyhow::Result<Stats> {
    shown_from(conn, compute(conn, rank_total)?)
}

fn shown_from(conn: &Connection, mut stats: Stats) -> anyhow::Result<Stats> {
    let best = badge_best(conn)?;
    for (key, value) in stats.0.iter_mut() {
        *value = (*value).max(best.get(*key).copied().unwrap_or(0));
    }
    Ok(stats)
}

/// The rank and how far it is to the next one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Rank {
    pub tier: usize,
    pub name: &'static str,
    pub tokens: i64,
    pub next: Option<&'static str>,
    pub next_threshold: Option<i64>,
    /// 0.0..=1.0 from this tier's threshold to the next; 1.0 at the top.
    pub progress: f64,
}

/// One achievement's level and how far it is to the next.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Level {
    pub key: &'static str,
    pub name: &'static str,
    pub value: i64,
    pub unit: &'static str,
    /// 0 none, 1 bronze, 2 silver, 3 gold.
    pub level: usize,
    pub level_name: Option<&'static str>,
    pub next_level: Option<&'static str>,
    pub next_threshold: Option<i64>,
    pub progress: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Badges {
    pub rank: Rank,
    pub achievements: Vec<Level>,
}

fn progress(value: i64, from: i64, to: Option<i64>) -> f64 {
    match to {
        Some(to) if to > from => ((value - from) as f64 / (to - from) as f64).clamp(0.0, 1.0),
        _ => 1.0,
    }
}

/// Levels and progress from shown values.
pub fn badges(stats: &Stats) -> Badges {
    let tokens = stats.get(RANK_KEY);
    let tier = TIERS.iter().rposition(|t| tokens >= t.min).unwrap_or(0);
    let next = TIERS.get(tier + 1);
    let rank = Rank {
        tier,
        name: TIERS[tier].name,
        tokens,
        next: next.map(|t| t.name),
        next_threshold: next.map(|t| t.min),
        progress: progress(tokens, TIERS[tier].min, next.map(|t| t.min)),
    };
    let achievements = ACHIEVEMENTS
        .iter()
        .map(|a| {
            let value = stats.get(a.key);
            let level = a.levels.iter().filter(|&&min| value >= min).count();
            let from = if level == 0 { 0 } else { a.levels[level - 1] };
            let next_threshold = a.levels.get(level).copied();
            Level {
                key: a.key,
                name: a.name,
                value,
                unit: a.unit,
                level,
                level_name: level.checked_sub(1).map(|i| LEVELS[i]),
                next_level: LEVELS.get(level).copied(),
                next_threshold,
                progress: progress(value, from, next_threshold),
            }
        })
        .collect();
    Badges { rank, achievements }
}

/// The last announced level per stat (rank: tier index; achievements: 0 none to 3 gold).
pub type Levels = BTreeMap<String, u64>;

pub fn levels(b: &Badges) -> Levels {
    let mut out = Levels::new();
    out.insert(RANK_KEY.into(), b.rank.tier as u64);
    for a in &b.achievements {
        out.insert(a.key.into(), a.level as u64);
    }
    out
}

/// One combined note for every level above the recorded one ("rank up: Expert · new: gold
/// streak"), or None when nothing rose. A stat missing from `recorded` counts as unchanged.
fn diff(recorded: &Levels, b: &Badges) -> Option<String> {
    let rose = |key: &str, now: usize| recorded.get(key).is_some_and(|&was| (now as u64) > was);
    let mut parts = Vec::new();
    if rose(RANK_KEY, b.rank.tier) {
        parts.push(format!("rank up: {}", b.rank.name));
    }
    let new: Vec<String> = b
        .achievements
        .iter()
        .zip(&ACHIEVEMENTS)
        .filter(|(l, _)| rose(l.key, l.level))
        .filter_map(|(l, a)| l.level_name.map(|n| format!("{n} {}", a.short)))
        .collect();
    if !new.is_empty() {
        parts.push(format!("new: {}", new.join(", ")));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Recorded levels raised to the current ones (never lowered).
fn merged(recorded: Option<Levels>, b: &Badges) -> Levels {
    let mut out = recorded.unwrap_or_default();
    for (key, level) in levels(b) {
        let slot = out.entry(key).or_insert(level);
        *slot = (*slot).max(level);
    }
    out
}

/// The note to show for levels that rose since the last announcement. The first time (no
/// `badges_announced` yet) the current levels are recorded silently instead, as are stats the
/// record doesn't know yet.
fn pending_at(path: &Path, b: &Badges) -> anyhow::Result<Option<String>> {
    let Some(recorded) = crate::settings::badges_announced_at(path) else {
        crate::settings::set_badges_announced_at(path, &levels(b))?;
        return Ok(None);
    };
    let note = diff(&recorded, b);
    if note.is_none() && levels(b).keys().any(|k| !recorded.contains_key(k)) {
        crate::settings::set_badges_announced_at(path, &merged(Some(recorded), b))?;
    }
    Ok(note)
}

/// Records the current levels as announced, once the dashboard has drawn the note.
fn mark_at(path: &Path, b: &Badges) -> anyhow::Result<()> {
    crate::settings::set_badges_announced_at(path, &merged(crate::settings::badges_announced_at(path), b))
}

pub fn pending_announcement(b: &Badges) -> anyhow::Result<Option<String>> {
    pending_at(&crate::settings::path(), b)
}

pub fn mark_announced(b: &Badges) -> anyhow::Result<()> {
    mark_at(&crate::settings::path(), b)
}

#[derive(Args)]
pub struct BadgesArgs {
    #[arg(long, value_enum, default_value = "table")]
    format: Format,
    /// Don't index new transcript data first.
    #[arg(long)]
    no_sync: bool,
}

/// `marsad badges`: the rank, each achievement's level and progress, and the stats behind them.
pub fn run(a: BadgesArgs) -> anyhow::Result<()> {
    if !a.no_sync {
        // The sync raises the stored best values.
        store::open_and_sync()?;
    }
    let conn = store::open_readonly()?;
    let total = store::lifetime(&conn)?.total;
    let b = badges(&shown(&conn, total)?);
    print!("{}", render(&b, a.format));
    Ok(())
}

/// Thresholds read as 1M, 250M, 5B.
fn fmt_threshold(n: i64, unit: &str) -> String {
    if unit != "tokens" {
        return format!("{n} {unit}");
    }
    let n = n as f64;
    let s = match n {
        _ if n >= 1e9 => format!("{}B", trim(n / 1e9)),
        _ if n >= 1e6 => format!("{}M", trim(n / 1e6)),
        _ if n >= 1e3 => format!("{}k", trim(n / 1e3)),
        _ => trim(n),
    };
    format!("{s} tokens")
}

fn trim(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_string)
}

fn bar(p: f64) -> String {
    let filled = (p * 10.0).round() as usize;
    format!("[{}{}] {:>3}%", "#".repeat(filled), "-".repeat(10 - filled), (p * 100.0).floor() as u32)
}

fn render(b: &Badges, format: Format) -> String {
    let r = &b.rank;
    match format {
        Format::Json => {
            let v = json!({ "rank": r, "achievements": b.achievements });
            format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default())
        }
        Format::Csv => {
            let opt = |v: Option<&str>| v.unwrap_or("").to_string();
            let num = |v: Option<i64>| v.map_or(String::new(), |n| n.to_string());
            let mut out = String::from("badge,value,level,next_level,next_threshold,progress\n");
            out += &format!("rank,{},{},{},{},{:.4}\n", r.tokens, r.name, opt(r.next), num(r.next_threshold), r.progress);
            for a in &b.achievements {
                out += &format!(
                    "{},{},{},{},{},{:.4}\n",
                    a.key,
                    a.value,
                    opt(a.level_name),
                    opt(a.next_level),
                    num(a.next_threshold),
                    a.progress
                );
            }
            out
        }
        Format::Table => {
            let tokens = crate::model::fmt_tokens(r.tokens.max(0) as u64);
            let next = match (r.next, r.next_threshold) {
                (Some(n), Some(t)) => format!("next {n} at {}", fmt_threshold(t, "tokens")),
                _ => "top rank".to_string(),
            };
            let mut out = format!("Rank: {} ({tokens} tokens)  {}  {next}\n", r.name, bar(r.progress));
            for a in &b.achievements {
                let value = if a.unit == "tokens" {
                    format!("{} tokens", crate::model::fmt_tokens(a.value.max(0) as u64))
                } else {
                    format!("{} {}", a.value, a.unit)
                };
                let next = match (a.next_level, a.next_threshold) {
                    (Some(n), Some(t)) => format!("next {n} at {}", fmt_threshold(t, a.unit)),
                    _ => "complete".to_string(),
                };
                out += &format!(
                    "  {:<16} {:<16} {:<7} {}  {next}\n",
                    a.name,
                    value,
                    a.level_name.unwrap_or("-"),
                    bar(a.progress)
                );
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(rows: &str) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(store::SCHEMA).unwrap();
        conn.execute_batch(rows).unwrap();
        conn
    }

    /// One main-agent row of `tokens` on each day.
    fn days(days: &[&str], tokens: i64) -> String {
        days.iter()
            .map(|d| format!("INSERT INTO token_days VALUES ('s', 'main', '{d}', 0, {tokens}, 0, 0, {tokens}, 1);"))
            .collect()
    }

    fn level(b: &Badges, key: &str) -> usize {
        b.achievements.iter().find(|a| a.key == key).unwrap().level
    }

    #[test]
    fn best_values_never_go_down() {
        let conn = db("");
        assert!(badge_best(&conn).unwrap().is_empty());
        raise_best(&conn, "streak", 7).unwrap();
        raise_best(&conn, "streak", 3).unwrap();
        raise_best(&conn, "rank", 10).unwrap();
        raise_best(&conn, "rank", 12).unwrap();
        let best = badge_best(&conn).unwrap();
        assert_eq!((best["streak"], best["rank"], best.len()), (7, 12, 2), "lower keeps the old value; higher replaces it");
    }

    #[test]
    fn pruned_days_never_lower_a_recorded_streak() {
        // Covers AE1.
        let week = ["2026-09-01", "2026-09-02", "2026-09-03", "2026-09-04", "2026-09-05", "2026-09-06", "2026-09-07"];
        let conn = db(&days(&week, 5));
        let total = store::lifetime(&conn).unwrap().total;
        let before = record(&conn, total).unwrap();
        assert_eq!(before.get("streak"), 7);
        assert_eq!(badges(&before).achievements[0].level_name, Some("silver"));

        // Retention pruning only touches detail tables, but even losing the counter rows
        // behind the streak keeps it.
        conn.execute_batch("DELETE FROM token_days WHERE day < '2026-09-07';").unwrap();
        let after = shown(&conn, store::lifetime(&conn).unwrap().total).unwrap();
        assert_eq!(after.get("streak"), 7, "shown streak stays at the best value");
        assert_eq!(badges(&after).achievements[0].level_name, Some("silver"));
    }

    #[test]
    fn tier_boundaries() {
        let rank = |tokens: i64| badges(&Stats(BTreeMap::from([(RANK_KEY, tokens)]))).rank;
        assert_eq!(rank(0).name, "Novice");
        assert_eq!(rank(999_999).name, "Novice");
        let apprentice = rank(1_000_000);
        assert_eq!((apprentice.name, apprentice.next, apprentice.next_threshold), ("Apprentice", Some("Practitioner"), Some(10_000_000)));
        assert_eq!(apprentice.progress, 0.0);
        assert_eq!(rank(5_000_000_000).name, "Grandmaster");
        let top = rank(9_000_000_000);
        assert_eq!((top.name, top.next, top.next_threshold, top.progress), ("Grandmaster", None, None, 1.0));
        let expert = rank(344_000_000);
        assert_eq!(expert.name, "Expert");
        assert!((expert.progress - 94.0 / 750.0).abs() < 1e-9);
    }

    #[test]
    fn streak_counts_consecutive_days_and_a_gap_resets_it() {
        let d = |v: &[&str]| longest_streak(v.iter().copied());
        assert_eq!(d(&[]), 0);
        assert_eq!(d(&["2026-09-01"]), 1);
        assert_eq!(d(&["2026-09-01", "2026-09-02", "2026-09-04", "2026-09-05", "2026-09-06"]), 3);
        assert_eq!(d(&["2026-09-29", "2026-09-30", "2026-10-01"]), 3, "across a month end");
        // Days with zero tokens are not active days.
        let conn = db(&format!("{}{}", days(&["2026-09-01", "2026-09-03"], 5), days(&["2026-09-02"], 0)));
        assert_eq!(compute(&conn, 0).unwrap().get("streak"), 1);
    }

    #[test]
    fn busiest_day_sums_a_day_and_subagents_skip_the_main_agent() {
        let conn = db(
            "INSERT INTO token_days VALUES ('s1', 'main', '2026-09-01', 0, 30, 0, 0, 30, 1);
             INSERT INTO token_days VALUES ('s2', 'main', '2026-09-01', 0, 40, 0, 0, 40, 1);
             INSERT INTO token_days VALUES ('s1', 'a1', '2026-09-02', 0, 50, 0, 0, 50, 1);
             INSERT INTO token_days VALUES ('s1', 'a1', '2026-09-03', 0, 1, 0, 0, 1, 1);
             INSERT INTO token_days VALUES ('s2', 'a1', '2026-09-03', 0, 1, 0, 0, 1, 1);
             INSERT INTO skills_seen VALUES ('ce-plan', '2026-09-01'), ('ce-work', '2026-09-02');",
        );
        let s = compute(&conn, 123).unwrap();
        assert_eq!(s.get("busiest_day"), 70, "the highest daily total, across sessions");
        assert_eq!(s.get("subagents"), 2, "distinct (session, agent) pairs, main excluded");
        assert_eq!(s.get("skills"), 2);
        assert_eq!(s.get(RANK_KEY), 123, "the rank figure is the caller's");
        let empty = compute(&db(""), 0).unwrap();
        assert!(empty.0.values().all(|&v| v == 0));
    }

    #[test]
    fn a_lowering_restore_import_leaves_every_level() {
        let mut rows = days(&["2026-09-01", "2026-09-02", "2026-09-03"], 20_000_000);
        for i in 0..10 {
            rows += &format!("INSERT INTO token_days VALUES ('s', 'a{i}', '2026-09-01', 0, 1, 0, 0, 1, 1);");
            rows += &format!("INSERT INTO skills_seen VALUES ('skill{i}', '2026-09-01');");
        }
        let conn = db(&rows);
        let before = badges(&record(&conn, store::lifetime(&conn).unwrap().total).unwrap());
        assert_eq!((before.rank.name, level(&before, "streak"), level(&before, "skills")), ("Specialist", 1, 2));
        assert_eq!((level(&before, "subagents"), level(&before, "busiest_day")), (1, 1));

        // A restore drops days inside its legacy range and the skills table came back smaller.
        conn.execute_batch("DELETE FROM token_days WHERE day > '2026-09-01'; DELETE FROM skills_seen WHERE skill != 'skill0';")
            .unwrap();
        let after = badges(&record(&conn, store::lifetime(&conn).unwrap().total).unwrap());
        assert_eq!(after, before);
    }

    #[test]
    fn announcement_seeds_silently_then_combines_rises_and_records_them() {
        let dir = std::env::temp_dir().join(format!("marsad-badges-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("settings.json");
        let at = |rank: i64, streak: i64| badges(&Stats(BTreeMap::from([(RANK_KEY, rank), ("streak", streak)])));

        let start = at(60_000_000, 7);
        assert_eq!(pending_at(&file, &start).unwrap(), None, "no record: seeded silently");
        assert_eq!(crate::settings::badges_announced_at(&file), Some(levels(&start)));
        assert_eq!(pending_at(&file, &start).unwrap(), None, "nothing rose");

        let up = at(300_000_000, 30);
        assert_eq!(pending_at(&file, &up).unwrap().as_deref(), Some("rank up: Expert · new: gold streak"));
        assert_eq!(pending_at(&file, &up).unwrap().as_deref(), Some("rank up: Expert · new: gold streak"), "until recorded");
        mark_at(&file, &up).unwrap();
        assert_eq!(pending_at(&file, &up).unwrap(), None, "announced once");
        assert_eq!(crate::settings::badges_announced_at(&file).unwrap()["rank"], 4);

        // A record that knows fewer stats fills them in silently.
        let mut partial = levels(&start);
        partial.remove("skills");
        crate::settings::set_badges_announced_at(&file, &partial).unwrap();
        assert_eq!(pending_at(&file, &start).unwrap(), None);
        assert!(crate::settings::badges_announced_at(&file).unwrap().contains_key("skills"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn diff_names_each_new_level_once() {
        let recorded = Levels::from([("rank".into(), 4), ("skills".into(), 0), ("streak".into(), 2)]);
        let b = badges(&Stats(BTreeMap::from([(RANK_KEY, 300_000_000), ("skills", 12), ("streak", 7), ("subagents", 50)])));
        assert_eq!(diff(&recorded, &b).as_deref(), Some("new: silver skills"), "no rank change, unknown subagents skipped");
        let lower = Levels::from([("rank".into(), 6)]);
        assert_eq!(diff(&lower, &b), None, "a higher record is not a rise");
    }

    #[test]
    fn json_has_rank_next_threshold_and_achievement_levels() {
        let b = badges(&Stats(BTreeMap::from([(RANK_KEY, 344_000_000), ("streak", 8), ("skills", 2)])));
        let v: serde_json::Value = serde_json::from_str(&render(&b, Format::Json)).unwrap();
        assert_eq!(v["rank"]["name"], "Expert");
        assert_eq!(v["rank"]["next_threshold"], 1_000_000_000);
        assert_eq!(v["achievements"].as_array().unwrap().len(), ACHIEVEMENTS.len());
        let streak = &v["achievements"][0];
        assert_eq!((streak["key"].clone(), streak["level_name"].clone(), streak["next_threshold"].clone()), (json!("streak"), json!("silver"), json!(30)));
        assert!(streak["progress"].as_f64().unwrap() > 0.0);
        assert_eq!(v["achievements"][1]["level_name"], serde_json::Value::Null);

        let csv = render(&b, Format::Csv);
        assert_eq!(csv.lines().count(), 1 + 1 + ACHIEVEMENTS.len());
        assert!(csv.lines().nth(1).unwrap().starts_with("rank,344000000,Expert,Master,1000000000,"));
        let table = render(&b, Format::Table);
        assert!(table.starts_with("Rank: Expert (344.0M tokens)"), "{table}");
        assert!(table.contains("next Master at 1B tokens"));
    }
}
