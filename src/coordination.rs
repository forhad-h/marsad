//! The bundled multi-session coordination protocol: its file under marsad's data dir and the
//! `claude` command that launches a session with it.

use anyhow::Context;
use std::path::{Path, PathBuf};

/// Protocol appended to the system prompt of sessions that should coordinate on a shared board.
const PROTOCOL_MD: &str = include_str!("../assets/COORDINATION.md");

/// Absolute path of marsad's copy of the protocol. Never canonicalized (Windows would yield `\\?\`).
pub fn protocol_path() -> PathBuf {
    let path = crate::data_dir().join("coordination.md");
    std::path::absolute(&path).unwrap_or(path)
}

/// Writes or refreshes the protocol file and returns its path.
pub fn ensure_protocol() -> anyhow::Result<PathBuf> {
    let path = protocol_path();
    ensure_protocol_at(&path)?;
    Ok(path)
}

fn ensure_protocol_at(path: &Path) -> anyhow::Result<()> {
    if std::fs::read(path).is_ok_and(|current| current == PROTOCOL_MD.as_bytes()) {
        return Ok(());
    }
    let dir = path.parent().context("protocol path has no parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path.file_name().context("protocol path has no file name")?.to_string_lossy();
    // Write beside the target and rename, so a launching `claude` never reads a torn file.
    let tmp = dir.join(format!("{name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, PROTOCOL_MD).with_context(|| format!("writing {}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("replacing {}", path.display()));
    }
    Ok(())
}

/// Removes the protocol file. Ok(false) when it was not there.
pub fn remove_protocol() -> anyhow::Result<bool> {
    remove_protocol_at(&protocol_path())
}

fn remove_protocol_at(path: &Path) -> anyhow::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Quotes `s` for the current platform's shell.
pub fn quote_arg(s: &str) -> String {
    quote_for(s, cfg!(windows))
}

fn quote_for(s: &str, windows: bool) -> String {
    let bare = |c: char| c.is_ascii_alphanumeric() || "_./:@%+=,-".contains(c);
    if !s.is_empty() && s.chars().all(bare) {
        s.to_string()
    } else if windows {
        format!("\"{s}\"")
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// The command that starts a Claude Code session with the protocol, without a trailing newline.
pub fn launch_command(path: &Path) -> String {
    format!("claude --append-system-prompt-file {}", quote_arg(&path.to_string_lossy()))
}

#[derive(clap::Args)]
pub struct CoordinationArgs {
    #[arg(long, value_enum, default_value = "table")]
    format: crate::report::Format,
}

/// `marsad coordination`: refreshes the protocol file and prints the command that loads it.
pub fn run(args: CoordinationArgs) -> anyhow::Result<()> {
    let path = ensure_protocol()?;
    println!("{}", render(args.format, &launch_command(&path), &path));
    Ok(())
}

fn render(format: crate::report::Format, command: &str, path: &Path) -> String {
    use crate::report::Format;
    let path = path.display().to_string();
    match format {
        // The command alone on the first line, so it can be copied or piped as is.
        Format::Table => format!(
            "{command}\nProtocol: {path}\nStart new Claude Code sessions with this command; resumed conversations keep their old prompt."
        ),
        Format::Json => serde_json::to_string_pretty(&serde_json::json!({ "command": command, "protocol_path": path }))
            .unwrap_or_default(),
        Format::Csv => {
            let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
            format!("command,protocol_path\n{},{}", quote(command), quote(&path))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marsad-coordination-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn cli_output_puts_the_command_first() {
        use crate::report::Format;
        let path = Path::new("/d/coordination.md");
        let cmd = launch_command(path);
        let table = render(Format::Table, &cmd, path);
        assert_eq!(table.lines().next(), Some("claude --append-system-prompt-file /d/coordination.md"));
        assert!(table.contains("Protocol: /d/coordination.md"));
        let json: serde_json::Value = serde_json::from_str(&render(Format::Json, &cmd, path)).unwrap();
        assert_eq!(json["command"], cmd.as_str());
        assert_eq!(json["protocol_path"], "/d/coordination.md");
        assert_eq!(
            render(Format::Csv, &cmd, path),
            "command,protocol_path\n\"claude --append-system-prompt-file /d/coordination.md\",\"/d/coordination.md\""
        );
    }

    #[test]
    fn fresh_dir_gets_the_protocol_and_a_second_call_leaves_it_alone() {
        let dir = temp_dir("fresh");
        let path = dir.join("nested").join("coordination.md");
        ensure_protocol_at(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), PROTOCOL_MD);
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(20));
        ensure_protocol_at(&path).unwrap();
        let after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after, "unchanged content must not be rewritten");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn different_content_is_replaced_without_leaving_a_temp_file() {
        let dir = temp_dir("stale");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("coordination.md");
        std::fs::write(&path, "old protocol").unwrap();

        ensure_protocol_at(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), PROTOCOL_MD);
        assert_eq!(names(&dir), ["coordination.md"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removal_deletes_only_the_protocol() {
        let dir = temp_dir("remove");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("coordination.md");
        std::fs::write(dir.join("events.jsonl"), "{}").unwrap();
        ensure_protocol_at(&path).unwrap();

        assert!(remove_protocol_at(&path).unwrap());
        assert_eq!(names(&dir), ["events.jsonl"]);
        assert!(!remove_protocol_at(&path).unwrap(), "absent file reports false");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quoting_follows_the_platform_shell() {
        assert_eq!(quote_for("/Users/me/.claude/marsad/coordination.md", false), "/Users/me/.claude/marsad/coordination.md");
        assert_eq!(quote_for("/Users/Jane Doe/coordination.md", false), "'/Users/Jane Doe/coordination.md'");
        assert_eq!(quote_for("/Users/o'neil/coordination.md", false), "'/Users/o'\\''neil/coordination.md'");
        assert_eq!(quote_for("C:/Users/me/coordination.md", true), "C:/Users/me/coordination.md");
        assert_eq!(quote_for(r"C:\Users\Jane Doe\coordination.md", true), r#""C:\Users\Jane Doe\coordination.md""#);
        assert_eq!(quote_for("", false), "''", "an empty argument is still an argument");
    }

    #[test]
    fn launch_command_appends_the_quoted_path() {
        let cmd = launch_command(Path::new("/srv/marsad/coordination.md"));
        assert_eq!(cmd, "claude --append-system-prompt-file /srv/marsad/coordination.md");
        let cmd = launch_command(Path::new("/Users/Jane Doe/coordination.md"));
        assert_eq!(cmd, format!("claude --append-system-prompt-file {}", quote_arg("/Users/Jane Doe/coordination.md")));
        assert!(!cmd.ends_with('\n'));
    }

    #[test]
    fn protocol_path_is_absolute_and_named() {
        let path = protocol_path();
        assert!(path.is_absolute());
        assert_eq!(path.file_name().unwrap(), "coordination.md");
    }

    #[test]
    fn embedded_protocol_is_project_agnostic_and_has_the_template() {
        for banned in ["Prisma", "package.json", "sleep 300", "BOARD ="] {
            assert!(!PROTOCOL_MD.contains(banned), "protocol mentions {banned}");
        }
        for heading in ["## Agents", "## Claims", "## Decisions", "## Messages", "## Conflicts", "## Handoff notes"] {
            assert!(PROTOCOL_MD.contains(heading), "protocol lacks {heading}");
        }
        assert!(PROTOCOL_MD.contains("--git-common-dir"));
        assert!(PROTOCOL_MD.contains("sleep 60"));
    }
}
