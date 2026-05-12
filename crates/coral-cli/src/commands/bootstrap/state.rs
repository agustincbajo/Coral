//! Persistent checkpoint + lockfile for `coral bootstrap --apply`.
//!
//! v0.34.0 (M1) — FR-ONB-30.
//!
//! ## What this module owns
//!
//! - [`BootstrapState`] (the JSON document persisted at
//!   `.wiki/.bootstrap-state.json`) — schema-versioned record of the
//!   plan, per-page status, cumulative cost, and `--max-cost` cap.
//! - Atomic write path (`save_atomic`) gated by an `fs4` advisory
//!   lock on a sibling `.wiki/.bootstrap.lock` file. The lockfile is
//!   held for the lifetime of one bootstrap run — re-entrant runs
//!   abort cleanly with a "another `coral bootstrap` holds the lock"
//!   message rather than racing the state file.
//! - Schema-mismatch policy:
//!   - Hard abort when `schema_version` exceeds the binary's
//!     [`STATE_SCHEMA_VERSION`] (the file is newer than the binary
//!     can read — we cannot guarantee non-destructive resume).
//!   - Soft warn when `coral_version` differs (same schema version
//!     but a build mismatch) — log to stderr + continue.
//!
//! ## What this module does NOT own
//!
//! - The bootstrap event loop (per-page calls + max-cost gating)
//!   lives in `bootstrap/mod.rs`.
//! - The cost model (token → USD) lives in `coral_core::cost`.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::super::plan::PlanEntry;

/// Schema version of the persisted checkpoint. Bump only when a field
/// is removed or re-typed; additive changes are non-breaking because
/// `serde(default)` covers missing fields.
///
/// PRD FR-ONB-30 migration policy:
/// - Equal version → load.
/// - File version GREATER than this → hard abort.
/// - File version LESS than this → soft-allowed (zero v0 users exist;
///   bumps are additive). Future versions may add a migration hook.
pub const STATE_SCHEMA_VERSION: u32 = 1;

/// Per-page status in the bootstrap state machine.
///
/// Transitions:
/// `Pending → InProgress → Completed` (happy path) or
/// `Pending → InProgress → Failed` (runner error). A re-run with
/// `--resume` re-tries every page that is NOT `Completed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageStatus {
    /// Not yet attempted in this run.
    Pending,
    /// Currently being generated. If the run crashes mid-call, the
    /// state is left as `InProgress`; `--resume` re-tries the page
    /// (paying that single page's cost again in the worst case).
    InProgress,
    /// Successfully written + indexed. `--resume` skips this page.
    Completed,
    /// Runner returned an error. `--resume` re-tries the page; if
    /// this run was halted by `--max-cost`, the partial flag is set
    /// at the document level too.
    Failed,
}

/// Per-page state in the checkpoint. Token counts are zero on
/// `Pending` / `InProgress`; populated from real `Runner.usage` (or
/// the fallback heuristic) on `Completed` / `Failed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageState {
    pub slug: String,
    pub status: PageStatus,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub error: Option<String>,
}

impl PageState {
    /// A fresh `Pending` entry — token counts + cost are zero.
    pub fn pending(slug: impl Into<String>) -> Self {
        Self {
            slug: slug.into(),
            status: PageStatus::Pending,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            completed_at: None,
            error: None,
        }
    }
}

/// Top-level checkpoint document persisted at
/// `.wiki/.bootstrap-state.json`.
///
/// The `plan` is persisted on the first write so that `--resume`
/// re-uses identical ordering and never re-calls the LLM to
/// regenerate it (planner non-determinism would otherwise drift
/// resumed runs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapState {
    pub schema_version: u32,
    pub coral_version: String,
    pub started_at: DateTime<Utc>,
    pub provider: String,
    /// SHA-256 of the canonical (slug-ordered) JSON encoding of the
    /// plan entries. Surfaced for diagnostics — a mismatch on
    /// `--resume` would indicate the user manually edited the state.
    pub plan_fingerprint: String,
    pub plan: Vec<PlanEntry>,
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    #[serde(default)]
    pub cost_spent_usd: f64,
    pub pages: Vec<PageState>,
    /// Set to `true` when `--max-cost` aborted mid-flight. `--resume`
    /// clears this on the next successful page.
    #[serde(default)]
    pub partial: bool,
}

impl BootstrapState {
    /// Conventional checkpoint path: `<wiki_root>/.bootstrap-state.json`.
    pub fn path(wiki_root: &Path) -> PathBuf {
        wiki_root.join(".bootstrap-state.json")
    }

    /// Conventional lockfile path: `<wiki_root>/.bootstrap.lock`.
    pub fn lock_path(wiki_root: &Path) -> PathBuf {
        wiki_root.join(".bootstrap.lock")
    }

    /// Loads the checkpoint from `<wiki_root>/.bootstrap-state.json`.
    /// Returns `Ok(None)` when the file is absent (no prior run).
    ///
    /// Enforces the [`STATE_SCHEMA_VERSION`] policy on load — a
    /// future-versioned file hard-aborts; a past-versioned file is
    /// accepted (additive evolution).
    pub fn load(wiki_root: &Path) -> Result<Option<Self>> {
        let path = Self::path(wiki_root);
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading bootstrap state: {}", path.display()))?;
        let state: BootstrapState = serde_json::from_str(&raw)
            .with_context(|| format!("parsing bootstrap state: {}", path.display()))?;
        state.check_schema_compat()?;
        Ok(Some(state))
    }

    /// Build a fresh state document for a brand-new bootstrap run.
    /// Records the plan, computes the fingerprint, and seeds one
    /// `Pending` entry per plan entry.
    pub fn fresh(plan: Vec<PlanEntry>, provider: String, max_cost: Option<f64>) -> Self {
        let pages = plan.iter().map(|e| PageState::pending(&e.slug)).collect();
        let fingerprint = plan_fingerprint(&plan);
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            coral_version: env!("CARGO_PKG_VERSION").to_string(),
            started_at: Utc::now(),
            provider,
            plan_fingerprint: fingerprint,
            plan,
            max_cost_usd: max_cost,
            cost_spent_usd: 0.0,
            pages,
            partial: false,
        }
    }

    /// Validates that this binary can read the loaded state.
    ///
    /// - Hard abort when the file's schema version exceeds the
    ///   binary's.
    /// - Soft warn (stderr) when only `coral_version` differs — same
    ///   schema, different build. The PRD policy: continue.
    pub fn check_schema_compat(&self) -> Result<()> {
        if self.schema_version > STATE_SCHEMA_VERSION {
            anyhow::bail!(
                "Checkpoint schema v{}, binary expects v{}. \
                 Run `coral bootstrap --apply` without --resume to start over \
                 (this will overwrite .wiki/.bootstrap-state.json).",
                self.schema_version,
                STATE_SCHEMA_VERSION
            );
        }
        let bin_version = env!("CARGO_PKG_VERSION");
        if self.coral_version != bin_version {
            eprintln!(
                "warn: checkpoint from coral {}; current {}; continuing.",
                self.coral_version, bin_version
            );
        }
        Ok(())
    }

    /// Atomically rewrites `.bootstrap-state.json` with the current
    /// in-memory state. Caller is expected to hold the
    /// [`BootstrapLock`] returned by [`BootstrapLock::acquire`] —
    /// `save_atomic` does NOT take its own lock so callers can batch
    /// many writes inside one critical section.
    ///
    /// Atomicity: write to a sibling `*.tmp.<pid>.<counter>` then
    /// rename. Crash mid-write leaves the old file intact.
    pub fn save_atomic(&self, wiki_root: &Path) -> Result<()> {
        let path = Self::path(wiki_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating wiki root: {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("serializing bootstrap state")?;
        coral_core::atomic::atomic_write_string(&path, &json)
            .with_context(|| format!("writing bootstrap state: {}", path.display()))?;
        Ok(())
    }

    /// Returns the first `Pending` or `InProgress` page to work on,
    /// or `None` when every page is `Completed` / `Failed`.
    ///
    /// `--resume` semantics: `InProgress` pages (interrupted runs) are
    /// retried first; `Failed` pages are also retried; `Completed`
    /// pages are skipped.
    pub fn next_unfinished_index(&self) -> Option<usize> {
        self.pages.iter().position(|p| {
            matches!(
                p.status,
                PageStatus::Pending | PageStatus::InProgress | PageStatus::Failed
            )
        })
    }
}

/// SHA-256 of `[(slug, action_str, type_str)]` over the plan, slug-
/// ordered. Stable across runs of the same plan; changes when the
/// LLM regenerates a different page set.
fn plan_fingerprint(plan: &[PlanEntry]) -> String {
    let mut entries: Vec<(&str, String, String)> = plan
        .iter()
        .map(|e| {
            (
                e.slug.as_str(),
                format!("{:?}", e.action),
                e.r#type
                    .map(|t| format!("{t:?}"))
                    .unwrap_or_else(|| "_".into()),
            )
        })
        .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut hasher = Sha256::new();
    for (slug, action, ty) in entries {
        hasher.update(slug.as_bytes());
        hasher.update(b"|");
        hasher.update(action.as_bytes());
        hasher.update(b"|");
        hasher.update(ty.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

/// RAII guard around an `fs4` exclusive advisory lock on
/// `.wiki/.bootstrap.lock`.
///
/// Construct with [`BootstrapLock::acquire`]; the lock is released
/// on drop. Re-entrant calls fail fast with an actionable error
/// message rather than racing the state file.
#[derive(Debug)]
pub struct BootstrapLock {
    file: File,
    path: PathBuf,
}

impl BootstrapLock {
    /// Try to acquire the exclusive lock at
    /// `<wiki_root>/.bootstrap.lock`. Fails fast (does NOT block) if
    /// another process holds it.
    pub fn acquire(wiki_root: &Path) -> Result<Self> {
        let path = BootstrapState::lock_path(wiki_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating wiki root: {}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening bootstrap lockfile: {}", path.display()))?;
        // Non-blocking try-lock — if held by another process, abort.
        // fs4 0.13: returns Ok(true) when acquired, Ok(false) when
        // another holder has it (no Err in that case), Err(_) for
        // unrelated I/O failures.
        match file.try_lock_exclusive() {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                anyhow::bail!(
                    "another `coral bootstrap` run holds the lock at {}; \
                     wait for it to finish or remove the lockfile after \
                     confirming no process is running.",
                    path.display()
                );
            }
        }
        // Best-effort: record the PID inside the lockfile for triage.
        // The lock is on the file *itself*, not the contents — fine
        // to truncate + write.
        let mut f = &file;
        let _ = f.set_len(0);
        let _ = f.write_all(format!("{}\n", std::process::id()).as_bytes());
        Ok(Self { file, path })
    }

    /// Path of the lockfile — surfaced for diagnostics / docs.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for BootstrapLock {
    fn drop(&mut self) {
        // Best-effort unlock; OS releases on close anyway.
        let _ = FileExt::unlock(&self.file);
        // Leave the empty lockfile in place — its presence is not
        // harmful (subsequent runs reacquire), and removing it has
        // a small race window where another process could observe
        // the unlock-and-remove pair non-atomically.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::plan::Action;
    use coral_core::frontmatter::PageType;
    use tempfile::TempDir;

    fn sample_plan() -> Vec<PlanEntry> {
        vec![
            PlanEntry {
                slug: "alpha".into(),
                action: Action::Create,
                r#type: Some(PageType::Module),
                confidence: Some(0.7),
                rationale: "first".into(),
                body: None,
            },
            PlanEntry {
                slug: "beta".into(),
                action: Action::Create,
                r#type: Some(PageType::Concept),
                confidence: Some(0.6),
                rationale: "second".into(),
                body: None,
            },
        ]
    }

    /// `load` returns `Ok(None)` when the checkpoint doesn't exist —
    /// the zero-state for a fresh `--apply` invocation.
    #[test]
    fn load_returns_none_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        let res = BootstrapState::load(&wiki).expect("must not error on missing file");
        assert!(res.is_none());
    }

    /// `fresh + save_atomic + load` round-trips byte-equal state.
    #[test]
    fn fresh_save_then_load_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();

        let state = BootstrapState::fresh(sample_plan(), "claude".into(), Some(0.50));
        state.save_atomic(&wiki).unwrap();

        let loaded = BootstrapState::load(&wiki).unwrap().expect("must load");
        assert_eq!(loaded.schema_version, STATE_SCHEMA_VERSION);
        assert_eq!(loaded.provider, "claude");
        assert_eq!(loaded.max_cost_usd, Some(0.50));
        assert_eq!(loaded.pages.len(), 2);
        assert_eq!(loaded.pages[0].slug, "alpha");
        assert_eq!(loaded.pages[0].status, PageStatus::Pending);
        assert_eq!(loaded.plan_fingerprint, state.plan_fingerprint);
    }

    /// FR-ONB-30: hard abort when the persisted schema version is
    /// greater than what this binary understands.
    #[test]
    fn load_hard_aborts_on_future_schema_version() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        let path = BootstrapState::path(&wiki);
        // Hand-craft a state JSON with schema_version = current + 1.
        let bad = serde_json::json!({
            "schema_version": STATE_SCHEMA_VERSION + 1,
            "coral_version": "999.999.999",
            "started_at": "2026-05-12T00:00:00Z",
            "provider": "claude",
            "plan_fingerprint": "deadbeef",
            "plan": [],
            "pages": [],
            "cost_spent_usd": 0.0,
            "partial": false,
        });
        std::fs::write(&path, bad.to_string()).unwrap();

        let err = BootstrapState::load(&wiki).expect_err("must hard-abort");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("schema") && msg.contains("v2"),
            "expected schema-mismatch message, got: {msg}"
        );
    }

    /// FR-ONB-30 soft policy: `coral_version` mismatch logs a warn
    /// to stderr but does NOT abort. Verified by checking that
    /// `load` returns `Ok(Some(_))` even though we hand-crafted a
    /// different `coral_version`.
    #[test]
    fn load_soft_warns_on_coral_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        let path = BootstrapState::path(&wiki);
        let doc = serde_json::json!({
            "schema_version": STATE_SCHEMA_VERSION,
            "coral_version": "0.1.0-prehistoric",
            "started_at": "2026-05-12T00:00:00Z",
            "provider": "claude",
            "plan_fingerprint": "abc",
            "plan": [],
            "pages": [],
            "cost_spent_usd": 0.0,
            "partial": false,
        });
        std::fs::write(&path, doc.to_string()).unwrap();
        let loaded = BootstrapState::load(&wiki).expect("must load despite version skew");
        assert!(loaded.is_some());
    }

    /// `save_atomic` is idempotent — calling it twice with the same
    /// state yields the same bytes on disk.
    #[test]
    fn save_atomic_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        let state = BootstrapState::fresh(sample_plan(), "claude".into(), None);
        state.save_atomic(&wiki).unwrap();
        let first = std::fs::read_to_string(BootstrapState::path(&wiki)).unwrap();
        state.save_atomic(&wiki).unwrap();
        let second = std::fs::read_to_string(BootstrapState::path(&wiki)).unwrap();
        assert_eq!(first, second, "save_atomic must be byte-stable");
    }

    /// State machine: a freshly-built state has every page `Pending`;
    /// once page[0] flips to `Completed`, `next_unfinished_index`
    /// returns `Some(1)`. When all are `Completed`, returns `None`.
    #[test]
    fn next_unfinished_walks_pages_in_order() {
        let mut state = BootstrapState::fresh(sample_plan(), "claude".into(), None);
        assert_eq!(state.next_unfinished_index(), Some(0));
        state.pages[0].status = PageStatus::Completed;
        assert_eq!(state.next_unfinished_index(), Some(1));
        state.pages[1].status = PageStatus::Completed;
        assert_eq!(state.next_unfinished_index(), None);
    }

    /// Failed and InProgress pages are also "unfinished" — resume
    /// retries them.
    #[test]
    fn next_unfinished_includes_failed_and_in_progress() {
        let mut state = BootstrapState::fresh(sample_plan(), "claude".into(), None);
        state.pages[0].status = PageStatus::Completed;
        state.pages[1].status = PageStatus::Failed;
        assert_eq!(state.next_unfinished_index(), Some(1));

        state.pages[1].status = PageStatus::InProgress;
        assert_eq!(state.next_unfinished_index(), Some(1));
    }

    /// FR-ONB-30 lockfile contract: two `BootstrapLock::acquire`
    /// calls on the same wiki root must not both succeed.
    #[test]
    fn bootstrap_lock_rejects_concurrent_acquire() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();

        let _first = BootstrapLock::acquire(&wiki).expect("first acquire");
        let err = BootstrapLock::acquire(&wiki).expect_err("second must fail");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("lock"),
            "expected lock-held error, got: {msg}"
        );
    }

    /// Drop releases the lock; a subsequent acquire works.
    #[test]
    fn bootstrap_lock_releases_on_drop() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();

        {
            let _first = BootstrapLock::acquire(&wiki).expect("first acquire");
        }
        // Lock released — second acquire succeeds.
        let _second = BootstrapLock::acquire(&wiki).expect("second after drop");
    }
}
