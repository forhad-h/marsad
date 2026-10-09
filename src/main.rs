mod app;
mod events;
mod hook;
mod install;
mod jsonl;
mod model;
mod transcript;
mod ui;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// A live dashboard for every Claude Code session on this machine.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Hook entrypoint called by Claude Code (reads hook JSON on stdin).
    Hook,
    /// Register marsad hooks in ~/.claude/settings.json.
    Install,
    /// Remove marsad hooks from ~/.claude/settings.json.
    Uninstall,
}

/// Where marsad keeps its event log. Override with MARSAD_HOME.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MARSAD_HOME") {
        return PathBuf::from(dir);
    }
    claude_dir().join("marsad")
}

pub fn claude_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".claude")
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Some(Cmd::Hook) => {
            // Never fail or print: a hook must not disturb the session.
            let _ = hook::run();
            Ok(())
        }
        Some(Cmd::Install) => install::install(),
        Some(Cmd::Uninstall) => install::uninstall(),
        None => app::run(),
    }
}
