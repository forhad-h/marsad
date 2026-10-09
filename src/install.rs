//! Adds/removes marsad hook entries in ~/.claude/settings.json without touching anything else.

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

pub const EVENTS: &[&str] = &[
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Notification",
    "Elicitation",
    "Stop",
    "StopFailure",
    "SubagentStart",
    "SubagentStop",
    "PreCompact",
];

fn settings_path() -> PathBuf {
    crate::claude_dir().join("settings.json")
}

fn is_ours(entry: &Value) -> bool {
    entry["hooks"].as_array().is_some_and(|hs| {
        hs.iter().any(|h| {
            h["command"].as_str().is_some_and(|c| c.contains("marsad") && c.ends_with(" hook"))
        })
    })
}

fn load() -> anyhow::Result<(PathBuf, Map<String, Value>)> {
    let path = settings_path();
    let settings = match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str::<Value>(&text)
            .with_context(|| format!("{} is not valid JSON", path.display()))?,
        _ => json!({}),
    };
    let Value::Object(map) = settings else { bail!("{} is not a JSON object", path.display()) };
    Ok((path, map))
}

fn save(path: &PathBuf, settings: Map<String, Value>) -> anyhow::Result<()> {
    if path.exists() {
        let backup = path.with_extension("json.marsad-backup");
        std::fs::copy(path, &backup)?;
        println!("Backed up settings to {}", backup.display());
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let text = serde_json::to_string_pretty(&Value::Object(settings))? + "\n";
    std::fs::write(path, text)?;
    Ok(())
}

/// Removes our entries from every event; drops events (and `hooks`) left empty.
fn strip(settings: &mut Map<String, Value>) {
    let Some(Value::Object(hooks)) = settings.get_mut("hooks") else { return };
    for entries in hooks.values_mut() {
        if let Value::Array(list) = entries {
            list.retain(|e| !is_ours(e));
        }
    }
    hooks.retain(|_, v| !v.as_array().is_some_and(Vec::is_empty));
    if hooks.is_empty() {
        settings.remove("hooks");
    }
}

pub fn install() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let command = format!("\"{}\" hook", exe.display());
    let (path, mut settings) = load()?;
    strip(&mut settings);

    let hooks = settings.entry("hooks").or_insert_with(|| json!({}));
    let Value::Object(hooks) = hooks else { bail!("`hooks` in settings.json is not an object") };
    for event in EVENTS {
        let list = hooks.entry(*event).or_insert_with(|| json!([]));
        let Value::Array(list) = list else { bail!("hooks.{event} is not an array") };
        list.push(json!({
            "hooks": [{ "type": "command", "command": command, "async": true, "timeout": 10 }]
        }));
    }
    save(&path, settings)?;
    std::fs::create_dir_all(crate::data_dir())?;
    println!("Installed marsad hooks for {} events in {}", EVENTS.len(), path.display());
    println!("Sessions already running pick them up after a restart (or /hooks reload). Run `marsad` to open the dashboard.");
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let (path, mut settings) = load()?;
    strip(&mut settings);
    save(&path, settings)?;
    println!("Removed marsad hooks from {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_keeps_foreign_hooks() {
        let mut s = json!({
            "model": "opus",
            "hooks": {
                "Stop": [
                    { "hooks": [{ "type": "command", "command": "\"/x/marsad\" hook" }] },
                    { "hooks": [{ "type": "command", "command": "say done" }] }
                ],
                "PreToolUse": [{ "hooks": [{ "type": "command", "command": "\"/x/marsad\" hook" }] }]
            }
        });
        let Value::Object(ref mut map) = s else { unreachable!() };
        strip(map);
        assert_eq!(s["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert!(s["hooks"].get("PreToolUse").is_none());
        assert_eq!(s["model"], "opus");
    }
}
