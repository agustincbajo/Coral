//! Coral MCP server (v0.19+).
//!
//! Exposes the wiki + manifest as [Model Context Protocol](https://modelcontextprotocol.io/)
//! resources / tools / prompts so any MCP-speaking agent (Claude Code,
//! Cursor, Continue, Cline, Goose, Codex…) can read Coral's structured
//! context cross-session.
//!
//! v0.20.0 ships **stdio** as the only supported transport, with
//! `WikiResourceProvider` + `CoralToolDispatcher` end-to-end (catalogs
//! advertised; tools `query` / `search` / `find_backlinks` /
//! `affected_repos` / `verify` reachable in `--read-only` mode; write
//! tools gated by `--allow-write-tools`). Streamable HTTP/SSE was
//! deferred during the v0.20.1 cycle-4 audit (H6) — every shipped MCP
//! client (Claude Desktop, Cursor, Continue, Cline, Goose, OpenCode,
//! Crush, Codex CLI) speaks stdio, and an inflated docs surface for an
//! unimplemented transport was deemed worse than the absence of the
//! feature. HTTP/SSE returns to the roadmap when client demand
//! materializes; track via the GitHub roadmap.
//!
//! Strategic positioning (PRD §3.10): Coral is "the project manifest
//! for AI-era development", and this crate is what makes it
//! consumable by any agent.

pub mod prompts;
pub mod resources;
pub mod server;
pub mod tools;

pub use prompts::{PromptCatalog, PromptDescriptor};
pub use resources::{Resource, ResourceProvider, WikiResourceProvider};
pub use server::{McpHandler, NoOpDispatcher, PROTOCOL_VERSION, ToolCallResult, ToolDispatcher};
pub use tools::{Tool, ToolCatalog, ToolKind};

use serde::{Deserialize, Serialize};

/// Configuration the CLI wraps when invoking `coral mcp serve`.
/// Currently only `stdio` ships; if HTTP/SSE returns to the roadmap
/// the new variant lands here without a breaking change.
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
    /// Reserved for a future HTTP/SSE transport (deferred during the
    /// v0.20.1 cycle-4 audit; see crate docstring). `None` until that
    /// transport ships; ignored by the stdio path.
    pub port: Option<u16>,
}

/// Transport variants. v0.20.x ships `Stdio` only; HTTP/SSE was
/// deferred during the v0.20.1 cycle-4 audit (H6) — see crate
/// docstring. The enum is intentionally not narrowed to a single
/// variant so the wire format (`"transport": "stdio"`) stays stable
/// across the eventual `HttpSse` re-introduction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Stdio,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            transport: Transport::Stdio,
            read_only: true,
            allow_write_tools: false,
            port: None,
        }
    }
}
