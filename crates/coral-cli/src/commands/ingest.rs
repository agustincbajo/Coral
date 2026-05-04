use anyhow::{Context, Result};
use clap::Args;
use coral_core::frontmatter::{PageType, Status};
use coral_core::gitdiff;
use coral_core::index::{IndexEntry, WikiIndex};
use coral_core::log::WikiLog;
use coral_core::page::Page;
use coral_runner::{Prompt, Runner};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::plan::{Action, Plan, build_page, page_type_subdir};

#[derive(Args, Debug, Default)]
pub struct IngestArgs {
    /// Override start commit. If not provided, reads `last_commit` from .wiki/index.md.
    #[arg(long)]
    pub from: Option<String>,
    /// Optional model override.
    #[arg(long)]
    pub model: Option<String>,
    /// LLM provider: claude (default) | gemini. Or set CORAL_PROVIDER env.
    #[arg(long)]
    pub provider: Option<String>,
    /// Print the plan without writing pages.
    #[arg(long, conflicts_with = "apply")]
    pub dry_run: bool,
    /// Apply the plan: create / update / retire pages, update the index and append the log.
    #[arg(long)]
    pub apply: bool,
}

pub fn run(args: IngestArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let provider = super::runner_helper::resolve_provider(args.provider.as_deref())
        .map_err(|e| anyhow::anyhow!(e))?;
    let runner = super::runner_helper::make_runner(provider);
    run_with_runner(args, wiki_root, runner.as_ref())
}

pub fn run_with_runner(
    args: IngestArgs,
    wiki_root: Option<&Path>,
    runner: &dyn Runner,
) -> Result<ExitCode> {
    let root: PathBuf = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));
    if !root.exists() {
        anyhow::bail!(
            "wiki root not found: {}. Run `coral init` first.",
            root.display()
        );
    }
    let cwd = std::env::current_dir().context("getting cwd")?;

    let from = match args.from {
        Some(f) => f,
        None => {
            let idx_path = root.join("index.md");
            let idx_content =
                std::fs::read_to_string(&idx_path).context("reading .wiki/index.md")?;
            let idx = WikiIndex::parse(&idx_content)?;
            idx.last_commit
        }
    };
    // Soft-fail: if git is missing or `cwd` isn't a repo, fall back to the
    // literal `"HEAD"` and let downstream `git diff` decide how to behave.
    // Surface the failure as a `WARN` rather than swallowing silently —
    // pre-v0.19.3 the prompt would have ended up with a `from..HEAD` range
    // and an empty diff, and the user would get a confused LLM response
    // with no explanation.
    let head = match gitdiff::head_sha(&cwd) {
        Ok(sha) => sha,
        Err(e) => {
            tracing::warn!(
                error = %e,
                cwd = %cwd.display(),
                "ingest: head_sha failed; range will use the literal `HEAD`"
            );
            "HEAD".to_string()
        }
    };
    let range = format!("{from}..{head}");

    let entries = match gitdiff::run(&cwd, &range) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(
                error = %e,
                range = %range,
                "ingest: gitdiff::run failed; LLM will see an empty diff context"
            );
            Vec::new()
        }
    };
    let summary = entries
        .iter()
        .map(|e| format!("{:?} {}", e.kind, e.path.display()))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt_template = super::prompt_loader::load_or_fallback("ingest", INGEST_SYSTEM_FALLBACK);
    let prompt = Prompt {
        system: Some(prompt_template.content),
        user: format!(
            "Diff range: {range}\n\nChanged files:\n{summary}\n\nWhich pages of the wiki should be created, updated or retired? Output a YAML plan as in the ingest prompt template."
        ),
        model: args.model,
        cwd: None,
        timeout: None,
    };

    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("runner failed: {e}"))?;

    let apply = args.apply;
    let dry_run = args.dry_run || !apply;
    if !args.dry_run && !apply {
        eprintln!(
            "No --dry-run / --apply flag passed; defaulting to --dry-run. Pass --apply to mutate disk.",
        );
    }

    if dry_run {
        println!("# Ingest plan for range {range} (preview)\n");
        println!("{}", out.stdout);
        println!("\n# (run with --apply to mutate pages, update index and append log)");
        return Ok(ExitCode::SUCCESS);
    }

    // Apply path.
    let plan = match Plan::parse(&out.stdout) {
        Ok(p) => p,
        Err(e) => {
            println!("# Raw runner output (failed to parse as YAML):\n");
            println!("{}", out.stdout);
            anyhow::bail!("failed to parse plan: {e}");
        }
    };

    let idx_path = root.join("index.md");

    // Collect per-page IndexEntry rows OUTSIDE the index lock — each
    // page write is still atomic via Page::write() so a partial run
    // leaves consistent on-disk state.
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut retired = 0usize;
    let mut warnings: Vec<String> = Vec::new();
    let mut upserts: Vec<IndexEntry> = Vec::new();

    for entry in &plan.plan {
        match entry.action {
            Action::Create => {
                let page = match build_page(entry, &head, &root) {
                    Ok(p) => p,
                    Err(e) => {
                        warnings.push(format!("create `{}` failed: {e}", entry.slug));
                        continue;
                    }
                };
                page.write()?;
                upserts.push(IndexEntry {
                    slug: page.frontmatter.slug.clone(),
                    page_type: page.frontmatter.page_type,
                    path: relative_path(page.frontmatter.page_type, &page.frontmatter.slug),
                    confidence: page.frontmatter.confidence,
                    status: page.frontmatter.status,
                    last_updated_commit: page.frontmatter.last_updated_commit.clone(),
                });
                created += 1;
            }
            Action::Update => {
                let path = match locate_page(&root, &entry.slug) {
                    Some(p) => p,
                    None => {
                        warnings.push(format!(
                            "update `{}` skipped: page not found in `.wiki/`",
                            entry.slug
                        ));
                        continue;
                    }
                };
                let mut page = Page::from_file(&path)?;
                page.bump_last_commit(head.clone());
                page.write()?;
                upserts.push(IndexEntry {
                    slug: page.frontmatter.slug.clone(),
                    page_type: page.frontmatter.page_type,
                    path: relative_path(page.frontmatter.page_type, &page.frontmatter.slug),
                    confidence: page.frontmatter.confidence,
                    status: page.frontmatter.status,
                    last_updated_commit: page.frontmatter.last_updated_commit.clone(),
                });
                updated += 1;
            }
            Action::Retire => {
                let path = match locate_page(&root, &entry.slug) {
                    Some(p) => p,
                    None => {
                        warnings.push(format!(
                            "retire `{}` skipped: page not found in `.wiki/`",
                            entry.slug
                        ));
                        continue;
                    }
                };
                let mut page = Page::from_file(&path)?;
                page.frontmatter.status = Status::Stale;
                page.write()?;
                upserts.push(IndexEntry {
                    slug: page.frontmatter.slug.clone(),
                    page_type: page.frontmatter.page_type,
                    path: relative_path(page.frontmatter.page_type, &page.frontmatter.slug),
                    confidence: page.frontmatter.confidence,
                    status: page.frontmatter.status,
                    last_updated_commit: page.frontmatter.last_updated_commit.clone(),
                });
                retired += 1;
            }
        }
    }

    // v0.19.5 audit H7: read-modify-write of `.wiki/index.md` MUST
    // happen inside the exclusive flock to avoid a lost-update race
    // when two `coral ingest --apply` invocations interleave.
    // Pre-v0.19.5 the read happened OUTSIDE the lock, the mutation
    // was applied to that stale snapshot, and the write inside the
    // lock clobbered concurrent additions.
    coral_core::atomic::with_exclusive_lock(&idx_path, || {
        let raw =
            std::fs::read_to_string(&idx_path).map_err(|e| coral_core::error::CoralError::Io {
                path: idx_path.clone(),
                source: e,
            })?;
        let mut index = WikiIndex::parse(&raw)?;
        for u in &upserts {
            index.upsert(u.clone());
        }
        index.bump_last_commit(head.clone());
        coral_core::atomic::atomic_write_string(&idx_path, &index.to_string()?)
    })
    .context("writing .wiki/index.md")?;

    let log_path = root.join("log.md");
    let summary = format!("range {range}: {created} created, {updated} updated, {retired} retired");
    // Atomic append — race-free under concurrent invocations (v0.14).
    WikiLog::append_atomic(&log_path, "ingest", &summary)?;

    if !warnings.is_empty() {
        for w in &warnings {
            eprintln!("warn: {w}");
        }
    }
    println!("Ingest applied: {created} created, {updated} updated, {retired} retired.");
    Ok(ExitCode::SUCCESS)
}

fn locate_page(root: &Path, slug: &str) -> Option<PathBuf> {
    // Try every typed subdir; fall back to root.
    for t in [
        PageType::Module,
        PageType::Concept,
        PageType::Entity,
        PageType::Flow,
        PageType::Decision,
        PageType::Synthesis,
        PageType::Operation,
        PageType::Source,
        PageType::Gap,
        PageType::Reference,
    ] {
        let subdir = page_type_subdir(t);
        let candidate = root.join(subdir).join(format!("{slug}.md"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let candidate = root.join(format!("{slug}.md"));
    if candidate.is_file() {
        return Some(candidate);
    }
    None
}

fn relative_path(page_type: PageType, slug: &str) -> String {
    let subdir = page_type_subdir(page_type);
    if subdir == "." {
        format!("{slug}.md")
    } else {
        format!("{subdir}/{slug}.md")
    }
}

const INGEST_SYSTEM_FALLBACK: &str = "You are the Coral wiki bibliotecario. Translate a git diff into a wiki update plan. Output ONLY a YAML plan as in the ingest prompt template (`plan: - {slug, action, type, confidence, rationale, body}`).";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CWD_LOCK;
    use coral_runner::MockRunner;
    use tempfile::TempDir;

    fn write_index(wiki: &Path, last_commit: &str) {
        let idx = format!(
            "---\nlast_commit: {last_commit}\ngenerated_at: 2026-04-30T10:00:00Z\n---\n\n# Wiki index\n\n| Type | Slug | Path | Confidence | Status | Last commit |\n|------|------|------|------------|--------|-------------|\n"
        );
        std::fs::write(wiki.join("index.md"), idx).unwrap();
    }

    fn write_log(wiki: &Path) {
        std::fs::write(
            wiki.join("log.md"),
            "---\ntype: log\n---\n\n# Wiki operation log\n\n",
        )
        .unwrap();
    }

    fn write_module_page(wiki: &Path, slug: &str, status: &str) {
        let modules = wiki.join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        let body = format!(
            "---\nslug: {slug}\ntype: module\nlast_updated_commit: aaa\nconfidence: 0.7\nstatus: {status}\n---\n\n# {slug}\n\nbody.\n"
        );
        std::fs::write(modules.join(format!("{slug}.md")), body).unwrap();
    }

    #[test]
    fn ingest_invokes_runner_with_range() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        write_index(&wiki, "abc");
        write_log(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok(
            "plan:\n  - slug: order\n    action: update\n    rationale: handler signature changed",
        );
        let exit = run_with_runner(
            IngestArgs {
                from: Some("abc".into()),
                dry_run: true,
                ..Default::default()
            },
            Some(wiki.as_path()),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].user.contains("abc.."));
    }

    #[test]
    fn ingest_dry_run_does_not_mutate() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        write_index(&wiki, "abc");
        write_log(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok(
            "plan:\n  - slug: order\n    action: create\n    type: module\n    confidence: 0.6\n    rationale: anchor\n    body: |\n      # Order",
        );
        run_with_runner(
            IngestArgs {
                from: Some("abc".into()),
                dry_run: true,
                ..Default::default()
            },
            Some(wiki.as_path()),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();

        assert!(
            !wiki.join("modules").join("order.md").exists(),
            "dry run must not write pages"
        );
    }

    #[test]
    fn ingest_apply_handles_create_update_retire() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        write_index(&wiki, "abc");
        write_log(&wiki);
        // Pre-existing pages for update + retire.
        write_module_page(&wiki, "existing", "reviewed");
        write_module_page(&wiki, "todrop", "reviewed");
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok(
            "plan:\n  - slug: brandnew\n    action: create\n    type: module\n    confidence: 0.7\n    rationale: new service\n    body: |\n      # brandnew\n  - slug: existing\n    action: update\n    rationale: handler changed\n  - slug: todrop\n    action: retire\n    rationale: removed",
        );
        run_with_runner(
            IngestArgs {
                from: Some("abc".into()),
                apply: true,
                ..Default::default()
            },
            Some(wiki.as_path()),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();

        // Create wrote a new page.
        assert!(wiki.join("modules").join("brandnew.md").exists());

        // Update bumped commit on existing page.
        let existing = std::fs::read_to_string(wiki.join("modules").join("existing.md")).unwrap();
        assert!(
            !existing.contains("last_updated_commit: aaa"),
            "update must bump commit; got {existing}"
        );

        // Retire flipped status to stale.
        let retired = std::fs::read_to_string(wiki.join("modules").join("todrop.md")).unwrap();
        assert!(
            retired.contains("status: stale"),
            "expected stale: {retired}"
        );

        // Log line written.
        let log = std::fs::read_to_string(wiki.join("log.md")).unwrap();
        assert!(log.contains("ingest"), "log missing ingest: {log}");
        assert!(
            log.contains("1 created, 1 updated, 1 retired"),
            "log missing counts: {log}"
        );
    }

    #[test]
    fn ingest_apply_skips_missing_page_for_update() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        write_index(&wiki, "abc");
        write_log(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok("plan:\n  - slug: ghost\n    action: update\n    rationale: nothing here");
        // Should NOT error — just warn and skip.
        run_with_runner(
            IngestArgs {
                from: Some("abc".into()),
                apply: true,
                ..Default::default()
            },
            Some(wiki.as_path()),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();

        let log = std::fs::read_to_string(wiki.join("log.md")).unwrap();
        assert!(
            log.contains("0 created, 0 updated, 0 retired"),
            "log should reflect skip: {log}"
        );
    }
}
