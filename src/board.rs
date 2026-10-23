//! The shared `AGENT-BOARD.md`: where a session's board lives, and what it says.

use chrono::NaiveDateTime;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The file name agents write in the repo root.
pub const FILE: &str = "AGENT-BOARD.md";
/// The most of a board that is read; anything past it is dropped.
pub const READ_CAP: usize = 512 * 1024;
/// Agents not seen for longer than this are stale.
const STALE_MINUTES: i64 = 120;

/// Where a session's board lives.
#[derive(Debug, PartialEq, Eq)]
pub enum Root {
    Board(PathBuf),
    /// The session has no working directory yet.
    NoCwd,
    /// The working directory is gone and no repo around it could be found.
    Gone,
}

/// Finds the board for a session's cwd: one board per repo, in the main checkout (or its git dir when there is none), so every worktree shares it.
pub fn resolve(cwd: &str) -> Root {
    resolve_with(cwd, git_common_dir)
}

fn git_common_dir(dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--git-common-dir"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    common_dir_from(dir, &String::from_utf8(out.stdout).ok()?)
}

/// The absolute common dir from `git rev-parse --git-common-dir` output, which may be relative to `dir`.
/// Anything but one path line (git echoes flags it doesn't know) is no answer.
fn common_dir_from(dir: &Path, out: &str) -> Option<PathBuf> {
    let path = out.trim_end_matches(['\n', '\r']);
    if path.is_empty() || path.starts_with('-') || path.contains(['\n', '\r']) {
        return None;
    }
    dir.join(path).canonicalize().ok()
}

/// Beside a `.git` dir, in the main checkout. Any other common dir (a bare clone, a submodule's
/// `.git/modules/<name>`) has no checkout of its own, so the board goes inside it.
fn board_in(common: &Path) -> Option<PathBuf> {
    if common.file_name().is_some_and(|n| n == ".git") {
        common.parent().map(|p| p.join(FILE))
    } else {
        Some(common.join(FILE))
    }
}

fn resolve_with(cwd: &str, git: impl Fn(&Path) -> Option<PathBuf>) -> Root {
    if cwd.trim().is_empty() {
        return Root::NoCwd;
    }
    let in_repo = |dir: &Path| git(dir).and_then(|common| board_in(&common));
    let dir = Path::new(cwd);
    if dir.is_dir() {
        return Root::Board(in_repo(dir).unwrap_or_else(|| dir.join(FILE)));
    }
    // A deleted worktree: only a repo around it counts, never an unrelated parent dir.
    match dir.ancestors().skip(1).find(|a| a.is_dir()).and_then(in_repo) {
        Some(board) => Root::Board(board),
        None => Root::Gone,
    }
}

/// Reads at most `READ_CAP` bytes as lossy UTF-8, and says whether the file was longer.
/// Only a regular file is read: a FIFO or a device (a committed symlink can point at one) would block the UI.
pub fn read(path: &Path) -> std::io::Result<(String, bool)> {
    if !std::fs::metadata(path)?.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a regular file"));
    }
    let mut buf = Vec::new();
    std::fs::File::open(path)?.take(READ_CAP as u64 + 1).read_to_end(&mut buf)?;
    let truncated = buf.len() > READ_CAP;
    buf.truncate(READ_CAP);
    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Agent {
    pub name: String,
    pub task: String,
    pub branch: String,
    pub status: String,
    pub started: String,
    pub last_seen: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Claim {
    pub path: String,
    pub agent: String,
    pub since: String,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStatus {
    Open,
    Resolved,
    NeedsHuman,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub text: String,
    pub status: ConflictStatus,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Board {
    pub agents: Vec<Agent>,
    pub claims: Vec<Claim>,
    pub decisions: Vec<String>,
    pub messages: Vec<String>,
    pub conflicts: Vec<Conflict>,
    pub handoffs: Vec<String>,
    /// Lines that fit no known section or shape, kept as written.
    pub other: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Preamble,
    Agents,
    Claims,
    Decisions,
    Messages,
    Conflicts,
    Handoffs,
    Unknown,
}

impl Section {
    fn named(heading: &str) -> Section {
        match heading.trim().to_lowercase().as_str() {
            "agents" => Section::Agents,
            "claims" => Section::Claims,
            "decisions" => Section::Decisions,
            "messages" => Section::Messages,
            "conflicts" => Section::Conflicts,
            "handoff notes" => Section::Handoffs,
            _ => Section::Unknown,
        }
    }
}

/// Parses a board leniently: comments are dropped, tables map by header name, and lines that fit nowhere go to `other`.
pub fn parse(text: &str) -> Board {
    let text = strip_comments(text);
    let mut board = Board::default();
    let mut section = Section::Preamble;
    // The current table's header, as normalized column names.
    let mut header: Option<Vec<String>> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = line.strip_prefix("## ") {
            section = Section::named(name);
            header = None;
            if section == Section::Unknown {
                board.other.push(line.to_string());
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }
        match section {
            Section::Preamble => {}
            Section::Unknown => board.other.push(line.to_string()),
            Section::Agents | Section::Claims => {
                let Some(row) = line.strip_prefix('|') else {
                    board.other.push(line.to_string());
                    continue;
                };
                let cells: Vec<&str> = row.strip_suffix('|').unwrap_or(row).split('|').map(str::trim).collect();
                if cells.iter().all(|c| c.chars().all(|ch| matches!(ch, '-' | ':' | ' '))) {
                    continue;
                }
                let Some(cols) = &header else {
                    header = Some(cells.iter().map(|c| column(c)).collect());
                    continue;
                };
                let get = |names: &[&str]| -> String {
                    cols.iter().position(|c| names.contains(&c.as_str())).and_then(|i| cells.get(i)).map(|c| c.to_string()).unwrap_or_default()
                };
                if section == Section::Agents {
                    board.agents.push(Agent {
                        name: get(&["agent", "name"]),
                        task: get(&["task"]),
                        branch: get(&["branchworktree", "branch", "worktree"]),
                        status: get(&["status"]),
                        started: get(&["started"]),
                        last_seen: get(&["lastseen"]),
                    });
                } else {
                    board.claims.push(Claim { path: get(&["pathglob", "path", "glob"]), agent: get(&["agent"]), since: get(&["since"]), note: get(&["note"]) });
                }
            }
            Section::Decisions | Section::Messages | Section::Conflicts | Section::Handoffs => {
                let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).map(str::trim) else {
                    board.other.push(line.to_string());
                    continue;
                };
                let item = item.to_string();
                match section {
                    Section::Decisions => board.decisions.push(item),
                    Section::Messages => board.messages.push(item),
                    Section::Handoffs => board.handoffs.push(item),
                    _ => board.conflicts.push(Conflict { status: conflict_status(&item), text: item }),
                }
            }
        }
    }
    board
}

/// Removes `<!-- -->` comments, which may span lines; an unterminated one drops the rest of the text.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start + 4..].find("-->") {
            Some(end) => rest = &rest[start + 4 + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// A header cell reduced to its letters, so `Last seen`, `last_seen` and `path / glob` all match.
fn column(cell: &str) -> String {
    cell.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A conflict's status is its last `|` segment, so a mention of needs-human elsewhere does not count.
fn conflict_status(item: &str) -> ConflictStatus {
    let Some((_, last)) = item.rsplit_once('|') else {
        return ConflictStatus::Unknown;
    };
    let last = last.trim().to_lowercase();
    let mut words = last.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-'));
    match (words.next(), words.next()) {
        (Some("open"), _) => ConflictStatus::Open,
        (Some("resolved"), _) => ConflictStatus::Resolved,
        (Some("needs-human"), _) | (Some("needs"), Some("human")) => ConflictStatus::NeedsHuman,
        _ => ConflictStatus::Unknown,
    }
}

/// Whether an agent last seen at `last_seen` (local `YYYY-MM-DD HH:MM`) is stale; `None` when the time can't be read or is in the future.
pub fn agent_stale(last_seen: &str, now: NaiveDateTime) -> Option<bool> {
    let s = last_seen.trim();
    let seen = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")).ok()?;
    if seen > now {
        return None;
    }
    Some((now - seen).num_seconds() > STALE_MINUTES * 60)
}

/// Which agents and claims to mark stale, index for index with the board's rows.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Staleness {
    pub agents: Vec<bool>,
    pub claims: Vec<bool>,
    /// Claims whose agent has no row; these are stale too.
    pub orphans: Vec<bool>,
}

/// Marks agents by their last-seen time, and claims by their agent's row; a claim with no agent row is an orphan.
pub fn staleness(board: &Board, now: NaiveDateTime) -> Staleness {
    let agents: Vec<bool> = board.agents.iter().map(|a| agent_stale(&a.last_seen, now) == Some(true)).collect();
    let mut out = Staleness { agents, ..Staleness::default() };
    for claim in &board.claims {
        let owner = claim.agent.trim();
        let rows: Vec<bool> = board.agents.iter().zip(&out.agents).filter(|(a, _)| a.name.trim().eq_ignore_ascii_case(owner)).map(|(_, &stale)| stale).collect();
        let orphan = rows.is_empty();
        out.orphans.push(orphan);
        out.claims.push(orphan || rows.iter().all(|&stale| stale));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = "# AGENT BOARD
Shared by all Claude Code sessions in this repo. One-line anchored edits only. Never edit another agent's entries.

## Agents
| agent | task | branch/worktree | status | started | last seen |
|---|---|---|---|---|---|

## Claims
| path/glob | agent | since | note |
|---|---|---|---|

## Decisions
<!-- - YYYY-MM-DD HH:MM agent: decision (binding for all) -->

## Messages
<!-- - YYYY-MM-DD HH:MM from -> to: text   (to = agent name or all; reply as a new line) -->

## Conflicts
<!-- - YYYY-MM-DD HH:MM agent-a vs agent-b: issue | open / resolved / needs-human -->

## Handoff notes
<!-- - YYYY-MM-DD HH:MM agent: done / left / others must know -->
";

    fn has_git() -> bool {
        Command::new("git").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?} in {}", dir.display());
    }

    /// A fresh temp dir holding a repo at `<root>/repo` with one commit.
    fn repo(name: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("marsad-board-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "marsad test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "commit.gpgsign", "false"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
        (root, repo)
    }

    fn board_at(dir: &Path) -> Root {
        Root::Board(dir.canonicalize().unwrap().join(FILE))
    }

    fn s(p: &Path) -> &str {
        p.to_str().unwrap()
    }

    #[test]
    fn worktrees_share_the_main_checkout_board() {
        if !has_git() {
            return;
        }
        let (root, repo) = repo("wt");
        let wt = root.join("wt");
        git(&repo, &["worktree", "add", "-q", "-b", "side", s(&wt)]);
        std::fs::create_dir_all(wt.join("src")).unwrap();
        assert_eq!(resolve(s(&repo)), board_at(&repo), "a plain checkout uses its root");
        assert_eq!(resolve(s(&wt)), board_at(&repo), "a worktree uses the main checkout");
        assert_eq!(resolve(s(&wt.join("src"))), board_at(&repo), "a subdir of a worktree too");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_non_git_dir_uses_itself() {
        let root = std::env::temp_dir().join(format!("marsad-board-plain-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(resolve(s(&root)), Root::Board(root.join(FILE)));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_deleted_worktree_inside_the_repo_still_resolves() {
        if !has_git() {
            return;
        }
        let (root, repo) = repo("gone-in");
        let wt = repo.join("trees").join("a");
        git(&repo, &["worktree", "add", "-q", "-b", "a", s(&wt)]);
        std::fs::remove_dir_all(&wt).unwrap();
        assert_eq!(resolve(s(&wt.join("src"))), board_at(&repo));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_bare_clone_keeps_one_board_for_its_worktrees() {
        if !has_git() {
            return;
        }
        let (root, repo) = repo("bare");
        let bare = root.join("proj.git");
        git(&root, &["clone", "-q", "--bare", s(&repo), s(&bare)]);
        let (a, b) = (root.join("a"), root.join("b"));
        git(&bare, &["worktree", "add", "-q", s(&a)]);
        git(&bare, &["worktree", "add", "-q", "-b", "other", s(&b)]);
        let board = Root::Board(bare.canonicalize().unwrap().join(FILE));
        assert_eq!(resolve(s(&a)), board, "never the shared parent dir");
        assert_eq!(resolve(s(&b)), board);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_deleted_sibling_worktree_under_a_non_git_parent_is_gone() {
        if !has_git() {
            return;
        }
        let (root, repo) = repo("gone-out");
        let wt = root.join("wt");
        git(&repo, &["worktree", "add", "-q", "-b", "side", s(&wt)]);
        std::fs::remove_dir_all(&wt).unwrap();
        assert_eq!(resolve(s(&wt)), Root::Gone, "never the unrelated parent's board");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn an_empty_cwd_has_no_board() {
        assert_eq!(resolve(""), Root::NoCwd);
        assert_eq!(resolve_with("  ", |_| panic!("git must not run")), Root::NoCwd);
    }

    #[test]
    fn resolution_falls_back_without_git() {
        let root = std::env::temp_dir().join(format!("marsad-board-inject-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let fake = PathBuf::from("/x/main/.git");
        assert_eq!(resolve_with(s(&root), |_| Some(fake.clone())), Root::Board(PathBuf::from("/x/main").join(FILE)));
        assert_eq!(resolve_with(s(&root), |_| None), Root::Board(root.join(FILE)));
        let gone = root.join("a/b");
        assert_eq!(resolve_with(s(&gone), |d| (d == root).then(|| fake.clone())), Root::Board(PathBuf::from("/x/main").join(FILE)));
        assert_eq!(resolve_with(s(&gone), |_| None), Root::Gone);
        let bare = PathBuf::from("/x/proj.git");
        assert_eq!(resolve_with(s(&root), |_| Some(bare.clone())), Root::Board(bare.join(FILE)), "a bare clone keeps its board inside");
        let module = PathBuf::from("/x/main/.git/modules/sub");
        assert_eq!(resolve_with(s(&root), |_| Some(module.clone())), Root::Board(module.join(FILE)), "each submodule has its own");
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_large_board_is_truncated() {
        let root = std::env::temp_dir().join(format!("marsad-board-big-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(FILE);
        std::fs::write(&path, "x".repeat(2 * 1024 * 1024)).unwrap();
        let (text, truncated) = read(&path).unwrap();
        assert!(truncated);
        assert_eq!(text.len(), READ_CAP);
        std::fs::write(&path, b"small \xff board").unwrap();
        assert_eq!(read(&path).unwrap(), ("small \u{fffd} board".to_string(), false));
        assert!(read(&root.join("missing.md")).is_err());
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn only_a_regular_file_is_read() {
        let root = std::env::temp_dir().join(format!("marsad-board-fifo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("dir")).unwrap();
        assert_eq!(read(&root.join("dir")).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        #[cfg(unix)]
        {
            let fifo = root.join("fifo");
            if Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()) {
                std::os::unix::fs::symlink(&fifo, root.join(FILE)).unwrap();
                assert_eq!(read(&root.join(FILE)).unwrap_err().kind(), std::io::ErrorKind::InvalidInput, "must not block on a FIFO");
            }
        }
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn git_output_must_be_one_path() {
        let root = std::env::temp_dir().join(format!("marsad-board-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let real = root.canonicalize().unwrap();
        assert_eq!(common_dir_from(&root, ".git\n"), Some(real.join(".git")), "relative to the dir");
        assert_eq!(common_dir_from(&root, &format!("{}\n", real.join(".git").display())), Some(real.join(".git")));
        assert_eq!(common_dir_from(&root, "--path-format=absolute\n.git\n"), None, "old git echoes unknown flags");
        assert_eq!(common_dir_from(&root, "-x\n"), None);
        assert_eq!(common_dir_from(&root, "\n"), None);
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn the_fresh_template_is_empty() {
        let b = parse(TEMPLATE);
        assert_eq!(b, Board::default());
        assert!(!b.conflicts.iter().any(|c| c.status == ConflictStatus::NeedsHuman));
    }

    #[test]
    fn a_filled_board_parses() {
        let text = format!(
            "{TEMPLATE}
## agents
| agent | task | branch/worktree | status | started | last seen |
|---|---|---|---|---|---|
| agent-a | fix login | feat/login | working | 2026-10-23 9:00 | 2026-10-23 10:30 |
| agent-b | docs |
"
        );
        let text = text.replace(
            "<!-- - YYYY-MM-DD HH:MM from -> to: text   (to = agent name or all; reply as a new line) -->",
            "- 2026-10-23 10:00 agent-a -> all: is this needs-human? | open\n- 2026-10-23 10:01 agent-b -> agent-a: no",
        );
        let text = text.replace(
            "<!-- - YYYY-MM-DD HH:MM agent-a vs agent-b: issue | open / resolved / needs-human -->",
            "- 2026-10-23 10:02 agent-a vs agent-b: schema | needs-human\n- a needs-human mention | resolved\n- x vs y: lockfile | Open\nstray line",
        );
        let text = text.replace(
            "| path/glob | agent | since | note |\n|---|---|---|---|",
            "| path/glob | agent | since | note |\n|---|---|---|---|\n| src/a.rs | agent-a | 10:00 | login |\n| src/b.rs | agent-z | 10:00 | | extra |",
        );
        let b = parse(&text);
        assert_eq!(b.agents.len(), 2);
        assert_eq!(b.agents[0].name, "agent-a");
        assert_eq!(b.agents[0].last_seen, "2026-10-23 10:30");
        assert_eq!(b.agents[1], Agent { name: "agent-b".into(), task: "docs".into(), ..Agent::default() });
        assert_eq!(b.claims.len(), 2);
        assert_eq!((b.claims[1].path.as_str(), b.claims[1].agent.as_str()), ("src/b.rs", "agent-z"));
        assert_eq!(b.messages.len(), 2);
        assert!(b.messages[0].starts_with("2026-10-23 10:00 agent-a -> all"));
        let statuses: Vec<_> = b.conflicts.iter().map(|c| c.status).collect();
        assert_eq!(statuses, [ConflictStatus::NeedsHuman, ConflictStatus::Resolved, ConflictStatus::Open]);
        assert_eq!(b.conflicts.iter().filter(|c| c.status == ConflictStatus::NeedsHuman).count(), 1);
        assert_eq!(b.other, ["stray line"]);
    }

    #[test]
    fn columns_map_by_header_name() {
        let b = parse(
            "## Agents\n| status | last seen | extra | agent |\n| :-- | --- | --- | --- |\n| idle | 2026-10-23 10:00 | x | agent-q | more |\n",
        );
        assert_eq!(b.agents.len(), 1);
        assert_eq!((b.agents[0].name.as_str(), b.agents[0].last_seen.as_str()), ("agent-q", "2026-10-23 10:00"));
        assert_eq!(b.agents[0].status, "idle");
    }

    #[test]
    fn comments_are_stripped_even_across_lines() {
        let b = parse("## Decisions\n- keep <!-- hidden -->this\n<!--\n- gone\n-->\n- after\n## Messages\n<!-- open\n- lost\n");
        assert_eq!(b.decisions, ["keep this", "after"]);
        assert!(b.messages.is_empty(), "an unterminated comment drops the rest");
    }

    #[test]
    fn parsing_never_panics() {
        for text in ["", "|", "||||", "## ", "##", "## Agents\n|", "## Agents\n|---|\n| a |", "## Conflicts\n-\n- |\n- ||", "<!--", "-->", "## Claims\n\u{1F600}|é|\n|é"] {
            let _ = parse(text);
        }
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn staleness_starts_after_two_hours() {
        let now = at("2026-10-23 12:00");
        assert_eq!(agent_stale("2026-10-23 09:59", now), Some(true), "121 minutes");
        assert_eq!(agent_stale("2026-10-23 10:00", now), Some(false), "exactly 120 minutes");
        assert_eq!(agent_stale("2026-10-23 10:01", now), Some(false), "119 minutes");
        assert_eq!(agent_stale("2026-10-23 9:00", now), Some(true), "a single-digit hour");
        assert_eq!(agent_stale(" 2026-10-23 09:00 ", now), Some(true));
        assert_eq!(agent_stale("2026-10-23 12:30", now), None, "the future is not marked");
        assert_eq!(agent_stale("yesterday", now), None);
        assert_eq!(agent_stale("", now), None);
    }

    #[test]
    fn claims_follow_their_agent_and_orphans_are_stale() {
        let agent = |name: &str, seen: &str| Agent { name: name.into(), last_seen: seen.into(), ..Agent::default() };
        let claim = |who: &str| Claim { path: "src".into(), agent: who.into(), ..Claim::default() };
        let board = Board {
            agents: vec![agent("old", "2026-10-23 09:00"), agent("new", "2026-10-23 11:30"), agent("odd", "soon")],
            claims: vec![claim("old"), claim("new"), claim("nobody"), claim("odd")],
            ..Board::default()
        };
        let s = staleness(&board, at("2026-10-23 12:00"));
        assert_eq!(s.agents, [true, false, false]);
        assert_eq!(s.claims, [true, false, true, false]);
        assert_eq!(s.orphans, [false, false, true, false]);
    }
}
