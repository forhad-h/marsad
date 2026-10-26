//! The bundled rules files a Claude Code session can start with (coordination, output hygiene),
//! their files under marsad's data dir, and the `claude` command that loads a chosen set.
//!
//! `claude` reads only the last `--append-system-prompt-file` it is given, so a set of several
//! rules is merged into one file per combination, named after its keys (`coordination+output-hygiene.md`).

use anyhow::Context;
use std::path::{Path, PathBuf};

pub struct Rule {
    /// The name on the command line and in the file name.
    pub key: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    body: &'static str,
}

/// Every bundled rules file, in the order they are listed and merged. Add new ones at the end.
pub const RULES: &[Rule] = &[
    Rule {
        key: "coordination",
        title: "Coordination",
        summary: "sessions in one repo share AGENT-BOARD.md and claim files before editing",
        body: include_str!("../assets/COORDINATION.md"),
    },
    Rule {
        key: "output-hygiene",
        title: "Output hygiene",
        summary: "keeps the request, the requester and the process out of code, commits and docs",
        body: include_str!("../assets/OUTPUT-HYGIENE.md"),
    },
];

/// A set of rules, one bit per index into `RULES`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection(u32);

impl Selection {
    pub const COORDINATION: Selection = Selection(1);

    fn all() -> Self {
        Selection((1 << RULES.len()) - 1)
    }

    pub fn contains(self, i: usize) -> bool {
        self.0 & (1 << i) != 0
    }

    pub fn toggle(&mut self, i: usize) {
        if i < RULES.len() {
            self.0 ^= 1 << i;
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn rules(self) -> impl Iterator<Item = &'static Rule> {
        RULES.iter().enumerate().filter(move |(i, _)| self.contains(*i)).map(|(_, r)| r)
    }

    pub fn keys(self) -> Vec<&'static str> {
        self.rules().map(|r| r.key).collect()
    }

    /// The titles joined with " + ", for messages.
    pub fn titles(self) -> String {
        self.rules().map(|r| r.title.to_lowercase()).collect::<Vec<_>>().join(" + ")
    }

    /// Parses rule keys, dropping any this version doesn't know (a saved choice from a newer one).
    pub fn parse_known<S: AsRef<str>>(keys: &[S]) -> Self {
        let known: Vec<&str> = keys.iter().map(|k| k.as_ref()).filter(|k| *k == "all" || RULES.iter().any(|r| r.key == *k)).collect();
        Selection::parse(&known).unwrap_or_default()
    }

    /// Parses rule keys; `all` stands for every rule.
    pub fn parse<S: AsRef<str>>(keys: &[S]) -> anyhow::Result<Self> {
        let mut sel = Selection::default();
        for key in keys {
            let key = key.as_ref();
            if key == "all" {
                sel = Selection::all();
                continue;
            }
            match RULES.iter().position(|r| r.key == key) {
                Some(i) => sel.0 |= 1 << i,
                None => {
                    let known: Vec<_> = RULES.iter().map(|r| r.key).collect();
                    anyhow::bail!("no rules named {key:?}; choose from {} or all", known.join(", "));
                }
            }
        }
        Ok(sel)
    }

    fn file_name(self) -> String {
        format!("{}.md", self.keys().join("+"))
    }

    fn content(self) -> String {
        let bodies: Vec<&str> = self.rules().map(|r| r.body.trim_end()).collect();
        bodies.join("\n\n") + "\n"
    }

    /// The sets with more than one rule.
    fn combinations() -> impl Iterator<Item = Selection> {
        (1..=Selection::all().0).map(Selection).filter(|s| s.0.count_ones() > 1)
    }
}

/// Absolute path of the file for `sel`. Never canonicalized (Windows would yield `\\?\`).
pub fn path(sel: Selection) -> PathBuf {
    path_in(&crate::data_dir(), sel)
}

fn path_in(dir: &Path, sel: Selection) -> PathBuf {
    let path = dir.join(sel.file_name());
    std::path::absolute(&path).unwrap_or(path)
}

/// Writes or refreshes the file for `sel` and returns its path.
pub fn ensure(sel: Selection) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!sel.is_empty(), "no rules chosen");
    let path = path(sel);
    write_if_changed(&path, &sel.content())?;
    Ok(path)
}

/// Rewrites each single rule's file and any merged file already on disk with this build's text,
/// since `marsad update` only swaps the binary.
pub fn refresh() -> anyhow::Result<()> {
    refresh_in(&crate::data_dir())
}

fn refresh_in(dir: &Path) -> anyhow::Result<()> {
    for i in 0..RULES.len() {
        let sel = Selection(1 << i);
        write_if_changed(&path_in(dir, sel), &sel.content())?;
    }
    for sel in Selection::combinations() {
        let path = path_in(dir, sel);
        if path.exists() {
            write_if_changed(&path, &sel.content())?;
        }
    }
    Ok(())
}

fn write_if_changed(path: &Path, content: &str) -> anyhow::Result<()> {
    if std::fs::read(path).is_ok_and(|current| current == content.as_bytes()) {
        return Ok(());
    }
    let dir = path.parent().context("rules path has no parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = path.file_name().context("rules path has no file name")?.to_string_lossy();
    // Write beside the target and rename, so a launching `claude` never reads a torn file.
    let tmp = dir.join(format!("{name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("replacing {}", path.display()));
    }
    Ok(())
}

/// Removes every rules file. Returns how many were there.
pub fn remove_all() -> anyhow::Result<usize> {
    remove_all_in(&crate::data_dir())
}

fn remove_all_in(dir: &Path) -> anyhow::Result<usize> {
    let mut removed = 0;
    for sel in (1..=Selection::all().0).map(Selection) {
        let path = path_in(dir, sel);
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("removing {}", path.display())),
        }
    }
    Ok(removed)
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

/// The command that starts a Claude Code session with the rules file at `path`, without a trailing newline.
pub fn launch_command(path: &Path) -> String {
    format!("claude --append-system-prompt-file {}", quote_arg(&path.to_string_lossy()))
}

#[derive(clap::Args)]
pub struct RulesArgs {
    /// Rules to start with, e.g. `coordination output-hygiene`, or `all`. Without any, lists them.
    keys: Vec<String>,
    #[arg(long, value_enum, default_value = "table")]
    format: crate::report::Format,
}

#[derive(clap::Args)]
pub struct CoordinationArgs {
    #[arg(long, value_enum, default_value = "table")]
    format: crate::report::Format,
}

/// `marsad rules [keys…]`: lists the rules, or refreshes the chosen set's file and prints the command that loads it.
pub fn run(args: RulesArgs) -> anyhow::Result<()> {
    if args.keys.is_empty() {
        println!("{}", render_list(args.format));
        return Ok(());
    }
    print_command(Selection::parse(&args.keys)?, args.format)
}

/// `marsad coordination`: the same as `marsad rules coordination`.
pub fn run_coordination(args: CoordinationArgs) -> anyhow::Result<()> {
    print_command(Selection::COORDINATION, args.format)
}

fn print_command(sel: Selection, format: crate::report::Format) -> anyhow::Result<()> {
    // Written before anything is printed: a command pointing at a missing file is worse than none.
    let path = ensure(sel)?;
    println!("{}", render(format, sel, &launch_command(&path), &path));
    Ok(())
}

fn render_list(format: crate::report::Format) -> String {
    use crate::report::Format;
    match format {
        Format::Table => {
            let width = RULES.iter().map(|r| r.key.len()).max().unwrap_or(0);
            let mut out: Vec<String> = RULES.iter().map(|r| format!("{:<width$}  {}", r.key, r.summary)).collect();
            out.push(String::new());
            out.push("`marsad rules <name>…` prints the command that starts Claude Code with them, e.g.".into());
            out.push(format!("marsad rules {}", RULES.iter().map(|r| r.key).collect::<Vec<_>>().join(" ")));
            out.join("\n")
        }
        Format::Json => {
            let list: Vec<_> = RULES.iter().map(|r| serde_json::json!({ "name": r.key, "title": r.title, "summary": r.summary })).collect();
            serde_json::to_string_pretty(&list).unwrap_or_default()
        }
        Format::Csv => {
            let mut out = vec!["name,title,summary".to_string()];
            out.extend(RULES.iter().map(|r| format!("{},\"{}\",\"{}\"", r.key, r.title, r.summary)));
            out.join("\n")
        }
    }
}

fn render(format: crate::report::Format, sel: Selection, command: &str, path: &Path) -> String {
    use crate::report::Format;
    let path = path.display().to_string();
    match format {
        // The command alone on the first line, so it can be copied or piped as is.
        Format::Table => format!(
            "{command}\nRules: {path}\nStart new Claude Code sessions with this command; resumed conversations keep their old prompt."
        ),
        Format::Json => {
            serde_json::to_string_pretty(&serde_json::json!({ "command": command, "protocol_path": path, "rules": sel.keys() }))
                .unwrap_or_default()
        }
        Format::Csv => {
            let quote = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
            format!("command,protocol_path,rules\n{},{},{}", quote(command), quote(&path), quote(&sel.keys().join(" ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: Selection = Selection(0b11);

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marsad-rules-{name}-{}", std::process::id()));
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
        let sel = Selection::COORDINATION;
        let table = render(Format::Table, sel, &cmd, path);
        assert_eq!(table.lines().next(), Some("claude --append-system-prompt-file /d/coordination.md"));
        assert!(table.contains("Rules: /d/coordination.md"));
        let json: serde_json::Value = serde_json::from_str(&render(Format::Json, sel, &cmd, path)).unwrap();
        assert_eq!(json["command"], cmd.as_str());
        assert_eq!(json["protocol_path"], "/d/coordination.md");
        assert_eq!(json["rules"], serde_json::json!(["coordination"]));
        assert_eq!(
            render(Format::Csv, sel, &cmd, path),
            "command,protocol_path,rules\n\"claude --append-system-prompt-file /d/coordination.md\",\"/d/coordination.md\",\"coordination\""
        );
    }

    #[test]
    fn keys_parse_into_a_set_in_rules_order() {
        assert_eq!(Selection::parse(&["coordination"]).unwrap(), Selection::COORDINATION);
        assert_eq!(Selection::parse(&["output-hygiene", "coordination"]).unwrap(), BOTH);
        assert_eq!(Selection::parse(&["all"]).unwrap(), BOTH);
        assert_eq!(BOTH.keys(), ["coordination", "output-hygiene"]);
        assert_eq!(BOTH.titles(), "coordination + output hygiene");
        let err = Selection::parse(&["nope"]).unwrap_err().to_string();
        assert!(err.contains("coordination, output-hygiene"), "{err}");
        assert_eq!(Selection::parse_known(&["retired", "output-hygiene"]).keys(), ["output-hygiene"]);
    }

    #[test]
    fn the_coordination_constant_names_the_coordination_rules() {
        assert_eq!(Selection::COORDINATION.keys(), ["coordination"], "RULES[0] must stay coordination");
    }

    #[test]
    fn each_set_has_its_own_file_and_merged_text() {
        assert_eq!(Selection::COORDINATION.file_name(), "coordination.md", "the path earlier versions printed");
        assert_eq!(BOTH.file_name(), "coordination+output-hygiene.md");
        assert_eq!(Selection::COORDINATION.content().trim_end(), RULES[0].body.trim_end());
        let both = BOTH.content();
        let (a, b) = (both.find("# Multi-session coordination").unwrap(), both.find("# Output hygiene").unwrap());
        assert!(a < b, "merged in rules order");
    }

    #[test]
    fn toggling_skips_unknown_indexes() {
        let mut sel = Selection::default();
        sel.toggle(1);
        sel.toggle(RULES.len());
        assert_eq!(sel.keys(), ["output-hygiene"]);
        sel.toggle(1);
        assert!(sel.is_empty());
    }

    #[test]
    fn fresh_dir_gets_the_file_and_a_second_write_leaves_it_alone() {
        let dir = temp_dir("fresh");
        let path = dir.join("nested").join("coordination.md");
        let content = Selection::COORDINATION.content();
        write_if_changed(&path, &content).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(20));
        write_if_changed(&path, &content).unwrap();
        let after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after, "unchanged content must not be rewritten");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refresh_rewrites_singles_and_existing_merges_only() {
        let dir = temp_dir("refresh");
        std::fs::create_dir_all(&dir).unwrap();
        refresh_in(&dir).unwrap();
        assert_eq!(names(&dir), ["coordination.md", "output-hygiene.md"], "no merged file until one is asked for");

        std::fs::write(path_in(&dir, BOTH), "old rules").unwrap();
        std::fs::write(path_in(&dir, Selection::COORDINATION), "old rules").unwrap();
        refresh_in(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(path_in(&dir, BOTH)).unwrap(), BOTH.content());
        assert_eq!(std::fs::read_to_string(path_in(&dir, Selection::COORDINATION)).unwrap(), Selection::COORDINATION.content());
        assert_eq!(names(&dir).len(), 3, "no temp file left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn removal_deletes_only_rules_files() {
        let dir = temp_dir("remove");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("events.jsonl"), "{}").unwrap();
        refresh_in(&dir).unwrap();
        write_if_changed(&path_in(&dir, BOTH), &BOTH.content()).unwrap();

        assert_eq!(remove_all_in(&dir).unwrap(), 3);
        assert_eq!(names(&dir), ["events.jsonl"]);
        assert_eq!(remove_all_in(&dir).unwrap(), 0, "nothing left to remove");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quoting_follows_the_platform_shell() {
        assert_eq!(quote_for("/Users/me/.claude/marsad/coordination.md", false), "/Users/me/.claude/marsad/coordination.md");
        assert_eq!(quote_for("/Users/me/.claude/marsad/coordination+output-hygiene.md", false), "/Users/me/.claude/marsad/coordination+output-hygiene.md");
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
    fn paths_are_absolute_and_named() {
        let path = path(Selection::COORDINATION);
        assert!(path.is_absolute());
        assert_eq!(path.file_name().unwrap(), "coordination.md");
    }

    #[test]
    fn embedded_protocol_is_project_agnostic_and_has_the_template() {
        let protocol = RULES[0].body;
        for banned in ["Prisma", "package.json", "sleep 300", "BOARD ="] {
            assert!(!protocol.contains(banned), "protocol mentions {banned}");
        }
        for heading in ["## Agents", "## Claims", "## Decisions", "## Messages", "## Conflicts", "## Handoff notes"] {
            assert!(protocol.contains(heading), "protocol lacks {heading}");
        }
        assert!(protocol.contains("--git-common-dir"));
        assert!(protocol.contains("sleep 60"));
    }
}
