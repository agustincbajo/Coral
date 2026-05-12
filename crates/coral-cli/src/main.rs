#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
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
    /// Deployment safety gate: aggregates lint + contracts into a
    /// single green/yellow/red verdict for CI. Usage:
    /// `coral guarantee --can-i-deploy [--strict] [--format json]`
    Guarantee(commands::guarantee::GuaranteeArgs),
    /// Auto-generate TestCases from OpenAPI specs in the project's
    /// repos (no LLM). Print, emit YAML, or `--commit` to disk.
    #[command(name = "test-discover")]
    TestDiscover(commands::test_discover::DiscoverArgs),
    /// MCP server (Model Context Protocol). Exposes wiki + manifest
    /// as resources/tools/prompts to coding agents.
    Mcp(commands::mcp::McpArgs),
    /// Manifest-driven instruction file emit (AGENTS.md, CLAUDE.md,
    /// .cursor/rules, .github/copilot-instructions.md, llms.txt).
    /// **Not LLM-driven** — deterministic templates from `coral.toml`.
    #[command(name = "export-agents")]
    ExportAgents(commands::export_agents::ExportAgentsArgs),
    /// Smart context loader: rank wiki pages by query, walk
    /// backlinks, fill under a token budget. Output ready to paste
    /// into any prompt.
    #[command(name = "context-build")]
    ContextBuild(commands::context_build::ContextBuildArgs),
    /// Cross-repo interface drift detection. Walks each repo's
    /// `openapi.{yaml,yml,json}` (provider) and `.coral/tests/*` (consumer),
    /// then for every `depends_on` edge reports unknown endpoints,
    /// unknown methods, and status-code drift.
    Contract(commands::contract::ContractArgs),
    /// **v0.20.0**: Capture + distill agent transcripts (Claude Code
    /// today; Cursor / ChatGPT tracked in #16). Five subcommands:
    /// `capture`, `list`, `forget`, `distill`, `show`. The
    /// distillation flow emits wiki pages with `reviewed: false`
    /// frontmatter — the `coral lint` trust-by-curation gate blocks
    /// any commit until a human flips it to `true`.
    Session(commands::session::SessionArgs),
    /// **v0.22.6**: Build / publish Coral as an Anthropic-Skills
    /// bundle. `build` produces a deterministic deflate-zip at
    /// `dist/coral-skill-<version>.zip` (or `--output PATH`)
    /// containing the agent personas, prompt templates, hooks, and
    /// an auto-generated `SKILL.md` manifest. `publish` is a stub
    /// pointing users at the Anthropic-Skills repo; the real
    /// fork+PR flow is deferred to v0.23+.
    Skill(SkillArgs),
    /// **v0.23.0**: Chaos-engineering against the running dev env
    /// via a Toxiproxy sidecar. `inject` adds latency / bandwidth /
    /// timeout / slicer / slow_close toxics to a `depends_on` edge,
    /// `clear` removes them, `list` reports active toxics, and
    /// `run <scenario>` dispatches a pre-canned scenario from
    /// `[[chaos_scenarios]]`. Requires `[environments.<env>.chaos]`
    /// in `coral.toml` and `coral up --env <name>` first.
    Chaos(commands::chaos::ChaosArgs),
    /// Generate CI workflow templates. Currently supports GitHub Actions.
    /// Usage: `coral ci generate [--stdout] [--output PATH]`
    Ci(commands::ci::CiArgs),
    /// **v0.23.1**: scheduled TestCase loops against a long-lived
    /// environment. `up` runs a foreground monitor (Ctrl-C exits
    /// cleanly), `list` enumerates declared monitors with best-effort
    /// running/stopped status, `history` tails the JSONL ledger, and
    /// `stop` is a v0.23.1 stub that points at Ctrl-C. Requires
    /// `[[environments.<env>.monitors]]` in `coral.toml`.
    Monitor(commands::monitor::MonitorArgs),
    /// View the wiki at a historical git ref (time-travel access).
    /// `coral wiki at <ref>` extracts `.wiki/` at that ref into a temp
    /// directory, reads pages, and outputs a summary. Optional flags
    /// let you search (`--search`) or filter (`--filter`) individual
    /// pages from that historical snapshot.
    Wiki(commands::wiki::WikiArgs),
    /// **v0.32.0**: REST API + embedded SPA via `coral ui serve`.
    /// Loopback-only and read-only by default; `--token` /
    /// `CORAL_UI_TOKEN` is required when binding off-loopback or
    /// when calling `/api/v1/query` (which spends LLM tokens).
    #[cfg(feature = "ui")]
    Ui(commands::ui::UiArgs),
    /// **v0.24 M2.3**: Watch `.wiki/` for changes to Interface-typed
    /// pages and emit structured notifications. Daemon command for
    /// real-time interface contract drift detection.
    Interface(commands::interface::InterfaceArgs),
    /// **v0.25 M3.6**: Draft migration PRs for consumer repos when a
    /// breaking change is introduced. Writes draft PR specs to
    /// `.coral/migrations/<timestamp>/` (opt-in, does not create
    /// actual GitHub PRs in this version).
    #[command(name = "migrate-consumers")]
    MigrateConsumers(commands::migrate::MigrateArgs),
    /// **v0.25 M3.6**: Wiki-driven scaffolding. Reads an existing
    /// wiki page's structure and generates a new page with the same
    /// headings but placeholder content.
    Scaffold(commands::scaffold::ScaffoldArgs),
    /// **v0.34.0** (FR-ONB-6): diagnostic probe for Claude Code's
    /// SessionStart hook + the `coral-doctor` skill. Emits JSON (for
    /// hooks) or human-readable text. `--quick` skips slow probes
    /// (target: <100ms p95 Linux/macOS for the hook). The pinned
    /// JSON Schema is the contract; `--print-schema` emits it.
    #[command(name = "self-check")]
    SelfCheck(commands::self_check::SelfCheckArgs),
    /// **v0.34.0** (FR-ONB-26): patch `.claude/settings.json` so the
    /// Coral marketplace is auto-registered in Claude Code without
    /// requiring the 3-line paste flow. Called by `install.sh
    /// --with-claude-config`. Backs up + atomic-writes.
    #[command(name = "self-register-marketplace")]
    SelfRegisterMarketplace(commands::self_register_marketplace::RegisterMarketplaceArgs),
    /// **v0.34.0** (FR-ONB-33): clean removal of the binary + state
    /// dir. Refuses non-interactive runs without `--yes`. NEVER
    /// touches `.wiki/` of any repo. Reminds the user to also run
    /// `/plugin uninstall coral@coral` in Claude Code.
    #[command(name = "self-uninstall")]
    SelfUninstall(commands::self_uninstall::SelfUninstallArgs),
    /// **v0.34.0**: `coral self <subcommand>` parent group, kept
    /// in sync with the top-level hyphenated forms above so install
    /// scripts and skills can use either spelling (PRD §6.1 +
    /// FR-ONB-26 install.sh caller spells it `coral self register-
    /// marketplace`). Aliases route to the same module entry points.
    #[command(name = "self")]
    SelfCmd(SelfArgs),
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

/// `coral skill <build|publish>` argument shell.
///
/// We use a nested `Subcommand` (rather than two top-level
/// commands `coral skill-build` / `coral skill-publish`) to keep
/// the CLI surface forward-compatible: v0.23+ will add `coral
/// skill list`, `coral skill verify`, etc., once the
/// Anthropic-Skills publish flow lands.
#[derive(Args, Debug)]
struct SkillArgs {
    #[command(subcommand)]
    command: SkillCmd,
}

/// `coral self <subcommand>` parent. Mirrors the top-level
/// `coral self-check` / `coral self-register-marketplace` /
/// `coral self-uninstall` forms so users (and the install scripts)
/// can spell either `coral self check` or `coral self-check`. The
/// PRD §6.1 install.sh caller uses the space-separated form for
/// `register-marketplace`; the hyphenated top-level is here for
/// hook contexts where shell quoting is finicky.
#[derive(Args, Debug)]
struct SelfArgs {
    #[command(subcommand)]
    command: SelfCmd,
}

#[derive(Subcommand, Debug)]
enum SelfCmd {
    /// Mirror of top-level `coral self-check`.
    Check(commands::self_check::SelfCheckArgs),
    /// Mirror of top-level `coral self-register-marketplace`.
    #[command(name = "register-marketplace")]
    RegisterMarketplace(commands::self_register_marketplace::RegisterMarketplaceArgs),
    /// Mirror of top-level `coral self-uninstall`.
    Uninstall(commands::self_uninstall::SelfUninstallArgs),
}

#[derive(Subcommand, Debug)]
enum SkillCmd {
    /// Build the skill bundle zip from `template/`.
    Build {
        /// Override the default output path
        /// (`dist/coral-skill-<version>.zip` relative to cwd).
        /// When provided, the `dist/` directory is NOT created —
        /// the caller picked a different location on purpose.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Publish stub. v0.22.6 only prints a deferred-message;
    /// real implementation lands in v0.23+.
    Publish,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    setup_tracing(cli.quiet, cli.verbose);

    let result: Result<ExitCode> = match cli.command {
        Cmd::Init(args) => commands::init::run(args, cli.wiki_root.as_deref()),
        Cmd::Bootstrap(args) => commands::bootstrap::run(args, cli.wiki_root.as_deref()),
        Cmd::Ingest(args) => commands::ingest::run(args, cli.wiki_root.as_deref()),
        Cmd::Query(args) => commands::query::run(args, cli.wiki_root.as_deref()),
        // v0.30.0 audit cycle 5 B2: `lint` / `verify` / `contract check`
        // adopt the documented exit-code contract (0 clean / 1 findings /
        // 2 usage / 3 internal). The command itself returns
        // `Ok(ExitCode::from(1))` for findings and propagates `Err` only
        // for internal failures; `map_b2_internal_err` rewrites that
        // `Err` into `Ok(ExitCode::from(3))` at the dispatch boundary so
        // CI can distinguish "real lint findings" from "lint crashed".
        Cmd::Lint(args) => map_b2_internal_err(commands::lint::run(args, cli.wiki_root.as_deref())),
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
        // v0.30.0 audit cycle 5 B2: see `Cmd::Lint` above for the
        // exit-code contract this wrapper implements.
        Cmd::Verify(args) => {
            map_b2_internal_err(commands::verify::run(args, cli.wiki_root.as_deref()))
        }
        Cmd::Test(args) => commands::test::run(args, cli.wiki_root.as_deref()),
        Cmd::Guarantee(args) => commands::guarantee::run(args, cli.wiki_root.as_deref()),
        Cmd::TestDiscover(args) => commands::test_discover::run(args, cli.wiki_root.as_deref()),
        Cmd::Mcp(args) => commands::mcp::run(args, cli.wiki_root.as_deref()),
        Cmd::ExportAgents(args) => commands::export_agents::run(args, cli.wiki_root.as_deref()),
        Cmd::ContextBuild(args) => commands::context_build::run(args, cli.wiki_root.as_deref()),
        // v0.30.0 audit cycle 5 B2: see `Cmd::Lint` above for the
        // exit-code contract this wrapper implements.
        Cmd::Contract(args) => {
            map_b2_internal_err(commands::contract::run(args, cli.wiki_root.as_deref()))
        }
        Cmd::Session(args) => commands::session::run(args, cli.wiki_root.as_deref()),
        Cmd::Skill(args) => match args.command {
            SkillCmd::Build { output } => commands::skill::build(output),
            SkillCmd::Publish => commands::skill::publish(),
        },
        Cmd::Chaos(args) => commands::chaos::run(args, cli.wiki_root.as_deref()),
        Cmd::Ci(args) => commands::ci::run(args),
        Cmd::Monitor(args) => commands::monitor::run(args, cli.wiki_root.as_deref()),
        Cmd::Wiki(args) => commands::wiki::run(args, cli.wiki_root.as_deref()),
        #[cfg(feature = "ui")]
        Cmd::Ui(args) => commands::ui::run(args, cli.wiki_root.as_deref()),
        Cmd::Interface(args) => commands::interface::run(args, cli.wiki_root.as_deref()),
        Cmd::MigrateConsumers(args) => commands::migrate::run(args, cli.wiki_root.as_deref()),
        Cmd::Scaffold(args) => commands::scaffold::run(args, cli.wiki_root.as_deref()),
        Cmd::SelfCheck(args) => commands::self_check::run(args),
        Cmd::SelfRegisterMarketplace(args) => commands::self_register_marketplace::run(args),
        Cmd::SelfUninstall(args) => commands::self_uninstall::run(args),
        Cmd::SelfCmd(args) => match args.command {
            SelfCmd::Check(a) => commands::self_check::run(a),
            SelfCmd::RegisterMarketplace(a) => commands::self_register_marketplace::run(a),
            SelfCmd::Uninstall(a) => commands::self_uninstall::run(a),
        },
        Cmd::TestLockIncr { path } => run_test_lock_incr(&path),
    };

    // v0.30.0 audit cycle 5 B2: for `lint`, `verify`, and `contract check`
    // we apply the documented exit-code contract — internal failures map
    // to ExitCode 3, NOT 1, so callers (CI especially) can distinguish a
    // backend-down crash from real findings. We detect "did this Result
    // come from one of those commands" by stashing a marker on the
    // command variant; here we just use the simpler approach of having
    // the routes themselves opt in via `map_b2_internal_err` below.
    match result {
        Ok(exit_code) => exit_code,
        Err(err) => {
            eprintln!("error: {err:#}");
            // Note: `lint`/`verify`/`contract` already map their internal
            // errors to `Ok(ExitCode::from(3))` before reaching here (see
            // the `map_b2_internal_err`-wrapped routes above). The
            // generic 1 below covers every other subcommand whose error
            // contract we haven't formalized yet.
            ExitCode::FAILURE
        }
    }
}

/// v0.30.0 audit cycle 5 B2: convert an `anyhow::Error` from a
/// contract-adopting command (`lint` / `verify` / `contract check`) into
/// an `Ok(ExitCode::from(3))`. The caller is expected to print the
/// error to stderr first so the user still sees what happened — we
/// only change the EXIT CODE, not the diagnostic output.
fn map_b2_internal_err(result: Result<ExitCode>) -> Result<ExitCode> {
    use commands::exit_codes::INTERNAL;
    match result {
        Ok(code) => Ok(code),
        Err(e) => {
            // Match the same `error: {err:#}` shape as the generic
            // dispatch tail so users see one consistent envelope.
            eprintln!("error: {e:#}");
            Ok(ExitCode::from(INTERNAL))
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
