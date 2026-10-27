//! Sharing to LinkedIn: one frozen stats snapshot, a line drafted by an isolated `claude -p` over a
//! redacted summary of today's work (or a template), the post text, the card, and the confirm steps
//! that save, open and reveal. The TUI share view and `marsad share --print` both use it.

use crate::badges::{self, Badges};
use crate::card;
use crate::model::fmt_count;
use crate::model::MAIN_AGENT;
use crate::store;
use anyhow::Context;
use clap::Args;
use rusqlite::Connection;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The longest the drafting `claude -p` may run before it is killed.
pub const DRAFT_WAIT: Duration = Duration::from_secs(60);

/// The longest `git config` or a waited-on desktop tool may take.
const TOOL_WAIT: Duration = Duration::from_secs(2);

/// Above this length LinkedIn's prefill link is dropped for the plain feed.
const MAX_URL: usize = 2000;

const FEED_URL: &str = "https://www.linkedin.com/feed/";

const SYSTEM_PROMPT: &str = "You write one short line for a LinkedIn post about the author's AI-assisted software work today. \
The user message is a summary of the author's Claude Code work today: session count, skills, subagent types and token figures. \
Write 1-3 sentences in the first person, professional and concrete, for recruiters and business owners judging the author's AI fluency. \
Describe the kind of work and how AI agents were used, in general terms only: categories such as planning, building features, \
debugging, testing, performance work, code review, security review and shipping. \
The post is public and the work may be confidential: never name projects, clients, companies, products or people, \
and never mention specific features, bugs, systems, languages, frameworks, servers, environments, branches or infrastructure, \
even if the summary seems to contain one. \
Never mention security findings, vulnerabilities, exploits, incidents, outages, authentication or login problems, credentials, \
secrets, customer data or production and staging systems; say only that changes were security reviewed. \
No hashtags, no emoji, no quotes, no lists, no token figures. Reply with the line only.";

/// One of today's sessions: where it ran, so its names can be kept out. Titles are never read:
/// they say what was worked on, which a public post must not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkSession {
    pub project: String,
    pub cwd: String,
}

/// Today's work: never sent as is; `summary` redacts it first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Work {
    /// Busiest first.
    pub sessions: Vec<WorkSession>,
    /// Skill and runs.
    pub skills: Vec<(String, i64)>,
    /// Subagent type and how many ran.
    pub agents: Vec<(String, i64)>,
}

/// Everything one share shows, frozen after the sync so the text, the card and the file name agree.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The local day, `YYYY-MM-DD`.
    pub date: String,
    pub today: i64,
    pub week: i64,
    pub month: i64,
    /// The per-day average over the last 30 days.
    pub daily_average: i64,
    /// The rank figure: the dashboard header's all-time total.
    pub all_time: i64,
    pub badges: Badges,
    /// `git config user.name`, for the card.
    pub name: Option<String>,
    pub work: Work,
    /// The user's home directory: cwd components at or above it are never names.
    pub home: Option<PathBuf>,
}

/// Freezes today's stats and work. `rank_total` is the all-time figure (`store::lifetime` in the
/// CLI, the header's live total in the dashboard); call it right after a sync.
pub fn snapshot(conn: &Connection, today: &str, rank_total: i64, home: Option<PathBuf>, name: Option<String>) -> anyhow::Result<Snapshot> {
    let periods = store::periods(conn, today)?;
    let period = |key: &str| periods.iter().find(|p| p.period == key);
    let total = |key: &str| period(key).map_or(0, |p| p.total);
    let badges = badges::badges(&badges::shown(conn, rank_total)?);
    Ok(Snapshot {
        date: today.to_string(),
        today: total("today"),
        week: total("7d"),
        month: total("30d"),
        daily_average: period("30d").map_or(0, |p| p.daily_average),
        all_time: rank_total,
        badges,
        name,
        work: work(conn, today)?,
        home,
    })
}

fn work(conn: &Connection, day: &str) -> anyhow::Result<Work> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(s.project, ''), COALESCE(s.cwd, '')
         FROM usage_daily u LEFT JOIN sessions s USING (session_id)
         WHERE u.day = ?1 GROUP BY u.session_id ORDER BY SUM(u.total) DESC",
    )?;
    let sessions = stmt
        .query_map([day], |r| Ok(WorkSession { project: r.get(0)?, cwd: r.get(1)? }))?
        .collect::<Result<_, _>>()?;
    let skills = store::skills_in(conn, store::Days::on(day))?.into_iter().map(|s| (s.skill, s.calls)).collect();
    let mut stmt = conn.prepare(
        "SELECT agent_type, COUNT(DISTINCT session_id || '/' || agent_id) FROM usage_daily
         WHERE day = ?1 AND agent_id != ?2 GROUP BY agent_type ORDER BY 2 DESC, 1",
    )?;
    let agents = stmt.query_map([day, MAIN_AGENT], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    Ok(Work { sessions, skills, agents })
}

/// The user's name from `git config user.name`, or None when git is missing, slow or unset.
pub fn git_user_name() -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(["config", "user.name"]);
    let out = run_captured(&mut cmd, b"", TOOL_WAIT, &AtomicBool::new(false)).ok()?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.success && !name.is_empty()).then_some(name)
}

// ---------- redaction ----------

/// Container folders that hold projects rather than name one; they never count as names.
const CONTAINERS: [&str; 18] = [
    "users", "home", "projects", "project", "code", "src", "dev", "documents", "desktop", "downloads", "work",
    "repos", "repositories", "git", "github", "workspace", "workspaces", "sites",
];

/// System folders above a cwd outside the home directory; they never count as names.
const SYSTEM: [&str; 15] =
    ["srv", "opt", "var", "tmp", "private", "mnt", "media", "volumes", "usr", "local", "data", "home", "users", "root", "folders"];

/// The tool's own name: never a name, even as a project's whole name, since every line says "Claude Code".
const TOOL: [&str; 3] = ["claude", "code", "anthropic"];

/// Tool and everyday software words. A folder or project named after one still counts as a name, but
/// as a word of a longer name (`.claude`, `skill-web`, `css-core`) it names nothing, and a drafted line
/// that says "Claude Code" or "web testing" must not be thrown away for it.
const GENERIC: [&str; 59] = [
    "claude", "code", "anthropic", "worktree", "worktrees", "agent", "agents", "skill", "skills", "session",
    "sessions", "plugin", "plugins", "app", "apps", "web", "api", "apis", "core", "test", "tests", "testing",
    "load", "query", "queries", "optimization", "performance", "engineering", "docs", "doc", "frontend",
    "backend", "server", "client", "service", "services", "database", "data", "mobile",
    "site", "website", "platform", "tool", "tools", "cli", "lib", "library", "sdk", "ui", "ux", "dashboard",
    "migration", "deploy", "release", "review", "fix", "feature", "main", "new",
];

/// The words of a name: split on `-`, `_`, `.`, spaces and camelCase boundaries.
fn tokens(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in name.split(['-', '_', '.', ' ']) {
        let chars: Vec<char> = part.chars().collect();
        let mut word = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let prev = i.checked_sub(1).map(|j| chars[j]);
            let next = chars.get(i + 1);
            // `acmePortal` splits before `P`; `ACMEPortal` before the `P` that starts a lower-case run.
            let starts = c.is_uppercase()
                && prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit() || (p.is_uppercase() && next.is_some_and(|n| n.is_lowercase())));
            if starts && !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            word.push(c);
        }
        if !word.is_empty() {
            out.push(word);
        }
    }
    out
}

/// Every project and folder name of today's sessions, with its words, longest first: what must
/// never leave the machine or come back in the reply.
fn names(work: &Work, home: Option<&Path>) -> Vec<String> {
    let mut raw = Vec::new();
    for s in &work.sessions {
        raw.push(s.project.clone());
        let cwd = Path::new(&s.cwd);
        // Below the home directory, every component (never home itself or its ancestors); outside
        // it, every component but system folders, so `/srv/clients/acme/app` names `acme` too.
        let below_home = home.and_then(|h| cwd.strip_prefix(h).ok());
        raw.extend(below_home.unwrap_or(cwd).components().filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        }).filter(|p| below_home.is_some() || !SYSTEM.contains(&p.to_lowercase().as_str())));
    }
    let project = |n: &str| work.sessions.iter().any(|s| s.project.eq_ignore_ascii_case(n));
    let mut out: Vec<String> = Vec::new();
    for name in raw {
        let lower = name.to_lowercase();
        if CONTAINERS.contains(&lower.as_str()) && !project(&name) {
            continue;
        }
        let whole = std::iter::once((name.clone(), true));
        for (n, is_whole) in whole.chain(tokens(&name).into_iter().map(|t| (t, false))) {
            let n = n.to_lowercase();
            let generic = TOOL.contains(&n.as_str()) || (GENERIC.contains(&n.as_str()) && !(is_whole && project(&n)));
            if n.chars().count() >= 3 && !generic && !out.contains(&n) {
                out.push(n);
            }
        }
    }
    out.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then_with(|| a.cmp(b)));
    out
}

/// Where `name` occurs in `text` as a whole word, case-insensitively: (start, end) char indexes.
fn find_word(text: &[char], name: &[char], from: usize) -> Option<(usize, usize)> {
    if name.is_empty() || text.len() < name.len() {
        return None;
    }
    let same = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    (from..=text.len() - name.len()).find_map(|i| {
        let end = i + name.len();
        let matches = text[i..end].iter().zip(name).all(|(&a, &b)| same(a, b));
        let open = i == 0 || !text[i - 1].is_alphanumeric();
        let close = end == text.len() || !text[end].is_alphanumeric();
        (matches && open && close).then_some((i, end))
    })
}

/// Word beginnings a public post about possibly confidential work must not carry: security findings,
/// auth, and the systems they were found in. A drafted line with a word that starts with one falls
/// back to the template. Over-matching (a memory "leak") only costs a template line.
const SENSITIVE_STEMS: [&str; 24] = [
    "vulnerab", "exploit", "cve", "breach", "leak", "incident", "outage", "passw", "credential", "secret",
    "malware", "ransom", "phish", "xss", "csrf", "idor", "sqli", "ssrf", "rce", "bypass", "authenticat",
    "authoriz", "login", "staging",
];

/// Whole words with the same rule, too short or too common to match as beginnings (`prod` vs `product`).
const SENSITIVE_WORDS: [&str; 4] = ["prod", "production", "auth", "pentest"];

/// The first word of `text` that is a security or environment detail, lower-cased.
fn sensitive(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .find(|w| SENSITIVE_WORDS.contains(&w.as_str()) || SENSITIVE_STEMS.iter().any(|s| w.starts_with(s)))
}

/// The first name in `text` as a whole word.
fn mentioned<'a>(text: &str, names: &'a [String]) -> Option<&'a str> {
    let chars: Vec<char> = text.chars().collect();
    names.iter().find(|n| find_word(&chars, &n.chars().collect::<Vec<_>>(), 0).is_some()).map(String::as_str)
}

/// `text` with every name removed as a whole word, then the leftover separators tidied.
fn strip(text: &str, names: &[String]) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    for name in names {
        let name: Vec<char> = name.chars().collect();
        let mut from = 0;
        while let Some((start, end)) = find_word(&chars, &name, from) {
            chars.drain(start..end);
            from = start;
        }
    }
    let s: String = chars.into_iter().collect();
    // Collapse the gaps a removed word leaves: runs of spaces, and separators doubled up or stranded at an end.
    let mut out = String::new();
    for word in s.split_whitespace() {
        let word = word.trim_matches(|c: char| matches!(c, '-' | '_' | '.' | ':' | '/'));
        if !word.chars().any(char::is_alphanumeric) {
            continue;
        }
        let mut tidy = String::new();
        for c in word.chars() {
            if matches!(c, '-' | '_') && tidy.ends_with(['-', '_']) {
                continue;
            }
            tidy.push(c);
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&tidy);
    }
    out.trim_matches(|c: char| !c.is_alphanumeric() && !matches!(c, ')' | '"' | '\'')).to_string()
}

/// What goes to `claude -p` on stdin: the session count, skills and subagent types with every name
/// stripped (emptied ones dropped), and the period figures. Never a title, project or cwd field.
fn summary(s: &Snapshot, names: &[String]) -> String {
    let mut out = String::from("Summary of my Claude Code work today (project and folder names removed).\n");
    out += &format!("Sessions today: {}\n", s.work.sessions.len());
    let counted = |items: &[(String, i64)]| -> Vec<String> {
        items.iter().filter_map(|(n, c)| Some(strip(n, names)).filter(|n| !n.is_empty()).map(|n| format!("{n} (x{c})"))).collect()
    };
    let skills = counted(&s.work.skills);
    if !skills.is_empty() {
        out += &format!("Skills used: {}\n", skills.join(", "));
    }
    let agents = counted(&s.work.agents);
    if !agents.is_empty() {
        out += &format!("Subagents run: {}\n", agents.join(", "));
    }
    out += &format!(
        "Tokens: today {}, last 7 days {}, last 30 days {} ({}/day), all time {}. Rank: {}.\n",
        count(s.today),
        count(s.week),
        count(s.month),
        count(s.daily_average),
        count(s.all_time),
        s.badges.rank.name
    );
    out
}

fn count(n: i64) -> String {
    fmt_count(n.max(0) as u64)
}

// ---------- drafting the line ----------

/// Why the post carries the template line instead of a drafted one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// `--no-ai`.
    Off,
    NotInstalled,
    /// Claude Code doesn't know an isolation flag.
    TooOld,
    /// Non-zero exit, unreadable output or the spawn failed.
    Failed(String),
    /// The JSON reply has `is_error`.
    Error(String),
    Timeout,
    Cancelled,
    Empty,
    /// The reply named a project or folder.
    Rejected(String),
    /// The reply carried a security or environment detail.
    Sensitive(String),
}

impl Note {
    pub fn message(&self) -> String {
        let why = match self {
            Note::Off => "AI drafting is off (--no-ai)".to_string(),
            Note::NotInstalled => "Claude Code (`claude`) was not found".to_string(),
            Note::TooOld => "your Claude Code is too old for an isolated draft; update it with `claude update`".to_string(),
            Note::Failed(why) => format!("claude failed: {why}"),
            Note::Error(why) => format!("claude returned an error: {why}"),
            Note::Timeout => format!("claude did not answer within {} s", DRAFT_WAIT.as_secs()),
            Note::Cancelled => "drafting was cancelled".to_string(),
            Note::Empty => "claude returned an empty line".to_string(),
            Note::Rejected(name) => format!("the drafted line named \"{name}\", one of your project or folder names"),
            Note::Sensitive(word) => format!("the drafted line said \"{word}\", a security or environment detail"),
        };
        format!("Template line, not generated: {why}.")
    }
}

/// The line for the post, and why it is the template when it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub line: String,
    pub note: Option<Note>,
}

impl Draft {
    pub fn template(s: &Snapshot, note: Note) -> Self {
        Draft { line: template_line(s), note: Some(note) }
    }
}

/// The kinds of work a skill or subagent name points to, in the order work usually flows, each
/// with the name words that mean it.
const ACTIVITIES: [(&str, &[&str]); 8] = [
    ("plan", &["plan", "planner", "brainstorm", "ideate", "strategy", "spec"]),
    ("build", &["work", "lfg", "implement", "prototype", "ui", "frontend"]),
    ("debug", &["debug", "rca", "cause", "trace", "investigate"]),
    ("refactor", &["simplify", "refactor", "cleanup"]),
    ("test", &["test", "tests", "e2e", "playwright", "healer", "qa"]),
    ("review", &["review", "reviewer", "reviewers", "audit"]),
    ("security-review", &["security", "redteam", "pentest", "blueteam", "attacker"]),
    ("ship", &["ship", "commit", "pr", "release", "deploy", "merge"]),
];

/// What a specialist reviewer's name may say it checks; any other word keeps it out of the line, so a
/// subagent named after a client or a finding is never published.
const REVIEW_FOCUS: [&str; 14] = [
    "security", "logic", "standards", "test", "performance", "accessibility", "architecture", "design", "plan",
    "doc", "code", "style", "api", "database",
];

/// `a, b and c`.
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The lower-case words of a skill or subagent name, without its `plugin:` prefix.
fn name_words(name: &str) -> Vec<String> {
    let name = name.rsplit(':').next().unwrap_or(name);
    tokens(name).into_iter().map(|w| w.to_lowercase()).collect()
}

/// The line built from today's skills and subagents, when no drafted line can be used: what kind of
/// work it was, not only how much. Names are stripped like the summary's.
pub fn template_line(s: &Snapshot) -> String {
    let n = s.work.sessions.len();
    if n == 0 && s.today <= 0 {
        return format!("No Claude Code sessions today; this is where my AI-assisted work stands at the {} rank.", s.badges.rank.name);
    }
    let names = names(&s.work, s.home.as_deref());
    let plural = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
    let used: Vec<Vec<String>> =
        s.work.skills.iter().chain(&s.work.agents).map(|(name, _)| name_words(&strip(name, &names))).collect();
    let activities: Vec<String> = ACTIVITIES
        .iter()
        .filter(|(_, words)| used.iter().any(|u| u.iter().any(|w| words.contains(&w.as_str()))))
        .map(|(verb, _)| verb.to_string())
        .collect();
    let mut line = if activities.is_empty() {
        format!("Today I built software with Claude Code across {}", plural(n.max(1), "session"))
    } else {
        format!("Today I used Claude Code to {} software across {}", and_list(&activities), plural(n.max(1), "session"))
    };
    let agents: i64 = s.work.agents.iter().map(|(_, c)| c).sum();
    let with: Vec<String> = [(s.work.skills.len(), "skill"), (agents.max(0) as usize, "subagent")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, one)| plural(n, one))
        .collect();
    if !with.is_empty() {
        line += &format!(", with {}", and_list(&with));
    }
    // Specialist reviewers say most about how the agents were used: `security-reviewer` is "security".
    let reviewers: Vec<String> = s
        .work
        .agents
        .iter()
        .filter_map(|(name, _)| {
            let words = name_words(&strip(name, &names));
            let (last, focus) = words.split_last()?;
            let known = !focus.is_empty() && focus.iter().all(|w| REVIEW_FOCUS.contains(&w.as_str()));
            (last == "reviewer" && known).then(|| focus.join("-"))
        })
        .fold(Vec::new(), |mut seen, focus| {
            if !seen.contains(&focus) {
                seen.push(focus);
            }
            seen
        });
    let reviewers = &reviewers[..reviewers.len().min(4)];
    if !reviewers.is_empty() {
        line += &format!(", including {} reviewer{}", and_list(reviewers), if reviewers.len() == 1 { "" } else { "s" });
    }
    line + "."
}

/// Drafts the line with the real `claude`.
pub fn draft(s: &Snapshot, cancel: &AtomicBool) -> Draft {
    draft_with(s, "claude", DRAFT_WAIT, cancel)
}

/// Drafts the line with `program`, falling back to the template with a note. Never retries
/// without the isolation flags.
fn draft_with(s: &Snapshot, program: &str, wait: Duration, cancel: &AtomicBool) -> Draft {
    let names = names(&s.work, s.home.as_deref());
    match ask(program, &summary(s, &names), wait, cancel) {
        Ok(line) => match (mentioned(&line, &names), sensitive(&line)) {
            (Some(name), _) => Draft::template(s, Note::Rejected(name.to_string())),
            (None, Some(word)) => Draft::template(s, Note::Sensitive(word)),
            (None, None) => Draft { line, note: None },
        },
        Err(note) => Draft::template(s, note),
    }
}

/// The isolated, single-turn, tool-less `claude -p` call: no hooks, CLAUDE.md, plugins or MCP,
/// and no transcript, so no ghost session.
fn claude_command(program: &str, cwd: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(["-p", "--safe-mode", "--no-session-persistence", "--tools", "", "--model", "haiku", "--output-format", "json"])
        .args(["--system-prompt", SYSTEM_PROMPT])
        .current_dir(cwd);
    cmd
}

/// Runs `program` over the summary in a fresh temp dir and returns the reply's text.
fn ask(program: &str, summary: &str, wait: Duration, cancel: &AtomicBool) -> Result<String, Note> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("marsad-draft-{}-{nanos:x}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| Note::Failed(format!("couldn't create a temp dir: {e}")))?;
    let out = run_captured(&mut claude_command(program, &dir), summary.as_bytes(), wait, cancel);
    let _ = std::fs::remove_dir_all(&dir);
    let out = out.map_err(|e| match e {
        RunError::NotFound => Note::NotInstalled,
        RunError::Spawn(why) => Note::Failed(why),
        RunError::Timeout => Note::Timeout,
        RunError::Cancelled => Note::Cancelled,
    })?;
    reply(&out)
}

/// The line from `claude -p --output-format json`, or the note for what went wrong.
fn reply(out: &Captured) -> Result<String, Note> {
    let json: Option<serde_json::Value> = serde_json::from_slice(&out.stdout).ok();
    let result = json.as_ref().and_then(|v| v["result"].as_str()).unwrap_or("");
    if json.as_ref().is_some_and(|v| v["is_error"].as_bool() == Some(true)) {
        return Err(Note::Error(first_line(result).unwrap_or("no details").to_string()));
    }
    if !out.success {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("unknown option") {
            return Err(Note::TooOld);
        }
        return Err(Note::Failed(first_line(&stderr).unwrap_or("non-zero exit").to_string()));
    }
    if json.is_none() {
        return Err(Note::Failed("unreadable output".into()));
    }
    // One paragraph: line breaks and runs of spaces become single spaces.
    let line = result.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = line.trim_matches('"').trim().to_string();
    if line.is_empty() { Err(Note::Empty) } else { Ok(line) }
}

fn first_line(s: &str) -> Option<&str> {
    s.lines().map(str::trim).find(|l| !l.is_empty())
}

// ---------- running a child with a deadline ----------

#[derive(Debug, PartialEq, Eq)]
enum RunError {
    NotFound,
    Spawn(String),
    Timeout,
    Cancelled,
}

#[derive(Debug, Default)]
struct Captured {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Reads a pipe to its end on its own thread.
fn drain(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

/// Runs `cmd` with `input` on stdin, capturing its output. Past `wait`, or once `cancel` is set,
/// the child is killed.
fn run_captured(cmd: &mut Command, input: &[u8], wait: Duration, cancel: &AtomicBool) -> Result<Captured, RunError> {
    let child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(RunError::NotFound),
        Err(e) => return Err(RunError::Spawn(e.to_string())),
    };
    // Dropping stdin once written closes it, so the child sees the end of the input.
    let stdin = child.stdin.take();
    let input = input.to_vec();
    std::thread::spawn(move || stdin.map(|mut s| s.write_all(&input)));
    let (stdout, stderr) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let deadline = Instant::now() + wait;
    let status = loop {
        let stop = if cancel.load(Ordering::Relaxed) {
            Some(RunError::Cancelled)
        } else if Instant::now() >= deadline {
            Some(RunError::Timeout)
        } else {
            None
        };
        if let Some(stop) = stop {
            let _ = child.kill();
            let _ = child.wait();
            return Err(stop);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(RunError::Spawn(e.to_string())),
        }
    };
    // A grandchild may still hold a pipe open; don't wait on it for long.
    let read = |rx: mpsc::Receiver<Vec<u8>>| rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
    Ok(Captured { success: status.success(), stdout: read(stdout), stderr: read(stderr) })
}

// ---------- the post ----------

/// The post: the line, a blank line, then the stats.
pub fn post(s: &Snapshot, line: &str) -> String {
    format!(
        "{}\n\nToday {} tokens · last 7 days {} · last 30 days {} ({}/day)\nAll-time {} tokens · Rank: {}",
        line.trim(),
        count(s.today),
        count(s.week),
        count(s.month),
        count(s.daily_average),
        count(s.all_time),
        s.badges.rank.name
    )
}

/// Percent-encodes every byte except the unreserved ones (and those `keep` allows).
fn percent_encode(s: &str, keep: &[u8]) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) || keep.contains(&b) {
            out.push(b as char);
        } else {
            out += &format!("%{b:02X}");
        }
    }
    out
}

/// LinkedIn's composer with the text filled in, or the plain feed when the link would be too long.
/// The prefill link is undocumented, so the clipboard copy is the guarantee.
pub fn linkedin_url(text: &str) -> String {
    let url = format!("{FEED_URL}?shareActive=true&text={}", percent_encode(text, b""));
    if url.len() > MAX_URL { FEED_URL.to_string() } else { url }
}

/// Where the card goes: the Downloads folder, else `<data_dir>/cards/`. A same-day re-share overwrites it.
pub fn card_path(downloads: Option<PathBuf>, data_dir: &Path, date: &str) -> PathBuf {
    downloads.unwrap_or_else(|| data_dir.join("cards")).join(format!("marsad-{date}.png"))
}

/// The card's path on this machine.
pub fn default_card_path(date: &str) -> PathBuf {
    card_path(dirs::download_dir(), &crate::data_dir(), date)
}

/// The card's contents from the snapshot. The only place a share builds `CardData`.
fn card_data(s: &Snapshot) -> card::CardData {
    let n = |v: i64| v.max(0) as u64;
    card::CardData {
        tier: s.badges.rank.tier,
        rank: s.badges.rank.name.to_string(),
        name: s.name.clone(),
        week: n(s.week),
        month: n(s.month),
        all_time: n(s.all_time),
        daily_average: n(s.daily_average),
        date: s.date.clone(),
        achievements: card::achievements(&s.badges),
    }
}

/// Renders the card and writes it to `path`.
pub fn save_card(s: &Snapshot, path: &Path) -> anyhow::Result<()> {
    card::write_at(&card_data(s), path)
}

// ---------- opening and revealing ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Macos,
    Windows,
    /// Linux and the BSDs: xdg-open and D-Bus.
    Unix,
}

/// Whether marsad can open things on the user's screen.
#[derive(Clone, Copy, Debug)]
pub struct Desktop {
    ssh: bool,
    display: bool,
    platform: Platform,
}

impl Desktop {
    pub fn current() -> Self {
        Self::from_env(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()))
    }

    /// A local desktop with a display, so tests see every launch.
    #[cfg(test)]
    pub fn for_tests() -> Self {
        Desktop { ssh: false, display: true, platform: Platform::Unix }
    }

    fn from_env(set: impl Fn(&str) -> bool) -> Self {
        let platform = if cfg!(target_os = "macos") {
            Platform::Macos
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Unix
        };
        let display = platform != Platform::Unix || set("DISPLAY") || set("WAYLAND_DISPLAY");
        Desktop { ssh: set("SSH_CONNECTION") || set("SSH_TTY"), display, platform }
    }

    /// Why nothing should be opened here, if so.
    fn skip(&self) -> Option<&'static str> {
        if self.ssh {
            Some("over SSH")
        } else if !self.display {
            Some("no display")
        } else {
            None
        }
    }
}

/// What opening the browser or revealing the card did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    Spawned,
    Skipped(&'static str),
    Failed(String),
}

/// One desktop tool to run. `wait` tools must exit successfully within a short deadline;
/// the others are spawned detached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub program: &'static str,
    pub args: Vec<String>,
    pub wait: bool,
}

fn step(program: &'static str, args: &[&str], wait: bool) -> Step {
    Step { program, args: args.iter().map(|a| a.to_string()).collect(), wait }
}

/// The tool that opens `url` in the browser. On Windows never `cmd /C start`, which splits on `&`.
fn open_steps(platform: Platform, url: &str) -> Vec<Step> {
    match platform {
        Platform::Macos => vec![step("open", &[url], false)],
        Platform::Windows => vec![step("rundll32", &["url.dll,FileProtocolHandler", url], false)],
        Platform::Unix => vec![step("xdg-open", &[url], false)],
    }
}

/// The tools that show `path` selected in the file manager, tried in order.
fn reveal_steps(platform: Platform, path: &Path) -> Vec<Step> {
    let p = path.to_string_lossy();
    match platform {
        Platform::Macos => vec![step("open", &["-R", &p], false)],
        Platform::Windows => vec![step("explorer", &[&format!("/select,{p}")], false)],
        Platform::Unix => {
            let uri = format!("array:string:file://{}", percent_encode(&p, b"/"));
            let folder = path.parent().map_or_else(|| ".".to_string(), |d| d.to_string_lossy().into_owned());
            vec![
                step(
                    "dbus-send",
                    &[
                        "--session",
                        "--print-reply",
                        "--dest=org.freedesktop.FileManager1",
                        "--type=method_call",
                        "/org/freedesktop/FileManager1",
                        "org.freedesktop.FileManager1.ShowItems",
                        &uri,
                        "string:",
                    ],
                    true,
                ),
                step("xdg-open", &[&folder], false),
            ]
        }
    }
}

/// Runs one step for real.
pub fn launch(step: &Step) -> Result<(), String> {
    if !step.wait {
        let args: Vec<&str> = step.args.iter().map(String::as_str).collect();
        return crate::alerts::detached(step.program, &args).map_err(|e| format!("couldn't run {}: {e}", step.program));
    }
    let mut cmd = Command::new(step.program);
    cmd.args(&step.args);
    match run_captured(&mut cmd, b"", TOOL_WAIT, &AtomicBool::new(false)) {
        Ok(out) if out.success => Ok(()),
        Ok(out) => Err(first_line(&String::from_utf8_lossy(&out.stderr)).unwrap_or("failed").to_string()),
        Err(RunError::NotFound) => Err(format!("{} is not installed", step.program)),
        Err(RunError::Spawn(why)) => Err(why),
        Err(_) => Err(format!("{} did not answer", step.program)),
    }
}

/// Tries each step until one works.
fn run_steps(desktop: &Desktop, steps: &[Step], run: &dyn Fn(&Step) -> Result<(), String>) -> Launch {
    if let Some(why) = desktop.skip() {
        return Launch::Skipped(why);
    }
    let mut last = "nothing to run".to_string();
    for s in steps {
        match run(s) {
            Ok(()) => return Launch::Spawned,
            Err(why) => last = why,
        }
    }
    Launch::Failed(last)
}

/// Opens LinkedIn's composer with `text` filled in.
pub fn open_linkedin(desktop: &Desktop, text: &str, run: &dyn Fn(&Step) -> Result<(), String>) -> (String, Launch) {
    let url = linkedin_url(text);
    let launched = run_steps(desktop, &open_steps(desktop.platform, &url), run);
    (url, launched)
}

/// Shows the card selected in the file manager.
pub fn reveal(desktop: &Desktop, path: &Path, run: &dyn Fn(&Step) -> Result<(), String>) -> Launch {
    run_steps(desktop, &reveal_steps(desktop.platform, path), run)
}

// ---------- `marsad share` ----------

#[derive(Args)]
pub struct ShareArgs {
    /// Print the post and write the card without opening anything (for terminals without a desktop).
    #[arg(long)]
    print: bool,
    /// Where to write the card (default: Downloads/marsad-YYYY-MM-DD.png).
    #[arg(long, requires = "print")]
    out: Option<PathBuf>,
    /// Use the template line instead of drafting one with Claude Code.
    #[arg(long)]
    no_ai: bool,
}

/// `marsad share --print`: sync, snapshot, draft the line, print the post and write the card.
pub fn run(a: ShareArgs) -> anyhow::Result<()> {
    if !a.print {
        anyhow::bail!("use `marsad share --print`, or press p in the dashboard to review and share");
    }
    store::open_and_sync()?;
    let conn = store::open_readonly()?;
    let total = store::lifetime(&conn)?.total;
    let s = snapshot(&conn, &store::day_n_ago(0), total, dirs::home_dir(), git_user_name())?;
    let d = if a.no_ai { Draft::template(&s, Note::Off) } else { draft(&s, &AtomicBool::new(false)) };
    let path = a.out.unwrap_or_else(|| default_card_path(&s.date));
    let text = print(&s, &d, &path)?;
    println!("{text}");
    if let Some(note) = &d.note {
        eprintln!("{}", note.message());
    }
    eprintln!("card: {}", path.display());
    Ok(())
}

/// Writes the card and returns the post. Opens nothing.
fn print(s: &Snapshot, d: &Draft, path: &Path) -> anyhow::Result<String> {
    save_card(s, path).with_context(|| format!("couldn't write the card to {}", path.display()))?;
    Ok(post(s, &d.line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marsad-share-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn db(rows: &str) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(store::SCHEMA).unwrap();
        conn.execute_batch(rows).unwrap();
        conn
    }

    const DAY: &str = "2026-10-24";

    /// Today: one session in `acme-portal` titled "Acme Corp invoice parser" (never read), the skill `acme-deploy`
    /// and `ce-plan`, and two Explore subagents.
    fn acme() -> Snapshot {
        let conn = db(&format!(
            "INSERT INTO sessions VALUES ('s1', 'acme-portal', '/Users/x/Projects/acme-portal', 'Acme Corp invoice parser', 'opus', NULL, NULL, NULL);
             INSERT INTO usage_daily VALUES ('s1', 'main', 'main', '{DAY}', 0, 0, 0, 0, 12000000, 3);
             INSERT INTO usage_daily VALUES ('s1', 'a1', 'Explore', '{DAY}', 0, 0, 0, 0, 1000, 1);
             INSERT INTO usage_daily VALUES ('s1', 'a2', 'Explore', '{DAY}', 0, 0, 0, 0, 1000, 1);
             INSERT INTO token_days VALUES ('s1', 'main', '{DAY}', 0, 0, 0, 0, 12002000, 3);
             INSERT INTO skill_runs VALUES ('s1', 'main', 'main', 1, 'acme-deploy', 'tool', NULL, '{DAY}', 1, 0, 0, 0, 0, 20);
             INSERT INTO skill_runs VALUES ('s1', 'main', 'main', 2, 'ce-plan', 'tool', NULL, '{DAY}', 1, 0, 0, 0, 0, 10);"
        ));
        snapshot(&conn, DAY, 30_000_000_000, Some("/Users/x".into()), Some("Ada".into())).unwrap()
    }

    /// A fake `claude` that records its args and stdin next to itself, then runs `body`.
    #[cfg(unix)]
    fn fake(dir: &Path, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("claude");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{d}/args'\npwd > '{d}/cwd'\ncat > '{d}/stdin'\n{body}\n",
            d = dir.display()
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[cfg(unix)]
    fn answer(text: &str) -> String {
        format!("printf '%s\\n' '{}'", serde_json::json!({ "type": "result", "is_error": false, "result": text }))
    }

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn snapshot_freezes_periods_rank_and_todays_work() {
        let s = acme();
        assert_eq!((s.today, s.week, s.month, s.all_time), (12_002_000, 12_002_000, 12_002_000, 30_000_000_000));
        assert_eq!(s.daily_average, 12_002_000 / 30);
        assert_eq!(s.badges.rank.name, "Expert", "the rank comes from the all-time figure");
        assert_eq!(s.work.sessions.len(), 1);
        assert_eq!(s.work.skills.len(), 2);
        assert_eq!(s.work.agents, [("Explore".to_string(), 2)]);
        assert_eq!(s.name.as_deref(), Some("Ada"));
    }

    #[test]
    fn covers_ae3_a_missing_claude_gives_the_template_and_says_so() {
        let s = acme();
        let d = draft_with(&s, "marsad-no-such-claude", Duration::from_secs(5), &never());
        assert_eq!(d.line, template_line(&s));
        assert_eq!(d.note, Some(Note::NotInstalled));
        assert!(d.note.unwrap().message().contains("not generated"));
    }

    #[test]
    fn skills_and_agents_go_out_without_titles_or_project_names() {
        let s = acme();
        let names = names(&s.work, s.home.as_deref());
        let payload = summary(&s, &names);
        assert!(!payload.contains("invoice") && payload.contains("Sessions today: 1\n"), "no session title: {payload}");
        assert!(!payload.to_lowercase().contains("acme"), "{payload}");
        assert!(!payload.contains("portal") && !payload.contains("/Users"), "no project or cwd field: {payload}");
        assert!(payload.contains("Skills used: deploy (x1), ce-plan (x1)"), "acme-deploy loses acme: {payload}");
        assert!(payload.contains("Subagents run: Explore (x2)"));
        assert!(payload.contains("Rank: Expert."));
    }

    #[test]
    fn a_name_that_empties_a_skill_drops_it() {
        let mut s = acme();
        s.work.skills = vec![("acme".into(), 2), ("portal".into(), 1), ("review".into(), 1)];
        s.work.agents = vec![("acme-portal".into(), 1)];
        let payload = summary(&s, &names(&s.work, s.home.as_deref()));
        assert!(payload.contains("Skills used: review (x1)\n"), "{payload}");
        assert!(!payload.contains("Subagents run"), "{payload}");
    }

    #[test]
    fn names_are_project_and_folders_below_home_with_their_words() {
        let work = |project: &str, cwd: &str| Work {
            sessions: vec![WorkSession { project: project.into(), cwd: cwd.into() }],
            ..Work::default()
        };
        let n = names(&work("acme-portal", "/Users/x/Projects/acme-portal"), Some(Path::new("/Users/x")));
        assert_eq!(n, ["acme-portal", "portal", "acme"]);
        let n = names(&work("billingApp", "/Users/x/clients/GlobexCorp/billingApp"), Some(Path::new("/Users/x")));
        for want in ["billingapp", "billing", "globexcorp", "globex", "corp", "clients"] {
            assert!(n.contains(&want.to_string()), "{want} missing from {n:?}");
        }
        assert!(!n.contains(&"app".to_string()), "a generic word of a longer name is not a name: {n:?}");
        // A worktree under `.claude` and a project of everyday words: "Claude Code" and "web" stay usable.
        let n = names(&work("skill-web", "/Users/x/Projects/skill-web/.claude/worktrees/load-test"), Some(Path::new("/Users/x")));
        assert_eq!(n, ["load-test", "skill-web", ".claude"]);
        let mut n = names(&work("web", "/Users/x/Projects/web"), Some(Path::new("/Users/x")));
        n.sort();
        assert_eq!(n, ["web"], "a project named after a generic word is still a name");
        let n = names(&work("claude", "/Users/x/Projects/claude"), Some(Path::new("/Users/x")));
        assert!(n.is_empty(), "the tool's own name never blocks \"Claude Code\": {n:?}");
        let n = names(&work("acme-auth", "/Users/x/Projects/acme-auth"), Some(Path::new("/Users/x")));
        assert!(n.contains(&"auth".to_string()), "{n:?}");
        let n = names(&work("ae-seeded-Rf", "/private/var/folders/cr/T/ae-seeded-Rf"), Some(Path::new("/Users/x")));
        assert!(!n.contains(&"folders".to_string()), "{n:?}");
        assert!(!n.contains(&"x".to_string()) && !n.contains(&"users".to_string()));
        // Outside the home directory every folder counts except system ones; short tokens never do.
        let n = names(&work("wt-u5", "/private/tmp/wt-u5"), Some(Path::new("/Users/x")));
        assert_eq!(n, ["wt-u5"]);
        let mut n = names(&work("app", "/srv/clients/acme/app"), Some(Path::new("/Users/x")));
        n.sort();
        assert_eq!(n, ["acme", "app", "clients"], "a client folder above the project is a name too");
        let n = names(&work("web", "/Volumes/Work/Globex/web"), None);
        assert!(n.contains(&"globex".to_string()) && !n.contains(&"volumes".to_string()), "{n:?}");
        assert_eq!(tokens("ACMEPortal_v2.web"), ["ACME", "Portal", "v2", "web"]);
    }

    #[test]
    fn the_post_check_matches_whole_words_only() {
        let s = acme();
        let names = names(&s.work, s.home.as_deref());
        assert_eq!(mentioned("I organised users and projects with AI agents.", &names), None);
        assert_eq!(mentioned("I built an Acme invoice parser.", &names), Some("acme"));
        assert_eq!(mentioned("Shipped the ACME-PORTAL release.", &names), Some("acme-portal"));
        assert_eq!(mentioned("Acmes and portals are not whole words.", &names), None);
        assert_eq!(strip("Fix acme: Acme-portal login (ACME)", &names), "Fix login");
    }

    #[cfg(unix)]
    #[test]
    fn covers_ae4_a_reply_naming_the_client_is_rejected() {
        let dir = scratch("ae4");
        let s = acme();
        let d = draft_with(&s, &fake(&dir, &answer("I built an Acme invoice parser with AI agents.")), Duration::from_secs(10), &never());
        assert_eq!(d.note, Some(Note::Rejected("acme".into())));
        assert_eq!(d.line, template_line(&s));
        assert!(!d.line.to_lowercase().contains("acme"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_reply_with_a_security_or_environment_detail_is_rejected() {
        let dir = scratch("sensitive");
        let s = acme();
        for (reply, word) in [
            ("I found and fixed an IDOR vulnerability with review agents.", "idor"),
            ("I shipped reviewed changes to Production.", "production"),
            ("I patched an authentication bypass and rotated API tokens.", "authentication"),
            ("I fixed two CVEs and stopped secrets leaking.", "cves"),
            ("I hardened the auth flow.", "auth"),
        ] {
            let d = draft_with(&s, &fake(&dir, &answer(reply)), Duration::from_secs(10), &never());
            assert_eq!(d.note, Some(Note::Sensitive(word.into())), "{reply}");
            assert_eq!(d.line, template_line(&s));
        }
        let ok = "I planned, built and security reviewed product changes with parallel review agents.";
        assert_eq!(draft_with(&s, &fake(&dir, &answer(ok)), Duration::from_secs(10), &never()).note, None);
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_clean_reply_is_the_line_and_the_call_is_isolated() {
        let dir = scratch("clean");
        let s = acme();
        let program = fake(&dir, &answer("  I planned and shipped an invoice parser,\nusing users and projects data. "));
        let d = draft_with(&s, &program, Duration::from_secs(10), &never());
        assert_eq!(d, Draft { line: "I planned and shipped an invoice parser, using users and projects data.".into(), note: None });
        let args = std::fs::read_to_string(dir.join("args")).unwrap();
        let args: Vec<&str> = args.lines().collect();
        assert_eq!(&args[..10], ["-p", "--safe-mode", "--no-session-persistence", "--tools", "", "--model", "haiku", "--output-format", "json", "--system-prompt"]);
        assert!(!args.contains(&"--max-turns") && !args.contains(&"--resume") && !args.contains(&"--continue"));
        let stdin = std::fs::read_to_string(dir.join("stdin")).unwrap();
        assert!(!stdin.contains("invoice") && !stdin.to_lowercase().contains("acme"), "{stdin}");
        let cwd = std::fs::read_to_string(dir.join("cwd")).unwrap();
        assert!(cwd.contains("marsad-draft-"), "a fresh temp cwd: {cwd}");
        assert!(!Path::new(cwd.trim()).exists(), "the temp cwd is removed afterwards");
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_claude_past_the_deadline_is_killed() {
        let dir = scratch("slow");
        let s = acme();
        let start = Instant::now();
        let d = draft_with(&s, &fake(&dir, "exec sleep 30"), Duration::from_millis(300), &never());
        assert_eq!(d.note, Some(Note::Timeout));
        assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn cancelling_kills_the_child() {
        let dir = scratch("cancel");
        let s = acme();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::Relaxed);
        });
        let start = Instant::now();
        let d = draft_with(&s, &fake(&dir, "exec sleep 30"), Duration::from_secs(30), &cancel);
        assert_eq!(d.note, Some(Note::Cancelled));
        assert!(start.elapsed() < Duration::from_secs(5));
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn errors_old_versions_and_empty_replies_each_have_their_note() {
        let dir = scratch("notes");
        let s = acme();
        let cases = [
            (r#"echo '{"type":"result","is_error":true,"result":"Not logged in"}'; exit 1"#, Note::Error("Not logged in".into())),
            ("echo \"error: unknown option '--safe-mode'\" >&2; exit 1", Note::TooOld),
            ("echo boom >&2; exit 2", Note::Failed("boom".into())),
            ("echo not json", Note::Failed("unreadable output".into())),
            (&answer("   "), Note::Empty),
        ];
        for (body, want) in cases {
            let d = draft_with(&s, &fake(&dir, body), Duration::from_secs(10), &never());
            assert_eq!(d.note, Some(want.clone()), "{body}");
            assert_eq!(d.line, template_line(&s));
        }
        let messages: Vec<String> = [Note::Off, Note::NotInstalled, Note::TooOld, Note::Timeout, Note::Cancelled, Note::Empty]
            .iter()
            .map(Note::message)
            .collect();
        assert!(messages.iter().all(|m| m.starts_with("Template line, not generated")));
        assert!(Note::TooOld.message().contains("too old") && !Note::TooOld.message().contains("failed"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn a_reply_saying_claude_code_is_kept_when_a_folder_is_dot_claude() {
        let dir = scratch("dotclaude");
        let mut s = acme();
        s.work.sessions.push(WorkSession {
            project: "load-test".into(),
            cwd: "/Users/x/Projects/acme-portal/.claude/worktrees/load-test".into(),
        });
        let line = "I used Claude Code to plan and load test a web app with parallel review agents.";
        let d = draft_with(&s, &fake(&dir, &answer(line)), Duration::from_secs(10), &never());
        assert_eq!(d, Draft { line: line.into(), note: None });
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_template_says_what_kind_of_work_it_was() {
        let s = acme();
        assert_eq!(template_line(&s), "Today I used Claude Code to plan and ship software across 1 session, with 2 skills and 2 subagents.");
        let mut busy = acme();
        busy.work.skills = ["compound-engineering:ce-work", "assunnah-engineering:ae-review", "ce-commit-push-pr", "ae-test", "acme-deploy"]
            .iter()
            .map(|n| (n.to_string(), 1))
            .collect();
        busy.work.agents = vec![
            ("general-purpose".into(), 30),
            ("assunnah-engineering:security-reviewer".into(), 2),
            ("security-reviewer".into(), 1),
            ("logic-reviewer".into(), 2),
            ("acme-reviewer".into(), 1),
            ("globex-billing-reviewer".into(), 1),
            ("vulnerability-reviewer".into(), 1),
        ];
        assert_eq!(
            template_line(&busy),
            "Today I used Claude Code to build, test, review, security-review and ship software across 1 session, \
             with 5 skills and 38 subagents, including security and logic reviewers.",
            "duplicates, client-named and finding-named reviewers stay out"
        );
        let plain = Snapshot { work: Work { sessions: s.work.sessions.clone(), ..Work::default() }, ..s.clone() };
        assert_eq!(template_line(&plain), "Today I built software with Claude Code across 1 session.");
        let quiet = Snapshot { today: 0, work: Work::default(), ..s };
        assert!(template_line(&quiet).starts_with("No Claude Code sessions today"));
    }

    #[test]
    fn the_post_has_the_line_then_the_stats() {
        let s = acme();
        assert_eq!(
            post(&s, " I shipped it. "),
            "I shipped it.\n\nToday 12M tokens · last 7 days 12M · last 30 days 12M (400.1k/day)\nAll-time 30B tokens · Rank: Expert"
        );
    }

    #[test]
    fn the_url_encodes_the_text_and_falls_back_to_the_feed_when_long() {
        let url = linkedin_url("Line one\n#AI & more");
        assert_eq!(url, "https://www.linkedin.com/feed/?shareActive=true&text=Line%20one%0A%23AI%20%26%20more");
        assert_eq!(linkedin_url("·"), format!("{FEED_URL}?shareActive=true&text=%C2%B7"));
        assert_eq!(linkedin_url(&"a".repeat(MAX_URL)), FEED_URL);
        let base = format!("{FEED_URL}?shareActive=true&text=").len();
        assert_eq!(linkedin_url(&"a".repeat(MAX_URL - base)).len(), MAX_URL, "exactly the cap still prefills");
    }

    #[test]
    fn the_card_goes_to_downloads_else_the_data_dir() {
        assert_eq!(card_path(Some("/d".into()), Path::new("/m"), DAY), Path::new("/d/marsad-2026-10-24.png"));
        assert_eq!(card_path(None, Path::new("/m"), DAY), Path::new("/m/cards/marsad-2026-10-24.png"));
    }

    #[test]
    fn ssh_or_no_display_skips_open_and_reveal() {
        let ran = std::cell::Cell::new(0);
        let run = |_: &Step| -> Result<(), String> {
            ran.set(ran.get() + 1);
            Ok(())
        };
        let ssh = Desktop::from_env(|name| name == "SSH_TTY");
        assert_eq!(open_linkedin(&ssh, "hi", &run).1, Launch::Skipped("over SSH"));
        assert_eq!(reveal(&ssh, Path::new("/d/c.png"), &run), Launch::Skipped("over SSH"));
        let headless = Desktop { ssh: false, display: false, platform: Platform::Unix };
        assert_eq!(reveal(&headless, Path::new("/d/c.png"), &run), Launch::Skipped("no display"));
        assert_eq!(ran.get(), 0);
        let linux = Desktop::from_env(|name| name == "WAYLAND_DISPLAY");
        assert!(linux.skip().is_none());
    }

    #[test]
    fn each_platform_opens_and_reveals_with_its_own_tools() {
        let url = "https://www.linkedin.com/feed/?shareActive=true&text=a%26b";
        assert_eq!(open_steps(Platform::Macos, url), [step("open", &[url], false)]);
        let win = &open_steps(Platform::Windows, url)[0];
        assert_ne!(win.program, "cmd", "cmd /C start splits on &");
        assert_eq!(win.args.last().unwrap(), url);
        let card = Path::new("/home/x/Down loads/marsad,1.png");
        assert_eq!(reveal_steps(Platform::Macos, card), [step("open", &["-R", "/home/x/Down loads/marsad,1.png"], false)]);
        assert_eq!(reveal_steps(Platform::Windows, Path::new(r"C:\d\c.png"))[0].args, [r"/select,C:\d\c.png"]);
        let unix = reveal_steps(Platform::Unix, card);
        assert_eq!((unix[0].program, unix[0].wait), ("dbus-send", true));
        assert!(unix[0].args.contains(&"array:string:file:///home/x/Down%20loads/marsad%2C1.png".to_string()));
        assert_eq!(unix[1], step("xdg-open", &["/home/x/Down loads"], false));
        // When D-Bus has no file manager, the folder opens instead.
        let desktop = Desktop { ssh: false, display: true, platform: Platform::Unix };
        let run = |s: &Step| if s.program == "dbus-send" { Err("no FileManager1".to_string()) } else { Ok(()) };
        assert_eq!(reveal(&desktop, card, &run), Launch::Spawned);
        let none = |_: &Step| Err("not installed".to_string());
        assert_eq!(reveal(&desktop, card, &none), Launch::Failed("not installed".into()));
    }

    #[test]
    fn print_writes_a_png_and_returns_the_post() {
        let dir = scratch("print");
        let s = acme();
        let out = dir.join("c.png");
        let d = Draft::template(&s, Note::Off);
        // `print` has no way to open anything: it takes no launcher.
        let text = print(&s, &d, &out).unwrap();
        assert_eq!(text, post(&s, &template_line(&s)));
        let png = std::fs::read(&out).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), card::WIDTH);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_card_data_matches_the_snapshot() {
        let c = card_data(&acme());
        assert_eq!((c.tier, c.rank.as_str(), c.all_time, c.date.as_str()), (4, "Expert", 30_000_000_000, DAY));
    }
}
