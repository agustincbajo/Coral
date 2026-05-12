//! `coral mcp serve [--transport stdio|http] [--port <p>] [--bind <addr>] [--read-only]`
//!
//! Exposes the wiki + manifest as a Model Context Protocol server.
//! v0.21.1+ ships **both** transports — stdio (the canonical one
//! every shipped MCP client speaks) and HTTP/SSE (Streamable HTTP per
//! MCP 2025-11-25). The HTTP transport binds `127.0.0.1` by default
//! and validates `Origin` against `null` / `http://localhost*` /
//! `http://127.0.0.1*` (DNS-rebinding mitigation). `--bind 0.0.0.0`
//! is opt-in and emits a stderr warning banner.
//!
//! v0.19.5 audit C1 wired the real `ToolDispatcher` and resource
//! `read()` paths — wave 1 had stub implementations that always
//! returned None, so MCP clients couldn't actually consume anything.

use anyhow::Result;
use clap::{Args, Subcommand};
use coral_core::{search, walk};
use coral_mcp::{
    McpHandler, PromptCatalog, ResourceProvider, ServerConfig, ToolCallResult, ToolCatalog,
    ToolDispatcher, Transport, WikiResourceProvider, server_card,
    state::shared_state,
    transport::HttpSseTransport,
    watcher::{WatcherConfig, start_watcher_with_state},
};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Args, Debug)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpCmd,
}

#[derive(Subcommand, Debug)]
pub enum McpCmd {
    /// Serve the MCP protocol over stdin/stdout.
    Serve(ServeArgs),
    /// Print the discoverable MCP server card as pretty-printed JSON to
    /// stdout and exit. Mirrors the body served at
    /// `GET /.well-known/mcp/server-card.json` on the HTTP transport
    /// (modulo the trailing newline `println!` adds).
    ///
    /// v0.22.5: registries and curious humans can hit either surface
    /// to learn what this Coral build advertises (capabilities, vendor,
    /// build provenance) before deciding to connect.
    Card,
    /// Preview what MCP clients would see (resources, tools, prompts)
    /// without starting the server. Useful for verifying the MCP surface.
    ///
    /// v0.24.3 M1.14: standalone introspection — no transport, no
    /// watcher, no IO loop. Helps devs confirm the advertised surface
    /// matches expectations before wiring up an IDE.
    Preview(PreviewArgs),
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Transport. v0.21.1+ supports `stdio` (default) and `http`.
    #[arg(long, default_value = "stdio")]
    pub transport: TransportArg,

    /// HTTP transport port. Required when `--transport http`; ignored
    /// for stdio. Default 3737 — picked to dodge the 3000-3100 React/
    /// Next dev-server cluster and the 8000-8100 Python clusters.
    #[arg(long)]
    pub port: Option<u16>,

    /// HTTP transport bind address. Defaults to `127.0.0.1` for safety;
    /// `--bind 0.0.0.0` is opt-in and emits a stderr warning banner
    /// (the MCP spec's DNS-rebinding mitigation only protects
    /// `Origin`-aware browser clients, so 0.0.0.0 + a permissive
    /// downstream proxy is still a footgun).
    #[arg(long)]
    pub bind: Option<IpAddr>,

    /// Default-deny: write-tools (`up`, `down`, `run_test`) are
    /// disabled unless `--allow-write-tools` is also passed.
    ///
    /// v0.19.5 audit H3: previous `ArgAction::SetTrue` made
    /// `--read-only false` an error. Switched to explicit
    /// `default_value_t = true` + plain bool so `--read-only false`
    /// parses (clap derives `clap::Action::Set` for bool by default).
    #[arg(long, default_value_t = true, num_args = 0..=1, default_missing_value = "true", action = clap::ArgAction::Set)]
    pub read_only: bool,

    /// Enable write tools. Mutually exclusive with `--read-only true`
    /// (clap's `default_value_t = true` means `--read-only false` is
    /// the explicit opt-out).
    #[arg(long)]
    pub allow_write_tools: bool,

    /// Surface `reviewed: false` distilled pages on `resources/list`
    /// and `resources/read`. Off by default — the MCP boundary
    /// mirrors the v0.20.1 pre-commit `unreviewed-distilled` lint
    /// gate, so attacker-influenced (via prompt injection through an
    /// original transcript) distilled content cannot reach a remote
    /// agent before a human reviewer flips `reviewed: true`.
    ///
    /// v0.20.2 audit-followup #37. Use only when debugging a distill
    /// flow where you intentionally want the un-vetted draft visible.
    #[arg(long, default_value_t = false)]
    pub include_unreviewed: bool,

    /// Watch the wiki directory for changes and push MCP notifications.
    /// Enabled by default; use `--no-watch` to disable.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub watch: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum TransportArg {
    /// One JSON-RPC envelope per line on stdin/stdout. The canonical
    /// MCP transport every shipped client speaks.
    Stdio,
    /// Streamable HTTP/SSE per MCP 2025-11-25. POST /mcp for JSON-RPC
    /// envelopes, GET /mcp for the SSE keep-alive stream, DELETE /mcp
    /// for explicit session teardown.
    Http,
}

#[derive(Args, Debug)]
pub struct PreviewArgs {
    /// Output format: human (default) or json.
    #[arg(long, default_value = "human")]
    pub format: PreviewFormat,

    /// Include unreviewed distilled pages in the resource list.
    #[arg(long, default_value_t = false)]
    pub include_unreviewed: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum PreviewFormat {
    Human,
    Json,
}

/// Default port for the HTTP transport when `--port` is omitted.
/// Picked to dodge the busy 3000-3100 React/Next and 8000-8100 Python
/// dev-server clusters most projects already run.
pub const DEFAULT_HTTP_PORT: u16 = 3737;

pub fn run(args: McpArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        McpCmd::Serve(a) => serve(a),
        McpCmd::Card => card(),
        McpCmd::Preview(a) => preview(a),
    }
}

/// `coral mcp card` — print the server card as pretty-printed JSON.
///
/// v0.22.5 acceptance criterion #6: stdout is byte-identical to the
/// HTTP body modulo the trailing newline `println!` adds. Both
/// surfaces call [`server_card`] with the same `WikiResourceProvider`
/// / `ToolCatalog` / `PromptCatalog` instances, so capability counts
/// agree across them.
///
/// We construct the same `WikiResourceProvider` `coral mcp serve` uses
/// (rooted at the current working directory, `include_unreviewed =
/// false`) so a registry probing `coral mcp card` from a real project
/// observes the same `resources.count` it would see on
/// `GET /.well-known/mcp/server-card.json`. The CLI subcommand is
/// otherwise a thin wrapper — no flags, no I/O beyond stdout, exit 0
/// on success and propagate errors via anyhow.
fn card() -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let resources: Box<dyn ResourceProvider> = Box::new(WikiResourceProvider::new(cwd));
    let card = server_card(resources.as_ref(), &ToolCatalog, &PromptCatalog);
    // `to_string_pretty` then `println!` is the canonical form: HTTP
    // body uses the same pretty serialization, so stdout matches it
    // byte-for-byte plus exactly one trailing `\n`.
    println!("{}", serde_json::to_string_pretty(&card)?);
    Ok(ExitCode::SUCCESS)
}

/// `coral mcp preview` — print what MCP clients would see without
/// starting the server.
///
/// v0.24.3 M1.14: standalone introspection — no transport, no watcher,
/// no IO loop. Constructs the same providers `coral mcp serve` uses,
/// queries them, and dumps the result to stdout in either human-
/// readable or JSON format. Useful for debugging the MCP surface or
/// for CI checks that assert the advertised catalog hasn't drifted.
fn preview(args: PreviewArgs) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let resources_provider =
        WikiResourceProvider::new(cwd).with_include_unreviewed(args.include_unreviewed);

    let resources = resources_provider.list();
    let tools = ToolCatalog::all();
    let prompts = PromptCatalog::list();

    match args.format {
        PreviewFormat::Human => {
            println!("=== Coral MCP Preview ===\n");

            println!("Resources ({}):", resources.len());
            for r in &resources {
                println!("  - {} -- {}", r.uri, r.name);
            }

            println!("\nTools ({}):", tools.len());
            for t in &tools {
                println!("  - {} -- {}", t.name, t.description);
            }

            println!("\nPrompts ({}):", prompts.len());
            for p in &prompts {
                println!("  - {} -- {}", p.name, p.description);
            }

            println!("\n---");
            println!("Transport: stdio (default) | http (--transport http --port 3737)");
            println!("Start: coral mcp serve");
        }
        PreviewFormat::Json => {
            let json = serde_json::json!({
                "resources": resources.iter().map(|r| serde_json::json!({
                    "uri": r.uri,
                    "name": r.name,
                    "mimeType": r.mime_type,
                })).collect::<Vec<_>>(),
                "tools": tools.iter().map(|t| serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "read_only": t.read_only,
                })).collect::<Vec<_>>(),
                "prompts": prompts.iter().map(|p| serde_json::json!({
                    "name": p.name,
                    "description": p.description,
                    "arguments": p.arguments.iter().map(|a| serde_json::json!({
                        "name": a.name,
                        "required": a.required,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&json)?);
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn serve(args: ServeArgs) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    // v0.30.0 audit #002: the resource provider and the file watcher
    // share one `Arc<RwLock<WikiState>>` so `mark_dirty()` from the
    // watcher actually invalidates the resource cache. Pre-fix the
    // provider had an OnceLock that could never be invalidated.
    let wiki_state = shared_state(cwd.join(".wiki"));
    // v0.20.2 audit-followup #37: opt-in flag to surface unreviewed
    // distilled pages. Default-deny — see `WikiResourceProvider`
    // doc comment.
    let resources: Arc<dyn ResourceProvider> = Arc::new(
        WikiResourceProvider::new(cwd.clone())
            .with_include_unreviewed(args.include_unreviewed)
            .with_state(Arc::clone(&wiki_state)),
    );
    let tools: Arc<dyn ToolDispatcher> = Arc::new(CoralToolDispatcher::new(cwd.clone()));
    // v0.20.2 audit-followup #38: surface BOTH `read_only` and
    // `allow_write_tools` to the server config. The handler uses
    // `allow_write_tools` as the single source of truth for both
    // `tools/list` advertisement and `tools/call` dispatch — the
    // two surfaces can no longer disagree. `read_only` is still
    // surfaced as a behavioural marker for resources / future
    // gating, but the write-tool catalog gate is now driven by
    // `allow_write_tools` alone.
    let read_only = args.read_only && !args.allow_write_tools;
    let transport = match args.transport {
        TransportArg::Stdio => Transport::Stdio,
        TransportArg::Http => Transport::HttpSse,
    };
    let bind_addr = args.bind;
    let port = args.port;
    let watch = args.watch;
    let config = ServerConfig {
        transport,
        read_only,
        allow_write_tools: args.allow_write_tools,
        port,
        bind_addr,
        allow_experimental_tasks: false,
    };
    let handler = Arc::new(McpHandler::new(config, resources, tools));
    let transport_label = match args.transport {
        TransportArg::Stdio => "stdio",
        TransportArg::Http => "http",
    };
    eprintln!(
        "coral mcp serve — transport={}, read_only={}, allow_write_tools={}",
        transport_label, read_only, args.allow_write_tools
    );
    // Optionally start the wiki file watcher. The `_watcher` handle
    // must stay alive for the duration of the serve loop — dropping it
    // signals the background thread to stop.
    let _watcher = if watch {
        eprintln!("  watcher: polling .wiki/ every 2s for change notifications");
        Some(start_watcher_with_state(
            WatcherConfig {
                wiki_root: cwd.join(".wiki"),
                ..Default::default()
            },
            Arc::clone(&handler),
            Some(Arc::clone(&wiki_state)),
        ))
    } else {
        None
    };
    // v0.30.0 audit B1: install a SIGINT/SIGTERM handler so Ctrl-C
    // doesn't kill the process mid-request. Stdio's read loop already
    // exits on EOF, but Ctrl-C still mostly bites HTTP/SSE — we don't
    // (yet) own the http_sse.rs serve loop, so when a signal fires we
    // spawn a watchdog that calls `std::process::exit(0)` once handlers
    // have a moment to flush. Pattern mirrors `serve.rs:62-64`,
    // `interface.rs:105`, and `monitor/up.rs:168`.
    let shutdown = Arc::new(AtomicBool::new(false));
    install_shutdown_handler(Arc::clone(&shutdown))?;
    {
        let watchdog_flag = Arc::clone(&shutdown);
        std::thread::Builder::new()
            .name("coral-mcp-shutdown".to_string())
            .spawn(move || {
                // Poll the flag; once set, give in-flight handlers a
                // brief grace window, then exit. Without this, the
                // tiny_http blocking iterator (HTTP) or stdin read
                // (stdio) would hang after Ctrl-C until the OS sent
                // SIGKILL.
                loop {
                    if watchdog_flag.load(Ordering::Relaxed) {
                        eprintln!("\ncoral mcp serve: shutdown signal received, exiting");
                        std::thread::sleep(std::time::Duration::from_millis(150));
                        std::process::exit(0);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            })
            .map_err(|e| anyhow::anyhow!("failed to spawn shutdown watchdog: {e}"))?;
    }
    match args.transport {
        TransportArg::Stdio => {
            handler
                .serve_stdio()
                .map_err(|e| anyhow::anyhow!("MCP stdio loop failed: {e}"))?;
        }
        TransportArg::Http => {
            let bind_ip = bind_addr.unwrap_or(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
            let port = port.unwrap_or(DEFAULT_HTTP_PORT);
            let socket = SocketAddr::new(bind_ip, port);
            // v0.21.1 acceptance criterion: --bind 0.0.0.0 must emit a
            // stderr warning banner. The MCP spec's DNS-rebinding
            // mitigation only protects browsers, so 0.0.0.0 + a
            // permissive proxy still hands a network-reachable agent
            // to anyone who can see the port.
            if matches!(bind_ip, IpAddr::V4(v4) if v4.is_unspecified())
                || matches!(bind_ip, IpAddr::V6(v6) if v6.is_unspecified())
            {
                tracing::warn!(
                    bind = %bind_ip,
                    port = port,
                    "MCP HTTP transport bound to 0.0.0.0 — exposed to every network interface; \
                     prefer --bind 127.0.0.1 unless you know what you're doing"
                );
                eprintln!(
                    "WARNING: coral mcp serve bound to {bind_ip}:{port} — reachable from \
                     every network interface. Origin validation defends browser clients but \
                     not native ones; consider --bind 127.0.0.1."
                );
            }
            // Bind FIRST, then print the resolved address — when the
            // user passes `--port 0` the OS picks a free port and the
            // banner needs to show the actual port (so smoke tests
            // and humans both know where to connect). The blocking
            // serve loop runs after the banner.
            let transport = HttpSseTransport::bind(socket, Arc::clone(&handler))
                .map_err(|e| anyhow::anyhow!("MCP HTTP/SSE bind failed: {e}"))?;
            let resolved = transport
                .local_addr()
                .map_err(|e| anyhow::anyhow!("could not query local addr: {e}"))?;
            eprintln!("coral mcp serve — listening on http://{resolved}/mcp");
            transport
                .serve_blocking()
                .map_err(|e| anyhow::anyhow!("MCP HTTP/SSE loop failed: {e}"))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[allow(dead_code)]
fn _ensure_tool_call_result_used(_t: ToolCallResult) {}

/// v0.30.0 audit B1: register SIGINT/SIGTERM so the serve loops can
/// shut down gracefully instead of being killed mid-request. The flag
/// is shared with a watchdog thread that calls `std::process::exit(0)`
/// once a signal flips it.
fn install_shutdown_handler(flag: Arc<AtomicBool>) -> Result<()> {
    use signal_hook::consts::{SIGINT, SIGTERM};
    signal_hook::flag::register(SIGINT, flag.clone())
        .map_err(|e| anyhow::anyhow!("failed to register SIGINT handler: {e}"))?;
    signal_hook::flag::register(SIGTERM, flag)
        .map_err(|e| anyhow::anyhow!("failed to register SIGTERM handler: {e}"))?;
    Ok(())
}

/// v0.30.0 audit B4: in-process serialization for audit-log appends
/// and rotations. The audit log lives in `coral-cli` (NOT `coral-mcp`)
/// because tools dispatched through `NoOpDispatcher` or any library
/// consumer of `coral-mcp` get no audit trail — only the CLI's
/// `CoralToolDispatcher::call` path appends to `.coral/audit.log`.
///
/// Under the HTTP transport, several dispatcher threads run
/// concurrently. Pre-fix, `remove_file(audit.log.1)` + `rename(...)`
/// + the subsequent `append` were three independent syscalls with no
/// serialization: two threads could race, lose log entries between
/// the metadata check and the rename, or partially clobber the rolled
/// file. The mutex below makes "check size → rotate → append" a single
/// atomic critical section within the process. Cross-process
/// serialization would need a real `flock` (which would require a new
/// dep); in practice nothing else writes the audit log for a given
/// project, so an in-process mutex covers the realistic race.
fn audit_log_mutex() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Real dispatcher backed by the same library APIs the CLI uses.
///
/// v0.19.5 audit C1: replaces `NoOpDispatcher` so MCP clients get real
/// answers from `tools/call`. Each handler is intentionally simple —
/// load pages from `<cwd>/.wiki`, run the existing core helper,
/// serialize. Write tools (`run_test` / `up` / `down`) are gated by
/// the server's `read_only` flag at the McpHandler layer; we still
/// return `Skip` for them here so a misconfigured handler doesn't
/// silently turn into a remote-exec foothold.
pub struct CoralToolDispatcher {
    project_root: PathBuf,
}

impl CoralToolDispatcher {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    fn wiki_root(&self) -> PathBuf {
        self.project_root.join(".wiki")
    }

    fn read_pages(&self) -> Vec<coral_core::page::Page> {
        let root = self.wiki_root();
        if !root.exists() {
            return Vec::new();
        }
        walk::read_pages(&root).unwrap_or_default()
    }

    fn append_audit(&self, tool: &str, args: &serde_json::Value, summary: &str) {
        // v0.19.5 audit M5: per-tool audit trail. Best-effort append;
        // the .coral dir might not exist yet (legacy projects), in
        // which case we skip silently rather than break the call.
        //
        // v0.19.6 audit M2: rotate the file once it crosses the cap so
        // long-running MCP servers don't grow `.coral/audit.log` past
        // the user's disk budget. Single-rotation is intentionally
        // simple — `.coral/audit.log.1` holds the previous epoch and
        // the active file restarts fresh. Users who need longer
        // retention can wire up logrotate externally; a single rolled
        // file keeps the binary policy-free.
        //
        // v0.30.0 audit B4: the rotation check, rotation, and append
        // all run inside the same in-process critical section. Pre-fix
        // these were three independent syscalls and concurrent
        // dispatcher threads under the HTTP transport could race,
        // dropping log lines between the size check and the rename.
        // The audit log lives in `coral-cli` (NOT `coral-mcp`) — any
        // consumer of the `coral-mcp` library that doesn't go through
        // `CoralToolDispatcher` gets no audit trail.
        let dir = self.project_root.join(".coral");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let path = dir.join("audit.log");
        // Acquire the in-process audit-log mutex for the whole
        // check-rotate-append sequence. `lock()` only errors on
        // poisoning; even then we proceed (a previous panic during a
        // write shouldn't permanently disable audit logging).
        let _guard = audit_log_mutex()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self::rotate_audit_log_if_needed(&path);
        let entry = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "tool": tool,
            "args": args,
            "result_summary": summary,
        });
        let line = format!("{entry}\n");
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    }

    /// Maximum size of the active audit log before it's rotated to
    /// `audit.log.1`. v0.19.6 audit M2: 16 MiB is the cap chosen so a
    /// busy day's worth of MCP traffic still fits in one file (~150 KB
    /// per call × 100k calls would still be under the cap), and so the
    /// rotated `audit.log.1` doesn't grow large enough to cost much
    /// disk.
    const AUDIT_LOG_MAX_BYTES: u64 = 16 * 1024 * 1024;

    /// If the active audit log exceeds `AUDIT_LOG_MAX_BYTES`, rename it
    /// to `audit.log.1` (replacing any prior rolled file). Best-effort:
    /// any I/O error is swallowed because audit logging must never
    /// fail a tool call.
    ///
    /// v0.30.0 audit B4: callers MUST hold `audit_log_mutex()` for the
    /// whole check-rotate-append sequence — this function performs no
    /// locking of its own. The two syscalls below (`remove_file` +
    /// `rename`) are not jointly atomic; the surrounding mutex is what
    /// makes the sequence race-free against other dispatcher threads
    /// in the same process. The order is deliberate:
    ///   1. `remove_file(rolled)` — Windows `rename` cannot overwrite
    ///      a target that exists; POSIX `rename` would have replaced
    ///      it, but we treat both platforms the same. ENOENT is
    ///      ignored (no prior rolled file is the common case).
    ///   2. `rename(active, rolled)` — atomic on POSIX/NTFS; if it
    ///      fails we leave both files in place and the next append
    ///      simply appends to the over-cap active file (best-effort).
    /// We also `sync_all` the parent directory on POSIX so the
    /// rename's directory entry survives a crash. On Windows opening
    /// a directory via `File::open` isn't supported, so that step is
    /// a no-op.
    fn rotate_audit_log_if_needed(path: &Path) {
        let size = match std::fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => return, // file doesn't exist yet (or unreadable)
        };
        if size < Self::AUDIT_LOG_MAX_BYTES {
            return;
        }
        // Determine sibling rolled path: `<name>.1`.
        let mut rolled = path.as_os_str().to_os_string();
        rolled.push(".1");
        let rolled = std::path::PathBuf::from(rolled);
        let _ = std::fs::remove_file(&rolled);
        if std::fs::rename(path, &rolled).is_ok() {
            // Best-effort fsync of the parent directory so the rename's
            // entry survives a crash. POSIX-only; on Windows opening a
            // directory as a file errors and we swallow it.
            if let Some(parent) = path.parent() {
                let _ = std::fs::File::open(parent).and_then(|f| f.sync_all());
            }
        }
    }
}

impl ToolDispatcher for CoralToolDispatcher {
    fn call(&self, name: &str, args: &serde_json::Value) -> ToolCallResult {
        let result = match name {
            "search" => self.tool_search(args),
            "find_backlinks" => self.tool_find_backlinks(args),
            "affected_repos" => self.tool_affected_repos(args),
            "verify" => self.tool_verify(args),
            "query" => {
                // `query` requires an LLM provider key — defer to the
                // CLI rather than reach out from inside the MCP loop
                // (no streaming, no key handling here).
                ToolCallResult::Skip {
                    reason: "use the CLI `coral query` for LLM-augmented queries; MCP `query` is not wired in this build".into(),
                }
            }
            "run_test" | "up" | "down" => ToolCallResult::Skip {
                reason: format!(
                    "tool '{name}' requires --allow-write-tools and is not implemented over MCP yet"
                ),
            },
            other => ToolCallResult::Error {
                message: format!("unknown tool: {other}"),
            },
        };
        let summary = match &result {
            ToolCallResult::Ok(_) => "ok",
            ToolCallResult::Skip { .. } => "skip",
            ToolCallResult::Error { .. } => "error",
        };
        self.append_audit(name, args, summary);
        result
    }
}

impl CoralToolDispatcher {
    fn tool_search(&self, args: &serde_json::Value) -> ToolCallResult {
        let q = match args.get("q").and_then(|v| v.as_str()) {
            Some(q) => q,
            None => {
                return ToolCallResult::Error {
                    message: "missing required arg 'q'".into(),
                };
            }
        };
        let pages = self.read_pages();
        let hits = search::search_hybrid(&pages, q, 10);
        let json: Vec<serde_json::Value> = hits
            .into_iter()
            .map(|h| {
                serde_json::json!({
                    "slug": h.slug,
                    "score": h.score,
                })
            })
            .collect();
        ToolCallResult::Ok(serde_json::json!({ "hits": json }))
    }

    fn tool_find_backlinks(&self, args: &serde_json::Value) -> ToolCallResult {
        let slug = match args.get("slug").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolCallResult::Error {
                    message: "missing required arg 'slug'".into(),
                };
            }
        };
        let pages = self.read_pages();
        let backlinks: Vec<String> = pages
            .iter()
            .filter(|p| p.outbound_links().iter().any(|l| l == slug))
            .map(|p| p.frontmatter.slug.clone())
            .collect();
        ToolCallResult::Ok(serde_json::json!({ "backlinks": backlinks }))
    }

    fn tool_affected_repos(&self, args: &serde_json::Value) -> ToolCallResult {
        let _since = args.get("since").and_then(|v| v.as_str()).unwrap_or("");
        // Reading the manifest gives us the repo list; without a
        // running git context here we can only return the configured
        // names — the CLI does the real --since walk. Return a thin
        // listing so agents at least see what's in the project.
        let manifest = self.project_root.join("coral.toml");
        let project = if manifest.exists() {
            match coral_core::project::Project::load_from_manifest(&manifest) {
                Ok(p) => p,
                Err(e) => {
                    return ToolCallResult::Error {
                        message: format!("manifest parse failed: {e}"),
                    };
                }
            }
        } else {
            coral_core::project::Project::synthesize_legacy(&self.project_root)
        };
        let repos: Vec<&str> = project.repos.iter().map(|r| r.name.as_str()).collect();
        ToolCallResult::Ok(serde_json::json!({ "repos": repos }))
    }

    fn tool_verify(&self, _args: &serde_json::Value) -> ToolCallResult {
        // Verify against running services touches the env layer; we
        // don't want to import `coral-env` from here for now. Surface
        // a useful Skip so the agent knows to fall back.
        ToolCallResult::Skip {
            reason: "verify is wired via the CLI; run `coral verify` in a shell".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_project(dir: &Path) {
        fs::create_dir_all(dir.join(".wiki/modules")).unwrap();
        fs::write(
            dir.join(".wiki/modules/order.md"),
            "---\nslug: order\ntype: module\nlast_updated_commit: abc\nconfidence: 0.6\nstatus: draft\n---\n\n# Order\n\nbody [[invoice]]\n",
        )
        .unwrap();
        fs::write(
            dir.join(".wiki/modules/invoice.md"),
            "---\nslug: invoice\ntype: module\nlast_updated_commit: abc\nconfidence: 0.6\nstatus: draft\n---\n\n# Invoice\n\nbody\n",
        )
        .unwrap();
    }

    #[test]
    fn search_returns_hits() {
        let dir = TempDir::new().unwrap();
        make_project(dir.path());
        let d = CoralToolDispatcher::new(dir.path().to_path_buf());
        let r = d.call("search", &serde_json::json!({"q": "Order"}));
        match r {
            ToolCallResult::Ok(v) => {
                let hits = v["hits"].as_array().unwrap();
                assert!(!hits.is_empty(), "expected hits");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn find_backlinks_returns_pages_linking_to_slug() {
        let dir = TempDir::new().unwrap();
        make_project(dir.path());
        let d = CoralToolDispatcher::new(dir.path().to_path_buf());
        let r = d.call("find_backlinks", &serde_json::json!({"slug": "invoice"}));
        match r {
            ToolCallResult::Ok(v) => {
                let bls = v["backlinks"].as_array().unwrap();
                assert!(bls.iter().any(|b| b == "order"), "got {:?}", bls);
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn unknown_tool_returns_error() {
        let dir = TempDir::new().unwrap();
        make_project(dir.path());
        let d = CoralToolDispatcher::new(dir.path().to_path_buf());
        match d.call("frobnicate", &serde_json::json!({})) {
            ToolCallResult::Error { message } => assert!(message.contains("unknown tool")),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    /// v0.19.5 audit M5: every tool call appends an audit line.
    #[test]
    fn tool_call_appends_audit_line() {
        let dir = TempDir::new().unwrap();
        make_project(dir.path());
        let d = CoralToolDispatcher::new(dir.path().to_path_buf());
        let _ = d.call("search", &serde_json::json!({"q": "Order"}));
        let log = std::fs::read_to_string(dir.path().join(".coral/audit.log"))
            .expect("audit log written");
        assert!(log.contains("\"tool\":\"search\""), "log was: {log}");
    }

    /// v0.19.6 audit M2: when `.coral/audit.log` crosses the cap, it
    /// is rotated to `.coral/audit.log.1` and the active file restarts
    /// fresh. Verifies single-rotation semantics — a second rotation
    /// replaces the first.
    #[test]
    fn audit_log_rotates_at_size_cap() {
        let dir = TempDir::new().unwrap();
        make_project(dir.path());
        let coral_dir = dir.path().join(".coral");
        std::fs::create_dir_all(&coral_dir).unwrap();
        let active = coral_dir.join("audit.log");
        // Pre-seed the active log past the cap with a marker we can
        // recognize after rotation.
        let marker = "OLD-EPOCH-MARKER";
        let mut content = String::new();
        content.push_str(marker);
        content.push('\n');
        // Pad up to just over the cap.
        while (content.len() as u64) < CoralToolDispatcher::AUDIT_LOG_MAX_BYTES + 1 {
            content.push_str("padding-padding-padding-padding\n");
        }
        std::fs::write(&active, &content).unwrap();
        let pre_size = std::fs::metadata(&active).unwrap().len();
        assert!(pre_size > CoralToolDispatcher::AUDIT_LOG_MAX_BYTES);

        // First call after the cap is exceeded must rotate.
        let d = CoralToolDispatcher::new(dir.path().to_path_buf());
        let _ = d.call("search", &serde_json::json!({"q": "Order"}));

        let rolled = coral_dir.join("audit.log.1");
        assert!(rolled.exists(), "audit.log.1 (rolled) must exist");
        let rolled_content = std::fs::read_to_string(&rolled).unwrap();
        assert!(
            rolled_content.contains(marker),
            "rolled file must contain the pre-rotation content"
        );
        // Active file restarted fresh — it contains only the new line.
        let new_active = std::fs::read_to_string(&active).unwrap();
        assert!(
            !new_active.contains(marker),
            "active log must be fresh after rotation; got:\n{new_active}"
        );
        assert!(
            new_active.contains("\"tool\":\"search\""),
            "post-rotation entry should land in the active file: {new_active}"
        );
    }

    /// v0.30.0 audit B4: concurrent appends across many threads with a
    /// small size cap must not drop entries. Pre-fix, the rotation
    /// race (`remove_file` + `rename` + `append` as three independent
    /// syscalls) could leave a window where appends between the size
    /// check and the rename landed on the file that was about to be
    /// renamed and then got truncated-by-rotation by the next thread.
    /// Post-fix, the `audit_log_mutex()` serializes the whole
    /// check-rotate-append sequence within the process, so every
    /// dispatched entry lands in either `audit.log` or `audit.log.1`
    /// with no losses or duplicates.
    #[test]
    fn audit_log_concurrent_appends_lose_no_entries_across_rotations() {
        use std::sync::Arc as StdArc;

        let dir = TempDir::new().unwrap();
        make_project(dir.path());
        // Force several rotations within the test run by shrinking the
        // effective cap. We can't change the const, so instead drive
        // many large args so the first few KiB of entries already
        // trip the 16 MiB cap... no — at 16 MiB this would be far too
        // slow. Instead, drop a pre-seeded log already past the cap
        // and verify the FIRST rotation works correctly across N
        // racing threads. (A multi-rotation test would need a config
        // knob; the lock invariant we're guarding is per-rotation.)
        let coral_dir = dir.path().join(".coral");
        std::fs::create_dir_all(&coral_dir).unwrap();
        let active = coral_dir.join("audit.log");
        let mut content = String::new();
        while (content.len() as u64) < CoralToolDispatcher::AUDIT_LOG_MAX_BYTES + 1 {
            content.push_str("seed-padding-padding-padding-padding-padding\n");
        }
        std::fs::write(&active, &content).unwrap();

        let n_threads = 8;
        let per_thread = 100;
        let dispatcher = StdArc::new(CoralToolDispatcher::new(dir.path().to_path_buf()));
        let mut handles = Vec::with_capacity(n_threads);
        for t in 0..n_threads {
            let d = StdArc::clone(&dispatcher);
            handles.push(std::thread::spawn(move || {
                for i in 0..per_thread {
                    let _ = d.call(
                        "search",
                        &serde_json::json!({"q": format!("t{}-i{}", t, i)}),
                    );
                }
            }));
        }
        for h in handles {
            h.join().expect("worker thread panicked");
        }

        // Every call also issues a `search` audit line. Count
        // occurrences of the marker that identifies one of OUR calls
        // (the `q` value) across BOTH the active file and the rolled
        // file. Total must equal n_threads * per_thread — no losses
        // due to rotation race.
        let active_content = std::fs::read_to_string(&active).unwrap_or_default();
        let rolled_content =
            std::fs::read_to_string(coral_dir.join("audit.log.1")).unwrap_or_default();
        let mut seen = 0usize;
        for t in 0..n_threads {
            for i in 0..per_thread {
                let needle = format!("\"q\":\"t{}-i{}\"", t, i);
                let in_active = active_content.matches(needle.as_str()).count();
                let in_rolled = rolled_content.matches(needle.as_str()).count();
                assert!(
                    in_active + in_rolled >= 1,
                    "entry t={t} i={i} missing from both audit.log and audit.log.1"
                );
                // No duplicate within a single file (each append is
                // single-line and the lock prevents a second writer
                // re-running the same payload).
                assert!(
                    in_active <= 1 && in_rolled <= 1,
                    "entry t={t} i={i} duplicated (active={in_active}, rolled={in_rolled})"
                );
                seen += in_active + in_rolled;
            }
        }
        assert_eq!(
            seen,
            n_threads * per_thread,
            "expected {} entries across both files, saw {}",
            n_threads * per_thread,
            seen
        );
    }

    /// v0.24.3 M1.14: preview constructs providers without panicking
    /// when no wiki exists (empty project / temp dir).
    ///
    /// v0.30.0 audit cycle 5 B11: acquire `CWD_LOCK` before mutating
    /// the process-wide cwd. Other test sites that touch
    /// `set_current_dir` (`project/new.rs`, `project/add.rs`,
    /// `project/lock.rs`, `bootstrap.rs`, `ingest.rs`, …) hold this
    /// lock; without it, this test races them under `cargo test`
    /// parallelism and intermittently sees the wrong cwd.
    #[test]
    fn preview_does_not_panic_on_empty_project() {
        let _guard = crate::commands::CWD_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let dir = TempDir::new().unwrap();
        // Set CWD to a temp dir with no wiki — preview must still work.
        let prev_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let result = preview(PreviewArgs {
            format: PreviewFormat::Human,
            include_unreviewed: false,
        });
        // Restore cwd BEFORE the assertion so a failure can't strand
        // the rest of the test suite in a deleted tempdir.
        let preview_human_outcome = result;
        let result = preview(PreviewArgs {
            format: PreviewFormat::Json,
            include_unreviewed: false,
        });
        let preview_json_outcome = result;

        std::env::set_current_dir(prev_cwd).unwrap();

        preview_human_outcome.expect("preview Human must succeed on empty project");
        preview_json_outcome.expect("preview Json must succeed on empty project");
    }
}
