mod alerts;
mod app;
mod context;
#[allow(dead_code)] mod coordination;
mod events;
mod hook;
mod install;
mod jsonl;
mod live;
mod model;
mod portable;
mod report;
mod settings;
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
    /// Show or set how many days of usage history are kept (default 30).
    Retention(report::RetentionArgs),
    /// Total tokens used since marsad started counting, including days no longer kept.
    Total(report::TotalArgs),
    /// Summarize token usage (by day, session, skill, project, model, agent).
    Report(report::ReportArgs),
    /// Run a read-only SQL query against the usage history.
    Sql(report::SqlArgs),
    /// Print the usage history database schema.
    Schema,
    /// Show live sessions: status, current activity, context left, and usage limits.
    Status(live::StatusArgs),
    /// Write all marsad data (day counter, history, settings) to one file, to back up or move it.
    Export {
        /// Output file (default: marsad-export-YYYY-MM-DD.json).
        file: Option<PathBuf>,
    },
    /// Merge a `marsad export` file into this machine's data; importing twice never double counts.
    Import {
        file: PathBuf,
        /// The file is this machine's own earlier data (a reinstall with the old transcripts still
        /// on disk): days its pre-0.4 totals already count are dropped from the day counter.
        #[arg(long)]
        restore: bool,
    },
    /// How full this session's context is, and the automatic handoff warning (--warn).
    Context(context::ContextArgs),
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
        Some(Cmd::Total(args)) => report::total(args),
        Some(Cmd::Report(args)) => report::report(args),
        Some(Cmd::Sql(args)) => report::sql(args),
        Some(Cmd::Schema) => {
            report::schema();
            Ok(())
        }
        Some(Cmd::Status(args)) => live::status(args),
        Some(Cmd::Context(args)) => context::run(args),
        Some(Cmd::Export { file }) => portable::export(file),
        Some(Cmd::Import { file, restore }) => portable::import(&file, restore),
        Some(Cmd::Update(args)) => update::run(args),
        None => app::run(),
    }
}
