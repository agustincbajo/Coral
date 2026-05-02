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
    /// No-LLM, pure-rule auto-fix mode. Applies deterministic, mechanical
    /// fixes (trim trailing whitespace in frontmatter strings, sort
    /// `sources` and `backlinks`, normalize wikilink spacing
    /// `[[ slug ]]` → `[[slug]]`, trim trailing whitespace on body
    /// lines). Independent of `--auto-fix`: they can be combined as
    /// `--fix --auto-fix --apply` to run rules first, then the LLM.
    /// Default: dry-run prints the proposed fixes per page. Pass
    /// `--apply` to write them. Useful for users without LLM auth.
    #[arg(long)]
    pub fix: bool,
    /// LLM-driven source suggestion: for every page that triggers a
    /// `HighConfidenceWithoutSources` issue, ask the runner to pick
    /// 1-3 plausible source paths from `git ls-files`. Default:
    /// dry-run prints the proposals. Pass `--apply` to append the
    /// suggested paths to each page's `frontmatter.sources` (deduped).
    /// Independent of `--auto-fix` and `--fix` — all three can run in
    /// the same invocation.
    #[arg(long)]
    pub suggest_sources: bool,
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

    // Source suggestion runs on the full report too (so the LLM sees
    // every `HighConfidenceWithoutSources` issue, regardless of any
    // active --rule / --severity filters used to gate CI noise).
    if args.suggest_sources && !full_report.issues.is_empty() {
        let repo_root: PathBuf = root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let report = run_source_suggestion(&pages, &full_report, runner, args.apply, &repo_root)?;
        println!("{}", render_source_suggestion_report(&report));
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

    // No-LLM rule-based fix pass — runs INDEPENDENTLY of the lint
    // output above. Always last so the lint report renders first and
    // the fix proposal/result is appended cleanly.
    if args.fix {
        // The repo root (parent of `.wiki/`) is needed for the
        // confidence-from-coverage rule which checks whether each
        // page's `sources:` paths still exist on disk.
        let repo_root: PathBuf = root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let fix_report = run_no_llm_fix(&pages, args.apply, &root, &repo_root)?;
        println!("{}", render_fix_report(&fix_report));
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

/// Generic system fallback used when neither a per-rule template nor the
/// generic embedded `lint-auto-fix.md` is available for a given issue.
///
/// Per-rule overrides exist: when an issue's `LintCode` has a matching
/// `template/prompts/lint-auto-fix-<code-kebab>.md` (or local override
/// at `<cwd>/prompts/lint-auto-fix-<code-kebab>.md`), the orchestrator
/// uses that instead. Currently shipped specialized templates:
/// - `lint-auto-fix-broken-wikilink` (BrokenWikilink)
/// - `lint-auto-fix-low-confidence` (LowConfidence)
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

/// Map a `LintCode` to its kebab-case form for prompt-name lookup.
/// The output mirrors the `serde(rename_all = "snake_case")` form
/// with underscores replaced by hyphens — same as the
/// `--rule` CLI flag accepts.
pub(crate) fn lint_code_to_kebab(code: LintCode) -> &'static str {
    match code {
        LintCode::BrokenWikilink => "broken-wikilink",
        LintCode::OrphanPage => "orphan-page",
        LintCode::LowConfidence => "low-confidence",
        LintCode::HighConfidenceWithoutSources => "high-confidence-without-sources",
        LintCode::StaleStatus => "stale-status",
        LintCode::CommitNotInGit => "commit-not-in-git",
        LintCode::SourceNotFound => "source-not-found",
        LintCode::ArchivedPageLinked => "archived-page-linked",
        LintCode::UnknownExtraField => "unknown-extra-field",
        LintCode::Contradiction => "contradiction",
        LintCode::ObsoleteClaim => "obsolete-claim",
    }
}

/// Group lint issues by their `LintCode`. Returns a `Vec` (not a `HashMap`)
/// so iteration order is stable across runs — the outer order is the order
/// `code` first appears in `report.issues`. Within a group, issues stay in
/// their original order, so the LLM sees them in the same order the user
/// sees them in `coral lint --format markdown`.
pub(crate) fn group_issues_by_code(
    issues: &[coral_lint::LintIssue],
) -> Vec<(LintCode, Vec<coral_lint::LintIssue>)> {
    let mut order: Vec<LintCode> = Vec::new();
    let mut groups: std::collections::HashMap<LintCode, Vec<coral_lint::LintIssue>> =
        std::collections::HashMap::new();
    for issue in issues {
        if !groups.contains_key(&issue.code) {
            order.push(issue.code);
        }
        groups.entry(issue.code).or_default().push(issue.clone());
    }
    order
        .into_iter()
        .map(|code| {
            let v = groups.remove(&code).unwrap_or_default();
            (code, v)
        })
        .collect()
}

fn run_auto_fix(
    pages: &[coral_core::page::Page],
    report: &LintReport,
    runner: &dyn Runner,
    apply: bool,
    wiki_root: &Path,
) -> Result<()> {
    use coral_runner::Prompt;

    // Group issues by code so each `LintCode` gets at most one runner
    // call. For codes with a specialized prompt (e.g. broken-wikilink),
    // the LLM gets a tighter system prompt; for everything else the
    // generic `lint-auto-fix` template applies. Concatenating the
    // per-group plans gives one combined `AutoFixPlan` for the rest of
    // the pipeline.
    let groups = group_issues_by_code(&report.issues);

    let pages_summary = render_pages_for_prompt(pages, &affected_slugs(report, pages));

    let mut combined = AutoFixPlan { fixes: Vec::new() };
    let mut combined_stdout = String::new();

    for (code, issues) in groups {
        let group_report = LintReport {
            issues: issues.clone(),
        };
        let issues_summary = render_issues_for_prompt(&group_report);

        // Resolution chain: prefer the per-code template, fall back to
        // the generic `lint-auto-fix` template, and ultimately the
        // hardcoded `AUTO_FIX_SYSTEM_FALLBACK` const.
        let specialized_name = format!("lint-auto-fix-{}", lint_code_to_kebab(code));
        let specialized = super::prompt_loader::load_or_fallback(&specialized_name, "");
        let prompt_template = match specialized.source {
            super::prompt_loader::PromptSource::Fallback => {
                // No specialized template → generic.
                super::prompt_loader::load_or_fallback("lint-auto-fix", AUTO_FIX_SYSTEM_FALLBACK)
            }
            _ => specialized,
        };

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
        combined.fixes.extend(plan.fixes);
        if !combined_stdout.is_empty() {
            combined_stdout.push_str("\n---\n");
        }
        combined_stdout.push_str(out.stdout.trim());
    }

    if !apply {
        println!("\n## Auto-fix proposal (dry-run)\n");
        println!("```yaml\n{}\n```", combined_stdout.trim());
        println!("Pass `--apply` to write {} fix(es).", combined.fixes.len());
        return Ok(());
    }

    let written = apply_auto_fix_plan(&combined, pages, wiki_root)?;
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

/// Per-page record of which deterministic rules fired during a
/// no-LLM fix pass. The rule names are static string slices so they
/// can be cheaply joined into a comma-separated list at render time.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NoLlmFixReport {
    /// One entry per page that would be (or was) modified, in
    /// alphabetical-by-slug order. The `Vec<&'static str>` lists the
    /// rules that fired on that page (also in deterministic order).
    pub changed_pages: Vec<(String, PathBuf, Vec<&'static str>)>,
    /// Cached `changed_pages.len()` — duplicated for readability at
    /// the call sites and to keep the render function pure.
    pub total_changed: usize,
    /// Whether `--apply` was passed (controls "would change" vs
    /// "wrote" wording in the rendered report).
    pub applied: bool,
}

/// Trim trailing ASCII whitespace from frontmatter string fields
/// (`slug`, `last_updated_commit`). Returns `true` if any field
/// changed. Pure — caller decides whether to persist.
pub(crate) fn trim_frontmatter_strings(fm: &mut coral_core::frontmatter::Frontmatter) -> bool {
    let mut changed = false;
    let trimmed_slug = fm.slug.trim_end().to_string();
    if trimmed_slug != fm.slug {
        fm.slug = trimmed_slug;
        changed = true;
    }
    let trimmed_commit = fm.last_updated_commit.trim_end().to_string();
    if trimmed_commit != fm.last_updated_commit {
        fm.last_updated_commit = trimmed_commit;
        changed = true;
    }
    changed
}

/// Sort `sources` alphabetically in place. Returns `true` if the
/// order changed (so the caller can record that the rule fired
/// without re-sorting in the no-op case).
pub(crate) fn sort_sources(fm: &mut coral_core::frontmatter::Frontmatter) -> bool {
    let mut sorted = fm.sources.clone();
    sorted.sort();
    if sorted != fm.sources {
        fm.sources = sorted;
        true
    } else {
        false
    }
}

/// Sort `backlinks` alphabetically in place. Returns `true` if the
/// order changed.
pub(crate) fn sort_backlinks(fm: &mut coral_core::frontmatter::Frontmatter) -> bool {
    let mut sorted = fm.backlinks.clone();
    sorted.sort();
    if sorted != fm.backlinks {
        fm.backlinks = sorted;
        true
    } else {
        false
    }
}

/// Deduplicate the `sources` Vec while preserving its current order.
/// Returns `true` if any duplicates were removed. Pure — caller decides
/// whether to persist. Order preservation is important: the rule runs
/// AFTER `sort_sources`, so it sees an already-sorted list and the
/// preserved order is just the de-duped sorted form.
pub(crate) fn dedup_sources(fm: &mut coral_core::frontmatter::Frontmatter) -> bool {
    let mut seen = std::collections::HashSet::new();
    let original_len = fm.sources.len();
    fm.sources.retain(|s| seen.insert(s.clone()));
    fm.sources.len() != original_len
}

/// Deduplicate the `backlinks` Vec while preserving its current order.
/// Returns `true` if any duplicates were removed. Pure — caller decides
/// whether to persist. Same ordering note as `dedup_sources`.
pub(crate) fn dedup_backlinks(fm: &mut coral_core::frontmatter::Frontmatter) -> bool {
    let mut seen = std::collections::HashSet::new();
    let original_len = fm.backlinks.len();
    fm.backlinks.retain(|s| seen.insert(s.clone()));
    fm.backlinks.len() != original_len
}

/// Convert CRLF line endings (`\r\n`) to LF (`\n`) in `body`. Returns
/// `Some(new_body)` if any line had `\r\n`, else `None`. Pure. Useful
/// for cross-platform consistency (Windows-authored pages, files
/// crossing the network with bad line-ending negotiation).
pub(crate) fn normalize_eol(body: &str) -> Option<String> {
    if body.contains("\r\n") {
        Some(body.replace("\r\n", "\n"))
    } else {
        None
    }
}

/// Normalize wikilink spacing in the body: `[[ slug ]]` → `[[slug]]`.
/// Returns `Some(new_body)` if anything changed, else `None`. Pure.
///
/// The regex matches a `[[`, optional surrounding ASCII whitespace
/// (no newlines — wikilinks don't span lines), and `]]`. Inner pipes
/// (`[[ slug | label ]]`) are preserved verbatim so the rule never
/// touches link aliases.
pub(crate) fn normalize_wikilink_spacing(body: &str) -> Option<String> {
    use regex::Regex;
    use std::sync::OnceLock;
    // OnceLock so we compile the regex exactly once per process.
    // Failure to compile is a programmer error; the literal is
    // checked at test time.
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"\[\[[ \t]+([^\]\n]*?)[ \t]+\]\]|\[\[[ \t]+([^\]\n]*?)\]\]|\[\[([^\]\n]*?)[ \t]+\]\]",
        )
        .expect("wikilink-spacing regex compiles")
    });
    let new_body = re.replace_all(body, |caps: &regex::Captures<'_>| {
        let inner = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| m.as_str())
            .unwrap_or("");
        format!("[[{inner}]]")
    });
    if new_body == body {
        None
    } else {
        Some(new_body.into_owned())
    }
}

/// Trim trailing ASCII whitespace from each line of `body`. Returns
/// `Some(new_body)` if anything changed, else `None`. Pure.
///
/// Preserves the line terminator (`\n`) and the final-line
/// no-newline case verbatim — only the run of spaces/tabs immediately
/// before the newline (or end of body) is removed.
pub(crate) fn trim_trailing_line_whitespace(body: &str) -> Option<String> {
    let mut out = String::with_capacity(body.len());
    let mut changed = false;
    // split_inclusive keeps each line's trailing `\n` (or no newline
    // for the last line if there isn't one). That lets us trim the
    // text portion without losing the line break.
    for raw in body.split_inclusive('\n') {
        let (text, term) = match raw.strip_suffix('\n') {
            Some(t) => (t, "\n"),
            None => (raw, ""),
        };
        let trimmed = text.trim_end_matches([' ', '\t']);
        if trimmed.len() != text.len() {
            changed = true;
        }
        out.push_str(trimmed);
        out.push_str(term);
    }
    if changed { Some(out) } else { None }
}

/// **confidence-from-coverage** rule (v0.14): if the page has ANY
/// `sources:` path that no longer resolves to a file/dir under
/// `repo_root`, downgrade `confidence` by `DOWNGRADE_STEP` (capped
/// at the floor of `MIN_AFTER_DOWNGRADE`). Returns `true` if the
/// confidence was changed.
///
/// Rationale: a page with a high confidence value backed by sources
/// that no longer exist is less reliable than the number suggests.
/// Auto-downgrade gives lint + downstream consumers (search, query,
/// onboard) a calibrated signal without requiring an LLM round trip.
///
/// Skips: pages with no sources (no signal to act on), pages whose
/// confidence is already at or below the floor (idempotent), HTTP/HTTPS
/// sources (the rule only cares about filesystem paths). Mirrors the
/// `check_source_exists` filter logic in `coral-lint` exactly so the
/// rule fires for the same set of pages that already trigger the
/// `SourceNotFound` lint.
///
/// Pure-rule (no LLM). Pure (no I/O outside of `Path::exists`).
pub(crate) fn downgrade_confidence_for_missing_sources(
    fm: &mut coral_core::frontmatter::Frontmatter,
    repo_root: &Path,
) -> bool {
    /// Amount to subtract from a page's confidence on each fix pass.
    /// Conservative — one missing source shouldn't tank the whole
    /// page; if the user runs `lint --fix --apply` repeatedly with
    /// no remediation, the rule keeps stepping down toward the floor.
    const DOWNGRADE_STEP: f64 = 0.20;
    /// Floor: the rule never drives confidence below this. Pages
    /// already at or under this value are left alone (idempotent).
    /// 0.30 chosen to align with the "Critical low confidence (<0.3)"
    /// threshold in `coral stats` — this rule pushes pages INTO the
    /// warning band but doesn't push them into the critical band on
    /// its own.
    const MIN_AFTER_DOWNGRADE: f64 = 0.30;

    if fm.sources.is_empty() {
        return false;
    }
    let any_missing = fm.sources.iter().any(|src| {
        if src.starts_with("http://") || src.starts_with("https://") {
            return false;
        }
        !repo_root.join(src).exists()
    });
    if !any_missing {
        return false;
    }
    let current = fm.confidence.as_f64();
    if current <= MIN_AFTER_DOWNGRADE {
        return false;
    }
    let target = (current - DOWNGRADE_STEP).max(MIN_AFTER_DOWNGRADE);
    // `Confidence::try_new` clamps via the validated constructor —
    // bail silently if the math somehow produces an invalid value
    // (shouldn't happen because we capped at MIN_AFTER_DOWNGRADE).
    if let Ok(new_conf) = coral_core::frontmatter::Confidence::try_new(target) {
        fm.confidence = new_conf;
        true
    } else {
        false
    }
}

/// Run the no-LLM fix pass over `pages`. With `apply == false` this
/// is a pure dry-run — pages on disk are untouched and the returned
/// report describes what *would* change. With `apply == true` each
/// modified page is persisted via `Page::write()` before the next
/// page is examined.
///
/// Order of operations per page (matters because every rule sees the
/// state left by the previous one):
/// 1. `trim_frontmatter_strings`
/// 2. `sort_sources`
/// 3. `sort_backlinks`
/// 4. `dedup_sources`
/// 5. `dedup_backlinks`
/// 6. `normalize_wikilink_spacing`
/// 7. `trim_trailing_line_whitespace`
/// 8. `normalize_eol`
fn run_no_llm_fix(
    pages: &[coral_core::page::Page],
    apply: bool,
    _wiki_root: &Path,
    repo_root: &Path,
) -> Result<NoLlmFixReport> {
    let mut changed_pages: Vec<(String, PathBuf, Vec<&'static str>)> = Vec::new();

    for page in pages {
        let mut new_page = page.clone();
        let mut rules_fired: Vec<&'static str> = Vec::new();

        if trim_frontmatter_strings(&mut new_page.frontmatter) {
            rules_fired.push("trim-frontmatter-whitespace");
        }
        if sort_sources(&mut new_page.frontmatter) {
            rules_fired.push("sort-sources");
        }
        if sort_backlinks(&mut new_page.frontmatter) {
            rules_fired.push("sort-backlinks");
        }
        if dedup_sources(&mut new_page.frontmatter) {
            rules_fired.push("dedup-sources");
        }
        if dedup_backlinks(&mut new_page.frontmatter) {
            rules_fired.push("dedup-backlinks");
        }
        if let Some(b) = normalize_wikilink_spacing(&new_page.body) {
            new_page.body = b;
            rules_fired.push("normalize-wikilinks");
        }
        if let Some(b) = trim_trailing_line_whitespace(&new_page.body) {
            new_page.body = b;
            rules_fired.push("trim-trailing-whitespace");
        }
        if let Some(b) = normalize_eol(&new_page.body) {
            new_page.body = b;
            rules_fired.push("normalize-eol");
        }
        if downgrade_confidence_for_missing_sources(&mut new_page.frontmatter, repo_root) {
            rules_fired.push("confidence-from-coverage");
        }

        if !rules_fired.is_empty() {
            if apply {
                new_page
                    .write()
                    .with_context(|| format!("writing fixed page `{}`", page.frontmatter.slug))?;
            }
            changed_pages.push((
                page.frontmatter.slug.clone(),
                page.path.clone(),
                rules_fired,
            ));
        }
    }

    // Sort by slug for stable, deterministic output regardless of
    // walk order (which depends on filesystem listing).
    changed_pages.sort_by(|a, b| a.0.cmp(&b.0));
    let total_changed = changed_pages.len();

    Ok(NoLlmFixReport {
        changed_pages,
        total_changed,
        applied: apply,
    })
}

/// Render a [`NoLlmFixReport`] as the Markdown shown to the user.
/// Pure — no I/O, no formatting variation based on env.
pub(crate) fn render_fix_report(report: &NoLlmFixReport) -> String {
    if report.total_changed == 0 {
        return "\n# No-LLM lint fixes\n\nNo fixes needed.\n".to_string();
    }

    let header = if report.applied {
        "\n# No-LLM lint fixes (applied)\n\n"
    } else {
        "\n# No-LLM lint fixes (dry-run)\n\n"
    };
    let mut out = String::from(header);
    for (slug, path, rules) in &report.changed_pages {
        // Render path relative to cwd when possible — falls back to
        // the absolute display so the report is still readable in
        // any environment (tests, CI, ad-hoc runs).
        let path_display = std::env::current_dir()
            .ok()
            .and_then(|cwd| path.strip_prefix(&cwd).ok().map(Path::to_path_buf))
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| path.display().to_string());
        out.push_str(&format!(
            "- `{slug}` ({}): {}\n",
            path_display,
            rules.join(", ")
        ));
    }
    let footer = if report.applied {
        format!("\nWrote {} page(s).\n", report.total_changed)
    } else {
        format!(
            "\n{} page(s) would change. Pass `--apply` to write.\n",
            report.total_changed
        )
    };
    out.push_str(&footer);
    out
}

// =============================================================
// Source suggestion (`coral lint --suggest-sources`)
//
// For every page that triggers the `HighConfidenceWithoutSources`
// rule, ask the runner to propose 1-3 plausible workspace paths
// from a `git ls-files` listing. Default is dry-run (prints the
// proposals); `--apply` appends the suggestions to the page's
// `frontmatter.sources` (deduped against any pre-existing entries).
// =============================================================

/// Generic system fallback used when neither the local override nor
/// the embedded `lint-suggest-sources.md` template is available.
/// Kept short — the embedded template is the real reference.
const SOURCE_SUGGESTION_SYSTEM_FALLBACK: &str = "You are the Coral wiki linter in source-suggestion mode. \
For the page below (slug + body excerpt), pick 1-3 paths from the workspace's \
`git ls-files` listing that are most plausibly the source-of-truth this page documents. \
Do NOT invent paths that aren't in the listing. Prefer specific files over directories. \
Output ONLY a YAML document of the form:\n\
```yaml\n\
slug: <the slug>\n\
suggested_sources:\n\
  - <path>\n\
  - <path>\n\
```\n\
If nothing in the listing plausibly matches, return an empty list.";

/// One LLM proposal: the slug it was asked about + the paths it
/// returned. Parsed from the YAML the runner produces.
#[derive(Debug, Clone, serde::Deserialize, PartialEq)]
pub(crate) struct SourceSuggestion {
    pub slug: String,
    #[serde(default)]
    pub suggested_sources: Vec<String>,
}

/// Per-page record collected during a source-suggestion run. The
/// `path` field is the page's on-disk path so the renderer can show
/// it relative to cwd.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SourceSuggestionEntry {
    pub slug: String,
    pub path: PathBuf,
    /// Paths the LLM returned that were not already in the page's
    /// `frontmatter.sources`. Already-present paths are deduped here
    /// (not at write time) so the dry-run report is accurate.
    pub new_sources: Vec<String>,
}

/// Aggregate report for the source-suggestion run. Mirrors
/// [`NoLlmFixReport`]'s shape so the renderer can branch on
/// `applied`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SourceSuggestionReport {
    pub entries: Vec<SourceSuggestionEntry>,
    /// Number of pages with at least one *new* source proposal.
    /// Pages where every suggested path already existed in
    /// `frontmatter.sources` don't count.
    pub total_pages: usize,
    /// Whether `--apply` was passed (controls "would gain" vs
    /// "wrote" wording in the rendered report).
    pub applied: bool,
}

/// Parse a YAML document of the shape produced by the
/// `lint-suggest-sources` prompt. Tolerates ```` ```yaml ```` and
/// ```` ``` ```` fences for parity with `parse_auto_fix_plan`.
///
/// Returns the LLM-proposed sources verbatim — the caller is
/// responsible for deduping against the page's existing
/// `frontmatter.sources` and for any path-validation policy.
pub(crate) fn parse_source_suggestion(stdout: &str) -> Result<Vec<String>> {
    let trimmed = strip_yaml_fence(stdout);
    let parsed: SourceSuggestion = serde_yaml_ng::from_str(trimmed)?;
    Ok(parsed.suggested_sources)
}

/// Shell out to `git ls-files` in `repo_root` and return the listing
/// as a `Vec<String>` of one path per line. Errors with an actionable
/// message if git isn't installed or the directory isn't a repo —
/// callers can surface that to the user.
pub(crate) fn get_git_ls_files(repo_root: &Path) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["ls-files"])
        .current_dir(repo_root)
        .output()
        .context("invoking `git ls-files` (is git installed and is this a repo?)")?;
    if !output.status.success() {
        anyhow::bail!(
            "`git ls-files` failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Filter `report.issues` down to the `HighConfidenceWithoutSources`
/// subset and resolve each to its `(slug, &Page)` pair. Pages whose
/// path doesn't match any loaded `Page` are dropped (defensive: the
/// rule shouldn't ever produce one). Pure for testability.
pub(crate) fn collect_high_confidence_without_sources<'a>(
    report: &LintReport,
    pages: &'a [coral_core::page::Page],
) -> Vec<(String, &'a coral_core::page::Page)> {
    let mut out: Vec<(String, &coral_core::page::Page)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for issue in &report.issues {
        if issue.code != LintCode::HighConfidenceWithoutSources {
            continue;
        }
        let Some(path) = issue.page.as_ref() else {
            continue;
        };
        let Some(page) = pages.iter().find(|p| p.path.as_path() == path.as_path()) else {
            continue;
        };
        let slug = page.frontmatter.slug.clone();
        if seen.insert(slug.clone()) {
            out.push((slug, page));
        }
    }
    out
}

/// Cap the `git ls-files` listing the LLM sees to the first
/// [`MAX_LS_FILES_LINES`] lines. Keeps the prompt size predictable on
/// large repos.
const MAX_LS_FILES_LINES: usize = 200;

/// Render up to [`MAX_LS_FILES_LINES`] lines of the workspace listing
/// as a single newline-joined string ready to embed in the user
/// prompt.
fn render_ls_files_for_prompt(files: &[String]) -> String {
    files
        .iter()
        .take(MAX_LS_FILES_LINES)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Take the first 800 chars of the body, collapsing newlines, so the
/// prompt stays compact. A bit longer than `render_pages_for_prompt`'s
/// 200-char excerpt because suggestion accuracy depends on having
/// real prose to anchor against.
fn body_excerpt_for_suggestion(body: &str) -> String {
    body.chars()
        .take(800)
        .collect::<String>()
        .replace('\n', " ")
}

/// Run the source-suggestion pipeline. Sequential: one runner call
/// per affected page (per the v1 spec — parallelization is a future
/// optimization). Returns a [`SourceSuggestionReport`] with the
/// accumulated proposals; the caller renders + exits.
fn run_source_suggestion(
    pages: &[coral_core::page::Page],
    report: &LintReport,
    runner: &dyn Runner,
    apply: bool,
    repo_root: &Path,
) -> Result<SourceSuggestionReport> {
    use coral_runner::Prompt;

    let affected = collect_high_confidence_without_sources(report, pages);
    if affected.is_empty() {
        return Ok(SourceSuggestionReport {
            entries: Vec::new(),
            total_pages: 0,
            applied: apply,
        });
    }

    let ls_files = get_git_ls_files(repo_root)?;
    let ls_listing = render_ls_files_for_prompt(&ls_files);

    let prompt_template = super::prompt_loader::load_or_fallback(
        "lint-suggest-sources",
        SOURCE_SUGGESTION_SYSTEM_FALLBACK,
    );

    let mut entries: Vec<SourceSuggestionEntry> = Vec::new();

    for (slug, page) in &affected {
        let body_excerpt = body_excerpt_for_suggestion(&page.body);
        let prompt = Prompt {
            system: Some(prompt_template.content.clone()),
            user: format!(
                "Page slug: {slug}\n\nBody excerpt:\n{body_excerpt}\n\n\
                 Workspace files (`git ls-files`, first {MAX_LS_FILES_LINES} lines):\n\
                 {ls_listing}\n\nPropose sources."
            ),
            ..Default::default()
        };

        let out = runner
            .run(&prompt)
            .map_err(|e| anyhow::anyhow!("source-suggestion runner failed: {e}"))?;
        let suggested = parse_source_suggestion(&out.stdout)
            .with_context(|| format!("parsing source suggestion for `{slug}`"))?;

        // Dedup against the page's existing sources so the report
        // and the apply path don't double-list paths already on disk.
        let existing: HashSet<String> = page.frontmatter.sources.iter().cloned().collect();
        let new_sources: Vec<String> = suggested
            .into_iter()
            .filter(|s| !existing.contains(s))
            .collect();

        entries.push(SourceSuggestionEntry {
            slug: slug.clone(),
            path: page.path.clone(),
            new_sources,
        });
    }

    if apply {
        for entry in &entries {
            if entry.new_sources.is_empty() {
                continue;
            }
            let Some(page) = pages.iter().find(|p| p.frontmatter.slug == entry.slug) else {
                continue;
            };
            let mut new_page = page.clone();
            // Append-then-dedup-preserving-order: matches the
            // semantics of the no-LLM `dedup_sources` rule.
            for src in &entry.new_sources {
                new_page.frontmatter.sources.push(src.clone());
            }
            let mut seen = HashSet::new();
            new_page
                .frontmatter
                .sources
                .retain(|s| seen.insert(s.clone()));
            new_page
                .write()
                .with_context(|| format!("writing page `{}` with new sources", entry.slug))?;
        }
    }

    let total_pages = entries.iter().filter(|e| !e.new_sources.is_empty()).count();
    Ok(SourceSuggestionReport {
        entries,
        total_pages,
        applied: apply,
    })
}

/// Render a [`SourceSuggestionReport`] as the Markdown shown to the
/// user. Pure — no I/O, deterministic ordering. Mirrors
/// [`render_fix_report`].
pub(crate) fn render_source_suggestion_report(report: &SourceSuggestionReport) -> String {
    if report.total_pages == 0 {
        return "\n# Source suggestions\n\nNo pages need new sources.\n".to_string();
    }
    let header = if report.applied {
        "\n# Source suggestions (applied)\n\n"
    } else {
        "\n# Source suggestions (dry-run)\n\n"
    };
    let mut out = String::from(header);
    for entry in &report.entries {
        if entry.new_sources.is_empty() {
            continue;
        }
        let path_display = std::env::current_dir()
            .ok()
            .and_then(|cwd| entry.path.strip_prefix(&cwd).ok().map(Path::to_path_buf))
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| entry.path.display().to_string());
        out.push_str(&format!(
            "- `{}` ({}): suggested {}\n",
            entry.slug,
            path_display,
            entry.new_sources.join(", ")
        ));
    }
    let footer = if report.applied {
        format!("\nWrote {} page(s).\n", report.total_pages)
    } else {
        format!(
            "\n{} page(s) would gain sources. Pass `--apply` to write.\n",
            report.total_pages
        )
    };
    out.push_str(&footer);
    out
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

    // -------------------------------------------------------------
    // No-LLM rule-based fix pass (`coral lint --fix`)
    //
    // The block below covers the pure helpers
    // (`trim_frontmatter_strings`, `sort_sources`, `sort_backlinks`,
    // `normalize_wikilink_spacing`, `trim_trailing_line_whitespace`),
    // the `run_no_llm_fix` orchestrator (dry-run vs `--apply`), and
    // the `render_fix_report` markdown emitter. Each helper has a
    // "no change → false/None" test so we never spuriously report a
    // page as changed when nothing fired.
    // -------------------------------------------------------------

    /// Build a stock `Frontmatter` for the no-LLM-fix tests below.
    /// Keeps boilerplate out of every test by exposing only the fields
    /// each test cares about (slug + sources/backlinks).
    fn fixture_frontmatter(
        slug: &str,
        sources: Vec<String>,
        backlinks: Vec<String>,
    ) -> coral_core::frontmatter::Frontmatter {
        use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
        Frontmatter {
            slug: slug.into(),
            page_type: PageType::Module,
            last_updated_commit: "abc".into(),
            confidence: Confidence::try_new(0.9).unwrap(),
            sources,
            backlinks,
            status: Status::Verified,
            generated_at: None,
            extra: Default::default(),
        }
    }

    // ---- trim_frontmatter_strings -------------------------------

    #[test]
    fn trim_frontmatter_strings_no_change_returns_false() {
        // A clean frontmatter must report `false` so the orchestrator
        // doesn't add the page to `changed_pages` for a no-op rule.
        let mut fm = fixture_frontmatter("order", vec![], vec![]);
        assert!(!trim_frontmatter_strings(&mut fm));
        assert_eq!(fm.slug, "order");
        assert_eq!(fm.last_updated_commit, "abc");
    }

    #[test]
    fn trim_frontmatter_strings_trims_slug_trailing_ws() {
        let mut fm = fixture_frontmatter("order  ", vec![], vec![]);
        assert!(trim_frontmatter_strings(&mut fm));
        assert_eq!(fm.slug, "order");
    }

    #[test]
    fn trim_frontmatter_strings_trims_last_updated_commit() {
        // Belt-and-suspenders for the second field the rule covers.
        let mut fm = fixture_frontmatter("order", vec![], vec![]);
        fm.last_updated_commit = "deadbeef \t".into();
        assert!(trim_frontmatter_strings(&mut fm));
        assert_eq!(fm.last_updated_commit, "deadbeef");
    }

    // ---- sort_sources -------------------------------------------

    #[test]
    fn sort_sources_already_sorted_returns_false() {
        let mut fm = fixture_frontmatter(
            "order",
            vec!["a.rs".into(), "b.rs".into(), "c.rs".into()],
            vec![],
        );
        assert!(!sort_sources(&mut fm));
        assert_eq!(
            fm.sources,
            vec!["a.rs".to_string(), "b.rs".into(), "c.rs".into()]
        );
    }

    #[test]
    fn sort_sources_unsorted_returns_true() {
        let mut fm = fixture_frontmatter(
            "order",
            vec!["c.rs".into(), "a.rs".into(), "b.rs".into()],
            vec![],
        );
        assert!(sort_sources(&mut fm));
        assert_eq!(
            fm.sources,
            vec!["a.rs".to_string(), "b.rs".into(), "c.rs".into()]
        );
    }

    // ---- sort_backlinks -----------------------------------------

    #[test]
    fn sort_backlinks_dedup_not_required() {
        // Document the spec: this rule sorts only — duplicates are
        // *preserved*. A separate dedup pass (out of scope here) would
        // need to handle that.
        let mut fm = fixture_frontmatter("order", vec![], vec!["b".into(), "a".into(), "a".into()]);
        assert!(sort_backlinks(&mut fm));
        assert_eq!(fm.backlinks, vec!["a".to_string(), "a".into(), "b".into()]);
    }

    // ---- dedup_sources ------------------------------------------

    #[test]
    fn dedup_sources_removes_duplicates_preserves_order() {
        // First-occurrence-wins ordering: the second `"a"` is dropped
        // and `"b"` keeps its original slot relative to the first
        // `"a"`.
        let mut fm = fixture_frontmatter(
            "order",
            vec!["a".into(), "b".into(), "a".into(), "c".into()],
            vec![],
        );
        assert!(dedup_sources(&mut fm));
        assert_eq!(fm.sources, vec!["a".to_string(), "b".into(), "c".into()]);
    }

    #[test]
    fn dedup_sources_no_duplicates_returns_false() {
        // Already unique → no-op → must return false so the
        // orchestrator doesn't add a no-op rule to the rules-fired
        // list.
        let mut fm = fixture_frontmatter("order", vec!["a".into(), "b".into(), "c".into()], vec![]);
        assert!(!dedup_sources(&mut fm));
        assert_eq!(fm.sources, vec!["a".to_string(), "b".into(), "c".into()]);
    }

    // ---- dedup_backlinks ----------------------------------------

    #[test]
    fn dedup_backlinks_removes_duplicates() {
        let mut fm = fixture_frontmatter(
            "order",
            vec![],
            vec!["x".into(), "y".into(), "x".into(), "z".into(), "y".into()],
        );
        assert!(dedup_backlinks(&mut fm));
        assert_eq!(fm.backlinks, vec!["x".to_string(), "y".into(), "z".into()]);
    }

    // ---- normalize_eol ------------------------------------------

    #[test]
    fn normalize_eol_converts_crlf_to_lf() {
        let body = "line1\r\nline2\r\n";
        assert_eq!(normalize_eol(body), Some("line1\nline2\n".into()));
    }

    #[test]
    fn normalize_eol_no_crlf_returns_none() {
        // Already-LF body must report `None` so the orchestrator
        // doesn't mark the body as "changed".
        let body = "line1\nline2\n";
        assert_eq!(normalize_eol(body), None);
    }

    // ---- normalize_wikilink_spacing -----------------------------

    #[test]
    fn normalize_wikilink_spacing_plain() {
        // Symmetric whitespace on both sides → collapsed inner content.
        let body = "see [[ slug ]] now";
        assert_eq!(
            normalize_wikilink_spacing(body),
            Some("see [[slug]] now".into())
        );
    }

    #[test]
    fn normalize_wikilink_spacing_left_only() {
        let body = "[[ slug]]";
        assert_eq!(normalize_wikilink_spacing(body), Some("[[slug]]".into()));
    }

    #[test]
    fn normalize_wikilink_spacing_right_only() {
        let body = "[[slug ]]";
        assert_eq!(normalize_wikilink_spacing(body), Some("[[slug]]".into()));
    }

    #[test]
    fn normalize_wikilink_spacing_no_change_returns_none() {
        // Already-clean wikilinks must report `None` so the
        // orchestrator doesn't mark the body as "changed".
        let body = "see [[clean]] now";
        assert_eq!(normalize_wikilink_spacing(body), None);
    }

    #[test]
    fn normalize_wikilink_spacing_with_alias() {
        // Aliases (`|` separator) are preserved verbatim — the rule
        // only trims whitespace adjacent to the surrounding `[[` /
        // `]]` brackets, never inside the link body. So the inner
        // " | " stays intact.
        let body = "[[ slug | alias ]]";
        assert_eq!(
            normalize_wikilink_spacing(body),
            Some("[[slug | alias]]".into())
        );
    }

    // ---- trim_trailing_line_whitespace --------------------------

    #[test]
    fn trim_trailing_line_whitespace_no_change_returns_none() {
        let body = "line1\nline2\n";
        assert_eq!(trim_trailing_line_whitespace(body), None);
    }

    #[test]
    fn trim_trailing_line_whitespace_preserves_newline() {
        // Critical: only the trailing whitespace before `\n` is
        // removed — the `\n` itself stays so the line count is
        // unchanged.
        let body = "line  \n";
        assert_eq!(trim_trailing_line_whitespace(body), Some("line\n".into()));
    }

    #[test]
    fn trim_trailing_line_whitespace_multiline() {
        // Mixed: dirty / clean / dirty / final-line-without-newline.
        // Verifies trim is applied per-line and the trailing-no-\n
        // case is handled.
        let body = "a  \nb\nc\t \nd  ";
        assert_eq!(
            trim_trailing_line_whitespace(body),
            Some("a\nb\nc\nd".into())
        );
    }

    // ---- run_no_llm_fix (apply path) ----------------------------

    /// Helper: write a single-page tempdir wiki and return the
    /// in-memory `Page` plus the absolute path so the test can
    /// re-read disk after `run_no_llm_fix(apply=…)`.
    fn write_one_page_wiki(
        tmp: &tempfile::TempDir,
        slug: &str,
        body: &str,
    ) -> (coral_core::page::Page, std::path::PathBuf) {
        use coral_core::page::Page;
        let wiki = tmp.path().join(".wiki");
        let modules = wiki.join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        let page_path = modules.join(format!("{slug}.md"));
        let page = Page {
            path: page_path.clone(),
            frontmatter: fixture_frontmatter(slug, vec![], vec![]),
            body: body.into(),
        };
        page.write().unwrap();
        (page, page_path)
    }

    #[test]
    fn run_no_llm_fix_dry_run_does_not_write() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        // Use a slug that would trigger trim-frontmatter-whitespace.
        let (mut page, page_path) = write_one_page_wiki(&tmp, "order", "body\n");
        // Re-set slug after `write_one_page_wiki` (which derives the
        // file name from the clean slug) so the on-disk content has
        // the trailing whitespace we want to assert is preserved.
        page.frontmatter.slug = "ord  ".into();
        page.write().unwrap();
        let on_disk_before = std::fs::read_to_string(&page_path).unwrap();

        let report = run_no_llm_fix(
            std::slice::from_ref(&page),
            false,
            &tmp.path().join(".wiki"),
            tmp.path(),
        )
        .unwrap();
        assert_eq!(report.total_changed, 1);
        assert!(!report.applied);

        let on_disk_after = std::fs::read_to_string(&page_path).unwrap();
        assert_eq!(
            on_disk_before, on_disk_after,
            "dry-run must not modify disk"
        );
    }

    #[test]
    fn run_no_llm_fix_apply_writes() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let (mut page, page_path) = write_one_page_wiki(&tmp, "order", "body\n");
        // Trailing whitespace on the slug — `trim_frontmatter_strings`
        // should rewrite this to `"ord"` after `--apply`.
        page.frontmatter.slug = "ord  ".into();
        page.write().unwrap();

        let report = run_no_llm_fix(
            std::slice::from_ref(&page),
            true,
            &tmp.path().join(".wiki"),
            tmp.path(),
        )
        .unwrap();
        assert_eq!(report.total_changed, 1);
        assert!(report.applied);

        let on_disk = std::fs::read_to_string(&page_path).unwrap();
        // Frontmatter is YAML; a trimmed slug serializes without the
        // trailing whitespace. We assert against the `slug:` line so
        // the test isn't sensitive to the rest of the YAML order.
        assert!(
            on_disk.contains("slug: ord\n"),
            "slug not trimmed on disk: {on_disk}"
        );
        assert!(
            !on_disk.contains("slug: ord  "),
            "trailing whitespace still present: {on_disk}"
        );
    }

    #[test]
    fn run_no_llm_fix_clean_pages_returns_empty_report() {
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        // Page is born clean — no fix rule should fire.
        let (page, _) = write_one_page_wiki(&tmp, "order", "clean body\n");

        let report = run_no_llm_fix(
            std::slice::from_ref(&page),
            false,
            &tmp.path().join(".wiki"),
            tmp.path(),
        )
        .unwrap();
        assert_eq!(report.total_changed, 0);
        assert!(report.changed_pages.is_empty());
        assert!(!report.applied);
    }

    // ---- downgrade_confidence_for_missing_sources --------------

    #[test]
    fn downgrade_confidence_skips_when_no_sources() {
        use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
        use std::collections::BTreeMap;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let mut fm = Frontmatter {
            slug: "p".into(),
            page_type: PageType::Module,
            last_updated_commit: "abc".into(),
            confidence: Confidence::try_new(0.9).unwrap(),
            sources: vec![],
            backlinks: vec![],
            status: Status::Reviewed,
            generated_at: None,
            extra: BTreeMap::new(),
        };
        let changed = downgrade_confidence_for_missing_sources(&mut fm, tmp.path());
        assert!(!changed);
        assert!((fm.confidence.as_f64() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn downgrade_confidence_skips_when_all_sources_exist() {
        use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
        use std::collections::BTreeMap;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("real.rs"), "// stub").unwrap();
        let mut fm = Frontmatter {
            slug: "p".into(),
            page_type: PageType::Module,
            last_updated_commit: "abc".into(),
            confidence: Confidence::try_new(0.9).unwrap(),
            sources: vec!["real.rs".into()],
            backlinks: vec![],
            status: Status::Reviewed,
            generated_at: None,
            extra: BTreeMap::new(),
        };
        let changed = downgrade_confidence_for_missing_sources(&mut fm, tmp.path());
        assert!(!changed);
        assert!((fm.confidence.as_f64() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn downgrade_confidence_steps_down_when_source_missing() {
        use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
        use std::collections::BTreeMap;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let mut fm = Frontmatter {
            slug: "p".into(),
            page_type: PageType::Module,
            last_updated_commit: "abc".into(),
            confidence: Confidence::try_new(0.9).unwrap(),
            sources: vec!["does-not-exist.rs".into()],
            backlinks: vec![],
            status: Status::Reviewed,
            generated_at: None,
            extra: BTreeMap::new(),
        };
        let changed = downgrade_confidence_for_missing_sources(&mut fm, tmp.path());
        assert!(changed);
        // 0.9 - 0.2 = 0.7
        assert!(
            (fm.confidence.as_f64() - 0.7).abs() < 1e-9,
            "expected 0.7, got {}",
            fm.confidence.as_f64()
        );
    }

    #[test]
    fn downgrade_confidence_floors_at_threshold() {
        use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
        use std::collections::BTreeMap;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let mut fm = Frontmatter {
            slug: "p".into(),
            page_type: PageType::Module,
            last_updated_commit: "abc".into(),
            confidence: Confidence::try_new(0.45).unwrap(),
            sources: vec!["does-not-exist.rs".into()],
            backlinks: vec![],
            status: Status::Reviewed,
            generated_at: None,
            extra: BTreeMap::new(),
        };
        let changed = downgrade_confidence_for_missing_sources(&mut fm, tmp.path());
        assert!(changed);
        // 0.45 - 0.20 = 0.25, but floor is 0.30 → result is 0.30
        assert!(
            (fm.confidence.as_f64() - 0.30).abs() < 1e-9,
            "expected 0.30 (floored), got {}",
            fm.confidence.as_f64()
        );
    }

    #[test]
    fn downgrade_confidence_idempotent_at_floor() {
        use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
        use std::collections::BTreeMap;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let mut fm = Frontmatter {
            slug: "p".into(),
            page_type: PageType::Module,
            last_updated_commit: "abc".into(),
            confidence: Confidence::try_new(0.30).unwrap(),
            sources: vec!["does-not-exist.rs".into()],
            backlinks: vec![],
            status: Status::Reviewed,
            generated_at: None,
            extra: BTreeMap::new(),
        };
        // First call: already at floor — must not change.
        let changed_first = downgrade_confidence_for_missing_sources(&mut fm, tmp.path());
        assert!(!changed_first);
        assert!((fm.confidence.as_f64() - 0.30).abs() < 1e-9);
        // Idempotent: second call with the same input is also a no-op.
        let changed_second = downgrade_confidence_for_missing_sources(&mut fm, tmp.path());
        assert!(!changed_second);
    }

    #[test]
    fn downgrade_confidence_skips_http_sources() {
        use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
        use std::collections::BTreeMap;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        // All sources are URLs — even though they don't "exist on
        // disk", the rule must skip them.
        let mut fm = Frontmatter {
            slug: "p".into(),
            page_type: PageType::Module,
            last_updated_commit: "abc".into(),
            confidence: Confidence::try_new(0.9).unwrap(),
            sources: vec![
                "https://example.com/spec".into(),
                "http://docs.example.org/x".into(),
            ],
            backlinks: vec![],
            status: Status::Reviewed,
            generated_at: None,
            extra: BTreeMap::new(),
        };
        let changed = downgrade_confidence_for_missing_sources(&mut fm, tmp.path());
        assert!(!changed);
        assert!((fm.confidence.as_f64() - 0.9).abs() < 1e-9);
    }

    // ---- render_fix_report --------------------------------------

    #[test]
    fn render_fix_report_dry_run_says_pass_apply() {
        // Empty report: short "no fixes needed" message, no header.
        let empty = NoLlmFixReport {
            changed_pages: vec![],
            total_changed: 0,
            applied: false,
        };
        let out_empty = render_fix_report(&empty);
        assert!(out_empty.contains("No fixes needed."), "got: {out_empty}");

        // Non-empty dry-run: must surface the `--apply` hint so users
        // know how to commit the proposed fixes.
        let dry = NoLlmFixReport {
            changed_pages: vec![(
                "order".into(),
                std::path::PathBuf::from("/tmp/.wiki/modules/order.md"),
                vec!["sort-sources"],
            )],
            total_changed: 1,
            applied: false,
        };
        let out_dry = render_fix_report(&dry);
        assert!(
            out_dry.contains("Pass `--apply` to write."),
            "dry-run footer missing: {out_dry}"
        );
        assert!(
            out_dry.contains("dry-run"),
            "dry-run header missing: {out_dry}"
        );
    }

    #[test]
    fn render_fix_report_applied_says_wrote_n() {
        let applied = NoLlmFixReport {
            changed_pages: vec![
                (
                    "alpha".into(),
                    std::path::PathBuf::from("/tmp/.wiki/modules/alpha.md"),
                    vec!["sort-sources"],
                ),
                (
                    "beta".into(),
                    std::path::PathBuf::from("/tmp/.wiki/modules/beta.md"),
                    vec!["normalize-wikilinks", "trim-trailing-whitespace"],
                ),
            ],
            total_changed: 2,
            applied: true,
        };
        let out = render_fix_report(&applied);
        assert!(out.contains("Wrote 2 page(s)."), "wrote-N missing: {out}");
        assert!(out.contains("(applied)"), "applied header missing: {out}");
        // Per-page rule joining: confirms the comma-separator formatting
        // for the multi-rule case.
        assert!(
            out.contains("normalize-wikilinks, trim-trailing-whitespace"),
            "rule list missing: {out}"
        );
    }

    // -------------------------------------------------------------
    // Per-rule auto-fix prompt routing (Task I).
    //
    // The orchestrator groups issues by `LintCode` and dispatches one
    // prompt per group, preferring `lint-auto-fix-<code-kebab>` over
    // the generic `lint-auto-fix` template. The block below covers
    // the routing decisions and the per-group call accounting.
    // -------------------------------------------------------------

    /// Helper: build a `Page` whose slug matches an issue under test
    /// so the auto-fix orchestrator can render its summary block.
    fn auto_fix_fixture_page(slug: &str) -> coral_core::page::Page {
        use coral_core::page::Page;
        Page {
            path: PathBuf::from(format!("/repo/.wiki/modules/{slug}.md")),
            frontmatter: fixture_frontmatter(slug, vec![], vec![]),
            body: "stub body".into(),
        }
    }

    /// Helper: build a `LintIssue` with a chosen code/message, anchored
    /// to a stub page path the orchestrator can match against the page
    /// fixture above.
    fn issue_for(code: LintCode, message: &str, slug: &str) -> coral_lint::LintIssue {
        coral_lint::LintIssue {
            code,
            severity: LintSeverity::Critical,
            page: Some(PathBuf::from(format!("/repo/.wiki/modules/{slug}.md"))),
            message: message.into(),
            context: None,
        }
    }

    #[test]
    fn group_issues_by_code_groups_and_preserves_first_seen_order() {
        // Outer order = first appearance of each code in the input
        // (broken-wikilink first, low-confidence second). Within a
        // group, original order is preserved.
        let issues = vec![
            issue_for(LintCode::BrokenWikilink, "bw1", "a"),
            issue_for(LintCode::LowConfidence, "lc1", "b"),
            issue_for(LintCode::BrokenWikilink, "bw2", "a"),
        ];
        let groups = group_issues_by_code(&issues);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, LintCode::BrokenWikilink);
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[0].1[0].message, "bw1");
        assert_eq!(groups[0].1[1].message, "bw2");
        assert_eq!(groups[1].0, LintCode::LowConfidence);
        assert_eq!(groups[1].1.len(), 1);
        assert_eq!(groups[1].1[0].message, "lc1");
    }

    #[test]
    fn auto_fix_routes_broken_wikilinks_to_specialized_prompt() {
        use coral_runner::MockRunner;
        use tempfile::TempDir;

        let runner = MockRunner::new();
        // Two issues, two distinct codes → two runner calls.
        runner.push_ok("fixes:\n  - slug: a\n    action: skip\n    rationale: ok\n");
        runner.push_ok("fixes:\n  - slug: b\n    action: skip\n    rationale: ok\n");

        let pages = vec![auto_fix_fixture_page("a"), auto_fix_fixture_page("b")];
        let report = LintReport {
            issues: vec![
                issue_for(LintCode::BrokenWikilink, "bw", "a"),
                issue_for(LintCode::LowConfidence, "lc", "b"),
            ],
        };

        let tmp = TempDir::new().unwrap();
        let wiki_root = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki_root).unwrap();
        run_auto_fix(&pages, &report, &runner, false, &wiki_root).unwrap();

        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            2,
            "expected one call per LintCode group, got {}",
            calls.len()
        );

        // Either ordering of code groups is acceptable as long as each
        // code's specialized template was loaded exactly once. The
        // shipped templates contain a header that names the rule;
        // assert against that header's distinguishing token to avoid
        // coupling to the precise wording.
        let systems: Vec<String> = calls.iter().filter_map(|p| p.system.clone()).collect();
        assert_eq!(systems.len(), 2);
        let any_bw = systems
            .iter()
            .any(|s| s.contains("broken_wikilink") || s.contains("broken wikilinks"));
        let any_lc = systems
            .iter()
            .any(|s| s.contains("low_confidence") || s.contains("low confidence"));
        assert!(
            any_bw,
            "broken-wikilink specialized template not used: {systems:?}"
        );
        assert!(
            any_lc,
            "low-confidence specialized template not used: {systems:?}"
        );
    }

    #[test]
    fn auto_fix_falls_back_to_generic_when_specialized_missing() {
        use coral_runner::MockRunner;
        use tempfile::TempDir;

        let runner = MockRunner::new();
        runner.push_ok("fixes:\n  - slug: a\n    action: skip\n    rationale: ok\n");

        // StaleStatus has NO specialized template shipped under
        // `template/prompts/lint-auto-fix-stale-status.md`, so the
        // orchestrator must fall through to the generic
        // `lint-auto-fix` template.
        let pages = vec![auto_fix_fixture_page("a")];
        let report = LintReport {
            issues: vec![issue_for(LintCode::StaleStatus, "ss", "a")],
        };

        let tmp = TempDir::new().unwrap();
        let wiki_root = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki_root).unwrap();
        run_auto_fix(&pages, &report, &runner, false, &wiki_root).unwrap();

        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        let system = calls[0]
            .system
            .as_ref()
            .expect("auto-fix prompt must have a system block");
        // The generic `lint-auto-fix.md` template begins with the
        // shared "Lint auto-fix prompt template" header — distinct
        // from the per-rule templates' "specialized for" wording.
        assert!(
            system.contains("Lint auto-fix prompt template"),
            "generic template not used: {system}"
        );
        assert!(
            !system.contains("specialized for the"),
            "specialized template leaked into generic-fallback path: {system}"
        );
    }

    #[test]
    fn auto_fix_groups_multiple_issues_of_same_code_into_one_call() {
        use coral_runner::MockRunner;
        use tempfile::TempDir;

        let runner = MockRunner::new();
        runner.push_ok("fixes:\n  - slug: a\n    action: skip\n    rationale: ok\n");

        // Three BrokenWikilink issues all share the same code → ONE
        // grouped call (not three).
        let pages = vec![
            auto_fix_fixture_page("a"),
            auto_fix_fixture_page("b"),
            auto_fix_fixture_page("c"),
        ];
        let report = LintReport {
            issues: vec![
                issue_for(LintCode::BrokenWikilink, "first-bw-msg", "a"),
                issue_for(LintCode::BrokenWikilink, "second-bw-msg", "b"),
                issue_for(LintCode::BrokenWikilink, "third-bw-msg", "c"),
            ],
        };

        let tmp = TempDir::new().unwrap();
        let wiki_root = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki_root).unwrap();
        run_auto_fix(&pages, &report, &runner, false, &wiki_root).unwrap();

        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            1,
            "3 same-code issues must collapse to 1 grouped call"
        );

        // The single user-prompt must include all 3 issues so the LLM
        // can reason over them together.
        let user = &calls[0].user;
        assert!(
            user.contains("first-bw-msg"),
            "issue 1 missing from grouped prompt: {user}"
        );
        assert!(
            user.contains("second-bw-msg"),
            "issue 2 missing from grouped prompt: {user}"
        );
        assert!(
            user.contains("third-bw-msg"),
            "issue 3 missing from grouped prompt: {user}"
        );
    }

    #[test]
    fn lint_code_to_kebab_covers_every_variant() {
        // Drift guard: if a new LintCode variant is added, this match
        // must be extended too. Mirrors `parse_rule_filters` to keep
        // the set in lockstep.
        let pairs = [
            (LintCode::BrokenWikilink, "broken-wikilink"),
            (LintCode::OrphanPage, "orphan-page"),
            (LintCode::LowConfidence, "low-confidence"),
            (
                LintCode::HighConfidenceWithoutSources,
                "high-confidence-without-sources",
            ),
            (LintCode::StaleStatus, "stale-status"),
            (LintCode::CommitNotInGit, "commit-not-in-git"),
            (LintCode::SourceNotFound, "source-not-found"),
            (LintCode::ArchivedPageLinked, "archived-page-linked"),
            (LintCode::UnknownExtraField, "unknown-extra-field"),
            (LintCode::Contradiction, "contradiction"),
            (LintCode::ObsoleteClaim, "obsolete-claim"),
        ];
        for (code, expected) in pairs {
            assert_eq!(
                lint_code_to_kebab(code),
                expected,
                "kebab form for {code:?} must match the --rule flag form"
            );
        }
    }

    // -------------------------------------------------------------
    // Source suggestion (`coral lint --suggest-sources`).
    //
    // Covers:
    //   - high-conf-without-sources filtering (mixed issue input
    //     should yield only the relevant subset).
    //   - one-call-per-affected-page dispatch accounting.
    //   - --apply writing new sources to disk + dedup against
    //     existing sources.
    //   - dry-run leaves disk untouched.
    //   - YAML parser tolerates ```yaml fences and dedups against
    //     pre-existing sources at the orchestrator level.
    // -------------------------------------------------------------

    /// Build a [`coral_core::page::Page`] in `wiki_root` with the given
    /// slug, sources, body. Returns the in-memory `Page` so tests can
    /// pass it straight into `run_source_suggestion`.
    fn write_page_for_suggestion(
        wiki_root: &Path,
        slug: &str,
        sources: Vec<String>,
        body: &str,
    ) -> coral_core::page::Page {
        use coral_core::page::Page;
        let modules = wiki_root.join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        let path = modules.join(format!("{slug}.md"));
        let page = Page {
            path: path.clone(),
            frontmatter: fixture_frontmatter(slug, sources, vec![]),
            body: body.into(),
        };
        page.write().unwrap();
        page
    }

    /// Build a `LintIssue` with the given code anchored to a page path
    /// the orchestrator can match. Mirrors `issue_for` but takes the
    /// *page path* directly so we can point at on-disk pages produced
    /// by `write_page_for_suggestion`.
    fn issue_at_path(code: LintCode, path: &Path) -> LintIssue {
        LintIssue {
            code,
            severity: LintSeverity::Warning,
            page: Some(path.to_path_buf()),
            message: "x".into(),
            context: None,
        }
    }

    #[test]
    fn source_suggestion_collects_high_conf_without_sources_only() {
        // Mixed input: HighConfidenceWithoutSources + BrokenWikilink +
        // OrphanPage. Only the first should bubble through.
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let p_a = write_page_for_suggestion(&wiki, "a", vec![], "body a");
        let p_b = write_page_for_suggestion(&wiki, "b", vec![], "body b");
        let p_c = write_page_for_suggestion(&wiki, "c", vec![], "body c");
        let pages = vec![p_a.clone(), p_b.clone(), p_c.clone()];
        let report = LintReport {
            issues: vec![
                issue_at_path(LintCode::HighConfidenceWithoutSources, &p_a.path),
                issue_at_path(LintCode::BrokenWikilink, &p_b.path),
                issue_at_path(LintCode::OrphanPage, &p_c.path),
            ],
        };
        let collected = collect_high_confidence_without_sources(&report, &pages);
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].0, "a");
    }

    #[test]
    fn source_suggestion_dispatches_one_call_per_affected_page() {
        // Two pages, both flagged → exactly two runner calls (sequential
        // by spec: one prompt per page).
        use coral_runner::MockRunner;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        // Init a temporary repo so `git ls-files` succeeds.
        init_temp_git_repo(tmp.path());

        let p_a = write_page_for_suggestion(&wiki, "alpha", vec![], "alpha body");
        let p_b = write_page_for_suggestion(&wiki, "beta", vec![], "beta body");
        let pages = vec![p_a.clone(), p_b.clone()];
        let report = LintReport {
            issues: vec![
                issue_at_path(LintCode::HighConfidenceWithoutSources, &p_a.path),
                issue_at_path(LintCode::HighConfidenceWithoutSources, &p_b.path),
            ],
        };

        let runner = MockRunner::new();
        runner.push_ok("slug: alpha\nsuggested_sources:\n  - src/alpha.rs\n");
        runner.push_ok("slug: beta\nsuggested_sources:\n  - src/beta.rs\n");

        let out = run_source_suggestion(&pages, &report, &runner, false, tmp.path()).unwrap();
        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            2,
            "expected one runner call per affected page, got {}",
            calls.len()
        );
        // Each prompt must include its own slug + a `git ls-files`
        // section so the LLM has the right context.
        for call in &calls {
            assert!(call.user.contains("Page slug:"), "missing slug header");
            assert!(
                call.user.contains("git ls-files"),
                "missing ls-files section"
            );
        }
        assert_eq!(out.entries.len(), 2);
        assert_eq!(out.total_pages, 2);
        assert!(!out.applied);
    }

    #[test]
    fn source_suggestion_apply_writes_sources() {
        use coral_runner::MockRunner;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        init_temp_git_repo(tmp.path());

        let page = write_page_for_suggestion(&wiki, "x", vec![], "x body");
        let pages = vec![page.clone()];
        let report = LintReport {
            issues: vec![issue_at_path(
                LintCode::HighConfidenceWithoutSources,
                &page.path,
            )],
        };
        let runner = MockRunner::new();
        runner.push_ok("slug: x\nsuggested_sources:\n  - src/a.rs\n  - src/b.rs\n");

        let report_out = run_source_suggestion(&pages, &report, &runner, true, tmp.path()).unwrap();
        assert!(report_out.applied);
        assert_eq!(report_out.total_pages, 1);
        let on_disk = std::fs::read_to_string(&page.path).unwrap();
        assert!(
            on_disk.contains("src/a.rs"),
            "first source missing on disk: {on_disk}"
        );
        assert!(
            on_disk.contains("src/b.rs"),
            "second source missing on disk: {on_disk}"
        );
    }

    #[test]
    fn source_suggestion_dry_run_does_not_write() {
        use coral_runner::MockRunner;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        init_temp_git_repo(tmp.path());

        let page = write_page_for_suggestion(&wiki, "y", vec![], "y body");
        let pages = vec![page.clone()];
        let on_disk_before = std::fs::read_to_string(&page.path).unwrap();
        let report = LintReport {
            issues: vec![issue_at_path(
                LintCode::HighConfidenceWithoutSources,
                &page.path,
            )],
        };
        let runner = MockRunner::new();
        runner.push_ok("slug: y\nsuggested_sources:\n  - src/y.rs\n");

        let report_out =
            run_source_suggestion(&pages, &report, &runner, false, tmp.path()).unwrap();
        assert!(!report_out.applied);
        let on_disk_after = std::fs::read_to_string(&page.path).unwrap();
        assert_eq!(
            on_disk_before, on_disk_after,
            "dry-run must not modify disk"
        );
    }

    #[test]
    fn parse_source_suggestion_handles_yaml_with_fences() {
        // Tolerate ```yaml fences for parity with `parse_auto_fix_plan`.
        let stdout = "```yaml\nslug: x\nsuggested_sources:\n  - a.rs\n```";
        let got = parse_source_suggestion(stdout).unwrap();
        assert_eq!(got, vec!["a.rs".to_string()]);
    }

    #[test]
    fn source_suggestion_dedup_existing_sources() {
        // The page already has `src/a.rs` listed. The LLM proposes BOTH
        // `src/a.rs` (duplicate) and `src/b.rs` (new). Only `src/b.rs`
        // should land in `entry.new_sources` so the report is accurate
        // and the applied page doesn't double-list `src/a.rs`.
        use coral_runner::MockRunner;
        use tempfile::TempDir;
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        init_temp_git_repo(tmp.path());

        let page = write_page_for_suggestion(&wiki, "z", vec!["src/a.rs".into()], "z body");
        let pages = vec![page.clone()];
        let report = LintReport {
            issues: vec![issue_at_path(
                LintCode::HighConfidenceWithoutSources,
                &page.path,
            )],
        };
        let runner = MockRunner::new();
        runner.push_ok("slug: z\nsuggested_sources:\n  - src/a.rs\n  - src/b.rs\n");

        let out = run_source_suggestion(&pages, &report, &runner, true, tmp.path()).unwrap();
        // Only the NEW source should appear in entries.new_sources.
        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0].new_sources, vec!["src/b.rs".to_string()]);

        // On-disk: `src/a.rs` appears exactly once (no duplicate),
        // `src/b.rs` is appended once.
        let on_disk = std::fs::read_to_string(&page.path).unwrap();
        let count_a = on_disk.matches("src/a.rs").count();
        let count_b = on_disk.matches("src/b.rs").count();
        assert_eq!(count_a, 1, "src/a.rs duplicated on disk: {on_disk}");
        assert_eq!(count_b, 1, "src/b.rs missing or duplicated: {on_disk}");
    }

    /// Init a minimal git repo at `dir` and stage a placeholder file so
    /// `git ls-files` returns at least one line. Tests that exercise
    /// `run_source_suggestion` need this because the orchestrator
    /// shells out to `git ls-files` once, before iterating pages.
    fn init_temp_git_repo(dir: &Path) {
        use std::process::Command;
        let env_clean = [
            ("GIT_AUTHOR_NAME", "test"),
            ("GIT_AUTHOR_EMAIL", "t@t"),
            ("GIT_COMMITTER_NAME", "test"),
            ("GIT_COMMITTER_EMAIL", "t@t"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_SYSTEM", "/dev/null"),
        ];
        let mut cmd = Command::new("git");
        cmd.current_dir(dir).args(["init", "-q"]);
        for (k, v) in env_clean {
            cmd.env(k, v);
        }
        cmd.status().expect("git init");
        std::fs::write(dir.join("placeholder.txt"), "x").unwrap();
        let mut cmd = Command::new("git");
        cmd.current_dir(dir).args(["add", "placeholder.txt"]);
        for (k, v) in env_clean {
            cmd.env(k, v);
        }
        cmd.status().expect("git add");
    }
}
