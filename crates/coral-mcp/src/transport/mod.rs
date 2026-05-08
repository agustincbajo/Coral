//! MCP transport surfaces.
//!
//! v0.21.1+ ships two: [`stdio`] (the canonical path every shipped
//! MCP client speaks) and [`http_sse`] (Streamable HTTP/SSE per the
//! MCP 2025-11-25 spec, behind `coral mcp serve --transport http`).
//!
//! Both transports share the same dispatcher: the per-request
//! body is fed through [`crate::McpHandler::handle_line`] and the
//! returned JSON-RPC envelope is what the transport layer ships back.
//! As a result the audit-log shape, the `--read-only` /
//! `--allow-write-tools` gate, the unreviewed-distilled filter, and
//! every JSON-RPC error code are byte-identical across the two
//! transports — only the framing differs.

pub mod http_sse;
pub mod stdio;

pub use http_sse::{HttpSseTransport, serve_http_sse};
pub use stdio::serve_stdio;
