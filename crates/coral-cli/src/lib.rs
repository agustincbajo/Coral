//! coral-cli library facade.
//!
//! Exposes the command modules so integration tests can call them
//! directly (with `MockRunner` injection). The binary at `src/main.rs`
//! is a thin clap dispatcher over these.

pub mod commands;
