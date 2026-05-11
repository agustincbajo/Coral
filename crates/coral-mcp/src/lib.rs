//! Coral MCP server (v0.19+).
//!
//! Exposes the wiki + manifest as [Model Context Protocol](https://modelcontextprotocol.io/)
//! resources / tools / prompts so any MCP-speaking agent (Claude Code,
//! Cursor, Continue, Cline, Goose, Codex…) can read Coral's structured
//! context cross-session.
//!
//! **v0.21.1+ ships both transports.** stdio remains the canonical
//! path every shipped MCP client (Claude Desktop, Cursor, Continue,
//! Cline, Goose, OpenCode, Crush, Codex CLI) speaks; the Streamable
//! HTTP/SSE transport (MCP 2025-11-25) lands behind
//! `coral mcp serve --transport http --port <p>`. The HTTP transport
//! defaults to binding `127.0.0.1` and validates `Origin` against
//! `null` / `http://localhost*` / `http://127.0.0.1*` only (the
//! DNS-rebinding mitigation the MCP spec calls for); `--bind 0.0.0.0`
//! is opt-in and emits a stderr warning banner because exposing the
//! server to a network reachable by other users turns it into an
//! exfiltration vector. The wire format (`"transport": "stdio"` vs.
//! `"http_sse"`) was kept stable across the v0.20.x → v0.21.1
//! reintroduction so older configs deserialize unchanged.
//!
//! Resource / tool / prompt surface is identical across the two
//! transports — the dispatcher is shared via [`McpHandler::handle_line`]
//! so the audit-log line shape, the `--read-only` /
//! `--allow-write-tools` gate, and the unreviewed-distilled filter
//! behave the same regardless of how the client connects. The only
//! transport-level concerns are framing (stdin/stdout one-line-per-
//! message vs. HTTP body), origin validation, the 4 MiB body cap, and
//! the `Mcp-Session-Id` cookie.
//!
//! Strategic positioning (PRD §3.10): Coral is "the project manifest
//! for AI-era development", and this crate is what makes it
//! consumable by any agent.

pub mod card;
pub mod prompts;
pub mod resources;
pub mod server;
pub mod tools;
pub mod transport;
pub mod watcher;

pub use card::server_card;
pub use prompts::{PromptCatalog, PromptDescriptor};
pub use resources::{Resource, ResourceProvider, WikiResourceProvider};
pub use server::{McpHandler, NoOpDispatcher, PROTOCOL_VERSION, ToolCallResult, ToolDispatcher};
pub use tools::{Tool, ToolCatalog, ToolKind};

use serde::{Deserialize, Serialize};

/// Configuration the CLI wraps when invoking `coral mcp serve`.
/// v0.21.1+ supports both `stdio` and `http_sse`; older configs
/// pinned `transport: stdio` continue to deserialize unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub transport: Transport,
    /// `true` until the user explicitly opts in via `--allow-write-tools`.
    /// The default-deny stance covers PRD risk #25 (MCP server as
    /// exfiltration vector).
    ///
    /// **v0.20.2 audit-followup #38 — listing semantics.** Pre-fix,
    /// this single field gated both the `tools/list` advertisement
    /// AND the `tools/call` dispatcher. So `--read-only false` alone
    /// would surface write tools in `tools/list` even though
    /// `tools/call` then errored with "requires --allow-write-tools"
    /// — a doc-vs-reality drift that surprised users. Post-fix the
    /// listing is gated by [`Self::allow_write_tools`] (the
    /// CLI-only opt-in), so the catalog matches the dispatcher.
    pub read_only: bool,
    /// `true` only when `coral mcp serve --allow-write-tools` was
    /// explicitly passed. Drives the `tools/list` advertisement of
    /// `run_test` / `up` / `down` AND the `tools/call` dispatcher's
    /// permission check. Default `false`.
    ///
    /// v0.20.2 audit-followup #38: previously the listing gate used
    /// `!read_only` — i.e. `--read-only false` alone was enough to
    /// list write tools. The dispatcher correctly required both
    /// flags. Now both surfaces share one contract.
    #[serde(default)]
    pub allow_write_tools: bool,
    /// HTTP transport port. Required when `transport == HttpSse`,
    /// ignored on stdio. CLI default is 3737 (chosen to avoid the
    /// 3000-3100 React/Next dev-server cluster and the 8000-8100
    /// Python clusters).
    pub port: Option<u16>,
    /// HTTP transport bind address. Defaults to `127.0.0.1` on
    /// the CLI; `0.0.0.0` is opt-in and prints a stderr warning
    /// banner. Ignored on stdio.
    ///
    /// v0.21.1: was previously not surfaced because the v0.20.x
    /// docstring left the HTTP transport "deferred". Now first-class.
    #[serde(default)]
    pub bind_addr: Option<std::net::IpAddr>,
}

/// Transport variants. v0.21.1+ ships both `Stdio` and `HttpSse`.
/// The wire format (`"transport": "stdio"` / `"http_sse"`) was held
/// stable across the v0.20.x → v0.21.1 reintroduction so any older
/// `ServerConfig` JSON / TOML deserializes unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Stdio,
    HttpSse,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            transport: Transport::Stdio,
            read_only: true,
            allow_write_tools: false,
            port: None,
            bind_addr: None,
        }
    }
}
