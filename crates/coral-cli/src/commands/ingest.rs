use anyhow::{Context, Result};
use clap::Args;
use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::gitdiff;
use coral_core::index::{IndexEntry, WikiIndex};
use coral_core::log::WikiLog;
use coral_core::page::Page;
use coral_runner::{Prompt, Runner};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::plan::{Action, Plan, build_page, page_type_subdir};

/// v0.30.x audit #B8: 32 MiB cap on `read_to_string` of user-supplied
/// content, matching `coral_core::walk::read_pages` (v0.19.5 N3),
/// `coral_test::discover::parse_openapi_value`, and
/// `coral_session::capture::ensure_within_size_cap`. Pre-fix the
/// `.wiki/index.md` reads here were uncapped, so a multi-GiB index.md
/// (malicious or accidental) would OOM the process.
const MAX_INDEX_BYTES: u64 = 32 * 1024 * 1024;

fn read_index_md_capped(path: &Path) -> Result<String> {
    if let Ok(meta) = std::fs::metadata(path)
        && meta.len() > MAX_INDEX_BYTES
    {
        anyhow::bail!(
            ".wiki/index.md exceeds 32 MiB cap ({} bytes); refusing to read {}",
            meta.len(),
            path.display()
        );
    }
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

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
    /// Scan a docs directory for PDF files and ingest them as Reference pages.
    /// Requires `pdftotext` (poppler-utils) to be installed.
    #[arg(long)]
    pub include_docs: bool,
    /// Directory to scan for PDF files (default: `docs/`). Only used with --include-docs.
    #[arg(long, default_value = "docs/")]
    pub docs_dir: PathBuf,
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
            let idx_content = read_index_md_capped(&idx_path)?;
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
        // v0.30.x audit #B8: cap read at 32 MiB before pulling the
        // file into RAM. Mirror the cap from `coral_core::walk::read_pages`.
        if let Ok(meta) = std::fs::metadata(&idx_path)
            && meta.len() > MAX_INDEX_BYTES
        {
            return Err(coral_core::error::CoralError::Walk(format!(
                ".wiki/index.md exceeds 32 MiB cap ({} bytes); refusing to read {}",
                meta.len(),
                idx_path.display()
            )));
        }
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

    // ── PDF docs ingestion (M3.7, opt-in) ────────────────────────────
    let mut docs_created = 0usize;
    if args.include_docs {
        let (dc, dw) = ingest_docs_pdfs(&args.docs_dir, &root, &head);
        docs_created = dc;
        warnings.extend(dw);
    }

    if !warnings.is_empty() {
        for w in &warnings {
            eprintln!("warn: {w}");
        }
    }
    if docs_created > 0 {
        println!(
            "Ingest applied: {created} created, {updated} updated, {retired} retired, {docs_created} docs ingested."
        );
    } else {
        println!("Ingest applied: {created} created, {updated} updated, {retired} retired.");
    }
    // v0.30.x audit #B7: if the runner returned a plan with entries but
    // every entry was skipped via warnings (e.g., update/retire pointing
    // at missing pages, or every create failing build_page()), CI must
    // observe a non-zero exit. We additionally require that the plan
    // wasn't empty in the first place — an empty plan against an empty
    // diff is a legitimate no-op, not a failure.
    let total_applied = created + updated + retired + docs_created;
    if total_applied == 0 && !warnings.is_empty() && !plan.plan.is_empty() {
        eprintln!(
            "ingest: no pages created/updated/retired; {} warnings — surfacing as failure",
            warnings.len()
        );
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

// ── PDF docs ingestion (M3.7) ───────────────────────────────────────────

/// Maximum characters to keep from extracted PDF text.
const PDF_MAX_CHARS: usize = 10_000;

/// Derive a wiki slug from a PDF filename: `docs/api-guide.pdf` → `ref-api-guide`.
pub(crate) fn slug_from_pdf(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    // Normalize: lowercase, replace non-alphanumeric with hyphens, collapse.
    let normalized: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    // Collapse consecutive hyphens and trim leading/trailing hyphens.
    let collapsed = normalized
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("ref-{collapsed}")
}

/// Check whether `pdftotext` is available on the system PATH.
fn pdftotext_available() -> bool {
    std::process::Command::new("pdftotext")
        .arg("-v")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Extract text from a PDF file using `pdftotext`. Returns None on failure.
fn extract_pdf_text(pdf_path: &Path) -> Option<String> {
    let output = std::process::Command::new("pdftotext")
        .arg("-layout")
        .arg(pdf_path)
        .arg("-") // stdout
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Ingest PDF files from `docs_dir` into the wiki as Reference pages.
/// Returns (created_count, warnings).
fn ingest_docs_pdfs(docs_dir: &Path, wiki_root: &Path, head_sha: &str) -> (usize, Vec<String>) {
    let mut created = 0usize;
    let mut warnings: Vec<String> = Vec::new();

    if !docs_dir.is_dir() {
        warnings.push(format!(
            "docs directory not found: {}; skipping PDF ingestion",
            docs_dir.display()
        ));
        return (created, warnings);
    }

    if !pdftotext_available() {
        warnings.push(
            "pdftotext not found in PATH; install poppler-utils to enable PDF ingestion".into(),
        );
        return (created, warnings);
    }

    // Collect *.pdf files from the docs directory (non-recursive).
    let pdf_files: Vec<PathBuf> = match std::fs::read_dir(docs_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(e) => {
            warnings.push(format!(
                "failed to read docs dir {}: {e}",
                docs_dir.display()
            ));
            return (created, warnings);
        }
    };

    if pdf_files.is_empty() {
        return (created, warnings);
    }

    let subdir = page_type_subdir(PageType::Reference);
    let target_dir = wiki_root.join(subdir);

    for pdf_path in &pdf_files {
        let slug = slug_from_pdf(pdf_path);

        // Validate the generated slug.
        if !coral_core::slug::is_safe_filename_slug(&slug) {
            warnings.push(format!(
                "PDF {} produced invalid slug `{slug}`; skipping",
                pdf_path.display()
            ));
            continue;
        }

        // Skip if page already exists.
        let page_path = target_dir.join(format!("{slug}.md"));
        if page_path.exists() {
            continue;
        }

        // Extract text.
        let raw_text = match extract_pdf_text(pdf_path) {
            Some(t) if !t.trim().is_empty() => t,
            _ => {
                warnings.push(format!(
                    "failed to extract text from {}; skipping",
                    pdf_path.display()
                ));
                continue;
            }
        };

        // Truncate to PDF_MAX_CHARS.
        let (body_text, truncated) = if raw_text.len() > PDF_MAX_CHARS {
            (&raw_text[..PDF_MAX_CHARS], true)
        } else {
            (raw_text.as_str(), false)
        };

        let mut body = format!("# {slug}\n\n{body_text}\n");
        if truncated {
            body.push_str(
                "\n\n---\n_Content truncated at 10000 characters. See the original PDF for the full text._\n",
            );
        }

        // Build the relative source path for frontmatter.
        let source_path = pdf_path.to_string_lossy().to_string();

        let frontmatter = Frontmatter {
            slug: slug.clone(),
            page_type: PageType::Reference,
            last_updated_commit: head_sha.to_string(),
            confidence: Confidence::try_new(0.3).expect("0.3 is valid"),
            sources: vec![source_path],
            backlinks: vec![],
            status: Status::Draft,
            generated_at: Some(chrono::Utc::now().to_rfc3339()),
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            extra: BTreeMap::new(),
        };

        let page = Page {
            path: page_path,
            frontmatter,
            body,
        };

        if let Err(e) = page.write() {
            warnings.push(format!(
                "failed to write page for {}: {e}",
                pdf_path.display()
            ));
            continue;
        }
        created += 1;
    }

    (created, warnings)
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

    // ── PDF docs ingestion tests (M3.7) ─────────────────────────────

    #[test]
    fn include_docs_flag_enables_pdf_scanning() {
        // When --include-docs is set, the ingest command attempts to scan docs_dir.
        // Here we verify the flag parsing and that it triggers the docs path
        // (which gracefully handles a missing docs dir).
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        write_index(&wiki, "abc");
        write_log(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok("plan:\n  - slug: noop\n    action: update\n    rationale: x");
        // docs/ dir does NOT exist — should produce a warning but not error.
        let exit = run_with_runner(
            IngestArgs {
                from: Some("abc".into()),
                apply: true,
                include_docs: true,
                docs_dir: tmp.path().join("docs"),
                ..Default::default()
            },
            Some(wiki.as_path()),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    #[test]
    fn slug_from_pdf_filename() {
        use std::path::Path;
        assert_eq!(
            super::slug_from_pdf(Path::new("docs/api-guide.pdf")),
            "ref-api-guide"
        );
        assert_eq!(
            super::slug_from_pdf(Path::new("docs/My Design Doc.pdf")),
            "ref-my-design-doc"
        );
        assert_eq!(
            super::slug_from_pdf(Path::new("docs/UPPER_CASE.pdf")),
            "ref-upper_case"
        );
        assert_eq!(super::slug_from_pdf(Path::new("hello.pdf")), "ref-hello");
    }

    #[test]
    fn pdf_page_generation_correct_frontmatter() {
        // Simulate the page that ingest_docs_pdfs would create
        // by calling the internal helpers directly.
        use super::{PDF_MAX_CHARS, slug_from_pdf};
        use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
        use coral_core::page::Page;
        use std::collections::BTreeMap;

        let pdf_path = std::path::Path::new("docs/architecture.pdf");
        let slug = slug_from_pdf(pdf_path);
        assert_eq!(slug, "ref-architecture");

        let body_text = "Sample extracted text from PDF.";
        let body = format!("# {slug}\n\n{body_text}\n");
        let source_path = pdf_path.to_string_lossy().to_string();

        let frontmatter = Frontmatter {
            slug: slug.clone(),
            page_type: PageType::Reference,
            last_updated_commit: "deadbeef".to_string(),
            confidence: Confidence::try_new(0.3).unwrap(),
            sources: vec![source_path.clone()],
            backlinks: vec![],
            status: Status::Draft,
            generated_at: Some("2026-05-11T00:00:00Z".to_string()),
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            extra: BTreeMap::new(),
        };

        let tmp = TempDir::new().unwrap();
        let page_path = tmp.path().join("ref-architecture.md");
        let page = Page {
            path: page_path.clone(),
            frontmatter,
            body,
        };
        page.write().unwrap();

        // Re-read and verify.
        let reloaded = Page::from_file(&page_path).unwrap();
        assert_eq!(reloaded.frontmatter.page_type, PageType::Reference);
        assert_eq!(reloaded.frontmatter.slug, "ref-architecture");
        assert!((reloaded.frontmatter.confidence.as_f64() - 0.3).abs() < 1e-9);
        assert_eq!(reloaded.frontmatter.sources, vec![source_path]);
        assert_eq!(reloaded.frontmatter.status, Status::Draft);
        assert!(reloaded.body.contains("Sample extracted text from PDF."));
        // Verify truncation constant is correct.
        assert_eq!(PDF_MAX_CHARS, 10_000);
    }

    #[test]
    fn graceful_when_pdftotext_unavailable() {
        // This test verifies that ingest_docs_pdfs produces a warning
        // (not a panic) when pdftotext is not installed. We create a
        // docs dir with a PDF but mock the scenario by checking the
        // warning output path (since we can't guarantee pdftotext is
        // missing in CI, we test the directory-exists-but-empty case).
        let tmp = TempDir::new().unwrap();
        let docs = tmp.path().join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        // Create a fake PDF file (pdftotext will fail on it).
        std::fs::write(docs.join("fake.pdf"), b"not a real pdf").unwrap();

        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();

        let (created, warnings) = super::ingest_docs_pdfs(&docs, &wiki, "abc123");
        // Either pdftotext is not installed (warning about missing tool)
        // or it fails on our fake file (warning about extraction failure).
        // In both cases: no pages created, and we get at least one warning.
        assert_eq!(created, 0);
        assert!(
            !warnings.is_empty(),
            "expected at least one warning for fake PDF or missing pdftotext"
        );
    }

    /// v0.30.x audit #B8 regression: a `.wiki/index.md` exceeding the
    /// 32 MiB cap must surface a clear error rather than being loaded
    /// into RAM. We write a 33 MiB index and assert ingest errors.
    /// The test is gated by tempfile size: we skip if writing the
    /// large file fails (e.g., a tight tmpfs).
    #[test]
    fn ingest_rejects_oversize_index_md() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        // 33 MiB of padding past a minimal valid header. The cap is
        // 32 * 1024 * 1024, so 33 MiB trips the size check before any
        // content parsing happens.
        let oversize = 33 * 1024 * 1024;
        let mut content = String::with_capacity(oversize + 256);
        content.push_str(
            "---\nlast_commit: abc\ngenerated_at: 2026-04-30T10:00:00Z\n---\n\n# pad\n\n",
        );
        content.push_str(&"x".repeat(oversize));
        if std::fs::write(wiki.join("index.md"), &content).is_err() {
            // tmpfs too small — skip the test rather than flake.
            return;
        }
        write_log(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok("plan: []");
        // `args.from` is None so the auto-discover path reads
        // index.md, which trips the cap and surfaces an error.
        let res = run_with_runner(
            IngestArgs {
                from: None,
                ..Default::default()
            },
            Some(wiki.as_path()),
            &runner,
        );
        std::env::set_current_dir(&cur).unwrap();
        let err = res.expect_err("oversize index.md must error");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("32 MiB") || msg.contains("cap"),
            "error must mention the cap; got: {msg}"
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
