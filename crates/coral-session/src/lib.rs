//! Coral session: capture + distill agent transcripts into the wiki.
//!
//! This crate implements the `coral session` subcommand family
//! (capture/list/forget/distill/show). The high-level flow:
//!
//! 1. **Capture** — copy a raw transcript from the agent client's
//!    local store into `.coral/sessions/<date>_<source>_<sha8>.jsonl`.
//!    The privacy scrubber runs by default (opt-out is a hard
//!    `--no-scrub --yes-i-really-mean-it` two-flag combo per the
//!    v0.20 PRD).
//! 2. **Distill** — single-pass `Runner::run` against the captured
//!    transcript that emits a YAML synthesis page with `reviewed:
//!    false` frontmatter. Trust-by-curation is enforced downstream
//!    by `coral lint` (a v0.20 critical lint rule rejects any
//!    `reviewed: false` page from being committed cleanly).
//! 3. **List / Forget / Show** — local housekeeping; no LLM.
//!
//! The CLI surface lives in `coral-cli/src/commands/session.rs`. The
//! crate exposes the building blocks so the CLI is a thin wrapper
//! and tests can drive the flow directly with a `MockRunner`.
//!
//! ## Privacy posture
//!
//! Sessions live under `.coral/sessions/` which `coral init` adds to
//! the project's `.gitignore` (see `init.rs` template patterns).
//! Distilled output under `.coral/sessions/distilled/` is also
//! ignored by default so curation flows through `--apply` (which
//! writes to `.wiki/pages/synthesis/<slug>.md` with `reviewed:
//! false` frontmatter; humans flip it to `true` before commit).
//!
//! See `docs/SESSIONS.md` for the full PRD-derived design notes.

pub mod capture;
pub mod claude_code;
pub mod distill;
pub mod error;
pub mod forget;
pub mod list;
pub mod scrub;

pub use capture::{CaptureOptions, CaptureOutcome, CaptureSource, capture_from_path};
pub use claude_code::{ClaudeCodeMessage, ClaudeCodeRecord, find_latest_for_cwd};
pub use distill::{DistillOptions, DistillOutcome, distill_session};
pub use error::{SessionError, SessionResult};
pub use forget::{ForgetOptions, forget_session};
pub use list::{ListFormat, SessionEntry, list_sessions};
pub use scrub::{Redaction, RedactionKind, scrub};
