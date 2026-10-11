mod app;
mod events;
mod hook;
mod install;
mod jsonl;
mod live;
mod model;
mod report;
mod statusline;
mod store;
mod transcript;
mod ui;
mod update;
mod usage;

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
    /// Status line entrypoint called by Claude Code (reads status JSON on stdin).
    Statusline,
    /// Register marsad hooks (and status line) in ~/.claude/settings.json.
    Install {
        /// Don't install the status line; context size is then estimated from transcripts.
        #[arg(long)]
        no_statusline: bool,
    },
    /// Remove marsad hooks from ~/.claude/settings.json.
    Uninstall,
    /// Index new transcript data into the usage history.
    Sync,
    /// Show or set how long usage history is kept (default: forever).
    Retention(report::RetentionArgs),
    /// Summarize token usage (by day, session, skill, project, model, agent).
    Report(report::ReportArgs),
    /// Run a read-only SQL query against the usage history.
    Sql(report::SqlArgs),
    /// Print the usage history database schema.
    Schema,
    /// Show live sessions: status, current activity, context left, and usage limits.
    Status(live::StatusArgs),
    /// Update marsad to the latest release (or turn automatic updates on/off with --auto).
    Update(update::UpdateArgs),
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
        Some(Cmd::Statusline) => {
            let _ = statusline::run();
            Ok(())
        }
        Some(Cmd::Install { no_statusline }) => install::install(!no_statusline),
        Some(Cmd::Uninstall) => install::uninstall(),
        Some(Cmd::Sync) => report::sync_now(),
        Some(Cmd::Retention(args)) => report::retention(args),
        Some(Cmd::Report(args)) => report::report(args),
        Some(Cmd::Sql(args)) => report::sql(args),
        Some(Cmd::Schema) => {
            report::schema();
            Ok(())
        }
        Some(Cmd::Status(args)) => live::status(args),
        Some(Cmd::Update(args)) => update::run(args),
        None => app::run(),
    }
}
