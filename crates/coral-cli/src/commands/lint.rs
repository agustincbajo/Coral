use anyhow::{Context, Result};
use clap::Args;
use coral_core::walk;
use coral_lint::{
    LintReport, run_structural_with_root,
    semantic::{SEMANTIC_SYSTEM_PROMPT, check_semantic_with_prompt},
};
use coral_runner::Runner;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct LintArgs {
    /// Run structural checks (links, frontmatter, orphans, confidence). Default: on.
    #[arg(long)]
    pub structural: bool,
    /// Run semantic checks (LLM-based). Stub in v0.1.
    #[arg(long)]
    pub semantic: bool,
    /// Run all checks (default if no flag is passed).
    #[arg(long)]
    pub all: bool,
    /// Output format: markdown (default) or json.
    #[arg(long, default_value = "markdown")]
    pub format: String,
    /// LLM provider used by --semantic: claude (default) | gemini. Or set CORAL_PROVIDER env.
    #[arg(long)]
    pub provider: Option<String>,
    /// Pre-commit-hook mode: load every page (so the graph stays intact for
    /// orphan / wikilink checks) but filter the report down to issues whose
    /// `page` field is in `git diff --cached --name-only`. Workspace-level
    /// issues (no `page`) are kept. Exit non-zero only if a critical issue
    /// touches a staged file.
    #[arg(long)]
    pub staged: bool,
    /// LLM-driven auto-fix: after structural lint runs, ask the runner to
    /// propose fixes (downgrade confidence, mark stale, add `_archive_`
    /// note, suggest source paths). Default: dry-run prints the YAML plan.
    /// Pass `--apply` to write changes back. Requires LLM auth.
    #[arg(long)]
    pub auto_fix: bool,
    /// With `--auto-fix`, write the proposed plan back to the wiki. Without
    /// this, `--auto-fix` is a preview only (matches `bootstrap` /
    /// `ingest` semantics).
    #[arg(long)]
    pub apply: bool,
}

pub fn run(args: LintArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let provider = super::runner_helper::resolve_provider(args.provider.as_deref())
        .map_err(|e| anyhow::anyhow!(e))?;
    let runner = super::runner_helper::make_runner(provider);
    run_with_runner(args, wiki_root, runner.as_ref())
}

pub fn run_with_runner(
    args: LintArgs,
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

    let pages = walk::read_pages(&root)
        .with_context(|| format!("reading pages from {}", root.display()))?;

    // If no flag is passed, run structural by default.
    let do_structural = args.structural || args.all || !args.semantic;
    let do_semantic = args.semantic || args.all;

    let mut issues = Vec::new();
    if do_structural {
        // The repo root is the parent of `.wiki/` — the context-aware
        // structural checks (commit-in-git, source-exists) need this to
        // shell out to `git` and to resolve `sources:` paths against the
        // workspace, not against `.wiki/`.
        let repo_root: PathBuf = root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let r = run_structural_with_root(&pages, &repo_root);
        issues.extend(r.issues);
    }
    if do_semantic {
        let prompt_template =
            super::prompt_loader::load_or_fallback("lint-semantic", SEMANTIC_SYSTEM_PROMPT);
        let semantic_issues = check_semantic_with_prompt(&pages, runner, &prompt_template.content);
        issues.extend(semantic_issues);
    }

    if args.staged {
        let cwd = std::env::current_dir().context("getting cwd")?;
        let staged = staged_wiki_paths(&cwd).context("listing staged files via git")?;
        let before = issues.len();
        issues = filter_issues_by_paths(issues, &staged);
        tracing::info!(
            staged_paths = staged.len(),
            kept = issues.len(),
            dropped = before - issues.len(),
            "lint --staged: filtered to issues touching staged paths"
        );
    }

    let report = LintReport::from_issues(issues);

    match args.format.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&report)?),
        _ => println!("{}", report.as_markdown()),
    }

    if args.auto_fix && !report.issues.is_empty() {
        run_auto_fix(&pages, &report, runner, args.apply, &root)?;
    }

    if report.critical_count() > 0 {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

const AUTO_FIX_SYSTEM_FALLBACK: &str = "You are the Coral wiki linter in auto-fix mode. \
For each lint issue listed below, propose the smallest semantic fix on the affected page: \
downgrade `confidence`, set `status` to `draft` or `stale`, append a `_(stale because …)_` \
italic note to the body, or suggest concrete `sources:` paths from the workspace. \
Do NOT rewrite whole bodies. Do NOT invent sources. Output ONLY a YAML document of the form:\n\
```yaml\n\
fixes:\n\
  - slug: <existing slug>\n\
    action: update | retire | skip\n\
    confidence: 0.5         # optional, only when changed\n\
    status: draft           # optional, only when changed\n\
    body_append: |          # optional; appended verbatim with two leading newlines\n\
      _Stale: …_\n\
    rationale: <one short sentence>\n\
```\n\
Skip with action=skip + rationale when the issue needs human judgment.";

fn run_auto_fix(
    pages: &[coral_core::page::Page],
    report: &LintReport,
    runner: &dyn Runner,
    apply: bool,
    wiki_root: &Path,
) -> Result<()> {
    use coral_runner::Prompt;

    let issues_summary = render_issues_for_prompt(report);
    let pages_summary = render_pages_for_prompt(pages, &affected_slugs(report, pages));
    let prompt_template =
        super::prompt_loader::load_or_fallback("lint-auto-fix", AUTO_FIX_SYSTEM_FALLBACK);
    let prompt = Prompt {
        system: Some(prompt_template.content),
        user: format!(
            "Lint issues:\n{issues_summary}\n\nAffected pages (slug, type, status, confidence, body excerpt):\n{pages_summary}\n\nPropose fixes."
        ),
        ..Default::default()
    };

    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("auto-fix runner failed: {e}"))?;
    let plan = parse_auto_fix_plan(&out.stdout).context("parsing auto-fix YAML plan")?;

    if !apply {
        println!("\n## Auto-fix proposal (dry-run)\n");
        println!("```yaml\n{}\n```", out.stdout.trim());
        println!("Pass `--apply` to write {} fix(es).", plan.fixes.len());
        return Ok(());
    }

    let written = apply_auto_fix_plan(&plan, pages, wiki_root)?;
    println!("\n## Auto-fix applied\n");
    println!("Updated {written} page(s).");
    Ok(())
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub(crate) struct AutoFixPlan {
    #[serde(default)]
    pub fixes: Vec<AutoFixEntry>,
}

#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub(crate) struct AutoFixEntry {
    pub slug: String,
    #[serde(default = "default_action")]
    pub action: AutoFixAction,
    pub confidence: Option<f64>,
    pub status: Option<String>,
    pub body_append: Option<String>,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AutoFixAction {
    Update,
    Retire,
    Skip,
}

fn default_action() -> AutoFixAction {
    AutoFixAction::Skip
}

pub(crate) fn parse_auto_fix_plan(stdout: &str) -> Result<AutoFixPlan> {
    let trimmed = strip_yaml_fence(stdout);
    Ok(serde_yaml_ng::from_str(trimmed)?)
}

fn strip_yaml_fence(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s
        .strip_prefix("```yaml\n")
        .or_else(|| s.strip_prefix("```\n"))
    {
        if let Some(end) = rest.rfind("```") {
            return rest[..end].trim_end();
        }
        return rest;
    }
    s
}

pub(crate) fn apply_auto_fix_plan(
    plan: &AutoFixPlan,
    pages: &[coral_core::page::Page],
    _wiki_root: &Path,
) -> Result<usize> {
    use coral_core::frontmatter::{Confidence, Status};
    use coral_core::page::Page;

    let mut written = 0usize;
    for entry in &plan.fixes {
        if entry.action == AutoFixAction::Skip {
            continue;
        }
        let Some(page) = pages.iter().find(|p| p.frontmatter.slug == entry.slug) else {
            tracing::warn!(slug = %entry.slug, "auto-fix: skipping unknown slug");
            continue;
        };
        let mut new_page = Page {
            path: page.path.clone(),
            frontmatter: page.frontmatter.clone(),
            body: page.body.clone(),
        };
        if entry.action == AutoFixAction::Retire {
            new_page.frontmatter.status = Status::Stale;
        }
        if let Some(c) = entry.confidence {
            new_page.frontmatter.confidence = Confidence::try_new(c)?;
        }
        if let Some(s) = &entry.status {
            new_page.frontmatter.status = parse_status(s)?;
        }
        if let Some(append) = &entry.body_append {
            if !new_page.body.ends_with('\n') {
                new_page.body.push('\n');
            }
            new_page.body.push('\n');
            new_page.body.push_str(append);
        }
        new_page
            .write()
            .with_context(|| format!("writing fixed page `{}`", entry.slug))?;
        written += 1;
    }
    Ok(written)
}

fn parse_status(s: &str) -> Result<coral_core::frontmatter::Status> {
    use coral_core::frontmatter::Status::*;
    Ok(match s.to_lowercase().as_str() {
        "draft" => Draft,
        "reviewed" => Reviewed,
        "verified" => Verified,
        "stale" => Stale,
        "archived" => Archived,
        "reference" => Reference,
        other => anyhow::bail!("unknown status `{other}`"),
    })
}

fn affected_slugs(report: &LintReport, pages: &[coral_core::page::Page]) -> Vec<String> {
    let mut out: Vec<String> = report
        .issues
        .iter()
        .filter_map(|i| i.page.as_ref())
        .filter_map(|path| {
            pages
                .iter()
                .find(|p| p.path.as_path() == path.as_path())
                .map(|p| p.frontmatter.slug.clone())
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn render_issues_for_prompt(report: &LintReport) -> String {
    let mut s = String::new();
    for i in &report.issues {
        let slug_hint = i
            .page
            .as_ref()
            .map(|p| {
                p.file_name()
                    .and_then(|x| x.to_str())
                    .unwrap_or("(unknown)")
            })
            .unwrap_or("(workspace)");
        s.push_str(&format!(
            "- [{:?}] {:?} on `{}`: {}\n",
            i.severity, i.code, slug_hint, i.message
        ));
    }
    s
}

fn render_pages_for_prompt(pages: &[coral_core::page::Page], slugs: &[String]) -> String {
    let mut s = String::new();
    for p in pages.iter().filter(|p| slugs.contains(&p.frontmatter.slug)) {
        s.push_str(&format!(
            "- {} ({:?}, status={:?}, confidence={:.2}): {}\n",
            p.frontmatter.slug,
            p.frontmatter.page_type,
            p.frontmatter.status,
            p.frontmatter.confidence.as_f64(),
            p.body
                .chars()
                .take(200)
                .collect::<String>()
                .replace('\n', " ")
        ));
    }
    s
}

/// Return the set of `.wiki/**/*.md` paths currently staged for commit.
/// Resolved against `cwd` so the comparison with `LintIssue::page` (also
/// rooted there) lines up.
fn staged_wiki_paths(cwd: &Path) -> Result<HashSet<PathBuf>> {
    let output = std::process::Command::new("git")
        .args(["diff", "--cached", "--name-only", "--diff-filter=ACM"])
        .current_dir(cwd)
        .output()
        .context("invoking git diff --cached (is git installed and is this a repo?)")?;
    if !output.status.success() {
        anyhow::bail!(
            "git diff --cached failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_staged_wiki_paths(&stdout, cwd))
}

/// Pure parser for `git diff --cached --name-only` output: keep lines that
/// look like `.wiki/**/*.md`, resolve them against `cwd`, return as a set.
pub(crate) fn parse_staged_wiki_paths(stdout: &str, cwd: &Path) -> HashSet<PathBuf> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| l.contains(".wiki/") && l.ends_with(".md"))
        .map(|l| cwd.join(l))
        .collect()
}

/// Keep issues whose `page` is in `staged`, plus workspace-level issues
/// (no `page`). Pure for testability.
pub(crate) fn filter_issues_by_paths(
    issues: Vec<coral_lint::LintIssue>,
    staged: &HashSet<PathBuf>,
) -> Vec<coral_lint::LintIssue> {
    issues
        .into_iter()
        .filter(|i| match &i.page {
            Some(p) => staged.contains(p),
            None => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_lint::{LintCode, LintIssue, LintSeverity};

    #[test]
    fn parse_staged_wiki_paths_keeps_only_dotwiki_markdown() {
        let stdout = ".wiki/modules/order.md\n\
                      .wiki/concepts/outbox.md\n\
                      src/main.rs\n\
                      README.md\n\
                      docs/ARCHITECTURE.md\n\
                      .wiki/log.md\n\
                      \n";
        let cwd = PathBuf::from("/repo");
        let got = parse_staged_wiki_paths(stdout, &cwd);
        assert_eq!(got.len(), 3);
        assert!(got.contains(&cwd.join(".wiki/modules/order.md")));
        assert!(got.contains(&cwd.join(".wiki/concepts/outbox.md")));
        assert!(got.contains(&cwd.join(".wiki/log.md")));
        assert!(!got.contains(&cwd.join("src/main.rs")));
    }

    fn issue(page: Option<&str>) -> LintIssue {
        LintIssue {
            code: LintCode::OrphanPage,
            severity: LintSeverity::Critical,
            page: page.map(PathBuf::from),
            message: "x".into(),
            context: None,
        }
    }

    #[test]
    fn filter_keeps_issues_in_staged_set() {
        let staged: HashSet<PathBuf> = [PathBuf::from("/repo/.wiki/modules/order.md")]
            .into_iter()
            .collect();
        let issues = vec![
            issue(Some("/repo/.wiki/modules/order.md")),
            issue(Some("/repo/.wiki/modules/payment.md")),
        ];
        let kept = filter_issues_by_paths(issues, &staged);
        assert_eq!(kept.len(), 1);
        assert_eq!(
            kept[0].page.as_deref().unwrap(),
            Path::new("/repo/.wiki/modules/order.md")
        );
    }

    #[test]
    fn auto_fix_plan_parses_yaml_with_fences() {
        let stdout = "```yaml\nfixes:\n  - slug: order\n    action: update\n    confidence: 0.4\n    rationale: dropped below threshold\n  - slug: ghost\n    action: skip\n    rationale: needs human review\n```";
        let plan = parse_auto_fix_plan(stdout).unwrap();
        assert_eq!(plan.fixes.len(), 2);
        assert_eq!(plan.fixes[0].slug, "order");
        assert_eq!(plan.fixes[0].action, AutoFixAction::Update);
        assert_eq!(plan.fixes[0].confidence, Some(0.4));
        assert_eq!(plan.fixes[1].action, AutoFixAction::Skip);
    }

    #[test]
    fn auto_fix_plan_action_defaults_to_skip_when_missing() {
        // Defensive: an LLM that omits `action` shouldn't accidentally apply changes.
        let stdout = "fixes:\n  - slug: ghost\n    rationale: missing action field\n";
        let plan = parse_auto_fix_plan(stdout).unwrap();
        assert_eq!(plan.fixes[0].action, AutoFixAction::Skip);
    }

    #[test]
    fn auto_fix_apply_writes_updated_frontmatter_and_appends_body() {
        use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
        use coral_core::page::Page;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let modules = wiki.join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        let page_path = modules.join("order.md");

        let page = Page {
            path: page_path.clone(),
            frontmatter: Frontmatter {
                slug: "order".into(),
                page_type: PageType::Module,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(0.9).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Verified,
                generated_at: None,
                extra: Default::default(),
            },
            body: "Original body.".into(),
        };
        page.write().unwrap();

        let plan = AutoFixPlan {
            fixes: vec![
                AutoFixEntry {
                    slug: "order".into(),
                    action: AutoFixAction::Update,
                    confidence: Some(0.5),
                    status: Some("draft".into()),
                    body_append: Some("_Stale: needs sources._".into()),
                    rationale: "high conf without sources".into(),
                },
                AutoFixEntry {
                    slug: "ghost".into(),
                    action: AutoFixAction::Skip,
                    confidence: None,
                    status: None,
                    body_append: None,
                    rationale: "unknown slug".into(),
                },
            ],
        };
        let pages = vec![page];
        let written = apply_auto_fix_plan(&plan, &pages, &wiki).unwrap();
        assert_eq!(written, 1);

        let on_disk = std::fs::read_to_string(&page_path).unwrap();
        assert!(
            on_disk.contains("confidence: 0.5"),
            "frontmatter not updated: {on_disk}"
        );
        assert!(
            on_disk.contains("status: draft"),
            "status not updated: {on_disk}"
        );
        assert!(on_disk.contains("Original body."), "body lost: {on_disk}");
        assert!(
            on_disk.contains("_Stale: needs sources._"),
            "append missing: {on_disk}"
        );
    }

    #[test]
    fn auto_fix_apply_marks_retired_pages_stale() {
        use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
        use coral_core::page::Page;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let modules = wiki.join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        let page_path = modules.join("dead.md");
        let page = Page {
            path: page_path.clone(),
            frontmatter: Frontmatter {
                slug: "dead".into(),
                page_type: PageType::Module,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(0.7).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Verified,
                generated_at: None,
                extra: Default::default(),
            },
            body: "going away".into(),
        };
        page.write().unwrap();

        let plan = AutoFixPlan {
            fixes: vec![AutoFixEntry {
                slug: "dead".into(),
                action: AutoFixAction::Retire,
                confidence: None,
                status: None,
                body_append: None,
                rationale: "obsolete".into(),
            }],
        };
        apply_auto_fix_plan(&plan, std::slice::from_ref(&page), &wiki).unwrap();
        let on_disk = std::fs::read_to_string(&page_path).unwrap();
        assert!(on_disk.contains("status: stale"));
    }

    #[test]
    fn filter_always_keeps_workspace_level_issues() {
        // page == None (e.g. "wiki has no SCHEMA.md") must not be dropped
        // even when no staged paths match.
        let staged: HashSet<PathBuf> = HashSet::new();
        let issues = vec![issue(None), issue(Some("/repo/.wiki/modules/order.md"))];
        let kept = filter_issues_by_paths(issues, &staged);
        assert_eq!(kept.len(), 1);
        assert!(kept[0].page.is_none());
    }
}
