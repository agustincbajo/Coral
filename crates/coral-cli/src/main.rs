use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

mod commands;

#[derive(Parser, Debug)]
#[command(
    name = "coral",
    version,
    about = "Karpathy-style LLM Wiki maintainer for Git repos."
)]
struct Cli {
    /// Override the wiki root (default: `.wiki/`).
    #[arg(long, global = true)]
    wiki_root: Option<PathBuf>,

    /// Suppress non-error output.
    #[arg(long, global = true)]
    quiet: bool,

    /// Verbose logs (sets RUST_LOG=coral=debug if not already set).
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Initialize a `.wiki/` in the current repo.
    Init(commands::init::InitArgs),
    /// First-time wiki compilation from HEAD (requires LLM, stub in v0.1).
    Bootstrap,
    /// Incremental ingest from last_commit (requires LLM, stub in v0.1).
    Ingest,
    /// Query the wiki (requires LLM, stub in v0.1).
    Query {
        /// Free-form question to ask the wiki.
        question: String,
    },
    /// Run lint on the wiki.
    Lint(commands::lint::LintArgs),
    /// Consolidate redundant pages (requires LLM, stub in v0.1).
    Consolidate,
    /// Print wiki health stats.
    Stats(commands::stats::StatsArgs),
    /// Sync subagents/scripts/templates from the embedded bundle.
    Sync(commands::sync::SyncArgs),
    /// Generate an onboarding reading path (requires LLM, stub in v0.1).
    Onboard,
    /// Semantic search over the wiki (Phase 4 — not yet implemented).
    Search {
        /// Search query.
        query: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    setup_tracing(cli.quiet, cli.verbose);

    let result = match cli.command {
        Cmd::Init(args) => commands::init::run(args, cli.wiki_root.as_deref()),
        Cmd::Bootstrap => stub("bootstrap"),
        Cmd::Ingest => stub("ingest"),
        Cmd::Query { .. } => stub("query"),
        Cmd::Lint(args) => commands::lint::run(args, cli.wiki_root.as_deref()),
        Cmd::Consolidate => stub("consolidate"),
        Cmd::Stats(args) => commands::stats::run(args, cli.wiki_root.as_deref()),
        Cmd::Sync(args) => commands::sync::run(args, cli.wiki_root.as_deref()),
        Cmd::Onboard => stub("onboard"),
        Cmd::Search { .. } => {
            eprintln!("`search` is not implemented in v0.1. Coming in v0.2.");
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(exit_code) => exit_code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn setup_tracing(quiet: bool, verbose: bool) {
    use tracing_subscriber::EnvFilter;
    let default = if quiet {
        "warn"
    } else if verbose {
        "coral=debug,info"
    } else {
        "info"
    };
    let filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new(default));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}

fn stub(name: &str) -> Result<ExitCode> {
    eprintln!("`{name}` requires the runner (Phase E2 wiring); not yet implemented in this build.");
    Ok(ExitCode::from(2))
}
