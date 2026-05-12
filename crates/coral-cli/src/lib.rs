//! coral-cli library facade.
//!
//! Exposes the command modules so integration tests can call them
//! directly (with `MockRunner` injection). The binary at `src/main.rs`
//! is a thin clap dispatcher over these.

// v0.32.2: clippy 1.94 made `doc_lazy_continuation` warn-by-default —
// 11 of our doc-comments use the legacy "non-indented list
// continuation" style. Reformatting all of them touches a lot of
// surface area that's already well-tested rendering-wise. Silenced
// crate-wide; tracked for a future docstring sweep.
#![allow(clippy::doc_lazy_continuation)]

pub mod commands;
