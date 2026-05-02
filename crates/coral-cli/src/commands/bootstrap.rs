use anyhow::{Context, Result};
use clap::Args;
use coral_core::frontmatter::PageType;
use coral_core::gitdiff;
use coral_core::index::{IndexEntry, WikiIndex};
use coral_core::log::WikiLog;
use coral_runner::{Prompt, Runner};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::plan::{Action, Plan, build_page, page_type_subdir};

#[derive(Args, Debug, Default)]
pub struct BootstrapArgs {
    /// Optional model override.
    #[arg(long)]
    pub model: Option<String>,
    /// LLM provider: claude (default) | gemini. Or set CORAL_PROVIDER env.
    #[arg(long)]
    pub provider: Option<String>,
    /// Print the plan without writing pages.
    #[arg(long, conflicts_with = "apply")]
    pub dry_run: bool,
    /// Apply the plan: create the pages, update the index and append the log.
    #[arg(long)]
    pub apply: bool,
}

pub fn run(args: BootstrapArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let provider = super::runner_helper::resolve_provider(args.provider.as_deref())
        .map_err(|e| anyhow::anyhow!(e))?;
    let runner = super::runner_helper::make_runner(provider);
    run_with_runner(args, wiki_root, runner.as_ref())
}

pub fn run_with_runner(
    args: BootstrapArgs,
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

    // Walk repo (exclude .git, .wiki, target, node_modules) to collect file list.
    let cwd = std::env::current_dir().context("getting cwd")?;
    let files = collect_repo_files(&cwd)?;
    let listing = files
        .iter()
        .take(200) // cap to keep prompts bounded
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let prompt_template =
        super::prompt_loader::load_or_fallback("bootstrap", BOOTSTRAP_SYSTEM_FALLBACK);
    let prompt = Prompt {
        system: Some(prompt_template.content),
        user: format!(
            "Repo file listing (truncated to 200):\n{listing}\n\nSuggest 5–15 wiki pages to seed `.wiki/`. Output a YAML plan as in the bootstrap prompt template."
        ),
        model: args.model,
        cwd: None,
        timeout: None,
    };

    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("runner failed: {e}"))?;

    // Resolve mode: dry-run | apply | default (=> dry-run with notice).
    let apply = args.apply;
    let dry_run = args.dry_run || !apply;
    if !args.dry_run && !apply {
        eprintln!(
            "No --dry-run / --apply flag passed; defaulting to --dry-run. Pass --apply to mutate disk.",
        );
    }

    if dry_run {
        println!("# Bootstrap suggestions (review before applying)\n");
        println!("{}", out.stdout);
        println!("\n# (run with --apply to write pages, update index and append log)");
        return Ok(ExitCode::SUCCESS);
    }

    // Apply path: parse → write pages → upsert index → log.
    let plan = match Plan::parse(&out.stdout) {
        Ok(p) => p,
        Err(e) => {
            println!("# Raw runner output (failed to parse as YAML):\n");
            println!("{}", out.stdout);
            anyhow::bail!("failed to parse plan: {e}");
        }
    };

    let head = gitdiff::head_sha(&cwd).unwrap_or_else(|_| "HEAD".to_string());
    let mut created = 0usize;
    let mut skipped: Vec<String> = Vec::new();

    let idx_path = root.join("index.md");
    let idx_content = std::fs::read_to_string(&idx_path).context("reading .wiki/index.md")?;
    let mut index = WikiIndex::parse(&idx_content)?;

    for entry in &plan.plan {
        // Bootstrap assumes `create`; tolerate the field being absent (default Create).
        if entry.action != Action::Create {
            skipped.push(format!(
                "{} (action={:?} not supported in bootstrap)",
                entry.slug, entry.action
            ));
            continue;
        }
        let page = match build_page(entry, &head, &root) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("warn: skipping `{}`: {e}", entry.slug);
                skipped.push(entry.slug.clone());
                continue;
            }
        };
        page.write()?;
        let rel_path =
            page_relative_path(&root, page.frontmatter.page_type, &page.frontmatter.slug);
        index.upsert(IndexEntry {
            slug: page.frontmatter.slug.clone(),
            page_type: page.frontmatter.page_type,
            path: rel_path,
            confidence: page.frontmatter.confidence,
            status: page.frontmatter.status,
            last_updated_commit: page.frontmatter.last_updated_commit.clone(),
        });
        created += 1;
    }

    index.bump_last_commit(head.clone());
    std::fs::write(&idx_path, index.to_string()?).context("writing .wiki/index.md")?;

    // Log line — atomic append, race-free under concurrent invocations (v0.14).
    let log_path = root.join("log.md");
    let summary = if skipped.is_empty() {
        format!("{created} pages created")
    } else {
        format!("{created} pages created, skipped: {}", skipped.join(", "))
    };
    WikiLog::append_atomic(&log_path, "bootstrap", &summary)?;

    println!(
        "Created {created} pages, updated index, appended log entry.{}",
        if skipped.is_empty() {
            String::new()
        } else {
            format!(" Skipped: {}.", skipped.join(", "))
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn page_relative_path(_root: &Path, page_type: PageType, slug: &str) -> String {
    let subdir = page_type_subdir(page_type);
    if subdir == "." {
        format!("{slug}.md")
    } else {
        format!("{subdir}/{slug}.md")
    }
}

const BOOTSTRAP_SYSTEM_FALLBACK: &str = "You are the Coral wiki bibliotecario. Suggest initial wiki pages based on a repo file listing. Output ONLY a YAML plan: see the bootstrap prompt template (`plan: - {slug, type, confidence, rationale, body}`).";

fn collect_repo_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git" | "target" | "node_modules" | ".wiki" | ".idea" | ".vscode"
            )
        })
    {
        let entry = entry.context("walking repo")?;
        if entry.file_type().is_file() {
            files.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|_| entry.path().to_path_buf()),
            );
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CWD_LOCK;
    use coral_runner::MockRunner;
    use tempfile::TempDir;

    fn seed_wiki_with_index(root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        let idx = "---\nlast_commit: zero\ngenerated_at: 2026-04-30T10:00:00Z\n---\n\n# Wiki index\n\n| Type | Slug | Path | Confidence | Status | Last commit |\n|------|------|------|------------|--------|-------------|\n";
        std::fs::write(root.join("index.md"), idx).unwrap();
        std::fs::write(
            root.join("log.md"),
            "---\ntype: log\n---\n\n# Wiki operation log\n\n",
        )
        .unwrap();
    }

    #[test]
    fn bootstrap_invokes_runner_with_file_listing() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::fs::write(tmp.path().join("README.md"), "# repo").unwrap();
        std::fs::write(tmp.path().join("src.rs"), "// code").unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok(
            "plan:\n  - slug: readme\n    type: source\n    confidence: 0.6\n    rationale: top-level overview\n    body: |\n      # readme",
        );
        let exit = run_with_runner(
            BootstrapArgs {
                dry_run: true,
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].user.contains("README.md") || calls[0].user.contains("src.rs"));
    }

    #[test]
    fn bootstrap_dry_run_does_not_mutate() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok(
            "plan:\n  - slug: order\n    type: module\n    confidence: 0.7\n    rationale: anchor\n    body: |\n      # Order",
        );
        run_with_runner(
            BootstrapArgs {
                dry_run: true,
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();
        // No `modules/` dir should have been created and no order.md exists.
        assert!(
            !wiki.join("modules").join("order.md").exists(),
            "dry run must not write pages"
        );
    }

    #[test]
    fn bootstrap_apply_writes_pages() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok(
            "plan:\n  - slug: order\n    type: module\n    confidence: 0.7\n    rationale: anchor\n    body: |\n      # Order\n\n      Body.\n  - slug: outbox\n    type: concept\n    confidence: 0.6\n    rationale: pattern\n    body: |\n      # Outbox\n\n      Body.\n",
        );
        run_with_runner(
            BootstrapArgs {
                apply: true,
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();

        assert!(wiki.join("modules").join("order.md").exists());
        assert!(wiki.join("concepts").join("outbox.md").exists());

        // Index.md mentions both slugs.
        let idx = std::fs::read_to_string(wiki.join("index.md")).unwrap();
        assert!(idx.contains("order"), "index missing order: {idx}");
        assert!(idx.contains("outbox"), "index missing outbox: {idx}");

        // Log.md has a fresh entry.
        let log = std::fs::read_to_string(wiki.join("log.md")).unwrap();
        assert!(
            log.contains("bootstrap"),
            "log missing bootstrap entry: {log}"
        );
        assert!(log.contains("2 pages created"), "log missing count: {log}");
    }

    #[test]
    fn bootstrap_apply_handles_malformed_yaml() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok("not yaml at all");
        let res = run_with_runner(
            BootstrapArgs {
                apply: true,
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        );
        std::env::set_current_dir(&cur).unwrap();
        assert!(res.is_err(), "malformed YAML must surface an error");
    }
}
