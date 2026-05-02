use anyhow::{Context, Result};
use clap::Args;
use coral_core::walk;
use coral_lint::{
    LintReport, run_structural,
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
        let r = run_structural(&pages);
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

    if report.critical_count() > 0 {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
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
