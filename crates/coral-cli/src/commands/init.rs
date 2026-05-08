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
}

const SCHEMA_BASE: &str = include_str!("../../../../template/schema/SCHEMA.base.md");

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
    let project_gitignore = cwd.join(".gitignore");
    let session_patterns = [
        ".coral/sessions/*.jsonl",
        ".coral/sessions/*.lock",
        ".coral/sessions/index.json",
        "!.coral/sessions/distilled/",
    ];
    append_gitignore_patterns(&project_gitignore, &session_patterns)
        .with_context(|| format!("updating {}", project_gitignore.display()))?;

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
}
