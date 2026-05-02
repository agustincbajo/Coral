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

    // .gitignore — keep `.coral-cache.json` and `.coral-embeddings.json` out of
    // git. Idempotent: when the file is missing we write both lines; when the
    // user already manages a .gitignore, we append any missing entries without
    // touching unrelated lines.
    let gitignore_path = root.join(".gitignore");
    let needed = [".coral-cache.json", ".coral-embeddings.json"];
    if !gitignore_path.exists() {
        let content = format!("{}\n{}\n", needed[0], needed[1]);
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

    println!("✔ `.wiki/` initialized at {}", root.display());
    Ok(ExitCode::SUCCESS)
}
