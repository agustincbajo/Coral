use anyhow::{Context, Result};
use clap::Args;
use coral_core::frontmatter::Status;
use coral_core::page::Page;
use coral_core::walk;
use coral_runner::{Prompt, Runner};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct ConsolidateArgs {
    #[arg(long)]
    pub model: Option<String>,
    /// LLM provider: claude (default) | gemini | local. Or set CORAL_PROVIDER env.
    #[arg(long)]
    pub provider: Option<String>,
    /// Apply the proposal: mark `retirements[]` pages as `status: stale`.
    /// `merges[]` and `splits[]` are surfaced as warnings — human review
    /// is required because they need body merging / partitioning that the
    /// MVP doesn't safely automate.
    #[arg(long)]
    pub apply: bool,
}

pub fn run(args: ConsolidateArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let provider = super::runner_helper::resolve_provider(args.provider.as_deref())
        .map_err(|e| anyhow::anyhow!(e))?;
    let runner = super::runner_helper::make_runner(provider);
    run_with_runner(args, wiki_root, runner.as_ref())
}

pub fn run_with_runner(
    args: ConsolidateArgs,
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

    let summary = pages
        .iter()
        .take(80)
        .map(|p| {
            format!(
                "- {} ({})",
                p.frontmatter.slug,
                serde_json::to_value(p.frontmatter.page_type)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt_template =
        super::prompt_loader::load_or_fallback("consolidate", CONSOLIDATE_SYSTEM_FALLBACK);
    let prompt = Prompt {
        system: Some(prompt_template.content),
        user: format!("Pages:\n{summary}\n\nProposed consolidations? Output YAML."),
        model: args.model,
        cwd: None,
        timeout: None,
    };

    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("runner failed: {e}"))?;

    if !args.apply {
        println!("# Consolidation suggestions (preview)\n");
        println!("{}", out.stdout);
        println!(
            "\n_(pass `--apply` to mark `retirements[]` slugs as stale; merges/splits stay manual.)_"
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Parse and apply.
    let plan = parse_consolidate_plan(&out.stdout)
        .context("parsing consolidate YAML plan (LLM output below)")?;
    let report = apply_consolidate_plan(&plan, &pages)?;
    println!("# Consolidation applied\n");
    println!("Retired: {} page(s)", report.retired.len());
    for slug in &report.retired {
        println!("- `{slug}` → status: stale");
    }
    if !report.unknown_retirements.is_empty() {
        println!(
            "\nWarning: retirements pointing at unknown slugs (skipped): {}",
            report.unknown_retirements.join(", ")
        );
    }
    if !plan.merges.is_empty() {
        println!(
            "\n{} merge proposal(s) need human review (body merging is not auto-applied):",
            plan.merges.len()
        );
        for m in &plan.merges {
            println!(
                "- target=`{}` sources={:?} — {}",
                m.target, m.sources, m.rationale
            );
        }
    }
    if !plan.splits.is_empty() {
        println!(
            "\n{} split proposal(s) need human review (body partitioning is not auto-applied):",
            plan.splits.len()
        );
        for s in &plan.splits {
            println!(
                "- source=`{}` targets={:?} — {}",
                s.source, s.targets, s.rationale
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
pub(crate) struct ConsolidatePlan {
    #[serde(default)]
    pub merges: Vec<MergeOp>,
    #[serde(default)]
    pub retirements: Vec<RetireOp>,
    #[serde(default)]
    pub splits: Vec<SplitOp>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct MergeOp {
    pub target: String,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct RetireOp {
    pub slug: String,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub(crate) struct SplitOp {
    pub source: String,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub rationale: String,
}

#[derive(Debug, Default)]
pub(crate) struct ApplyReport {
    pub retired: Vec<String>,
    pub unknown_retirements: Vec<String>,
}

pub(crate) fn parse_consolidate_plan(stdout: &str) -> Result<ConsolidatePlan> {
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

pub(crate) fn apply_consolidate_plan(
    plan: &ConsolidatePlan,
    pages: &[Page],
) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();
    for op in &plan.retirements {
        let Some(page) = pages.iter().find(|p| p.frontmatter.slug == op.slug) else {
            report.unknown_retirements.push(op.slug.clone());
            continue;
        };
        let mut new_page = Page {
            path: page.path.clone(),
            frontmatter: page.frontmatter.clone(),
            body: page.body.clone(),
        };
        new_page.frontmatter.status = Status::Stale;
        new_page
            .write()
            .with_context(|| format!("writing retired page `{}`", op.slug))?;
        report.retired.push(op.slug.clone());
    }
    Ok(report)
}

const CONSOLIDATE_SYSTEM_FALLBACK: &str = "You are the Coral wiki bibliotecario. Suggest page consolidations and archive candidates. \
     Output ONLY a YAML document with `merges:`, `retirements:`, `splits:` arrays. Each entry \
     has a one-sentence `rationale:`. Retirements need only `slug:` + `rationale:`.";

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType};
    use coral_runner::MockRunner;
    use tempfile::TempDir;

    fn page(wiki: &Path, slug: &str, status: Status) -> Page {
        let modules = wiki.join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        let p = Page {
            path: modules.join(format!("{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.into(),
                page_type: PageType::Module,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(0.7).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status,
                generated_at: None,
                extra: Default::default(),
            },
            body: format!("# {slug}\n\nbody."),
        };
        p.write().unwrap();
        p
    }

    #[test]
    fn consolidate_dry_run_prints_proposal_and_does_not_mutate() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let p = page(&wiki, "obsolete", Status::Reviewed);
        let runner = MockRunner::new();
        runner.push_ok("retirements:\n  - slug: obsolete\n    rationale: superseded\n");
        let exit =
            run_with_runner(ConsolidateArgs::default(), Some(wiki.as_path()), &runner).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        // Status unchanged.
        let on_disk = std::fs::read_to_string(&p.path).unwrap();
        assert!(on_disk.contains("status: reviewed"));
    }

    #[test]
    fn consolidate_apply_marks_retirements_stale() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let p = page(&wiki, "obsolete", Status::Reviewed);
        let runner = MockRunner::new();
        runner.push_ok(
            "retirements:\n  - slug: obsolete\n    rationale: superseded\n  - slug: ghost\n    rationale: never existed\n",
        );
        let exit = run_with_runner(
            ConsolidateArgs {
                apply: true,
                ..Default::default()
            },
            Some(wiki.as_path()),
            &runner,
        )
        .unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        let on_disk = std::fs::read_to_string(&p.path).unwrap();
        assert!(
            on_disk.contains("status: stale"),
            "page should be marked stale: {on_disk}"
        );
    }

    #[test]
    fn parse_consolidate_plan_handles_full_shape() {
        let yaml = "\
merges:
  - target: a-b
    sources: [a, b]
    rationale: redundant
retirements:
  - slug: ghost
    rationale: superseded
splits:
  - source: too-big
    targets: [part-a, part-b]
    rationale: covered two topics
";
        let plan = parse_consolidate_plan(yaml).unwrap();
        assert_eq!(plan.merges.len(), 1);
        assert_eq!(plan.merges[0].target, "a-b");
        assert_eq!(plan.retirements.len(), 1);
        assert_eq!(plan.retirements[0].slug, "ghost");
        assert_eq!(plan.splits.len(), 1);
        assert_eq!(plan.splits[0].targets, vec!["part-a", "part-b"]);
    }

    #[test]
    fn parse_consolidate_plan_handles_yaml_fence() {
        let yaml = "```yaml\nretirements:\n  - slug: x\n    rationale: y\n```";
        let plan = parse_consolidate_plan(yaml).unwrap();
        assert_eq!(plan.retirements[0].slug, "x");
    }
}
