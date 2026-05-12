//! `coral-ui` — REST API + embedded SPA assets for `coral ui serve`.
//!
//! This crate is the v0.32.0 WebUI surface. It exposes a `serve` function
//! that runs a blocking `tiny_http` server with a small REST API over the
//! wiki (`/api/v1/pages`, `/api/v1/search`, `/api/v1/graph`,
//! `/api/v1/manifest`, `/api/v1/lock`, `/api/v1/stats`, `/api/v1/query`),
//! plus serves the embedded SPA bundle (`assets/dist/`).
//!
//! Security defaults are conservative: loopback-only, read-only,
//! `Host`/`Origin` validation, bearer-token gating for any LLM- or
//! tool-touching endpoint. See `auth.rs` for the full policy and
//! `error.rs` for the canonical error envelope.
//!
//! There is NO tokio dependency. Everything is synchronous; the runtime
//! pattern mirrors `coral wiki serve` (legacy) and `coral mcp serve`.

pub use error::ApiError;
pub use server::{ServeConfig, serve};

mod auth;
mod error;
mod routes;
mod server;
mod state;
mod static_assets;
