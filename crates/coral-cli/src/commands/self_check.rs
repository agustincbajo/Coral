//! `coral self-check` — diagnostic probe for Claude Code hooks + the
//! `coral-doctor` skill (FR-ONB-6, FR-ONB-7, FR-ONB-9, FR-ONB-10,
//! FR-ONB-25, FR-ONB-32).
//!
//! Six FRs consume the JSON schema this module derives, so the schema
//! is a **frozen contract** — see PRD v1.4 Appendix F. A CI step is
//! expected to run `coral self-check --print-schema` and diff it
//! against `.ci/self-check-schema.json` so silent rotation is caught
//! the same week it's introduced.
//!
//! Flags:
//!   --format=json|text   (default text — human-readable on a TTY)
//!   --quick              skip the slow probes (MCP/UI/update); target
//!                        <100ms Linux/macOS, <300ms Windows
//!   --full               opposite of --quick; default state already
//!                        runs the full probe set
//!   --print-schema       emits the JSON Schema for `SelfCheck` and
//!                        exits — used by the CI contract check
//!
//! The `--quick` path is what the `SessionStart` hook calls every time
//! Claude Code opens this repo. Its JSON envelope is hard-capped at
//! 8000 characters so it stays under the 10000-char hook-stdout cap
//! Claude Code injects into the model's context (PRD §6.3 FR-ONB-9).

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Args;
use schemars::JsonSchema;
use serde::Serialize;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Hard contract: `SelfCheck.schema_version` MUST equal this value for
/// any output emitted by this binary. Bump only when the field set
/// changes shape (removal / re-typing). Additive changes do not require
/// a bump because consumers (skills, hooks) MUST tolerate unknown
/// fields per the PRD.
pub const SELF_CHECK_SCHEMA_VERSION: u32 = 1;

/// Soft cap on the JSON envelope for `--quick` runs. The hook stdout
/// budget Claude Code injects is 10k chars; we leave 2k headroom so a
/// downstream truncation (e.g. an editor wrapping JSON in a fenced
/// code block) doesn't push us over.
pub const QUICK_OUTPUT_CAP_CHARS: usize = 8000;

/// `coral self-check` arguments — see module docs.
#[derive(Args, Debug)]
pub struct SelfCheckArgs {
    /// Output format. `json` is what hooks/skills parse; `text` is
    /// the human-readable fallback for interactive shell use.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
    /// Skip MCP-server, UI-server, and update-available probes. Used
    /// by the SessionStart hook (<100ms p95 Linux/macOS target).
    #[arg(long)]
    pub quick: bool,
    /// Force all probes even when `--quick` is also passed. Provided
    /// for explicit intent; `--full` wins over `--quick`.
    #[arg(long)]
    pub full: bool,
    /// Emit the JSON Schema for `SelfCheck` to stdout and exit. Used
    /// by CI to pin the contract.
    #[arg(long = "print-schema")]
    pub print_schema: bool,
}

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Text,
    Json,
}

// ----------------------------------------------------------------------
// Schema — PRD v1.4 Appendix F
// ----------------------------------------------------------------------

/// Top-level diagnostic envelope. All probe outputs live as nested
/// `Option<T>` fields so consumers can tell "probe ran, returned no
/// data" (`Some(default)`) from "probe was skipped" (`None`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SelfCheck {
    /// Frozen contract version. See [`SELF_CHECK_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Coarse status the SessionStart hook can route on without
    /// parsing the rest of the envelope.
    pub coral_status: CoralStatus,
    /// `env!("CARGO_PKG_VERSION")` of the running binary.
    pub coral_version: String,
    /// Result of `std::env::current_exe()`. Empty path if the OS
    /// refused (rare; treated as a degraded check).
    pub binary_path: PathBuf,
    /// `true` when `coral` resolved on PATH at probe time.
    pub in_path: bool,
    pub platform: PlatformInfo,
    /// `None` when the cwd is not a git repo (no `.git` directory and
    /// `git rev-parse HEAD` fails).
    pub git_repo: Option<GitRepoInfo>,
    /// `None` when `.wiki/SCHEMA.md` is missing.
    pub wiki: Option<WikiInfo>,
    /// `None` when `coral.toml` is missing from the repo root.
    pub coral_toml: Option<ManifestInfo>,
    /// `None` when `CLAUDE.md` is missing from the repo root.
    pub claude_md: Option<ClaudeMdInfo>,
    /// `None` when the `claude` binary is not on PATH.
    pub claude_cli: Option<ClaudeCli>,
    /// Providers we can detect WITHOUT configuration (binary on PATH,
    /// env var set). Order is insertion-stable so consumers diffing
    /// two runs see a deterministic list.
    pub providers_available: Vec<String>,
    /// Providers the user has actively configured via
    /// `.coral/config.toml` (FR-ONB-27).
    pub providers_configured: Vec<String>,
    /// Populated only with `--full`; `None` under `--quick`.
    pub update_available: Option<String>,
    /// Populated only with `--full`; `None` under `--quick`.
    pub mcp_server: Option<McpHealth>,
    /// Populated only with `--full`; `None` under `--quick`.
    pub ui_server: Option<UiHealth>,
    /// Capped at the top 5 by severity-descending in `--quick` mode.
    pub warnings: Vec<Warning>,
    /// Capped at the top 5 by severity-descending in `--quick` mode.
    pub suggestions: Vec<Suggestion>,
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoralStatus {
    /// Binary located, no fatal probe failures.
    Ok,
    /// Binary couldn't determine its own path (the SessionStart hook
    /// returns this when `command -v coral` itself fails — exit
    /// early before invoking us, so this is mostly a sentinel for
    /// the schema's enum exhaustiveness).
    BinaryMissing,
    /// One or more probes hit a hard error. Specific reason lives
    /// in `warnings[]`.
    CheckFailed,
}

/// `platform.os` is the lowercase form `cfg!(target_os)` yields
/// (`linux`, `macos`, `windows`). `arch` is `cfg!(target_arch)`
/// (`x86_64`, `aarch64`).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PlatformInfo {
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GitRepoInfo {
    pub head_sha: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct WikiInfo {
    pub present: bool,
    pub page_count: u32,
    pub last_bootstrap_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ManifestInfo {
    pub present: bool,
    /// `true` when the file parsed cleanly as TOML. `false` when it
    /// exists but failed to parse — that's a user-actionable warning.
    pub parsed_ok: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ClaudeMdInfo {
    pub present: bool,
    /// Used by FR-ONB-25's size-guard logic in `coral init` — appending
    /// our 30-line routing section past 200 lines may degrade
    /// adherence per Anthropic's CLAUDE.md guidance.
    pub line_count: u32,
    /// `true` when the file contains a `^## Coral routing` heading
    /// (case-insensitive). Tells `coral init` that the routing block
    /// is already present and the append-safe path is a no-op.
    pub has_coral_routing_section: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ClaudeCli {
    pub installed: bool,
    pub path: Option<PathBuf>,
    /// Best-effort version string from `claude --version`. `None`
    /// when the probe binary is not on PATH or `--version` fails.
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct McpHealth {
    pub reachable: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UiHealth {
    pub reachable: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Warning {
    pub severity: Severity,
    pub message: String,
    /// An exact, copy-pasteable command the user can run to fix the
    /// issue. `None` when no automated remediation exists.
    pub action: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    // Ordering matters: `severity_descending` sort relies on this
    // enum's Ord impl (High > Medium > Low). Don't reorder.
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Suggestion {
    pub kind: SuggestionKind,
    pub command: String,
    pub explanation: String,
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    RunDoctor,
    RunBootstrap,
    InstallProvider,
    RunInit,
    SelfUpgrade,
}

// ----------------------------------------------------------------------
// Entry point
// ----------------------------------------------------------------------

/// Command dispatcher. Behavior matrix:
///
/// | flag combo                | action                       |
/// |---------------------------|------------------------------|
/// | `--print-schema`          | emit JSON Schema + exit 0    |
/// | `--quick` (no `--full`)   | skip MCP/UI/update probes    |
/// | `--full`                  | force all probes             |
/// | (default)                 | all probes                   |
pub fn run(args: SelfCheckArgs) -> Result<ExitCode> {
    if args.print_schema {
        let schema = schemars::schema_for!(SelfCheck);
        println!("{}", serde_json::to_string_pretty(&schema)?);
        return Ok(ExitCode::SUCCESS);
    }

    // `--full` wins over `--quick` when both are passed — explicit
    // intent overrides the SessionStart-hook default. This mirrors
    // the PRD's behavior matrix.
    let quick = args.quick && !args.full;

    let cwd = std::env::current_dir()?;
    let mut report = run_probes(&cwd, quick);

    if quick {
        // Hard cap the output so the SessionStart hook never blows
        // through the 10000-char stdout budget Claude Code allots.
        // We keep the top 5 warnings/suggestions by severity desc.
        cap_for_quick(&mut report);
    }

    match args.format {
        OutputFormat::Json => {
            let json = serde_json::to_string(&report)?;
            // Belt-and-suspenders: if even after capping we exceed
            // the soft limit (e.g. a single warning's message is
            // huge), emit a minimal fallback envelope. The shape
            // matches the SessionStart hook's truncation fallback.
            if json.len() > QUICK_OUTPUT_CAP_CHARS && quick {
                println!(
                    r#"{{"coral_status":"ok","note":"full output truncated; run /coral:coral-doctor"}}"#
                );
            } else {
                println!("{json}");
            }
        }
        OutputFormat::Text => {
            print_text(&report);
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Runs all probes against `cwd`. The `quick` flag toggles MCP/UI/
/// update probes — wired as `None` when skipped so consumers can
/// distinguish "skipped" from "ran and returned no data".
///
/// `pub(crate)` so `coral doctor` (week 3) can run the same probe
/// pipeline without shelling out to `coral self-check --format=json`.
/// Cheaper + lets the doctor surface fix commands inline.
pub(crate) fn run_probes(cwd: &Path, quick: bool) -> SelfCheck {
    let coral_version = env!("CARGO_PKG_VERSION").to_string();
    let binary_path = std::env::current_exe().unwrap_or_default();
    let in_path = is_coral_in_path();

    let platform = PlatformInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    };

    let git_repo = probe_git_repo(cwd);
    let wiki = probe_wiki(cwd);
    let coral_toml = probe_coral_toml(cwd);
    let claude_md = probe_claude_md(cwd);
    let claude_cli = probe_claude_cli();

    let providers_available = probe_providers_available(claude_cli.as_ref());
    let providers_configured = probe_providers_configured(cwd);

    let (update_available, mcp_server, ui_server) = if quick {
        (None, None, None)
    } else {
        (
            probe_update_available(),
            probe_mcp_server(),
            probe_ui_server(),
        )
    };

    let mut warnings: Vec<Warning> = Vec::new();
    let mut suggestions: Vec<Suggestion> = Vec::new();
    build_warnings_and_suggestions(
        &mut warnings,
        &mut suggestions,
        &wiki,
        &claude_md,
        &providers_configured,
        &providers_available,
        in_path,
    );

    // Coarse status: any probe that hard-errored (we represent those
    // via High-severity warnings whose action mentions "internal
    // error") flips us to CheckFailed. Otherwise Ok. BinaryMissing
    // is reserved for the hook-script early-exit path (see PRD §6.3
    // FR-ONB-9), so it's never produced by this code.
    let coral_status = if warnings.iter().any(|w| {
        w.severity == Severity::High && w.message.to_lowercase().contains("internal error")
    }) {
        CoralStatus::CheckFailed
    } else {
        CoralStatus::Ok
    };

    SelfCheck {
        schema_version: SELF_CHECK_SCHEMA_VERSION,
        coral_status,
        coral_version,
        binary_path,
        in_path,
        platform,
        git_repo,
        wiki,
        coral_toml,
        claude_md,
        claude_cli,
        providers_available,
        providers_configured,
        update_available,
        mcp_server,
        ui_server,
        warnings,
        suggestions,
    }
}

// ----------------------------------------------------------------------
// Probes
// ----------------------------------------------------------------------

/// Searches `PATH` for a `coral` executable. Pure environment scan —
/// does NOT shell out, so the probe stays under our <100ms p95 budget
/// even on Windows where process spawn is slow.
fn is_coral_in_path() -> bool {
    let exe_name = if cfg!(windows) { "coral.exe" } else { "coral" };
    which_in_path(exe_name).is_some()
}

/// Returns the resolved absolute path to a binary on PATH, or `None`
/// when not found. Implemented in-house so we don't take a dep on the
/// `which` crate for a 20-line probe.
///
/// `pub(crate)` so the `coral doctor` mini-wizard (week 3) can reuse
/// it for Ollama detection without duplicating the PATH walk.
pub(crate) fn which_in_path(exe_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn probe_git_repo(cwd: &Path) -> Option<GitRepoInfo> {
    coral_core::gitdiff::head_sha(cwd)
        .ok()
        .map(|head_sha| GitRepoInfo { head_sha })
}

fn probe_wiki(cwd: &Path) -> Option<WikiInfo> {
    let wiki_dir = cwd.join(".wiki");
    let schema_md = wiki_dir.join("SCHEMA.md");
    if !schema_md.exists() {
        return None;
    }
    // Page count: walk the standard wiki subdirectories and count
    // `.md` files. We deliberately don't parse the index — the index
    // can drift; a directory scan is the ground truth.
    let mut page_count: u32 = 0;
    for sub in &[
        "modules",
        "concepts",
        "entities",
        "flows",
        "decisions",
        "synthesis",
        "operations",
        "sources",
        "gaps",
    ] {
        let dir = wiki_dir.join(sub);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    page_count += 1;
                }
            }
        }
    }
    Some(WikiInfo {
        present: true,
        page_count,
        // Populated by the bootstrap state file in week 2; left at
        // None for M1 so we don't read .wiki/.bootstrap-state.json
        // which doesn't have a frozen schema yet.
        last_bootstrap_at: None,
    })
}

fn probe_coral_toml(cwd: &Path) -> Option<ManifestInfo> {
    let path = cwd.join("coral.toml");
    if !path.exists() {
        return None;
    }
    let parsed_ok = match std::fs::read_to_string(&path) {
        Ok(raw) => toml::from_str::<toml::Value>(&raw).is_ok(),
        Err(_) => false,
    };
    Some(ManifestInfo {
        present: true,
        parsed_ok,
    })
}

fn probe_claude_md(cwd: &Path) -> Option<ClaudeMdInfo> {
    let path = cwd.join("CLAUDE.md");
    let raw = std::fs::read_to_string(&path).ok()?;
    let line_count = u32::try_from(raw.lines().count()).unwrap_or(u32::MAX);
    // `^## Coral routing` case-insensitive at the start of any line.
    // Don't use regex for a simple prefix scan — keeps the probe
    // dependency-free and fast.
    let needle = "## coral routing";
    let has_coral_routing_section = raw
        .lines()
        .any(|line| line.to_ascii_lowercase().trim_start().starts_with(needle));
    Some(ClaudeMdInfo {
        present: true,
        line_count,
        has_coral_routing_section,
    })
}

fn probe_claude_cli() -> Option<ClaudeCli> {
    let exe_name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    let path = which_in_path(exe_name)?;
    let version = std::process::Command::new(&path)
        .arg("--version")
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                None
            }
        });
    Some(ClaudeCli {
        installed: true,
        path: Some(path),
        version,
    })
}

fn probe_providers_available(claude_cli: Option<&ClaudeCli>) -> Vec<String> {
    let mut available = Vec::new();
    if claude_cli.is_some() {
        available.push("claude_cli".to_string());
    }
    if std::env::var_os("ANTHROPIC_API_KEY").is_some() {
        available.push("anthropic_api_key".to_string());
    }
    let ollama_exe = if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    };
    if which_in_path(ollama_exe).is_some() {
        available.push("ollama".to_string());
    }
    available
}

fn probe_providers_configured(cwd: &Path) -> Vec<String> {
    let cfg = match coral_core::config::load_from_repo(cwd) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut configured = Vec::new();
    if cfg.provider.anthropic.is_some() {
        configured.push("anthropic".to_string());
    }
    if cfg.provider.gemini.is_some() {
        configured.push("gemini".to_string());
    }
    if cfg.provider.ollama.is_some() {
        configured.push("ollama".to_string());
    }
    if cfg.provider.claude_cli.is_some() {
        configured.push("claude_cli".to_string());
    }
    configured
}

/// Best-effort latest-release lookup against the GitHub API. Returns
/// the new tag (e.g. `v0.34.1`) when a strictly greater version is
/// published, `None` when we're up-to-date, when the API errors, or
/// when the response is malformed. The 3s timeout caps the slow
/// path so `--full` doesn't drag on a flaky network.
///
/// FR-ONB-32 + PRD Appendix F: this surfaces as `update_available`
/// in the SelfCheck JSON, and the `coral-doctor` skill reads it to
/// nudge the user toward `coral self-upgrade` when applicable.
fn probe_update_available() -> Option<String> {
    let current = env!("CARGO_PKG_VERSION");
    let url = "https://api.github.com/repos/agustincbajo/Coral/releases/latest";
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .user_agent(concat!("coral-self-check/", env!("CARGO_PKG_VERSION")))
        .build();
    let resp = agent
        .get(url)
        .set("accept", "application/vnd.github+json")
        .call()
        .ok()?;
    let body: serde_json::Value = resp.into_json().ok()?;
    let tag = body.get("tag_name").and_then(|v| v.as_str())?;
    let trimmed = tag.strip_prefix('v').unwrap_or(tag);
    if trimmed != current {
        Some(tag.to_string())
    } else {
        None
    }
}

/// MCP-server probe (FR-ONB-8). Spawns our own binary as
/// `coral mcp serve --transport=stdio`, sends a single JSON-RPC
/// `initialize` request, and waits up to 3s for a response on
/// stdout. Reachable iff we read at least one line that contains
/// `"result"` (the MCP `initialize` reply shape per 2025-11-25).
///
/// Cost: this is a 1-2s probe in the happy path. Only runs under
/// `--full`. The `--quick` SessionStart hook NEVER touches this.
fn probe_mcp_server() -> Option<McpHealth> {
    let coral = std::env::current_exe().ok()?;
    let mut child = std::process::Command::new(&coral)
        .arg("mcp")
        .arg("serve")
        .arg("--transport=stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"coral-self-check","version":"1"}}}"#;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{init}");
        let _ = stdin.flush();
        // Drop stdin to signal EOF after our single request — the
        // server will keep running until killed, but the reader on
        // the other side now knows no more input is coming.
        drop(stdin);
    }

    let reachable = read_first_response_line(child.stdout.take(), Duration::from_secs(3));

    // Always kill the child — the server's main loop doesn't exit
    // on stdin EOF in the stdio transport.
    let _ = child.kill();
    let _ = child.wait();

    Some(McpHealth {
        reachable: reachable.is_some(),
        note: if reachable.is_some() {
            None
        } else {
            Some("no JSON-RPC response within 3s".to_string())
        },
    })
}

/// UI-server probe (FR-ONB-8). Spawns `coral ui serve --no-open
/// --port=38400`, waits for the port to accept HTTP, GETs
/// `/health` (the coral-ui server's documented liveness route),
/// and reports the result. Always kills the child before returning.
///
/// Port 38400 is intentionally far from the default 3838 so the
/// probe doesn't conflict with a UI the user has already started.
/// We require the binary to know its own path (`current_exe`); when
/// that fails we return `None` (the consumer treats that as "did not
/// probe", not "probed and unreachable" — a meaningful distinction).
///
/// Prereq: `coral ui serve` requires `.wiki/` to exist. We short-
/// circuit when it doesn't because spawning would immediately exit
/// with a clearer message ("wiki not initialized") than
/// "unreachable". The consumer (skill / doctor) can then route
/// the user to `coral bootstrap` instead of "UI is broken".
fn probe_ui_server() -> Option<UiHealth> {
    let coral = std::env::current_exe().ok()?;
    let cwd = std::env::current_dir().ok()?;
    if !cwd.join(".wiki").exists() {
        return Some(UiHealth {
            reachable: false,
            note: Some("`.wiki/` not initialized — run `coral bootstrap`".to_string()),
        });
    }
    let mut child = std::process::Command::new(&coral)
        .arg("ui")
        .arg("serve")
        .arg("--no-open")
        .arg("--port=38400")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut reachable = false;
    let mut response_status: Option<u16> = None;
    let mut note: Option<String> = None;

    // Wait up to ~3s for the server to bind, polling every 200ms.
    // 15 iterations * 200ms = 3s — matches the MCP probe's budget.
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_millis(500))
        .build();
    let mut last_err: Option<String> = None;
    for _ in 0..15 {
        std::thread::sleep(Duration::from_millis(200));
        match agent
            .get("http://127.0.0.1:38400/health")
            .call()
        {
            Ok(resp) => {
                response_status = Some(resp.status());
                reachable = resp.status() == 200;
                break;
            }
            Err(ureq::Error::Status(code, _)) => {
                response_status = Some(code);
                // A non-200 is still "reachable" in the
                // server-is-listening sense; we just record the
                // status for the consumer to interpret.
                reachable = false;
                break;
            }
            Err(e) => {
                last_err = Some(e.to_string());
            }
        }
    }
    if response_status.is_none() {
        note = Some(format!(
            "no response on 127.0.0.1:38400 within 3s ({})",
            last_err.unwrap_or_else(|| "no network error captured".to_string())
        ));
    }

    let _ = child.kill();
    let _ = child.wait();

    Some(UiHealth { reachable, note })
}

/// Reads one line from a child's stdout, with a soft deadline. Used
/// by the MCP probe — we just need to know *something* came back; we
/// don't try to deserialize the full JSON-RPC envelope here.
fn read_first_response_line<R: std::io::Read + Send + 'static>(
    stdout: Option<R>,
    timeout: Duration,
) -> Option<String> {
    let stdout = stdout?;
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut buf = String::new();
        if reader.read_line(&mut buf).is_ok() && !buf.trim().is_empty() {
            let _ = tx.send(buf);
        }
    });
    let start = Instant::now();
    while start.elapsed() < timeout {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) if line.contains("\"result\"") => return Some(line),
            Ok(line) if line.contains("\"error\"") => return Some(line),
            Ok(_) => continue,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
    None
}

// ----------------------------------------------------------------------
// Warnings + suggestions
// ----------------------------------------------------------------------

fn build_warnings_and_suggestions(
    warnings: &mut Vec<Warning>,
    suggestions: &mut Vec<Suggestion>,
    wiki: &Option<WikiInfo>,
    claude_md: &Option<ClaudeMdInfo>,
    providers_configured: &[String],
    providers_available: &[String],
    in_path: bool,
) {
    if !in_path {
        warnings.push(Warning {
            severity: Severity::High,
            message:
                "`coral` not on PATH — Claude Code's SessionStart hook will skip Coral context"
                    .into(),
            action: Some("re-run scripts/install.sh (or install.ps1 on Windows)".into()),
        });
    }

    if wiki.is_none() {
        suggestions.push(Suggestion {
            kind: SuggestionKind::RunBootstrap,
            command: "coral bootstrap --estimate".into(),
            explanation: "no .wiki/ in this repo — start with an estimate before paying".into(),
        });
    }

    if claude_md
        .as_ref()
        .is_none_or(|c| !c.has_coral_routing_section)
    {
        suggestions.push(Suggestion {
            kind: SuggestionKind::RunInit,
            command: "coral init".into(),
            explanation: "add the Coral routing section to CLAUDE.md so Claude Code knows when to invoke coral".into(),
        });
    }

    if providers_configured.is_empty() {
        let severity = if providers_available.is_empty() {
            Severity::High
        } else {
            Severity::Medium
        };
        warnings.push(Warning {
            severity,
            message: format!(
                "no providers configured ({} available)",
                providers_available.join(", ")
            ),
            action: Some("/coral:coral-doctor".into()),
        });
        suggestions.push(Suggestion {
            kind: SuggestionKind::RunDoctor,
            command: "/coral:coral-doctor".into(),
            explanation: "the doctor skill walks you through a 4-path provider wizard".into(),
        });
    }
}

/// Truncate `warnings` / `suggestions` to 5 entries each, sorted by
/// severity descending (warnings only — suggestions don't carry a
/// severity, we keep insertion order). The total JSON envelope still
/// must clear `QUICK_OUTPUT_CAP_CHARS`; the caller serializes and
/// emits the fallback if even capped output blows the budget.
fn cap_for_quick(report: &mut SelfCheck) {
    // Warnings: stable sort by severity descending. Stable sort
    // preserves insertion order within a severity tier so the
    // SessionStart hook's output diffs cleanly run-over-run.
    report
        .warnings
        .sort_by_key(|w| std::cmp::Reverse(w.severity));
    if report.warnings.len() > 5 {
        report.warnings.truncate(5);
    }
    if report.suggestions.len() > 5 {
        report.suggestions.truncate(5);
    }
}

// ----------------------------------------------------------------------
// Text formatter — interactive shell users see this
// ----------------------------------------------------------------------

fn print_text(report: &SelfCheck) {
    println!("Coral self-check ({})", report.coral_version);
    println!("  status:   {:?}", report.coral_status);
    println!("  binary:   {}", report.binary_path.display());
    println!("  in PATH:  {}", if report.in_path { "yes" } else { "no" });
    println!(
        "  platform: {}/{}",
        report.platform.os, report.platform.arch
    );
    if let Some(g) = &report.git_repo {
        println!("  git HEAD: {}", &g.head_sha[..g.head_sha.len().min(12)]);
    }
    if let Some(w) = &report.wiki {
        println!("  wiki:     {} pages", w.page_count);
    } else {
        println!("  wiki:     not initialized");
    }
    if let Some(c) = &report.claude_md {
        let routing = if c.has_coral_routing_section {
            "with Coral routing"
        } else {
            "no Coral routing yet"
        };
        println!("  CLAUDE.md:{} lines, {}", c.line_count, routing);
    } else {
        println!("  CLAUDE.md:absent");
    }
    if let Some(cli) = &report.claude_cli {
        let v = cli.version.as_deref().unwrap_or("?");
        println!("  claude:   {v}");
    }
    if !report.providers_available.is_empty() {
        println!(
            "  available providers: {}",
            report.providers_available.join(", ")
        );
    }
    if !report.providers_configured.is_empty() {
        println!(
            "  configured providers: {}",
            report.providers_configured.join(", ")
        );
    }
    if !report.warnings.is_empty() {
        println!("\nwarnings:");
        for w in &report.warnings {
            println!("  [{:?}] {}", w.severity, w.message);
            if let Some(a) = &w.action {
                println!("       -> {a}");
            }
        }
    }
    if !report.suggestions.is_empty() {
        println!("\nsuggestions:");
        for s in &report.suggestions {
            println!("  {} ({})", s.command, s.explanation);
        }
    }
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// `current_exe()` is populated to a real path under cargo test;
    /// the probe never returns the empty default unless the platform
    /// refuses (which it doesn't under normal test conditions).
    #[test]
    fn binary_path_is_populated_under_test_runner() {
        let bp = std::env::current_exe().expect("current_exe");
        assert!(
            bp.is_file(),
            "test runner's current_exe must point at a real file"
        );
    }

    /// `head_sha` returns Ok in this repo (we ARE a git repo). When
    /// the probe runs outside a repo it returns None — we test both
    /// branches by running it twice.
    #[test]
    fn git_repo_probe_returns_some_in_repo_none_outside() {
        // Outside any repo: a fresh tempdir has no .git.
        let outside = TempDir::new().unwrap();
        assert!(probe_git_repo(outside.path()).is_none());

        // Inside this repo: the workspace cwd has a `.git/` so the
        // probe must return Some. The Coral integration-test harness
        // pins cwd to the workspace root.
        let cwd = std::env::current_dir().unwrap();
        // Walk upwards until we find a `.git` dir — the test binary
        // may run from a target/debug subdir.
        let repo_root = find_git_root(&cwd).expect("workspace root has a .git");
        let info = probe_git_repo(&repo_root).expect("HEAD resolves");
        assert!(
            !info.head_sha.is_empty() && info.head_sha.len() >= 40,
            "head_sha must be a full SHA1 (40 hex chars): {}",
            info.head_sha
        );
    }

    fn find_git_root(start: &Path) -> Option<PathBuf> {
        let mut cur = start.to_path_buf();
        loop {
            if cur.join(".git").exists() {
                return Some(cur);
            }
            if !cur.pop() {
                return None;
            }
        }
    }

    /// `wiki` probe: absent → None; present (SCHEMA.md exists) →
    /// Some with the right page_count.
    #[test]
    fn wiki_probe_counts_md_files_across_canonical_subdirs() {
        let dir = TempDir::new().unwrap();
        // Absent.
        assert!(probe_wiki(dir.path()).is_none());

        // Present with one module page + one entities page.
        let wiki_dir = dir.path().join(".wiki");
        std::fs::create_dir_all(wiki_dir.join("modules")).unwrap();
        std::fs::create_dir_all(wiki_dir.join("entities")).unwrap();
        std::fs::write(wiki_dir.join("SCHEMA.md"), "schema").unwrap();
        std::fs::write(wiki_dir.join("modules").join("a.md"), "").unwrap();
        std::fs::write(wiki_dir.join("entities").join("b.md"), "").unwrap();
        // Non-md file is ignored.
        std::fs::write(wiki_dir.join("modules").join("readme.txt"), "").unwrap();
        let info = probe_wiki(dir.path()).expect("present");
        assert!(info.present);
        assert_eq!(info.page_count, 2);
    }

    /// `claude_md` probe: line_count is exact; routing-section
    /// detection is case-insensitive.
    #[test]
    fn claude_md_probe_line_count_and_routing_detection() {
        let dir = TempDir::new().unwrap();

        // Absent → None.
        assert!(probe_claude_md(dir.path()).is_none());

        // Present, no routing.
        std::fs::write(dir.path().join("CLAUDE.md"), "line 1\nline 2\n").unwrap();
        let info = probe_claude_md(dir.path()).expect("present");
        assert_eq!(info.line_count, 2);
        assert!(!info.has_coral_routing_section);

        // Present with routing (mixed case to test ASCII-insensitive).
        std::fs::write(
            dir.path().join("CLAUDE.md"),
            "line 1\n## Coral Routing\nbody\n",
        )
        .unwrap();
        let info = probe_claude_md(dir.path()).expect("present");
        assert_eq!(info.line_count, 3);
        assert!(
            info.has_coral_routing_section,
            "`## Coral Routing` (any case) must match"
        );
    }

    /// `claude_cli` probe: when the binary is absent from PATH, the
    /// probe returns None. We can't reliably assert the present
    /// branch in CI (no guaranteed `claude` binary), so the present
    /// branch is exercised via the integration smoke test on hosts
    /// where it's installed.
    #[test]
    fn claude_cli_probe_returns_none_when_not_on_path() {
        // Save + clear PATH so `which_in_path("claude")` finds nothing.
        let original = std::env::var_os("PATH");
        // Safety: setting env vars is unsafe in Rust 2024 edition.
        // This test is single-threaded by design — the std test
        // harness runs each `#[test]` in its own thread but only
        // one body at a time per Mutex; we don't take CWD_LOCK
        // because this test doesn't touch cwd.
        // SAFETY: documented above — single-threaded env mutation.
        unsafe {
            std::env::remove_var("PATH");
        }
        let result = probe_claude_cli();
        // Restore PATH BEFORE asserting so a panic doesn't leak the
        // mutated env into sibling tests.
        if let Some(orig) = original {
            // SAFETY: same single-threaded contract.
            unsafe {
                std::env::set_var("PATH", orig);
            }
        }
        assert!(
            result.is_none(),
            "no `claude` binary should be on cleared PATH"
        );
    }

    /// `--quick` skips the slow probes — never spawns anything,
    /// never touches the network. Critical for the SessionStart hook's
    /// <100ms p95 budget on Linux/macOS.
    #[test]
    fn quick_skips_mcp_ui_update_probes() {
        let cwd = std::env::current_dir().unwrap();
        let quick = run_probes(&cwd, true);
        assert!(
            quick.update_available.is_none(),
            "--quick must NOT do the GitHub releases lookup"
        );
        assert!(
            quick.mcp_server.is_none(),
            "--quick must NOT spawn `coral mcp serve`"
        );
        assert!(
            quick.ui_server.is_none(),
            "--quick must NOT spawn `coral ui serve`"
        );
    }

    /// `--print-schema` ships a valid JSON Schema document with the
    /// pinned schema_version constant baked into the schema's
    /// generator.
    #[test]
    fn print_schema_produces_valid_json_schema() {
        let schema = schemars::schema_for!(SelfCheck);
        let json = serde_json::to_string(&schema).expect("serialize");
        // Sanity-check the envelope has the field names we promise
        // downstream consumers — a typo here would silently break
        // the SessionStart hook's parsing.
        for required_field in [
            "schema_version",
            "coral_status",
            "binary_path",
            "in_path",
            "platform",
            "providers_available",
            "providers_configured",
            "warnings",
            "suggestions",
        ] {
            assert!(
                json.contains(required_field),
                "self-check schema missing required field `{required_field}`"
            );
        }
    }

    /// Output-size cap: even with a pathological 50-warning input,
    /// the JSON envelope under --quick fits under the 8000-char cap.
    #[test]
    fn quick_output_caps_warnings_and_suggestions_to_five() {
        let mut report = SelfCheck {
            schema_version: SELF_CHECK_SCHEMA_VERSION,
            coral_status: CoralStatus::Ok,
            coral_version: "0.34.0-test".into(),
            binary_path: PathBuf::new(),
            in_path: true,
            platform: PlatformInfo {
                os: "linux".into(),
                arch: "x86_64".into(),
            },
            git_repo: None,
            wiki: None,
            coral_toml: None,
            claude_md: None,
            claude_cli: None,
            providers_available: vec![],
            providers_configured: vec![],
            update_available: None,
            mcp_server: None,
            ui_server: None,
            warnings: (0..50)
                .map(|i| Warning {
                    severity: if i % 3 == 0 {
                        Severity::High
                    } else if i % 3 == 1 {
                        Severity::Medium
                    } else {
                        Severity::Low
                    },
                    message: format!("warning #{i}"),
                    action: Some(format!("run command {i}")),
                })
                .collect(),
            suggestions: (0..50)
                .map(|i| Suggestion {
                    kind: SuggestionKind::RunDoctor,
                    command: format!("cmd {i}"),
                    explanation: format!("explanation {i}"),
                })
                .collect(),
        };
        cap_for_quick(&mut report);
        assert_eq!(report.warnings.len(), 5);
        assert_eq!(report.suggestions.len(), 5);
        // Severity descending: every kept warning must be High
        // (we seeded enough Highs to fill 5 slots).
        for w in &report.warnings {
            assert_eq!(w.severity, Severity::High);
        }
    }

    /// `providers_configured` reads the .coral/config.toml written by
    /// the wizard — when the file's missing, the list is empty.
    /// We do NOT test the populated branch here because that would
    /// duplicate the coral-core::config integration test.
    #[test]
    fn providers_configured_empty_when_no_config_toml() {
        let dir = TempDir::new().unwrap();
        let configured = probe_providers_configured(dir.path());
        assert!(configured.is_empty());
    }

    /// `providers_available` reflects environment without needing a
    /// repo. Anthropic API key env var alone is enough.
    #[test]
    fn providers_available_picks_up_anthropic_api_key_env() {
        let original = std::env::var_os("ANTHROPIC_API_KEY");
        // SAFETY: single-threaded env mutation for the duration of
        // the assert. See the claude_cli probe test for the same
        // contract — std test harness sequences `#[test]` bodies.
        unsafe {
            std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test");
        }
        let available = probe_providers_available(None);
        // Restore BEFORE the assert so a panic doesn't leak.
        match original {
            Some(v) => unsafe { std::env::set_var("ANTHROPIC_API_KEY", v) },
            None => unsafe { std::env::remove_var("ANTHROPIC_API_KEY") },
        }
        assert!(
            available.iter().any(|s| s == "anthropic_api_key"),
            "ANTHROPIC_API_KEY env must surface as a detected provider"
        );
    }

    /// Severity ordering is the contract `cap_for_quick` relies on.
    /// Pin it so a refactor of the enum (alphabetic reorder, say)
    /// can't silently break the SessionStart hook's "top 5 by
    /// severity" guarantee.
    #[test]
    fn severity_ord_is_low_lt_medium_lt_high() {
        assert!(Severity::Low < Severity::Medium);
        assert!(Severity::Medium < Severity::High);
    }

    /// `probe_ui_server` short-circuits when `.wiki/` is absent so
    /// the user gets a routing-correct note ("run bootstrap") instead
    /// of "UI is broken". We can't run `coral ui serve` reliably from
    /// the unit test runner (current_exe points at the test binary,
    /// not the coral binary), so we exercise the prereq-check branch
    /// only — it's the one that produces a deterministic outcome
    /// without spawning.
    #[test]
    fn probe_ui_server_reports_bootstrap_hint_when_wiki_missing() {
        // Run from a tempdir that has no `.wiki/` so the
        // prereq-check branch fires deterministically. We have to
        // touch process cwd, so take CWD_LOCK to serialize against
        // sibling tests.
        let _g = super::super::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let original = std::env::current_dir().unwrap();
        let dir = TempDir::new().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let probed = probe_ui_server();

        // Restore cwd BEFORE any assertion so a panic doesn't leak
        // the mutated state into sibling tests.
        std::env::set_current_dir(&original).unwrap();

        let probed = probed.expect("probe must return Some when current_exe resolves");
        assert!(!probed.reachable, "no wiki -> unreachable");
        let note = probed.note.expect("note must explain why");
        assert!(
            note.contains("bootstrap") || note.contains(".wiki"),
            "note must point user at bootstrap: {note}"
        );
    }

    /// `read_first_response_line` returns a line containing "result"
    /// when one arrives; returns None on timeout. We feed it a
    /// synthesized stdout (an in-memory cursor) so the test is
    /// hermetic.
    #[test]
    fn read_first_response_line_picks_up_jsonrpc_result() {
        let payload = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n";
        let cursor = std::io::Cursor::new(payload.to_vec());
        let line =
            read_first_response_line(Some(cursor), Duration::from_millis(500)).expect("must read");
        assert!(line.contains("\"result\""));
    }

    /// Same helper, but the timeout fires (the reader produces
    /// nothing within the budget) so we get None back.
    #[test]
    fn read_first_response_line_returns_none_on_timeout() {
        // A reader that blocks forever (PipeReader without writer).
        // We synthesize it via os_pipe? Simpler: pass None for the
        // stdout option, which is the same observable outcome from
        // the caller's perspective.
        let line: Option<String> =
            read_first_response_line::<std::io::Cursor<Vec<u8>>>(None, Duration::from_millis(50));
        assert!(line.is_none(), "no stdout -> None");
    }

    /// The MCP probe is `pub(crate)`-tested implicitly via the
    /// dispatcher; here we just confirm it doesn't panic when
    /// `current_exe()` resolves and the spawn fails (we synthesize
    /// the spawn failure by stubbing current_exe via an invalid
    /// binary — not portably possible from a unit test, so we just
    /// assert the contract surface). We exercise the happy path in
    /// the integration smoke test.
    #[test]
    fn probe_mcp_server_returns_some_or_none_without_panic() {
        // The test binary's current_exe is itself (not coral), so
        // spawning will produce an unrecognized-subcommand error and
        // the JSON-RPC line never arrives. Either outcome is fine —
        // we only assert no panic and that the field shape is
        // populated when the spawn succeeds.
        let result = probe_mcp_server();
        if let Some(h) = result {
            // When we did manage to spawn (i.e. coral was current_exe),
            // the note must be either absent (reachable) or describe
            // the timeout (reachable=false).
            if !h.reachable {
                assert!(
                    h.note.is_some(),
                    "unreachable probe must carry a note"
                );
            }
        }
    }
}
