//! API route handlers.
//!
//! Each submodule defines `pub fn handle(...)` (or similarly named) that
//! returns either `Vec<u8>` (a fully-buffered JSON envelope) or, for
//! `query`, takes ownership of the request to stream SSE frames.
//!
//! The actual method+path → handler dispatch lives in `server::handle`
//! so the dispatcher can keep ownership of the `tiny_http::Request`
//! until the very last moment (some routes need raw stream access).

pub mod graph;
pub mod health;
pub mod manifest;
pub mod pages;
pub mod query;
pub mod search;
