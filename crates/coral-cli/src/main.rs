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
    /// Validate that every version in `.coral-pins.toml` exists as a tag in the remote Coral repo.
    ValidatePin(commands::validate_pin::ValidatePinArgs),
    /// Diff two wiki pages structurally (frontmatter, sources, wikilinks, body stats).
    Diff(commands::diff::DiffArgs),
    /// Daily-use wiki snapshot: last commit, lint summary, stats summary, recent log.
    Status(commands::status::StatusArgs),
    /// Show log entries that mention a slug (reverse chronological).
    History(commands::history::HistoryArgs),
    /// Multi-repo project commands (`new`, `list`, `add`, `doctor`, `lock`).
    /// v0.16: aggregated wiki, dependency graph, lockfile.
    Project(commands::project::ProjectArgs),
    /// Bring up the dev environment (compose backend in v0.17). Requires
    /// `[[environments]]` in `coral.toml`.
    Up(commands::up::UpArgs),
    /// Tear down the dev environment.
    Down(commands::down::DownArgs),
    /// Environment introspection (`status`, `logs`, `exec`).
    Env(commands::env::EnvArgs),
    /// Run liveness healthchecks against the running environment (<30s).
    Verify(commands::verify::VerifyArgs),
    /// Run functional tests (healthcheck + user-defined YAML, with
    /// markdown / JSON / JUnit output for CI).
    Test(commands::test::TestArgs),
    /// **Hidden** test-only helper: acquires `with_exclusive_lock(path)`,
    /// reads the file as a u64 counter, increments by 1, writes back.
    /// Used by `tests/cross_process_lock.rs` to verify the v0.15
    /// flock-based lock works at the OS-process boundary, not just
    /// thread-in-process. The leading underscore is convention for
    /// "internal, do not depend on this".
    #[command(name = "_test_lock_incr", hide = true)]
    TestLockIncr {
        /// Path to the counter file (must contain a valid u64).
        path: PathBuf,
    },
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
        Cmd::ValidatePin(args) => commands::validate_pin::run(args),
        Cmd::Diff(args) => commands::diff::run(args, cli.wiki_root.as_deref()),
        Cmd::Status(args) => commands::status::run(args, cli.wiki_root.as_deref()),
        Cmd::History(args) => commands::history::run(args, cli.wiki_root.as_deref()),
        Cmd::Project(args) => commands::project::run(args, cli.wiki_root.as_deref()),
        Cmd::Up(args) => commands::up::run(args, cli.wiki_root.as_deref()),
        Cmd::Down(args) => commands::down::run(args, cli.wiki_root.as_deref()),
        Cmd::Env(args) => commands::env::run(args, cli.wiki_root.as_deref()),
        Cmd::Verify(args) => commands::verify::run(args, cli.wiki_root.as_deref()),
        Cmd::Test(args) => commands::test::run(args, cli.wiki_root.as_deref()),
        Cmd::TestLockIncr { path } => run_test_lock_incr(&path),
    };

    match result {
        Ok(exit_code) => exit_code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// **Hidden test-only handler** for the `_test_lock_incr` subcommand.
/// Acquires `with_exclusive_lock(path)`, reads `path` as a u64 counter,
/// increments by 1, atomic-writes back. Returns Ok(SUCCESS) on success.
///
/// Lives in `main.rs` (not `commands/`) because it's deliberately not a
/// public/documented subcommand — it's wiring for the cross-process
/// lock test in `tests/cross_process_lock.rs`. Keeping it here makes
/// it impossible for external callers to depend on as a stable API.
fn run_test_lock_incr(path: &std::path::Path) -> Result<ExitCode> {
    use anyhow::Context as _;
    coral_core::atomic::with_exclusive_lock(path, || {
        let current: u64 = std::fs::read_to_string(path)
            .map_err(|e| coral_core::error::CoralError::Io {
                path: path.to_path_buf(),
                source: e,
            })?
            .trim()
            .parse()
            .map_err(|e: std::num::ParseIntError| {
                coral_core::error::CoralError::Walk(format!("parse counter: {e}"))
            })?;
        coral_core::atomic::atomic_write_string(path, &(current + 1).to_string())
    })
    .with_context(|| format!("locked counter increment on {}", path.display()))?;
    Ok(ExitCode::SUCCESS)
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
