//! User preferences in `settings.json` under marsad's data dir. Unknown keys are kept, so
//! newer versions' settings survive an older marsad writing the file.

use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

fn path() -> PathBuf {
    crate::data_dir().join("settings.json")
}

fn load_at(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| match v {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

fn set_at(path: &Path, key: &str, value: Value) -> anyhow::Result<()> {
    let mut map = load_at(path);
    map.insert(key.into(), value);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(&Value::Object(map))?)?;
    Ok(())
}

fn alerts_at(path: &Path) -> bool {
    load_at(path).get("alerts").and_then(Value::as_bool).unwrap_or(true)
}

/// Whether the dashboard plays sounds and shows notifications (on unless turned off).
pub fn alerts() -> bool {
    alerts_at(&path())
}

pub fn set_alerts(on: bool) -> anyhow::Result<()> {
    set_at(&path(), "alerts", json!(on))
}

/// Preferences an export carries; anything else in the file stays on this machine.
const PORTABLE: &[&str] = &["retention_days", "alerts", "context_warning_pct"];

/// Where versions before 0.4 kept the retention.
fn legacy_retention_path() -> PathBuf {
    crate::data_dir().join("history.json")
}

/// Saved `retention_days` (0 = forever), moving a value from the old `history.json` into the
/// settings the first time it is found there.
fn retention_days_at(path: &Path, legacy: &Path) -> Option<u64> {
    if let Some(days) = load_at(path).get("retention_days").and_then(Value::as_u64) {
        return Some(days);
    }
    let old = std::fs::read_to_string(legacy)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|v| v.get("retention_days").and_then(Value::as_u64))?;
    if set_at(path, "retention_days", json!(old)).is_ok() {
        let _ = std::fs::remove_file(legacy);
    }
    Some(old)
}

/// Saved retention in days (0 = forever), or None when the user never set one.
pub fn retention_days() -> Option<u64> {
    retention_days_at(&path(), &legacy_retention_path())
}

pub fn set_retention_days(days: u64) -> anyhow::Result<()> {
    set_at(&path(), "retention_days", json!(days))
}

/// Default share of the auto-compact point at which a session is told to hand off.
pub const DEFAULT_CONTEXT_WARNING_PCT: u32 = 80;

fn context_warning_at(path: &Path) -> Option<u32> {
    match load_at(path).get("context_warning_pct") {
        Some(Value::Bool(false)) => None,
        Some(v) => v.as_u64().map(|p| p as u32).or(Some(DEFAULT_CONTEXT_WARNING_PCT)),
        None => Some(DEFAULT_CONTEXT_WARNING_PCT),
    }
}

/// Percent of the auto-compact point that triggers the handoff warning; None when it is off.
pub fn context_warning() -> Option<u32> {
    context_warning_at(&path())
}

pub fn set_context_warning(pct: Option<u32>) -> anyhow::Result<()> {
    set_at(&path(), "context_warning_pct", pct.map_or(json!(false), |p| json!(p)))
}

/// The preferences that travel in an export.
pub fn portable() -> Map<String, Value> {
    // Moves a retention still in `history.json` into the settings first.
    retention_days();
    let all = load_at(&path());
    PORTABLE.iter().filter_map(|k| all.get(*k).map(|v| (k.to_string(), v.clone()))).collect()
}

fn merge_missing_at(path: &Path, legacy: &Path, incoming: &Map<String, Value>) -> anyhow::Result<Vec<String>> {
    // A retention still in `history.json` is a local choice too.
    retention_days_at(path, legacy);
    let local = load_at(path);
    let mut applied = Vec::new();
    for key in PORTABLE.iter().filter(|k| !local.contains_key(**k)) {
        if let Some(v) = incoming.get(*key) {
            set_at(path, key, v.clone())?;
            applied.push(key.to_string());
        }
    }
    Ok(applied)
}

/// Applies imported preferences this machine hasn't set; returns the keys applied.
pub fn merge_missing(incoming: &Map<String, Value>) -> anyhow::Result<Vec<String>> {
    merge_missing_at(&path(), &legacy_retention_path(), incoming)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_default_on_round_trip_and_keep_other_keys() {
        let dir = std::env::temp_dir().join(format!("marsad-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("settings.json");
        assert!(alerts_at(&file), "missing file means on");

        set_at(&file, "future", json!(5)).unwrap();
        set_at(&file, "alerts", json!(false)).unwrap();
        assert!(!alerts_at(&file));
        assert_eq!(load_at(&file)["future"], 5, "other keys survive");

        std::fs::write(&file, "not json").unwrap();
        assert!(alerts_at(&file), "a corrupt file falls back to defaults");
        std::fs::remove_dir_all(dir).ok();
    }

    fn temp(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("marsad-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (dir.join("settings.json"), dir.join("history.json"))
    }

    #[test]
    fn retention_moves_out_of_history_json() {
        let (file, legacy) = temp("retention");
        assert_eq!(retention_days_at(&file, &legacy), None, "never set");
        std::fs::write(&legacy, r#"{"retention_days": 14}"#).unwrap();
        assert_eq!(retention_days_at(&file, &legacy), Some(14));
        assert_eq!(load_at(&file)["retention_days"], 14, "copied into settings");
        assert!(!legacy.exists(), "old file removed");
        set_at(&file, "retention_days", json!(0)).unwrap();
        assert_eq!(retention_days_at(&file, &legacy), Some(0), "0 keeps forever");
        std::fs::remove_dir_all(file.parent().unwrap()).ok();
    }

    #[test]
    fn context_warning_defaults_to_80_and_can_be_off() {
        let (file, _) = temp("warning");
        assert_eq!(context_warning_at(&file), Some(80));
        set_at(&file, "context_warning_pct", json!(false)).unwrap();
        assert_eq!(context_warning_at(&file), None);
        set_at(&file, "context_warning_pct", json!(70)).unwrap();
        assert_eq!(context_warning_at(&file), Some(70));
        std::fs::remove_dir_all(file.parent().unwrap()).ok();
    }

    #[test]
    fn import_fills_only_unset_preferences() {
        let (file, legacy) = temp("merge");
        std::fs::write(&legacy, r#"{"retention_days": 14}"#).unwrap();
        set_at(&file, "alerts", json!(false)).unwrap();
        let incoming = json!({"retention_days": 90, "alerts": true, "context_warning_pct": 60, "install_id": "x"});
        let applied = merge_missing_at(&file, &legacy, incoming.as_object().unwrap()).unwrap();
        assert_eq!(applied, ["context_warning_pct"]);
        let saved = load_at(&file);
        assert_eq!((saved["retention_days"].clone(), saved["alerts"].clone()), (json!(14), json!(false)));
        assert!(!saved.contains_key("install_id"));
        std::fs::remove_dir_all(file.parent().unwrap()).ok();
    }
}
