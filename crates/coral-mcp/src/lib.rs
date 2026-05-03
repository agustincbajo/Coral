//! Coral MCP server (v0.19+).
//!
//! Exposes the wiki + manifest as [Model Context Protocol](https://modelcontextprotocol.io/)
//! resources / tools / prompts so any MCP-speaking agent (Claude Code,
//! Cursor, Continue, Cline, Goose, Codex…) can read Coral's structured
//! context cross-session.
//!
//! v0.19 wave 1 ships the **type model and resource/tool/prompt
//! catalogs** without the runtime — the actual stdio + Streamable
//! HTTP/SSE transports come from the official `rmcp = "1.6"` SDK and
//! land in v0.19 wave 2 once the dep is pinned.
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
/// Shared between the future stdio and HTTP/SSE transports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub transport: Transport,
    /// `true` until the user explicitly opts in via `--allow-write-tools`.
    /// The default-deny stance covers PRD risk #25 (MCP server as
    /// exfiltration vector).
    pub read_only: bool,
    pub port: Option<u16>,
}

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
            port: None,
        }
    }
}
