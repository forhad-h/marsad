//! Self-update: checks GitHub releases for a newer version and installs it with the release's
//! own installer script (the same one the README tells people to pipe into `sh`).
//!
//! Network access goes through `curl`, which the installer needs anyway, so marsad carries no
//! HTTP or TLS dependency. Hooks and the status line never touch this module.

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};

const REPO: &str = "forhad-h/marsad";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// The dashboard asks GitHub at most this often (unauthenticated API: 60 requests/hour/IP).
const CHECK_EVERY_SECS: f64 = 24.0 * 3600.0;
/// "What's new" stays short: at most this many lines across all releases shown.
const MAX_NOTE_LINES: usize = 8;

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Toggle {
    On,
    Off,
}

#[derive(clap::Args)]
pub struct UpdateArgs {
    /// Only show whether a newer version exists; don't install it.
    #[arg(long)]
    check: bool,
    /// Turn automatic updates on or off (the dashboard installs new versions in the background).
    #[arg(long, value_name = "on|off")]
    auto: Option<Toggle>,
}

/// Persisted in `<data dir>/update.json`: the auto-update setting and the last check's result.
#[derive(Serialize, Deserialize, Default)]
struct State {
    #[serde(default)]
    auto: bool,
    #[serde(default)]
    checked_at: f64,
    #[serde(default)]
    latest: Option<String>,
}

fn state_path() -> PathBuf {
    crate::data_dir().join("update.json")
}

fn load_state() -> State {
    std::fs::read_to_string(state_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_state(state: &State) -> anyhow::Result<()> {
    let path = state_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, serde_json::to_string_pretty(state)?)?;
    Ok(())
}

/// `(major, minor, patch)` from "v1.2.3" or "1.2.3"; pre-release suffixes are ignored.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

fn is_newer(candidate: &str, than: &str) -> bool {
    matches!((parse_version(candidate), parse_version(than)), (Some(a), Some(b)) if a > b)
}

struct Release {
    version: String,
    notes: Vec<String>,
}

/// The release notes part of a release body. `dist` puts the CHANGELOG section first and its
/// install instructions after, under "## Install …".
fn release_notes(body: &str) -> Vec<String> {
    let notes = body.split("\n## Install ").next().unwrap_or_default();
    let notes = if notes.trim_start().starts_with("## Install ") { "" } else { notes };
    notes
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| match l.trim_start().strip_prefix("* ") {
            Some(rest) => format!("- {rest}"),
            None => l.to_string(),
        })
        .collect()
}

/// Published releases, newest first (drafts and pre-releases skipped).
fn parse_releases(json: &Value) -> Vec<Release> {
    let mut releases: Vec<Release> = json
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| !r["draft"].as_bool().unwrap_or(false) && !r["prerelease"].as_bool().unwrap_or(false))
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?;
            parse_version(tag)?;
            Some(Release {
                version: tag.trim_start_matches('v').to_string(),
                notes: release_notes(r["body"].as_str().unwrap_or_default()),
            })
        })
        .collect();
    releases.sort_by_key(|r| std::cmp::Reverse(parse_version(&r.version)));
    releases
}

fn curl(url: &str) -> anyhow::Result<Vec<u8>> {
    let out = Command::new("curl")
        .args(["--proto", "=https", "--tlsv1.2", "-fsSL", "--max-time", "15"])
        .args(["-H", "Accept: application/vnd.github+json", url])
        .stdin(Stdio::null())
        .output()
        .context("couldn't run curl (is it installed?)")?;
    if !out.status.success() {
        bail!("couldn't reach {url}: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

fn fetch_releases() -> anyhow::Result<Vec<Release>> {
    let body = curl(&format!("https://api.github.com/repos/{REPO}/releases?per_page=20"))?;
    let json: Value = serde_json::from_slice(&body).context("GitHub returned an unexpected response")?;
    Ok(parse_releases(&json))
}

/// The newest release, after recording the check.
fn check_now(state: &mut State) -> anyhow::Result<Vec<Release>> {
    let releases = fetch_releases()?;
    state.checked_at = crate::now();
    state.latest = releases.first().map(|r| r.version.clone());
    let _ = save_state(state);
    Ok(releases)
}

/// Where the installer should put the new binary so it replaces the one running now.
/// The installer writes to `$MARSAD_INSTALL_DIR/bin`, so the binary must live in a `bin` dir;
/// anything else (a `cargo build` in `target/`) wasn't installed by the installer.
fn install_root(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    (bin.file_name()? == "bin").then(|| bin.parent().map(Path::to_path_buf)).flatten()
}

fn current_exe() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe().context("can't locate the marsad binary")?;
    Ok(exe.canonicalize().unwrap_or(exe))
}

/// Downloads and runs the installer for `version`. With `quiet`, nothing is printed (the
/// dashboard is on screen); otherwise the installer's output streams to the terminal.
fn install(version: &str, quiet: bool) -> anyhow::Result<PathBuf> {
    let exe = current_exe()?;
    let Some(root) = install_root(&exe) else {
        bail!(
            "{} wasn't installed by the marsad installer, so it can't update itself.\n\
             Rebuild it, or install with: cargo install --git https://github.com/{REPO}",
            exe.display()
        );
    };
    let (script, ext) = if cfg!(windows) { ("marsad-installer.ps1", "ps1") } else { ("marsad-installer.sh", "sh") };
    let url = format!("https://github.com/{REPO}/releases/download/v{version}/{script}");
    let tmp = std::env::temp_dir().join(format!("marsad-installer-{}.{ext}", std::process::id()));
    std::fs::write(&tmp, curl(&url)?)?;

    // Windows can't overwrite a running .exe but can rename it, so move it out of the way.
    let parked = exe.with_extension("old.exe");
    if cfg!(windows) {
        let _ = std::fs::remove_file(&parked);
        std::fs::rename(&exe, &parked).context("couldn't move the running marsad.exe aside")?;
    }

    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&tmp);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg(&tmp);
        c
    };
    cmd.env("MARSAD_INSTALL_DIR", &root).env("MARSAD_NO_MODIFY_PATH", "1").stdin(Stdio::null());
    if quiet {
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
    }
    let status = cmd.status();
    let _ = std::fs::remove_file(&tmp);

    let ok = matches!(status, Ok(s) if s.success()) && installed_version(&exe).as_deref() == Some(version);
    if !ok {
        if cfg!(windows) && !exe.exists() {
            let _ = std::fs::rename(&parked, &exe);
        }
        bail!("the installer didn't finish installing v{version}");
    }
    Ok(exe)
}

/// The version the binary at `exe` reports ("marsad 1.2.3" → "1.2.3").
fn installed_version(exe: &Path) -> Option<String> {
    let out = Command::new(exe).arg("--version").stdin(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().nth(1).map(str::to_string)
}

/// "What's new" for every release newer than the running one, capped to a few lines.
fn whats_new(releases: &[Release]) -> Vec<String> {
    let newer: Vec<&Release> = releases.iter().filter(|r| is_newer(&r.version, CURRENT)).collect();
    let mut lines = Vec::new();
    let mut hidden = 0;
    for r in &newer {
        let header = if newer.len() > 1 { 1 } else { 0 };
        if r.notes.is_empty() {
            continue;
        }
        let room = MAX_NOTE_LINES.saturating_sub(lines.len() + header);
        if room == 0 {
            hidden += r.notes.len();
            continue;
        }
        if header == 1 {
            lines.push(format!("v{}", r.version));
        }
        lines.extend(r.notes.iter().take(room).map(|n| format!("  {n}")));
        hidden += r.notes.len().saturating_sub(room);
    }
    if hidden > 0 {
        lines.push(format!("  … and {hidden} more"));
    }
    lines
}

/// Terminal styles for `marsad update`, empty when stdout isn't a terminal.
struct Paint {
    yellow: &'static str,
    green: &'static str,
    bold: &'static str,
    dim: &'static str,
    reset: &'static str,
}

impl Paint {
    fn new() -> Self {
        use std::io::IsTerminal;
        if std::io::stdout().is_terminal() {
            Self { yellow: "\x1b[33m", green: "\x1b[32m", bold: "\x1b[1m", dim: "\x1b[2m", reset: "\x1b[0m" }
        } else {
            Self { yellow: "", green: "", bold: "", dim: "", reset: "" }
        }
    }
}

fn auto_hint(auto: bool) -> String {
    let Paint { dim, reset, .. } = Paint::new();
    if auto {
        format!("{dim}Automatic updates are on (turn off: marsad update --auto off){reset}")
    } else {
        format!("{dim}Automatic updates are off (turn on: marsad update --auto on){reset}")
    }
}

/// `marsad update`.
pub fn run(args: UpdateArgs) -> anyhow::Result<()> {
    let Paint { yellow, green, bold, dim, reset } = Paint::new();
    let mut state = load_state();
    if let Some(toggle) = args.auto {
        state.auto = matches!(toggle, Toggle::On);
        save_state(&state)?;
        let word = if state.auto { "on" } else { "off" };
        println!("Automatic updates are {word}.");
        if state.auto {
            println!("{dim}The dashboard checks once a day and installs new versions in the background.{reset}");
        }
        return Ok(());
    }

    println!("Checking for updates…");
    let releases = check_now(&mut state)?;
    let Some(latest) = releases.first().map(|r| r.version.clone()) else { bail!("no published releases found") };

    println!();
    println!("  Current  {bold}v{CURRENT}{reset}");
    println!("  Latest   {bold}v{latest}{reset}");
    println!();

    if !is_newer(&latest, CURRENT) {
        println!("{green}✓ You're on the latest version.{reset}");
        println!("{}", auto_hint(state.auto));
        return Ok(());
    }

    println!("{bold}What's new{reset}");
    let notes = whats_new(&releases);
    if notes.is_empty() {
        println!("  {dim}See https://github.com/{REPO}/releases/tag/v{latest}{reset}");
    }
    for line in notes {
        println!("  {line}");
    }
    println!();

    if args.check {
        println!("{yellow}Run `marsad update` to install v{latest}.{reset}");
        return Ok(());
    }

    println!("Installing v{latest}…");
    let exe = install(&latest, false)?;
    println!();
    println!("{green}✓ Updated v{CURRENT} → v{latest}{reset}  {dim}{}{reset}", exe.display());
    println!("{dim}Restart any open marsad dashboards to use the new version.{reset}");
    println!("{}", auto_hint(state.auto));
    Ok(())
}

/// What the dashboard shows in its footer.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    Available(String),
    Installed(String),
}

/// Starts a background check for the dashboard (at most once a day; otherwise the last result is
/// reused). With automatic updates on, a newer version is installed silently. Set
/// `MARSAD_NO_UPDATE_CHECK` to skip all of it.
pub fn background() -> Receiver<Notice> {
    let (tx, rx) = mpsc::channel();
    if std::env::var_os("MARSAD_NO_UPDATE_CHECK").is_some() {
        return rx;
    }
    std::thread::spawn(move || {
        let mut state = load_state();
        let mut latest = state.latest.clone();
        if crate::now() - state.checked_at >= CHECK_EVERY_SECS {
            // Record the attempt even if offline, so we don't retry on every launch.
            state.checked_at = crate::now();
            let _ = save_state(&state);
            if let Ok(releases) = check_now(&mut state) {
                latest = releases.first().map(|r| r.version.clone());
            }
        }
        let Some(latest) = latest.filter(|v| is_newer(v, CURRENT)) else { return };
        let auto_ok = state.auto && current_exe().ok().and_then(|e| install_root(&e)).is_some();
        let notice = if auto_ok && install(&latest, true).is_ok() {
            Notice::Installed(latest)
        } else {
            Notice::Available(latest)
        };
        let _ = tx.send(notice);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("v0.10.2"), Some((0, 10, 2)));
        assert_eq!(parse_version("1.2.3-beta.1"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.99.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn notes_stop_at_install_section() {
        let body = "## What's new\n\n- Add auto update\n* Fix sort\n\n## Install marsad 0.3.0\n\n```sh\ncurl …\n```";
        assert_eq!(release_notes(body), vec!["- Add auto update", "- Fix sort"]);
        // dist's body with no CHANGELOG section has no notes.
        assert!(release_notes(" ## Install marsad 0.2.0\n\n### Install prebuilt binaries").is_empty());
    }

    #[test]
    fn releases_skip_drafts_and_prereleases_and_sort_newest_first() {
        let json = json!([
            {"tag_name": "v0.2.0", "body": "- b\n## Install x", "draft": false, "prerelease": false},
            {"tag_name": "v0.10.0", "body": "- c", "draft": false, "prerelease": false},
            {"tag_name": "v0.11.0-rc.1", "body": "", "draft": false, "prerelease": true},
            {"tag_name": "v0.12.0", "body": "", "draft": true, "prerelease": false},
            {"tag_name": "nightly", "body": "", "draft": false, "prerelease": false},
        ]);
        let versions: Vec<String> = parse_releases(&json).into_iter().map(|r| r.version).collect();
        assert_eq!(versions, vec!["0.10.0", "0.2.0"]);
    }

    #[test]
    fn whats_new_is_capped() {
        let rel = |v: &str, n: usize| Release { version: v.into(), notes: (0..n).map(|i| format!("- {v} {i}")).collect() };
        let lines = whats_new(&[rel("99.1.0", 6), rel("99.0.0", 6), rel("0.0.1", 3)]);
        assert_eq!(lines[0], "v99.1.0");
        assert!(lines.len() <= MAX_NOTE_LINES + 1);
        assert_eq!(lines.last().unwrap(), "  … and 6 more");
        assert!(!lines.iter().any(|l| l.contains("0.0.1")));
        // A single newer release has no version header.
        assert_eq!(whats_new(&[rel("99.0.0", 1)]), vec!["  - 99.0.0 0"]);
    }

    #[test]
    fn install_root_requires_bin_dir() {
        assert_eq!(install_root(Path::new("/home/u/.cargo/bin/marsad")), Some(PathBuf::from("/home/u/.cargo")));
        assert_eq!(install_root(Path::new("/src/marsad/target/release/marsad")), None);
    }
}
