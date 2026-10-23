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
}
