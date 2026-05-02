use anyhow::{Context, Result};
use clap::Args;
use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::page::Page;
use coral_core::walk;
use coral_runner::{Prompt, Runner};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::plan::page_type_subdir;

#[derive(Args, Debug, Default)]
pub struct ConsolidateArgs {
    #[arg(long)]
    pub model: Option<String>,
    /// LLM provider: claude (default) | gemini | local. Or set CORAL_PROVIDER env.
    #[arg(long)]
    pub provider: Option<String>,
    /// Apply the proposal: mark `retirements[]` pages as `status: stale`,
    /// concatenate `merges[]` source bodies into a target page (marking sources stale),
    /// and create stub pages for `splits[]` targets (marking the source stale).
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
            "\n_(pass `--apply` to mark `retirements[]` slugs stale, materialize `merges[]` into a target page, and stub out `splits[]` targets.)_"
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

    if !report.merged.is_empty() {
        println!("\nMerged: {} target page(s)", report.merged.len());
        for (target, sources) in &report.merged {
            let formatted_sources = sources
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ");
            println!("- `{target}` ← {formatted_sources}");
        }
    }
    if !report.unknown_merge_targets.is_empty() {
        println!(
            "\nWarning: merge entries skipped (unknown sources or empty source list): {}",
            report.unknown_merge_targets.join(", ")
        );
    }

    if !report.split.is_empty() {
        println!("\nSplit: {} source page(s)", report.split.len());
        for (source, targets) in &report.split {
            let formatted_targets = targets
                .iter()
                .map(|t| format!("`{t}`"))
                .collect::<Vec<_>>()
                .join(", ");
            println!("- `{source}` → {formatted_targets}");
        }
    }
    if !report.unknown_split_sources.is_empty() {
        println!(
            "\nWarning: split entries skipped (source slug unknown or no targets): {}",
            report.unknown_split_sources.join(", ")
        );
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

/// Aggregated apply outcome — populated by `apply_consolidate_plan` and
/// consumed by the CLI's final print loop.
#[derive(Debug, Default)]
pub(crate) struct ApplyReport {
    /// Slugs that were marked `status: stale` via `retirements[]`.
    pub retired: Vec<String>,
    /// Retirement slugs that didn't resolve to any known page.
    pub unknown_retirements: Vec<String>,
    /// `(target_slug, source_slugs)` for each successful merge.
    pub merged: Vec<(String, Vec<String>)>,
    /// `(source_slug, target_slugs_created)` for each successful split.
    pub split: Vec<(String, Vec<String>)>,
    /// Merge entries skipped (no sources resolved, or `sources: []`).
    /// Each entry is the proposed target slug.
    pub unknown_merge_targets: Vec<String>,
    /// Split entries skipped (source slug unknown, or `targets: []`).
    /// Each entry is the proposed source slug.
    pub unknown_split_sources: Vec<String>,
}

/// Successful merge bookkeeping returned by `apply_merge`.
struct MergeOutcome {
    target_slug: String,
    source_slugs: Vec<String>,
}

/// Successful split bookkeeping returned by `apply_split`.
struct SplitOutcome {
    source_slug: String,
    created_targets: Vec<String>,
    skipped_targets: Vec<String>,
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

/// Applies a consolidation plan against the on-disk wiki. Mutates pages on
/// disk for retirements, merges, and splits. Returns a report describing
/// what was applied and what was skipped.
pub(crate) fn apply_consolidate_plan(
    plan: &ConsolidatePlan,
    pages: &[Page],
) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();

    // We mutate disk in-place; in-memory pages are only used as the read
    // baseline. To support a merge target whose body has been freshly
    // written (e.g. when target is also a source), we keep a
    // working_bodies map so a later step sees the freshly written body.
    let mut working_bodies: HashMap<String, String> = HashMap::new();

    // Retirements first — straightforward set-status-to-stale on each slug.
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

    // Merges + splits need a wiki root for new-page creation. We recover it
    // from any existing page's path: `<wiki_root>/<subdir>/<slug>.md` →
    // `parent().parent()`. If pages is empty we cannot create new pages, so
    // we error.
    let wiki_root = if !plan.merges.is_empty() || !plan.splits.is_empty() {
        Some(infer_wiki_root(pages)?)
    } else {
        None
    };
    let now = chrono::Utc::now().to_rfc3339();

    if let Some(root) = wiki_root.as_deref() {
        for merge in &plan.merges {
            match apply_merge(merge, pages, &mut working_bodies, root, &now) {
                Ok(Some(outcome)) => {
                    report
                        .merged
                        .push((outcome.target_slug, outcome.source_slugs));
                }
                Ok(None) => {
                    report.unknown_merge_targets.push(merge.target.clone());
                }
                Err(e) => {
                    return Err(e.context(format!("applying merge for target `{}`", merge.target)));
                }
            }
        }

        for split in &plan.splits {
            match apply_split(split, pages, root, &now) {
                Ok(Some(outcome)) => {
                    if !outcome.created_targets.is_empty() {
                        report
                            .split
                            .push((outcome.source_slug.clone(), outcome.created_targets));
                    }
                    if !outcome.skipped_targets.is_empty() {
                        eprintln!(
                            "warning: split source `{}` skipped existing targets: {}",
                            outcome.source_slug,
                            outcome.skipped_targets.join(", ")
                        );
                    }
                }
                Ok(None) => {
                    report.unknown_split_sources.push(split.source.clone());
                }
                Err(e) => {
                    return Err(e.context(format!("applying split for source `{}`", split.source)));
                }
            }
        }
    }

    Ok(report)
}

/// Recovers the wiki root from any page path by stripping
/// `<subdir>/<slug>.md`. Errors if `pages` is empty.
fn infer_wiki_root(pages: &[Page]) -> Result<PathBuf> {
    let first = pages.first().ok_or_else(|| {
        anyhow::anyhow!("cannot apply merges/splits: no pages exist to infer wiki root from")
    })?;
    first
        .path
        .parent()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot infer wiki root from page path `{}`: missing parents",
                first.path.display()
            )
        })
}

/// Materializes one merge entry: writes/updates the target page with
/// concatenated bodies from all sources, then marks every resolved source
/// `status: stale` with a footer pointing at the target. Returns
/// `Ok(None)` if the merge had no resolvable sources (skipped).
fn apply_merge(
    merge: &MergeOp,
    pages: &[Page],
    working_bodies: &mut HashMap<String, String>,
    wiki_root: &Path,
    now: &str,
) -> Result<Option<MergeOutcome>> {
    if merge.sources.is_empty() {
        return Ok(None);
    }

    // Resolve every source slug we can find. Sources we can't resolve are
    // dropped silently (the target body just won't include them).
    let resolved_sources: Vec<&Page> = merge
        .sources
        .iter()
        .filter_map(|slug| pages.iter().find(|p| p.frontmatter.slug == *slug))
        .collect();

    if resolved_sources.is_empty() {
        // Don't write a target made of nothing.
        return Ok(None);
    }

    let target_existing = pages.iter().find(|p| p.frontmatter.slug == merge.target);
    let target_in_sources = target_existing
        .map(|t| merge.sources.iter().any(|s| s == &t.frontmatter.slug))
        .unwrap_or(false);

    // Pick the path + page_type + base frontmatter for the target.
    let (target_path, target_page_type, base_frontmatter, base_body) =
        if let Some(t) = target_existing {
            (
                t.path.clone(),
                t.frontmatter.page_type,
                Some(t.frontmatter.clone()),
                t.body.clone(),
            )
        } else {
            // New target — type is the most common page_type among sources
            // (ties broken by alphabetical variant name).
            let inferred_type = most_common_page_type(&resolved_sources);
            let subdir = page_type_subdir(inferred_type);
            let path = if subdir == "." {
                wiki_root.join(format!("{}.md", merge.target))
            } else {
                wiki_root.join(subdir).join(format!("{}.md", merge.target))
            };
            (path, inferred_type, None, String::new())
        };

    // Build the concatenated body. Bodies come from `working_bodies`
    // (most recent on-disk write) when available, otherwise the cached
    // in-memory body. We keep the order from `merge.sources`.
    let mut new_body = base_body;
    for source_slug in &merge.sources {
        let Some(src_page) = resolved_sources
            .iter()
            .find(|p| p.frontmatter.slug == *source_slug)
        else {
            continue;
        };
        // If the target page is also one of the sources (in-place merge),
        // skip the duplicate concatenation of its own body.
        if target_in_sources
            && target_existing
                .map(|t| t.frontmatter.slug == src_page.frontmatter.slug)
                .unwrap_or(false)
        {
            continue;
        }
        let body = working_bodies
            .get(source_slug.as_str())
            .cloned()
            .unwrap_or_else(|| src_page.body.clone());
        if !new_body.is_empty() && !new_body.ends_with('\n') {
            new_body.push('\n');
        }
        new_body.push_str("\n---\n\n## Merged from `");
        new_body.push_str(source_slug);
        new_body.push_str("`\n\n");
        new_body.push_str(&body);
    }

    // Build target frontmatter (merge sources' metadata into base).
    let target_frontmatter = build_merged_frontmatter(
        &merge.target,
        target_page_type,
        base_frontmatter.as_ref(),
        &resolved_sources,
        &merge.sources,
        now,
    )?;

    let target_page = Page {
        path: target_path,
        frontmatter: target_frontmatter,
        body: new_body.clone(),
    };
    target_page
        .write()
        .with_context(|| format!("writing merged target page `{}`", merge.target))?;
    working_bodies.insert(merge.target.clone(), new_body);

    // Mark every resolved source page (excluding target if it's also a source)
    // stale + append footer. We do this in source-slug order from the
    // merge entry so the printed report is stable.
    let mut consumed_sources: Vec<String> = Vec::new();
    for source_slug in &merge.sources {
        // Skip if the source IS the target (in-place merge).
        if source_slug == &merge.target {
            consumed_sources.push(source_slug.clone());
            continue;
        }
        let Some(src_page) = resolved_sources
            .iter()
            .find(|p| p.frontmatter.slug == *source_slug)
        else {
            continue;
        };
        let mut updated = Page {
            path: src_page.path.clone(),
            frontmatter: src_page.frontmatter.clone(),
            body: src_page.body.clone(),
        };
        updated.frontmatter.status = Status::Stale;
        let mut footer = String::new();
        if !updated.body.is_empty() && !updated.body.ends_with('\n') {
            footer.push('\n');
        }
        footer.push_str("\n_Merged into `[[");
        footer.push_str(&merge.target);
        footer.push_str("]]` on ");
        footer.push_str(now);
        footer.push_str("._\n");
        updated.body.push_str(&footer);
        updated
            .write()
            .with_context(|| format!("writing stale source `{}`", source_slug))?;
        working_bodies.insert(source_slug.clone(), updated.body.clone());
        consumed_sources.push(source_slug.clone());
    }

    Ok(Some(MergeOutcome {
        target_slug: merge.target.clone(),
        source_slugs: consumed_sources,
    }))
}

/// Builds the merged target's frontmatter. Combines `sources`/`backlinks`
/// (deduped) and picks the lower of the existing target confidence and the
/// minimum source confidence. Status is forced to `draft` because merged
/// content needs review.
fn build_merged_frontmatter(
    target_slug: &str,
    target_page_type: PageType,
    existing_target: Option<&Frontmatter>,
    resolved_sources: &[&Page],
    declared_sources: &[String],
    now: &str,
) -> Result<Frontmatter> {
    let mut sources_union: Vec<String> = existing_target
        .map(|f| f.sources.clone())
        .unwrap_or_default();
    let mut backlinks_union: Vec<String> = existing_target
        .map(|f| f.backlinks.clone())
        .unwrap_or_default();

    for src in resolved_sources {
        for s in &src.frontmatter.sources {
            if !sources_union.contains(s) {
                sources_union.push(s.clone());
            }
        }
        for b in &src.frontmatter.backlinks {
            if !backlinks_union.contains(b) {
                backlinks_union.push(b.clone());
            }
        }
    }
    // Add the source slugs themselves to backlinks (pages that pointed at
    // a source now effectively backlink the target).
    for slug in declared_sources {
        if slug == target_slug {
            continue;
        }
        if !backlinks_union.contains(slug) {
            backlinks_union.push(slug.clone());
        }
    }

    // Confidence: lower of (existing target OR 0.5 default) and the
    // minimum across sources.
    let baseline = existing_target
        .map(|f| f.confidence.as_f64())
        .unwrap_or(0.5);
    let min_source = resolved_sources
        .iter()
        .map(|p| p.frontmatter.confidence.as_f64())
        .fold(f64::INFINITY, f64::min);
    let merged_conf = if min_source.is_finite() {
        baseline.min(min_source)
    } else {
        baseline
    };
    let confidence = Confidence::try_new(merged_conf)
        .context("building merged confidence (out-of-range source confidence)")?;

    let last_updated_commit = match existing_target {
        Some(f) if !f.last_updated_commit.is_empty() => f.last_updated_commit.clone(),
        _ => resolved_sources
            .first()
            .map(|p| p.frontmatter.last_updated_commit.clone())
            .unwrap_or_default(),
    };

    let extra: BTreeMap<String, serde_yaml_ng::Value> =
        existing_target.map(|f| f.extra.clone()).unwrap_or_default();
    let generated_at = existing_target
        .and_then(|f| f.generated_at.clone())
        .or_else(|| Some(now.to_string()));

    Ok(Frontmatter {
        slug: target_slug.to_string(),
        page_type: target_page_type,
        last_updated_commit,
        confidence,
        sources: sources_union,
        backlinks: backlinks_union,
        status: Status::Draft,
        generated_at,
        extra,
    })
}

/// Returns the most common `PageType` across `pages`. Ties broken by the
/// alphabetical order of the variant's serde name.
fn most_common_page_type(pages: &[&Page]) -> PageType {
    let mut counts: BTreeMap<String, (PageType, usize)> = BTreeMap::new();
    for p in pages {
        let key = serde_json::to_value(p.frontmatter.page_type)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        let entry = counts.entry(key).or_insert((p.frontmatter.page_type, 0));
        entry.1 += 1;
    }
    counts
        .into_iter()
        .max_by(|(name_a, (_, count_a)), (name_b, (_, count_b))| {
            // Higher count wins; ties go to whichever variant name is
            // alphabetically smaller (so we reverse the name comparison
            // to make smaller name "greater" under max_by).
            count_a.cmp(count_b).then_with(|| name_b.cmp(name_a))
        })
        .map(|(_, (page_type, _))| page_type)
        .unwrap_or(PageType::Concept)
}

/// Materializes one split entry: creates a stub page for each new target
/// (skipping any target slug that already exists), then marks the source
/// page `status: stale` with a footer pointing at the new targets.
/// Returns `Ok(None)` if the source slug doesn't resolve OR `targets: []`.
fn apply_split(
    split: &SplitOp,
    pages: &[Page],
    wiki_root: &Path,
    now: &str,
) -> Result<Option<SplitOutcome>> {
    if split.targets.is_empty() {
        return Ok(None);
    }
    let Some(source) = pages.iter().find(|p| p.frontmatter.slug == split.source) else {
        return Ok(None);
    };

    let mut created_targets: Vec<String> = Vec::new();
    let mut skipped_targets: Vec<String> = Vec::new();

    let subdir = page_type_subdir(source.frontmatter.page_type);
    for target_slug in &split.targets {
        if pages.iter().any(|p| p.frontmatter.slug == *target_slug) {
            skipped_targets.push(target_slug.clone());
            continue;
        }
        let path = if subdir == "." {
            wiki_root.join(format!("{}.md", target_slug))
        } else {
            wiki_root.join(subdir).join(format!("{}.md", target_slug))
        };
        let frontmatter = Frontmatter {
            slug: target_slug.clone(),
            page_type: source.frontmatter.page_type,
            last_updated_commit: source.frontmatter.last_updated_commit.clone(),
            confidence: Confidence::try_new(0.4)
                .context("building split-target confidence (0.4 should always validate)")?,
            sources: source.frontmatter.sources.clone(),
            backlinks: vec![source.frontmatter.slug.clone()],
            status: Status::Draft,
            generated_at: Some(now.to_string()),
            extra: BTreeMap::new(),
        };
        let body = format!(
            "# {}\n\n_Split from `[[{}]]` on {}. Fill in the body — this is a stub._\n\n_Source rationale: {}._\n",
            target_slug, source.frontmatter.slug, now, split.rationale
        );
        let stub = Page {
            path,
            frontmatter,
            body,
        };
        stub.write()
            .with_context(|| format!("writing split-target stub `{}`", target_slug))?;
        created_targets.push(target_slug.clone());
    }

    if created_targets.is_empty() {
        // Nothing was actually created — surface as a skip so the caller
        // doesn't claim success.
        return Ok(Some(SplitOutcome {
            source_slug: split.source.clone(),
            created_targets,
            skipped_targets,
        }));
    }

    // Mark source stale + append footer.
    let mut updated = Page {
        path: source.path.clone(),
        frontmatter: source.frontmatter.clone(),
        body: source.body.clone(),
    };
    updated.frontmatter.status = Status::Stale;
    let formatted_targets = split
        .targets
        .iter()
        .map(|t| format!("`[[{t}]]`"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut footer = String::new();
    if !updated.body.is_empty() && !updated.body.ends_with('\n') {
        footer.push('\n');
    }
    footer.push_str("\n_Split into ");
    footer.push_str(&formatted_targets);
    footer.push_str(" on ");
    footer.push_str(now);
    footer.push_str("._\n");
    updated.body.push_str(&footer);
    updated
        .write()
        .with_context(|| format!("writing stale split source `{}`", split.source))?;

    Ok(Some(SplitOutcome {
        source_slug: split.source.clone(),
        created_targets,
        skipped_targets,
    }))
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

    /// Like `page` but lets the caller customize body, confidence, page_type,
    /// and sources — enough to exercise every merge/split branch without
    /// needing one helper per shape.
    #[allow(clippy::too_many_arguments)]
    fn page_full(
        wiki: &Path,
        slug: &str,
        status: Status,
        page_type: PageType,
        confidence: f64,
        body: &str,
        sources: Vec<String>,
    ) -> Page {
        let subdir = page_type_subdir(page_type);
        let dir = if subdir == "." {
            wiki.to_path_buf()
        } else {
            wiki.join(subdir)
        };
        std::fs::create_dir_all(&dir).unwrap();
        let p = Page {
            path: dir.join(format!("{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.into(),
                page_type,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(confidence).unwrap(),
                sources,
                backlinks: vec![],
                status,
                generated_at: None,
                extra: Default::default(),
            },
            body: body.into(),
        };
        p.write().unwrap();
        p
    }

    /// Re-reads a page from disk using its slug + page_type to find the
    /// canonical path under `wiki_root`. Tests use this to confirm that
    /// `apply_*` actually mutated disk (vs. the in-memory `Page` value).
    fn read_back(wiki: &Path, page_type: PageType, slug: &str) -> Page {
        let subdir = page_type_subdir(page_type);
        let dir = if subdir == "." {
            wiki.to_path_buf()
        } else {
            wiki.join(subdir)
        };
        Page::from_file(dir.join(format!("{slug}.md"))).unwrap()
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

    // ---------- merge tests ----------

    #[test]
    fn apply_merge_in_place_uses_target_baseline_as_min_confidence() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        // Target IS one of the sources — the higher-confidence page absorbs
        // the lower-confidence one. Resulting confidence MUST be the
        // pairwise minimum.
        let a = page_full(
            &wiki,
            "a",
            Status::Reviewed,
            PageType::Module,
            0.9,
            "A body",
            vec![],
        );
        let b = page_full(
            &wiki,
            "b",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "B body",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![MergeOp {
                target: "a".into(),
                sources: vec!["a".into(), "b".into()],
                rationale: "redundant".into(),
            }],
            retirements: vec![],
            splits: vec![],
        };
        let report = apply_consolidate_plan(&plan, &[a.clone(), b.clone()]).unwrap();

        assert_eq!(report.merged.len(), 1);
        assert_eq!(report.merged[0].0, "a");
        assert_eq!(report.merged[0].1, vec!["a".to_string(), "b".to_string()]);
        assert!(report.unknown_merge_targets.is_empty());

        let target_after = read_back(&wiki, PageType::Module, "a");
        assert!(
            target_after.body.contains("A body"),
            "in-place merge should keep target body, got: {}",
            target_after.body
        );
        assert!(
            target_after.body.contains("Merged from `b`"),
            "missing merge marker for source `b`: {}",
            target_after.body
        );
        assert!(
            target_after.body.contains("B body"),
            "missing source `b` body in target: {}",
            target_after.body
        );
        assert!(
            (target_after.frontmatter.confidence.as_f64() - 0.7).abs() < f64::EPSILON,
            "confidence should be min(0.9, 0.7) = 0.7, got {}",
            target_after.frontmatter.confidence.as_f64()
        );
        assert_eq!(target_after.frontmatter.status, Status::Draft);

        let b_after = read_back(&wiki, PageType::Module, "b");
        assert_eq!(b_after.frontmatter.status, Status::Stale);
        assert!(
            b_after.body.contains("Merged into `[[a]]`"),
            "source `b` should have merge footer pointing at target: {}",
            b_after.body
        );
    }

    #[test]
    fn apply_merge_create_new_writes_target_under_inferred_subdir() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let a = page_full(
            &wiki,
            "a",
            Status::Reviewed,
            PageType::Module,
            0.8,
            "A body",
            vec!["src/a.rs".into()],
        );
        let b = page_full(
            &wiki,
            "b",
            Status::Reviewed,
            PageType::Module,
            0.8,
            "B body",
            vec!["src/b.rs".into()],
        );
        let plan = ConsolidatePlan {
            merges: vec![MergeOp {
                target: "ab".into(),
                sources: vec!["a".into(), "b".into()],
                rationale: "merge".into(),
            }],
            retirements: vec![],
            splits: vec![],
        };
        let report = apply_consolidate_plan(&plan, &[a.clone(), b.clone()]).unwrap();
        assert_eq!(report.merged.len(), 1);
        assert_eq!(report.merged[0].0, "ab");

        let target = read_back(&wiki, PageType::Module, "ab");
        assert_eq!(target.frontmatter.slug, "ab");
        assert_eq!(target.frontmatter.page_type, PageType::Module);
        assert_eq!(target.frontmatter.status, Status::Draft);
        // Body must contain both source bodies separated by the merge marker.
        assert!(target.body.contains("A body"));
        assert!(target.body.contains("B body"));
        assert!(target.body.contains("Merged from `a`"));
        assert!(target.body.contains("Merged from `b`"));
        // Sources union.
        assert!(target.frontmatter.sources.contains(&"src/a.rs".to_string()));
        assert!(target.frontmatter.sources.contains(&"src/b.rs".to_string()));
        // Backlinks include the source slugs themselves.
        assert!(target.frontmatter.backlinks.contains(&"a".to_string()));
        assert!(target.frontmatter.backlinks.contains(&"b".to_string()));

        let a_after = read_back(&wiki, PageType::Module, "a");
        let b_after = read_back(&wiki, PageType::Module, "b");
        assert_eq!(a_after.frontmatter.status, Status::Stale);
        assert_eq!(b_after.frontmatter.status, Status::Stale);
    }

    #[test]
    fn apply_merge_append_to_existing_target_not_in_sources() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let a = page_full(
            &wiki,
            "a",
            Status::Reviewed,
            PageType::Module,
            0.6,
            "A body",
            vec![],
        );
        let b = page_full(
            &wiki,
            "b",
            Status::Reviewed,
            PageType::Module,
            0.5,
            "B body",
            vec![],
        );
        let target = page_full(
            &wiki,
            "existing-target",
            Status::Reviewed,
            PageType::Module,
            0.95,
            "Existing",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![MergeOp {
                target: "existing-target".into(),
                sources: vec!["a".into(), "b".into()],
                rationale: "consolidate".into(),
            }],
            retirements: vec![],
            splits: vec![],
        };
        let report =
            apply_consolidate_plan(&plan, &[a.clone(), b.clone(), target.clone()]).unwrap();
        assert_eq!(report.merged.len(), 1);
        assert_eq!(report.merged[0].0, "existing-target");

        let target_after = read_back(&wiki, PageType::Module, "existing-target");
        // Existing body must be preserved at the start.
        assert!(
            target_after.body.starts_with("Existing"),
            "expected target body to start with 'Existing': {}",
            target_after.body
        );
        assert!(target_after.body.contains("Merged from `a`"));
        assert!(target_after.body.contains("Merged from `b`"));
        // Confidence: min(0.95, min(0.6, 0.5)) = 0.5.
        assert!(
            (target_after.frontmatter.confidence.as_f64() - 0.5).abs() < f64::EPSILON,
            "confidence should be min(0.95, 0.5) = 0.5, got {}",
            target_after.frontmatter.confidence.as_f64()
        );
        let a_after = read_back(&wiki, PageType::Module, "a");
        let b_after = read_back(&wiki, PageType::Module, "b");
        assert_eq!(a_after.frontmatter.status, Status::Stale);
        assert_eq!(b_after.frontmatter.status, Status::Stale);
    }

    #[test]
    fn apply_merge_skips_when_sources_empty() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        // Need at least one page so wiki root inference works.
        let _seed = page_full(
            &wiki,
            "seed",
            Status::Reviewed,
            PageType::Module,
            0.8,
            "seed",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![MergeOp {
                target: "x".into(),
                sources: vec![],
                rationale: String::new(),
            }],
            retirements: vec![],
            splits: vec![],
        };
        let report = apply_consolidate_plan(&plan, std::slice::from_ref(&_seed)).unwrap();
        assert!(report.merged.is_empty());
        assert!(
            report.unknown_merge_targets.contains(&"x".to_string()),
            "empty-sources merge should surface as unknown target, got {:?}",
            report.unknown_merge_targets
        );
        let target_path = wiki.join("modules").join("x.md");
        assert!(
            !target_path.exists(),
            "no target file should have been written: {:?}",
            target_path
        );
    }

    #[test]
    fn apply_merge_skips_when_all_sources_unknown() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let a = page_full(
            &wiki,
            "a",
            Status::Reviewed,
            PageType::Module,
            0.8,
            "A body",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![MergeOp {
                target: "x".into(),
                sources: vec!["ghost-1".into(), "ghost-2".into()],
                rationale: String::new(),
            }],
            retirements: vec![],
            splits: vec![],
        };
        let report = apply_consolidate_plan(&plan, std::slice::from_ref(&a)).unwrap();
        assert!(report.merged.is_empty());
        assert!(report.unknown_merge_targets.contains(&"x".to_string()));
        let target_path = wiki.join("modules").join("x.md");
        assert!(!target_path.exists());
    }

    // ---------- split tests ----------

    #[test]
    fn apply_split_creates_stub_targets_and_marks_source_stale() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let too_big = page_full(
            &wiki,
            "too-big",
            Status::Reviewed,
            PageType::Module,
            0.9,
            "original",
            vec!["src/big.rs".into()],
        );
        let plan = ConsolidatePlan {
            merges: vec![],
            retirements: vec![],
            splits: vec![SplitOp {
                source: "too-big".into(),
                targets: vec!["part-a".into(), "part-b".into()],
                rationale: "covered two topics".into(),
            }],
        };
        let report = apply_consolidate_plan(&plan, std::slice::from_ref(&too_big)).unwrap();
        assert_eq!(report.split.len(), 1);
        assert_eq!(report.split[0].0, "too-big");
        assert_eq!(
            report.split[0].1,
            vec!["part-a".to_string(), "part-b".to_string()]
        );

        let part_a = read_back(&wiki, PageType::Module, "part-a");
        let part_b = read_back(&wiki, PageType::Module, "part-b");
        for (slug, p) in [("part-a", &part_a), ("part-b", &part_b)] {
            assert_eq!(p.frontmatter.slug, slug);
            assert_eq!(p.frontmatter.page_type, PageType::Module);
            assert!(
                (p.frontmatter.confidence.as_f64() - 0.4).abs() < f64::EPSILON,
                "stub confidence should be 0.4, got {}",
                p.frontmatter.confidence.as_f64()
            );
            assert_eq!(p.frontmatter.status, Status::Draft);
            assert_eq!(p.frontmatter.sources, vec!["src/big.rs".to_string()]);
            assert_eq!(p.frontmatter.backlinks, vec!["too-big".to_string()]);
            assert!(p.body.contains(&format!("# {slug}")));
            assert!(p.body.contains("Split from `[[too-big]]`"));
            assert!(
                p.body.contains("covered two topics"),
                "stub body should include the split rationale: {}",
                p.body
            );
        }

        let too_big_after = read_back(&wiki, PageType::Module, "too-big");
        assert_eq!(too_big_after.frontmatter.status, Status::Stale);
        assert!(
            too_big_after.body.contains("Split into"),
            "source should have split footer: {}",
            too_big_after.body
        );
        assert!(
            too_big_after.body.contains("`[[part-a]]`")
                && too_big_after.body.contains("`[[part-b]]`"),
            "split footer should reference both targets: {}",
            too_big_after.body
        );
    }

    #[test]
    fn apply_split_skips_existing_target_but_creates_others() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let too_big = page_full(
            &wiki,
            "too-big",
            Status::Reviewed,
            PageType::Module,
            0.9,
            "original",
            vec![],
        );
        let part_a = page_full(
            &wiki,
            "part-a",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "preexisting",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![],
            retirements: vec![],
            splits: vec![SplitOp {
                source: "too-big".into(),
                targets: vec!["part-a".into(), "part-b".into()],
                rationale: "rationale".into(),
            }],
        };
        let report = apply_consolidate_plan(&plan, &[too_big.clone(), part_a.clone()]).unwrap();
        assert_eq!(report.split.len(), 1);
        assert_eq!(report.split[0].0, "too-big");
        // Only the newly-created `part-b` is reported.
        assert_eq!(report.split[0].1, vec!["part-b".to_string()]);

        // part-a stays untouched.
        let part_a_after = read_back(&wiki, PageType::Module, "part-a");
        assert_eq!(part_a_after.body, "preexisting");
        assert_eq!(part_a_after.frontmatter.status, Status::Reviewed);

        // part-b was created.
        let part_b_after = read_back(&wiki, PageType::Module, "part-b");
        assert_eq!(part_b_after.frontmatter.slug, "part-b");

        // too-big is stale because at least one target was created.
        let too_big_after = read_back(&wiki, PageType::Module, "too-big");
        assert_eq!(too_big_after.frontmatter.status, Status::Stale);
    }

    #[test]
    fn apply_split_skips_when_source_unknown() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let seed = page_full(
            &wiki,
            "seed",
            Status::Reviewed,
            PageType::Module,
            0.8,
            "seed",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![],
            retirements: vec![],
            splits: vec![SplitOp {
                source: "ghost".into(),
                targets: vec!["a".into(), "b".into()],
                rationale: String::new(),
            }],
        };
        let report = apply_consolidate_plan(&plan, std::slice::from_ref(&seed)).unwrap();
        assert!(report.split.is_empty());
        assert!(
            report.unknown_split_sources.contains(&"ghost".to_string()),
            "unknown source should be reported, got {:?}",
            report.unknown_split_sources
        );
        assert!(!wiki.join("modules").join("a.md").exists());
        assert!(!wiki.join("modules").join("b.md").exists());
    }

    #[test]
    fn apply_split_skips_when_targets_empty() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let too_big = page_full(
            &wiki,
            "too-big",
            Status::Reviewed,
            PageType::Module,
            0.9,
            "original",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![],
            retirements: vec![],
            splits: vec![SplitOp {
                source: "too-big".into(),
                targets: vec![],
                rationale: String::new(),
            }],
        };
        let report = apply_consolidate_plan(&plan, std::slice::from_ref(&too_big)).unwrap();
        assert!(report.split.is_empty());
        assert!(
            report
                .unknown_split_sources
                .contains(&"too-big".to_string()),
            "empty-targets split should be reported, got {:?}",
            report.unknown_split_sources
        );
        // Source must NOT be marked stale (no targets were created).
        let too_big_after = read_back(&wiki, PageType::Module, "too-big");
        assert_eq!(too_big_after.frontmatter.status, Status::Reviewed);
    }

    // ---------- combined plan ----------

    #[test]
    fn apply_consolidate_plan_handles_retire_merge_and_split_together() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let gone = page_full(
            &wiki,
            "gone",
            Status::Reviewed,
            PageType::Module,
            0.5,
            "gone body",
            vec![],
        );
        let a = page_full(
            &wiki,
            "a",
            Status::Reviewed,
            PageType::Module,
            0.8,
            "A body",
            vec![],
        );
        let b = page_full(
            &wiki,
            "b",
            Status::Reviewed,
            PageType::Module,
            0.8,
            "B body",
            vec![],
        );
        let too_big = page_full(
            &wiki,
            "too-big",
            Status::Reviewed,
            PageType::Module,
            0.9,
            "too big",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![MergeOp {
                target: "ab".into(),
                sources: vec!["a".into(), "b".into()],
                rationale: "redundant".into(),
            }],
            retirements: vec![RetireOp {
                slug: "gone".into(),
                rationale: "obsolete".into(),
            }],
            splits: vec![SplitOp {
                source: "too-big".into(),
                targets: vec!["p1".into(), "p2".into()],
                rationale: "two topics".into(),
            }],
        };
        let report = apply_consolidate_plan(
            &plan,
            &[gone.clone(), a.clone(), b.clone(), too_big.clone()],
        )
        .unwrap();
        assert_eq!(report.retired.len(), 1);
        assert_eq!(report.retired[0], "gone");
        assert_eq!(report.merged.len(), 1);
        assert_eq!(report.merged[0].0, "ab");
        assert_eq!(report.split.len(), 1);
        assert_eq!(report.split[0].0, "too-big");

        // gone is stale.
        let gone_after = read_back(&wiki, PageType::Module, "gone");
        assert_eq!(gone_after.frontmatter.status, Status::Stale);

        // ab created, a + b stale.
        let ab = read_back(&wiki, PageType::Module, "ab");
        assert_eq!(ab.frontmatter.slug, "ab");
        assert_eq!(
            read_back(&wiki, PageType::Module, "a").frontmatter.status,
            Status::Stale
        );
        assert_eq!(
            read_back(&wiki, PageType::Module, "b").frontmatter.status,
            Status::Stale
        );

        // p1, p2 created and too-big stale.
        let p1 = read_back(&wiki, PageType::Module, "p1");
        let p2 = read_back(&wiki, PageType::Module, "p2");
        assert_eq!(p1.frontmatter.slug, "p1");
        assert_eq!(p2.frontmatter.slug, "p2");
        assert_eq!(
            read_back(&wiki, PageType::Module, "too-big")
                .frontmatter
                .status,
            Status::Stale
        );
    }
}
