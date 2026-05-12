use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use coral_core::gitdiff;
use coral_core::index::WikiIndex;
use coral_core::log::WikiLog;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct InitArgs {
    /// Force overwrite of an existing .wiki/ (DESTRUCTIVE — re-creates index/log).
    #[arg(long)]
    pub force: bool,
    /// Auto-accept the CLAUDE.md append even when the existing file is
    /// already long (> 150 lines). Without this, `coral init` defaults
    /// to skipping the append and tells the user to use the
    /// `/coral:coral-doctor` slash command as a routing fallback.
    /// See FR-ONB-25 + R12 in PRD v1.4.
    #[arg(long)]
    pub yes: bool,
}

const SCHEMA_BASE: &str = include_str!("../../../../template/schema/SCHEMA.base.md");
const CLAUDE_MD_TEMPLATE: &str = include_str!("../../../../template/CLAUDE.md.tmpl");

/// FR-ONB-25: When the existing `CLAUDE.md` is already past this many
/// lines, appending our ~20-line routing block risks pushing the
/// document over the 200-line "adherence cliff" Anthropic documents
/// in `code.claude.com/docs/en/memory`. We warn + default to skip.
const CLAUDE_MD_SIZE_GUARD_LINES: usize = 150;

pub fn run(args: InitArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let root: PathBuf = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));
    let cwd = std::env::current_dir().context("getting cwd")?;

    if root.exists() && !args.force {
        let schema = root.join("SCHEMA.md");
        if schema.exists() {
            tracing::info!("`.wiki/` already exists; pass --force to re-create. Skipping.");
            return Ok(ExitCode::SUCCESS);
        }
    }

    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;

    // SCHEMA.md — embedded base.
    let schema_path = root.join("SCHEMA.md");
    if !schema_path.exists() || args.force {
        std::fs::write(&schema_path, SCHEMA_BASE)
            .with_context(|| format!("writing {}", schema_path.display()))?;
        tracing::info!(path = %schema_path.display(), "wrote SCHEMA.md");
    }

    // index.md — bootstrap with current HEAD or zeros.
    let index_path = root.join("index.md");
    if !index_path.exists() || args.force {
        let head = gitdiff::head_sha(&cwd)
            .unwrap_or_else(|_| "0000000000000000000000000000000000000000".into());
        let mut idx = WikiIndex::new(head);
        idx.generated_at = Utc::now();
        coral_core::atomic::atomic_write_string(&index_path, &idx.to_string()?)
            .with_context(|| format!("writing {}", index_path.display()))?;
        tracing::info!(path = %index_path.display(), "wrote index.md");
    }

    // log.md — append-only operation log seeded with the init event.
    // `--force` truncates first so the seed entry is the only line.
    let log_path = root.join("log.md");
    if !log_path.exists() || args.force {
        if args.force && log_path.exists() {
            std::fs::remove_file(&log_path)
                .with_context(|| format!("removing {} for --force re-init", log_path.display()))?;
        }
        WikiLog::append_atomic(&log_path, "init", "wiki initialized")
            .with_context(|| format!("writing {}", log_path.display()))?;
        tracing::info!(path = %log_path.display(), "wrote log.md");
    }

    // .gitignore — keep generated artifacts out of git. Idempotent: when
    // the file is missing we write all entries; when the user already
    // manages a .gitignore, we append any missing entries without
    // touching unrelated lines.
    //
    // v0.19.8 #32 (tracked deferral): `with_exclusive_lock` leaves
    // zero-byte sentinel files at `<path>.lock` after release. The
    // safe cleanup approach (unlink-after-release) breaks the
    // cross-process flock contract by detaching the inode while a
    // peer process holds the FD; documented in `atomic.rs`. Live with
    // the litter, ignore it in git so users don't accidentally commit
    // it. Patterns: `*.lock` covers single-suffix sentinels (e.g.
    // `coral.toml.lock`); `*.lock.lock` is also added explicitly so
    // a future user-managed `.gitignore` that drops `*.lock` still
    // hides the doubled form from `git status`.
    let gitignore_path = root.join(".gitignore");
    let needed = [
        ".coral-cache.json",
        ".coral-embeddings.json",
        "*.lock",
        "*.lock.lock",
    ];
    if !gitignore_path.exists() {
        let mut content = String::new();
        for entry in &needed {
            content.push_str(entry);
            content.push('\n');
        }
        std::fs::write(&gitignore_path, content)
            .with_context(|| format!("writing {}", gitignore_path.display()))?;
        tracing::info!(path = %gitignore_path.display(), "wrote .gitignore");
    } else {
        let mut existing = std::fs::read_to_string(&gitignore_path)
            .with_context(|| format!("reading {}", gitignore_path.display()))?;
        let mut changed = false;
        for entry in needed {
            let already_listed = existing.lines().any(|line| line.trim() == entry);
            if !already_listed {
                if !existing.is_empty() && !existing.ends_with('\n') {
                    existing.push('\n');
                }
                existing.push_str(entry);
                existing.push('\n');
                changed = true;
            }
        }
        if changed {
            std::fs::write(&gitignore_path, existing)
                .with_context(|| format!("updating {}", gitignore_path.display()))?;
            tracing::info!(path = %gitignore_path.display(), "appended to .gitignore");
        }
    }

    // Subdirectories so the structure exists from day 1.
    for sub in &[
        "modules",
        "concepts",
        "entities",
        "flows",
        "decisions",
        "synthesis",
        "operations",
        "sources",
        "gaps",
    ] {
        std::fs::create_dir_all(root.join(sub))?;
    }

    // v0.20.0 — session captures land at `<project_root>/.coral/sessions/`,
    // OUTSIDE `.wiki/`. The wiki-level `.gitignore` we just wrote can't cover
    // them. Touch the project-root `.gitignore` (idempotent: only append
    // patterns that aren't already listed). This is the v0.20 PRD design Q1
    // answer: "gitignored by default; curated distillations under
    // `.coral/sessions/distilled/` live with the rest of the wiki and are
    // explicitly NOT gitignored."
    //
    // Patterns:
    //   .coral/sessions/*.jsonl        — raw transcripts (PII-rich, never commit)
    //   .coral/sessions/*.lock         — flock sentinels left by capture
    //   .coral/sessions/index.json     — local-only session metadata
    //   !.coral/sessions/distilled/    — curated distillations DO ship in git
    //
    // v0.34.0 FR-ONB-34 (security-critical): also block `.coral/`
    // catch-all + the bootstrap state file. Without these, a user
    // running `git add .` after `coral init` would commit their
    // Anthropic API key (in `.coral/config.toml`) and the bootstrap
    // checkpoint (which may include LLM-token usage metadata). The
    // catch-all comes AFTER the per-pattern allowlist so the
    // negation `!.coral/sessions/distilled/` still wins.
    let project_gitignore = cwd.join(".gitignore");
    let session_patterns = [
        ".coral/sessions/*.jsonl",
        ".coral/sessions/*.lock",
        ".coral/sessions/index.json",
        "!.coral/sessions/distilled/",
        // FR-ONB-34: security-critical entries
        ".coral/",
        ".wiki/.bootstrap-state.json",
        ".wiki/.bootstrap.lock",
    ];
    append_gitignore_patterns(&project_gitignore, &session_patterns)
        .with_context(|| format!("updating {}", project_gitignore.display()))?;

    // FR-ONB-25: generate / append-safe CLAUDE.md with size guard.
    // The template documents the Coral routing block; we never
    // overwrite a user's existing content. See `apply_claude_md` for
    // the full decision tree.
    let claude_md_path = cwd.join("CLAUDE.md");
    let outcome = apply_claude_md(&claude_md_path, CLAUDE_MD_TEMPLATE, args.yes)?;
    match outcome {
        ClaudeMdOutcome::Created => {
            tracing::info!(path = %claude_md_path.display(), "wrote CLAUDE.md (new)");
        }
        ClaudeMdOutcome::Appended => {
            tracing::info!(path = %claude_md_path.display(), "appended Coral routing to CLAUDE.md");
        }
        ClaudeMdOutcome::AlreadyHasSection => {
            tracing::info!(
                path = %claude_md_path.display(),
                "CLAUDE.md already has `## Coral routing`; nothing to do"
            );
        }
        ClaudeMdOutcome::SkippedTooLong => {
            eprintln!(
                "warning: existing CLAUDE.md is long ({}+ lines); skipped Coral routing append. \
                 Use /coral:coral-doctor as a deterministic routing fallback, or re-run with `--yes` to force.",
                CLAUDE_MD_SIZE_GUARD_LINES
            );
        }
    }

    println!("✔ `.wiki/` initialized at {}", root.display());
    Ok(ExitCode::SUCCESS)
}

/// Idempotent append of `.gitignore` patterns. If the file doesn't
/// exist yet, creates it with just the requested entries. If it
/// does, appends only the entries not already present (line-exact
/// match, whitespace-trimmed). No-ops when every pattern is already
/// listed. Public-but-not-doc-published so the test module can
/// re-use it.
pub(crate) fn append_gitignore_patterns(path: &Path, patterns: &[&str]) -> std::io::Result<bool> {
    if !path.exists() {
        let mut content = String::new();
        for entry in patterns {
            content.push_str(entry);
            content.push('\n');
        }
        std::fs::write(path, content)?;
        return Ok(true);
    }
    let mut existing = std::fs::read_to_string(path)?;
    let mut changed = false;
    for entry in patterns {
        let already_listed = existing.lines().any(|line| line.trim() == *entry);
        if !already_listed {
            if !existing.is_empty() && !existing.ends_with('\n') {
                existing.push('\n');
            }
            existing.push_str(entry);
            existing.push('\n');
            changed = true;
        }
    }
    if changed {
        std::fs::write(path, existing)?;
    }
    Ok(changed)
}

/// Result of the `CLAUDE.md` handler. Surfaced so tests can assert
/// the right branch fires without inspecting filesystem state.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClaudeMdOutcome {
    /// No existing file — we wrote the full template.
    Created,
    /// File existed without the section + size guard passed — we
    /// appended the template.
    Appended,
    /// File existed with `## Coral routing` already — no-op.
    AlreadyHasSection,
    /// File existed without the section, but it was already too long
    /// (> CLAUDE_MD_SIZE_GUARD_LINES) and `--yes` was NOT passed.
    SkippedTooLong,
}

/// FR-ONB-25 implementation. See [`ClaudeMdOutcome`] for the branches.
///
/// Decision tree:
/// 1. If file is missing → write the full template (Created).
/// 2. If file contains `^## Coral routing` (case-insensitive at line
///    start) → no-op (AlreadyHasSection). We NEVER overwrite the
///    user's content.
/// 3. If file lacks the section and is short (<= 150 lines) → append
///    the template after a blank line (Appended).
/// 4. If file lacks the section and is long (> 150 lines):
///    a. If `yes_flag` is set → force-append (Appended).
///    b. Otherwise → SkippedTooLong (caller prints the warning).
pub(crate) fn apply_claude_md(
    path: &Path,
    template: &str,
    yes_flag: bool,
) -> std::io::Result<ClaudeMdOutcome> {
    if !path.exists() {
        std::fs::write(path, template)?;
        return Ok(ClaudeMdOutcome::Created);
    }
    let existing = std::fs::read_to_string(path)?;
    if claude_md_has_routing_section(&existing) {
        return Ok(ClaudeMdOutcome::AlreadyHasSection);
    }
    let line_count = existing.lines().count();
    if line_count > CLAUDE_MD_SIZE_GUARD_LINES && !yes_flag {
        return Ok(ClaudeMdOutcome::SkippedTooLong);
    }
    // Safe append: ensure trailing newline + a separating blank.
    let mut next = existing.clone();
    if !next.ends_with('\n') {
        next.push('\n');
    }
    next.push('\n');
    next.push_str(template);
    std::fs::write(path, next)?;
    Ok(ClaudeMdOutcome::Appended)
}

/// `^## Coral routing` (case-insensitive). Same logic the
/// `coral self-check` probe uses — kept in sync so the size-guard
/// decision matches what the SessionStart hook sees.
fn claude_md_has_routing_section(raw: &str) -> bool {
    let needle = "## coral routing";
    raw.lines()
        .any(|line| line.to_ascii_lowercase().trim_start().starts_with(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// `append_gitignore_patterns` creates the file when missing.
    #[test]
    fn append_gitignore_creates_when_missing() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join(".gitignore");
        let changed = append_gitignore_patterns(&p, &[".coral/sessions/*.jsonl"]).unwrap();
        assert!(changed);
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(body.contains(".coral/sessions/*.jsonl"));
    }

    /// Repeated calls are idempotent.
    #[test]
    fn append_gitignore_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join(".gitignore");
        let _ = append_gitignore_patterns(&p, &[".coral/sessions/*.jsonl"]).unwrap();
        let changed_again = append_gitignore_patterns(&p, &[".coral/sessions/*.jsonl"]).unwrap();
        assert!(!changed_again, "second call must be a no-op");
    }

    /// Existing user .gitignore is preserved; new patterns appended.
    #[test]
    fn append_gitignore_preserves_existing_lines() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join(".gitignore");
        std::fs::write(&p, "node_modules/\n*.log\n").unwrap();
        let _ = append_gitignore_patterns(&p, &[".coral/sessions/*.jsonl"]).unwrap();
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(body.contains("node_modules/"), "must preserve user lines");
        assert!(body.contains("*.log"));
        assert!(body.contains(".coral/sessions/*.jsonl"));
    }

    /// The session-pattern set includes the `!.coral/sessions/distilled/`
    /// negation so curated output stays in git.
    #[test]
    fn append_gitignore_negation_included_in_session_patterns() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join(".gitignore");
        let session_patterns = [
            ".coral/sessions/*.jsonl",
            ".coral/sessions/*.lock",
            ".coral/sessions/index.json",
            "!.coral/sessions/distilled/",
        ];
        let _ = append_gitignore_patterns(&p, &session_patterns).unwrap();
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(
            body.contains("!.coral/sessions/distilled/"),
            "negation pattern missing: {body}"
        );
    }

    // ------------------------------------------------------------------
    // FR-ONB-34: .gitignore security-critical entries
    // ------------------------------------------------------------------

    /// New repo: `.gitignore` is created with the security-critical
    /// `.coral/` + `.wiki/.bootstrap-*` entries.
    #[test]
    fn fr_onb_34_gitignore_created_with_security_entries() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join(".gitignore");
        let security_patterns = [
            ".coral/",
            ".wiki/.bootstrap-state.json",
            ".wiki/.bootstrap.lock",
        ];
        let changed = append_gitignore_patterns(&p, &security_patterns).unwrap();
        assert!(changed);
        let body = std::fs::read_to_string(&p).unwrap();
        for pat in security_patterns {
            assert!(body.contains(pat), ".gitignore must contain `{pat}`");
        }
    }

    /// Existing .gitignore: security-critical entries appended without
    /// touching user content.
    #[test]
    fn fr_onb_34_gitignore_appends_security_entries() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join(".gitignore");
        std::fs::write(&p, "target/\nnode_modules/\n").unwrap();
        let _ = append_gitignore_patterns(
            &p,
            &[
                ".coral/",
                ".wiki/.bootstrap-state.json",
                ".wiki/.bootstrap.lock",
            ],
        )
        .unwrap();
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(body.contains("target/"));
        assert!(body.contains("node_modules/"));
        assert!(body.contains(".coral/"));
        assert!(body.contains(".wiki/.bootstrap-state.json"));
    }

    /// Second call is a no-op (entries already present).
    #[test]
    fn fr_onb_34_gitignore_security_entries_idempotent() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join(".gitignore");
        let security_patterns = [
            ".coral/",
            ".wiki/.bootstrap-state.json",
            ".wiki/.bootstrap.lock",
        ];
        let _ = append_gitignore_patterns(&p, &security_patterns).unwrap();
        let changed_again = append_gitignore_patterns(&p, &security_patterns).unwrap();
        assert!(!changed_again, "second append must be a no-op");
    }

    /// We NEVER remove user entries — even ones that look like they
    /// belong to Coral. The append-only contract is load-bearing.
    #[test]
    fn fr_onb_34_gitignore_never_removes_user_entries() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join(".gitignore");
        // User wrote their own .coral/ pattern with a trailing comment.
        std::fs::write(&p, ".coral/sessions/important.jsonl\n# my notes\n").unwrap();
        let _ = append_gitignore_patterns(&p, &[".coral/"]).unwrap();
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(
            body.contains(".coral/sessions/important.jsonl"),
            "user line must survive"
        );
        assert!(body.contains("# my notes"), "user comment must survive");
        assert!(body.contains(".coral/"), "our pattern must be appended");
    }

    // ------------------------------------------------------------------
    // FR-ONB-25: CLAUDE.md append-safe + size guard
    // ------------------------------------------------------------------

    /// CLAUDE.md absent → write the full template.
    #[test]
    fn fr_onb_25_claude_md_created_when_missing() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("CLAUDE.md");
        let outcome = apply_claude_md(&p, CLAUDE_MD_TEMPLATE, false).unwrap();
        assert_eq!(outcome, ClaudeMdOutcome::Created);
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(body.contains("## Coral routing"));
        // Sanity: the fallback line FR-ONB-25 promises is present.
        assert!(body.contains("/coral:coral-doctor"));
    }

    /// CLAUDE.md short + no section → append.
    #[test]
    fn fr_onb_25_claude_md_appended_when_short_and_missing_section() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("CLAUDE.md");
        std::fs::write(&p, "# Project rules\n\nUse 4-space indentation.\n").unwrap();
        let outcome = apply_claude_md(&p, CLAUDE_MD_TEMPLATE, false).unwrap();
        assert_eq!(outcome, ClaudeMdOutcome::Appended);
        let body = std::fs::read_to_string(&p).unwrap();
        // User content survives.
        assert!(body.contains("Use 4-space indentation."));
        assert!(body.contains("## Coral routing"));
        // The user's existing block precedes our append.
        let user_idx = body.find("Use 4-space indentation.").unwrap();
        let routing_idx = body.find("## Coral routing").unwrap();
        assert!(user_idx < routing_idx, "user content must come first");
    }

    /// CLAUDE.md long + no section + no --yes → skip + warn.
    #[test]
    fn fr_onb_25_claude_md_skipped_when_long_and_no_yes() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("CLAUDE.md");
        let long = "line\n".repeat(CLAUDE_MD_SIZE_GUARD_LINES + 10);
        std::fs::write(&p, &long).unwrap();
        let outcome = apply_claude_md(&p, CLAUDE_MD_TEMPLATE, false).unwrap();
        assert_eq!(outcome, ClaudeMdOutcome::SkippedTooLong);
        // File on disk is unchanged.
        let body = std::fs::read_to_string(&p).unwrap();
        assert_eq!(body, long);
        assert!(
            !body.contains("## Coral routing"),
            "skip path must NOT append"
        );
    }

    /// CLAUDE.md long + no section + --yes → force append.
    #[test]
    fn fr_onb_25_claude_md_force_append_with_yes() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("CLAUDE.md");
        let long = "line\n".repeat(CLAUDE_MD_SIZE_GUARD_LINES + 10);
        std::fs::write(&p, &long).unwrap();
        let outcome = apply_claude_md(&p, CLAUDE_MD_TEMPLATE, true).unwrap();
        assert_eq!(outcome, ClaudeMdOutcome::Appended);
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(body.contains("## Coral routing"));
        // User content survives.
        assert!(body.starts_with("line\n"));
    }

    /// CLAUDE.md already has section → no-op regardless of length or
    /// `--yes` flag. We never re-append the same section.
    #[test]
    fn fr_onb_25_claude_md_noop_when_section_present() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("CLAUDE.md");
        let original = "# Notes\n\n## Coral routing\n\nalready here\n";
        std::fs::write(&p, original).unwrap();
        let outcome = apply_claude_md(&p, CLAUDE_MD_TEMPLATE, false).unwrap();
        assert_eq!(outcome, ClaudeMdOutcome::AlreadyHasSection);
        // File on disk unchanged byte-for-byte.
        let body = std::fs::read_to_string(&p).unwrap();
        assert_eq!(body, original);

        // And with --yes it's still a no-op.
        let outcome = apply_claude_md(&p, CLAUDE_MD_TEMPLATE, true).unwrap();
        assert_eq!(outcome, ClaudeMdOutcome::AlreadyHasSection);
        let body = std::fs::read_to_string(&p).unwrap();
        assert_eq!(body, original);
    }

    /// We NEVER overwrite the user's content — only ever append or
    /// no-op. This test pathologically tries an "append" against a
    /// short file and confirms the leading content is byte-stable.
    #[test]
    fn fr_onb_25_claude_md_never_overwrites_user_content() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("CLAUDE.md");
        let user = "USER WRITTEN CONTENT line 1\nUSER WRITTEN CONTENT line 2\n";
        std::fs::write(&p, user).unwrap();
        let _ = apply_claude_md(&p, CLAUDE_MD_TEMPLATE, false).unwrap();
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(
            body.starts_with(user),
            "user content must be the prefix of the post-append file:\n{body}"
        );
    }

    /// Case-insensitive match: `## CORAL ROUTING`, `## Coral Routing`,
    /// `##   coral routing` (with leading whitespace) all detected.
    #[test]
    fn fr_onb_25_section_detection_is_case_insensitive_and_whitespace_tolerant() {
        assert!(claude_md_has_routing_section("## Coral routing\n"));
        assert!(claude_md_has_routing_section("## CORAL ROUTING\n"));
        assert!(claude_md_has_routing_section("## coral routing\n"));
        assert!(claude_md_has_routing_section("   ## Coral Routing\n"));
        assert!(!claude_md_has_routing_section("# Coral routing\n"));
        assert!(!claude_md_has_routing_section("Just text.\n"));
    }
}
