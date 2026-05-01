use anyhow::Result;
use clap::{Parser, Subcommand};
use coral_cli::commands;
use std::path::PathBuf;
use std::process::ExitCode;

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
    /// First-time wiki compilation from HEAD (requires LLM).
    Bootstrap(commands::bootstrap::BootstrapArgs),
    /// Incremental ingest from last_commit (requires LLM).
    Ingest(commands::ingest::IngestArgs),
    /// Query the wiki (requires LLM).
    Query(commands::query::QueryArgs),
    /// Run lint on the wiki.
    Lint(commands::lint::LintArgs),
    /// Consolidate redundant pages (requires LLM).
    Consolidate(commands::consolidate::ConsolidateArgs),
    /// Print wiki health stats.
    Stats(commands::stats::StatsArgs),
    /// Sync subagents/scripts/templates from the embedded bundle.
    Sync(commands::sync::SyncArgs),
    /// Generate an onboarding reading path (requires LLM).
    Onboard(commands::onboard::OnboardArgs),
    /// Inspect prompt sources (local override, embedded, or fallback).
    Prompts(commands::prompts::PromptsArgs),
    /// TF-IDF search over the wiki (v0.2; v0.3 will switch to embeddings).
    Search(commands::search::SearchArgs),
    /// Export the wiki to Markdown bundle, JSON, Notion API bodies, or JSONL.
    Export(commands::export::ExportArgs),
    /// Push wiki pages to a Notion database (thin wrapper over `export --format notion-json` + curl).
    NotionPush(commands::notion_push::NotionPushArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    setup_tracing(cli.quiet, cli.verbose);

    let result: Result<ExitCode> = match cli.command {
        Cmd::Init(args) => commands::init::run(args, cli.wiki_root.as_deref()),
        Cmd::Bootstrap(args) => commands::bootstrap::run(args, cli.wiki_root.as_deref()),
        Cmd::Ingest(args) => commands::ingest::run(args, cli.wiki_root.as_deref()),
        Cmd::Query(args) => commands::query::run(args, cli.wiki_root.as_deref()),
        Cmd::Lint(args) => commands::lint::run(args, cli.wiki_root.as_deref()),
        Cmd::Consolidate(args) => commands::consolidate::run(args, cli.wiki_root.as_deref()),
        Cmd::Stats(args) => commands::stats::run(args, cli.wiki_root.as_deref()),
        Cmd::Sync(args) => commands::sync::run(args, cli.wiki_root.as_deref()),
        Cmd::Onboard(args) => commands::onboard::run(args, cli.wiki_root.as_deref()),
        Cmd::Prompts(args) => commands::prompts::run(args, cli.wiki_root.as_deref()),
        Cmd::Search(args) => commands::search::run(args, cli.wiki_root.as_deref()),
        Cmd::Export(args) => commands::export::run(args, cli.wiki_root.as_deref()),
        Cmd::NotionPush(args) => commands::notion_push::run(args, cli.wiki_root.as_deref()),
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
