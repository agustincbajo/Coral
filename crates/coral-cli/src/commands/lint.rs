use anyhow::{Context, Result};
use clap::Args;
use coral_core::walk;
use coral_lint::{
    LintCode, LintReport, LintSeverity, run_structural_with_root,
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
    /// Filter the report to issues at or above the given severity level:
    /// `critical` (most strict — only Critical), `warning` (Critical +
    /// Warning), `info` or `all` (every level — default). Useful for CI
    /// gates that should only fail on critical issues, or for noisy wikis
    /// where users want to see only warnings. The filter is applied AFTER
    /// auto-fix runs (so the LLM still sees the full report) and BEFORE
    /// the report is rendered + the exit code is determined.
    #[arg(long, default_value = "all")]
    pub severity: String,
    /// Filter the report to issues whose `code` is in this allowlist.
    /// Repeatable: `--rule broken-wikilink --rule orphan-page` keeps
    /// issues with EITHER code. Empty list (default) = no filter.
    /// Values are the `kebab-case` (or `snake_case`) form of any
    /// `LintCode` variant, e.g. `broken-wikilink`, `orphan-page`,
    /// `low-confidence`, `high-confidence-without-sources`,
    /// `stale-status`, `commit-not-in-git`, `source-not-found`,
    /// `archived-page-linked`, `unknown-extra-field`, `contradiction`,
    /// `obsolete-claim`. Useful for CI gates that should only fail on
    /// specific issue types (e.g. only `broken-wikilink`). Applied AFTER
    /// auto-fix (so the LLM sees the full report) and BEFORE
    /// `--severity` filtering, so `--rule X --severity critical` keeps
    /// only critical issues whose code is X.
    #[arg(long)]
    pub rule: Vec<String>,
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

    // Build the FULL report (pre-severity-filter) so auto-fix sees every
    // issue. Otherwise a CI run with `--severity critical --auto-fix` would
    // hide warnings/info from the LLM and it couldn't propose fixes for
    // them.
    let full_report = LintReport::from_issues(issues);

    if args.auto_fix && !full_report.issues.is_empty() {
        run_auto_fix(&pages, &full_report, runner, args.apply, &root)?;
    }

    // Apply the rule + severity filters AFTER auto-fix (so the LLM sees
    // the full report) but BEFORE rendering / exit-code determination,
    // so users only see (and CI only fails on) the filtered subset.
    //
    // Rule filter runs FIRST so that `--rule X --severity critical`
    // means "keep issues whose code is X *and* whose severity is
    // critical" — i.e. the two filters compose, narrowest first.
    let rule_filter = parse_rule_filters(&args.rule)?;
    let severity_filter = parse_severity_filter(&args.severity)?;
    let mut issues = full_report.issues;
    if let Some(allowed) = rule_filter {
        issues.retain(|i| allowed.contains(&i.code));
    }
    if let Some(min_sev) = severity_filter {
        let min_sev_rank = u8::from(min_sev);
        issues.retain(|i| u8::from(i.severity) <= min_sev_rank);
    }
    let report = LintReport::from_issues(issues);

    match args.format.as_str() {
        "json" => println!("{}", serde_json::to_string_pretty(&report)?),
        _ => println!("{}", report.as_markdown()),
    }

    if report.critical_count() > 0 {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Snake/kebab-case names of every `LintCode` variant — kept in lockstep
/// with the `#[serde(rename_all = "snake_case")]` form so users can type
/// the same identifiers they see in `coral lint --format json` output.
/// Listed in the same order as the enum declaration in `coral-lint`.
const VALID_RULE_CODES: &[&str] = &[
    "broken-wikilink",
    "orphan-page",
    "low-confidence",
    "high-confidence-without-sources",
    "stale-status",
    "commit-not-in-git",
    "source-not-found",
    "archived-page-linked",
    "unknown-extra-field",
    "contradiction",
    "obsolete-claim",
];

/// Parse a list of `--rule` values into an optional `LintCode` allowlist.
///
/// Returns `None` for an empty list (no filter — keep every issue), or
/// `Some(set)` for one or more values (keep only issues whose `code` is
/// in `set`). Accepts both `kebab-case` (`broken-wikilink`) and
/// `snake_case` (`broken_wikilink`) forms — both normalize to the same
/// variant. Unknown values produce an error listing every valid
/// kebab-case name so users can self-correct from the CLI message
/// without consulting docs.
fn parse_rule_filters(rules: &[String]) -> Result<Option<HashSet<LintCode>>> {
    if rules.is_empty() {
        return Ok(None);
    }
    let mut set = HashSet::with_capacity(rules.len());
    for raw in rules {
        // Normalize: lowercase + treat `_` and `-` interchangeably.
        let normalized = raw.to_lowercase().replace('_', "-");
        let code = match normalized.as_str() {
            "broken-wikilink" => LintCode::BrokenWikilink,
            "orphan-page" => LintCode::OrphanPage,
            "low-confidence" => LintCode::LowConfidence,
            "high-confidence-without-sources" => LintCode::HighConfidenceWithoutSources,
            "stale-status" => LintCode::StaleStatus,
            "commit-not-in-git" => LintCode::CommitNotInGit,
            "source-not-found" => LintCode::SourceNotFound,
            "archived-page-linked" => LintCode::ArchivedPageLinked,
            "unknown-extra-field" => LintCode::UnknownExtraField,
            "contradiction" => LintCode::Contradiction,
            "obsolete-claim" => LintCode::ObsoleteClaim,
            other => anyhow::bail!(
                "unknown --rule value `{other}` (expected one of: {})",
                VALID_RULE_CODES.join(", ")
            ),
        };
        set.insert(code);
    }
    Ok(Some(set))
}

/// Parse the `--severity` flag into an optional minimum-severity threshold.
///
/// Returns `None` for `"all"` (no filter — keep every issue), `Some(sev)`
/// for `critical|warning|info` (keep issues with `u8::from(severity) <=
/// u8::from(min_sev)` since Critical=0 < Warning=1 < Info=2 — lower rank
/// is more severe). Errors on any other value with a friendly hint.
fn parse_severity_filter(s: &str) -> Result<Option<LintSeverity>> {
    match s.to_lowercase().as_str() {
        "all" => Ok(None),
        "critical" => Ok(Some(LintSeverity::Critical)),
        "warning" => Ok(Some(LintSeverity::Warning)),
        "info" => Ok(Some(LintSeverity::Info)),
        other => anyhow::bail!(
            "unknown --severity value `{other}` (expected one of: critical, warning, info, all)"
        ),
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

    /// Variant of `issue` that lets tests pick the severity. Used by the
    /// severity-filter rank tests below.
    fn issue_sev(severity: LintSeverity) -> LintIssue {
        LintIssue {
            code: LintCode::OrphanPage,
            severity,
            page: None,
            message: "x".into(),
            context: None,
        }
    }

    /// Convenience: apply the same severity-filter logic the CLI uses
    /// (`u8::from(severity) <= u8::from(min_sev)`) without depending on the
    /// full `run_with_runner` plumbing.
    fn apply_severity_filter(
        issues: Vec<LintIssue>,
        min_sev: Option<LintSeverity>,
    ) -> Vec<LintIssue> {
        match min_sev {
            None => issues,
            Some(min) => {
                let rank = u8::from(min);
                issues
                    .into_iter()
                    .filter(|i| u8::from(i.severity) <= rank)
                    .collect()
            }
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

    #[test]
    fn parse_severity_filter_all_returns_none() {
        // "all" means: no filter — keep every issue regardless of severity.
        // This is the CLI default and must preserve historical behavior.
        assert!(parse_severity_filter("all").unwrap().is_none());
    }

    #[test]
    fn parse_severity_filter_is_case_insensitive() {
        // Users in pre-commit hooks tend to type "CRITICAL" in env vars.
        for variant in ["critical", "CRITICAL", "Critical", "CrItIcAl"] {
            let got = parse_severity_filter(variant)
                .unwrap_or_else(|e| panic!("`{variant}` should parse: {e}"));
            assert_eq!(
                got,
                Some(LintSeverity::Critical),
                "`{variant}` should map to Critical"
            );
        }
    }

    #[test]
    fn parse_severity_filter_warning_and_info() {
        assert_eq!(
            parse_severity_filter("warning").unwrap(),
            Some(LintSeverity::Warning)
        );
        assert_eq!(
            parse_severity_filter("info").unwrap(),
            Some(LintSeverity::Info)
        );
    }

    #[test]
    fn parse_severity_filter_unknown_value_errors_with_actionable_message() {
        let err = parse_severity_filter("foo").unwrap_err();
        let msg = format!("{err}");
        // Hint must list every legal value so the user can self-correct.
        for expected in ["foo", "critical", "warning", "info", "all"] {
            assert!(
                msg.contains(expected),
                "error message `{msg}` must mention `{expected}`"
            );
        }
    }

    #[test]
    fn severity_filter_critical_keeps_only_critical() {
        // Rank 0 (Critical): only issues with rank <= 0 survive.
        let issues = vec![
            issue_sev(LintSeverity::Critical),
            issue_sev(LintSeverity::Warning),
            issue_sev(LintSeverity::Info),
        ];
        let kept = apply_severity_filter(issues, Some(LintSeverity::Critical));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].severity, LintSeverity::Critical);
    }

    #[test]
    fn severity_filter_warning_keeps_critical_and_warning() {
        // Rank 1 (Warning): Critical (0) + Warning (1) survive; Info (2) drops.
        let issues = vec![
            issue_sev(LintSeverity::Critical),
            issue_sev(LintSeverity::Warning),
            issue_sev(LintSeverity::Info),
        ];
        let kept = apply_severity_filter(issues, Some(LintSeverity::Warning));
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().any(|i| i.severity == LintSeverity::Critical));
        assert!(kept.iter().any(|i| i.severity == LintSeverity::Warning));
        assert!(!kept.iter().any(|i| i.severity == LintSeverity::Info));
    }

    #[test]
    fn severity_filter_info_keeps_all_three() {
        // Rank 2 (Info): everything survives — semantically equivalent to None
        // but kept distinct because the user *did* type a level.
        let issues = vec![
            issue_sev(LintSeverity::Critical),
            issue_sev(LintSeverity::Warning),
            issue_sev(LintSeverity::Info),
        ];
        let kept = apply_severity_filter(issues, Some(LintSeverity::Info));
        assert_eq!(kept.len(), 3);
    }

    #[test]
    fn severity_filter_none_keeps_all_three() {
        // The "all" CLI value parses to None — same shape as Info but cheaper
        // (no filter pass at all).
        let issues = vec![
            issue_sev(LintSeverity::Critical),
            issue_sev(LintSeverity::Warning),
            issue_sev(LintSeverity::Info),
        ];
        let kept = apply_severity_filter(issues, None);
        assert_eq!(kept.len(), 3);
    }

    /// Variant of `issue` that lets tests pick both the code and severity.
    /// Used by the rule-filter tests so we can build heterogeneous inputs.
    fn issue_with_code(code: LintCode) -> LintIssue {
        LintIssue {
            code,
            severity: LintSeverity::Critical,
            page: None,
            message: "x".into(),
            context: None,
        }
    }

    #[test]
    fn parse_rule_filters_empty_returns_none() {
        // Empty list = no filter (matches the historical behaviour
        // before --rule existed; no surprise for users who don't pass it).
        let got = parse_rule_filters(&[]).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn parse_rule_filters_accepts_kebab_case() {
        let got = parse_rule_filters(&["broken-wikilink".into()])
            .unwrap()
            .unwrap();
        assert_eq!(got.len(), 1);
        assert!(got.contains(&LintCode::BrokenWikilink));
    }

    #[test]
    fn parse_rule_filters_accepts_snake_case_via_normalization() {
        // The JSON output emits `snake_case`, so users grepping that output
        // and feeding values back to `--rule` should work without manual
        // translation.
        let got = parse_rule_filters(&["broken_wikilink".into()])
            .unwrap()
            .unwrap();
        assert_eq!(got.len(), 1);
        assert!(got.contains(&LintCode::BrokenWikilink));
    }

    #[test]
    fn parse_rule_filters_is_case_insensitive() {
        // Pre-commit hook configs frequently uppercase CI env vars; we
        // shouldn't punish that.
        for variant in [
            "BROKEN-WIKILINK",
            "Broken-Wikilink",
            "broken-WIKILINK",
            "BROKEN_WIKILINK",
        ] {
            let got = parse_rule_filters(&[variant.into()])
                .unwrap_or_else(|e| panic!("`{variant}` should parse: {e}"))
                .unwrap();
            assert!(
                got.contains(&LintCode::BrokenWikilink),
                "`{variant}` did not map to BrokenWikilink"
            );
        }
    }

    #[test]
    fn parse_rule_filters_supports_repetition_oring_codes() {
        // `--rule X --rule Y` keeps issues with EITHER code (OR, not AND).
        let got = parse_rule_filters(&["broken-wikilink".into(), "orphan-page".into()])
            .unwrap()
            .unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.contains(&LintCode::BrokenWikilink));
        assert!(got.contains(&LintCode::OrphanPage));
    }

    #[test]
    fn parse_rule_filters_dedupes_repeated_values() {
        // HashSet semantics: passing the same code twice doesn't double-count.
        let got = parse_rule_filters(&["orphan-page".into(), "orphan-page".into()])
            .unwrap()
            .unwrap();
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn parse_rule_filters_recognizes_every_lintcode_variant() {
        // Guards against drift: if a new LintCode variant lands in coral-lint,
        // the maintainer has to add it to the parser too — this test fails
        // until they do, because the new variant won't be covered.
        let all_kebab = [
            ("broken-wikilink", LintCode::BrokenWikilink),
            ("orphan-page", LintCode::OrphanPage),
            ("low-confidence", LintCode::LowConfidence),
            (
                "high-confidence-without-sources",
                LintCode::HighConfidenceWithoutSources,
            ),
            ("stale-status", LintCode::StaleStatus),
            ("commit-not-in-git", LintCode::CommitNotInGit),
            ("source-not-found", LintCode::SourceNotFound),
            ("archived-page-linked", LintCode::ArchivedPageLinked),
            ("unknown-extra-field", LintCode::UnknownExtraField),
            ("contradiction", LintCode::Contradiction),
            ("obsolete-claim", LintCode::ObsoleteClaim),
        ];
        for (name, expected) in all_kebab {
            let got = parse_rule_filters(&[name.into()])
                .unwrap_or_else(|e| panic!("`{name}` should parse: {e}"))
                .unwrap();
            assert!(
                got.contains(&expected),
                "`{name}` did not map to expected variant"
            );
        }
    }

    #[test]
    fn parse_rule_filters_unknown_value_errors_with_full_legend() {
        // The error message must list every legal value so users can fix
        // their CI config without reading docs.
        let err = parse_rule_filters(&["nope".into()]).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("nope"), "error must echo bad value: {msg}");
        for legal in [
            "broken-wikilink",
            "orphan-page",
            "low-confidence",
            "high-confidence-without-sources",
            "stale-status",
            "commit-not-in-git",
            "source-not-found",
            "archived-page-linked",
            "unknown-extra-field",
            "contradiction",
            "obsolete-claim",
        ] {
            assert!(
                msg.contains(legal),
                "error must list legal value `{legal}`: {msg}"
            );
        }
    }

    #[test]
    fn parse_rule_filters_first_unknown_value_errors_even_after_valid_ones() {
        // Defensive: a valid value followed by an invalid one shouldn't
        // silently succeed with a partial set — CI users want loud failures.
        let err = parse_rule_filters(&["broken-wikilink".into(), "bogus".into()]).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("bogus"), "error must echo bad value: {msg}");
    }

    /// Convenience: apply the rule-filter logic the CLI uses (retain
    /// issues whose code is in the allowlist) without depending on
    /// `run_with_runner`. Mirrors `apply_severity_filter`.
    fn apply_rule_filter(
        issues: Vec<LintIssue>,
        allowed: Option<HashSet<LintCode>>,
    ) -> Vec<LintIssue> {
        match allowed {
            None => issues,
            Some(set) => issues
                .into_iter()
                .filter(|i| set.contains(&i.code))
                .collect(),
        }
    }

    #[test]
    fn rule_filter_none_keeps_all_codes() {
        let issues = vec![
            issue_with_code(LintCode::BrokenWikilink),
            issue_with_code(LintCode::OrphanPage),
            issue_with_code(LintCode::StaleStatus),
        ];
        let kept = apply_rule_filter(issues, None);
        assert_eq!(kept.len(), 3);
    }

    #[test]
    fn rule_filter_keeps_only_allowed_codes() {
        let issues = vec![
            issue_with_code(LintCode::BrokenWikilink),
            issue_with_code(LintCode::OrphanPage),
            issue_with_code(LintCode::StaleStatus),
        ];
        let allowed: HashSet<LintCode> = [LintCode::BrokenWikilink].into_iter().collect();
        let kept = apply_rule_filter(issues, Some(allowed));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].code, LintCode::BrokenWikilink);
    }

    #[test]
    fn rule_filter_with_two_allowed_codes_keeps_both() {
        // Verifies the OR semantics of repeated --rule flags end-to-end.
        let issues = vec![
            issue_with_code(LintCode::BrokenWikilink),
            issue_with_code(LintCode::OrphanPage),
            issue_with_code(LintCode::StaleStatus),
        ];
        let allowed: HashSet<LintCode> = [LintCode::BrokenWikilink, LintCode::OrphanPage]
            .into_iter()
            .collect();
        let kept = apply_rule_filter(issues, Some(allowed));
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().any(|i| i.code == LintCode::BrokenWikilink));
        assert!(kept.iter().any(|i| i.code == LintCode::OrphanPage));
        assert!(!kept.iter().any(|i| i.code == LintCode::StaleStatus));
    }

    #[test]
    fn rule_then_severity_filter_compose_narrowest_first() {
        // Belt-and-suspenders for the documented composition order:
        // `--rule broken-wikilink --severity critical` keeps issues that
        // are BOTH `BrokenWikilink` AND at-or-above Critical.
        let issues = vec![
            // Kept: matches code AND severity.
            LintIssue {
                code: LintCode::BrokenWikilink,
                severity: LintSeverity::Critical,
                page: None,
                message: "kept".into(),
                context: None,
            },
            // Dropped by rule filter (wrong code, but right severity).
            LintIssue {
                code: LintCode::OrphanPage,
                severity: LintSeverity::Critical,
                page: None,
                message: "dropped-by-rule".into(),
                context: None,
            },
            // Dropped by severity filter (right code, wrong severity).
            LintIssue {
                code: LintCode::BrokenWikilink,
                severity: LintSeverity::Info,
                page: None,
                message: "dropped-by-severity".into(),
                context: None,
            },
        ];
        let allowed: HashSet<LintCode> = [LintCode::BrokenWikilink].into_iter().collect();
        let after_rule = apply_rule_filter(issues, Some(allowed));
        let after_both = apply_severity_filter(after_rule, Some(LintSeverity::Critical));
        assert_eq!(after_both.len(), 1);
        assert_eq!(after_both[0].message, "kept");
    }
}
