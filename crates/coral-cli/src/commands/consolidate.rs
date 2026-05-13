use anyhow::{Context, Result};
use clap::Args;
use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::page::Page;
use coral_core::project::Project;
use coral_core::walk;
use coral_runner::{MultiStepRunner, Prompt, Runner, TieredRunner};
use regex::Regex;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;

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
    /// After applying merges/splits, scan every other page and rewrite outbound
    /// `[[wikilinks]]` that point at retired source slugs so they point at the
    /// merge target (or, for splits, the FIRST split target). Requires `--apply`.
    #[arg(long)]
    pub rewrite_links: bool,
    /// v0.21.4: route this consolidate run through a tiered
    /// planner→executor→reviewer pipeline. Reads `[runner.tiered]`
    /// from `coral.toml` to pick per-tier providers / models / budget.
    /// CLI flag wins over the manifest's `[runner.tiered.consolidate]
    /// enabled = true` opt-in.
    #[arg(long)]
    pub tiered: bool,
    /// v0.21.4: emit a single `tracing::info` summary line after a
    /// tiered run with planner/executor/reviewer call counts and the
    /// approximate `tokens_used`. No effect on non-tiered runs.
    #[arg(long)]
    pub verbose: bool,
    /// v0.24.2: run garbage collection analysis — detects orphan pages,
    /// stale/broken backlinks, and archived pages still referenced by
    /// live pages. Outputs a read-only report (no mutations).
    #[arg(long)]
    pub gc: bool,
    /// Output format for `--gc` report: `markdown` (default) or `json`.
    #[arg(long, default_value = "markdown")]
    pub format: GcFormat,
}

/// Output format for the `--gc` report.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum GcFormat {
    #[default]
    Markdown,
    Json,
}

pub fn run(args: ConsolidateArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    // ── GC mode (v0.24.2) ─────────────────────────────────────────────
    // When `--gc` is passed, we skip the LLM consolidation entirely and
    // run a fast, purely local garbage-collection analysis.
    if args.gc {
        return run_gc(&args, wiki_root);
    }

    // v0.21.4: discover the project manifest so we can read
    // `[runner.tiered]`. If no manifest exists (legacy single-repo
    // case) Project::discover synthesizes a default project whose
    // `runner` is `RunnerSection::default()` — i.e. tiered = off.
    // Project::discover walks upward from cwd; we use that for
    // manifest lookup but keep the wiki root logic exactly as before.
    let cwd = std::env::current_dir().context("resolving current directory")?;
    let project = Project::discover(&cwd).unwrap_or_else(|err| {
        // A malformed `coral.toml` would error here. We surface it to
        // the user via the same path the rest of Coral takes — but
        // wrap rather than panic so the consolidate command stays
        // usable when only `--tiered` was relevant.
        tracing::warn!(error = ?err, "could not load project manifest, defaulting to no tiered routing");
        Project::synthesize_legacy(&cwd)
    });

    // CLI flag wins over manifest. Default = single-tier (BC).
    let use_tiered = args.tiered || project.runner.tiered_enabled_for_consolidate();

    if use_tiered {
        let tiered_cfg = project.runner.tiered.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "--tiered requested but no [runner.tiered] block in coral.toml. \
                 Add [runner.tiered.planner], [runner.tiered.executor], and \
                 [runner.tiered.reviewer] (and optionally [runner.tiered.budget] / \
                 [runner.tiered.consolidate]) to your coral.toml. \
                 See docs/runner-tiered.md for the full schema."
            )
        })?;
        let tiered = build_tiered_runner(tiered_cfg).map_err(|e| anyhow::anyhow!(e))?;
        run_with_tiered_runner(args, wiki_root, &tiered)
    } else {
        let provider = super::runner_helper::resolve_provider(args.provider.as_deref())
            .map_err(|e| anyhow::anyhow!(e))?;
        let runner = super::runner_helper::make_runner(provider);
        run_with_runner(args, wiki_root, runner.as_ref())
    }
}

/// v0.24.2: runs the `--gc` garbage collection analysis and prints
/// the report in the requested format. No LLM calls, no disk mutations.
fn run_gc(args: &ConsolidateArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
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

    let report = coral_core::gc_analyze(&pages);

    match args.format {
        GcFormat::Markdown => print!("{}", coral_core::gc_render_markdown(&report)),
        GcFormat::Json => println!("{}", coral_core::gc_render_json(&report)),
    }

    if report.is_clean() {
        Ok(ExitCode::SUCCESS)
    } else {
        // Non-zero exit so CI scripts can gate on wiki cleanliness.
        Ok(ExitCode::from(1))
    }
}

/// v0.21.4: builds a [`TieredRunner`] from a parsed manifest's
/// `[runner.tiered]` block. Each tier's `provider` is resolved at
/// call time (not parse time) so an unknown provider name surfaces a
/// clean error here rather than at first network call.
pub(crate) fn build_tiered_runner(
    cfg: &coral_core::project::manifest::TieredManifest,
) -> Result<TieredRunner, String> {
    let planner = super::runner_helper::make_runner_for_provider_str(&cfg.planner.provider)
        .map_err(|e| format!("[runner.tiered.planner]: {e}"))?;
    let executor = super::runner_helper::make_runner_for_provider_str(&cfg.executor.provider)
        .map_err(|e| format!("[runner.tiered.executor]: {e}"))?;
    let reviewer = super::runner_helper::make_runner_for_provider_str(&cfg.reviewer.provider)
        .map_err(|e| format!("[runner.tiered.reviewer]: {e}"))?;

    let tcfg = coral_runner::TieredConfig {
        planner: coral_runner::TierSpec {
            provider: cfg.planner.provider.clone(),
            model: cfg.planner.model.clone(),
        },
        executor: coral_runner::TierSpec {
            provider: cfg.executor.provider.clone(),
            model: cfg.executor.model.clone(),
        },
        reviewer: coral_runner::TierSpec {
            provider: cfg.reviewer.provider.clone(),
            model: cfg.reviewer.model.clone(),
        },
        budget: coral_runner::BudgetConfig {
            max_tokens_per_run: cfg.budget.max_tokens_per_run,
        },
    };
    Ok(TieredRunner::new(planner, executor, reviewer, tcfg))
}

/// Build the consolidate prompt + load wiki pages. Shared by single-
/// and multi-tier paths so byte-identity to v0.21.3 is preserved on
/// the non-tiered path. Returns `(prompt, pages, wiki_root)`.
fn build_consolidate_prompt(
    args: &ConsolidateArgs,
    wiki_root: Option<&Path>,
) -> Result<(Prompt, Vec<Page>, PathBuf)> {
    if args.rewrite_links && !args.apply {
        anyhow::bail!("--rewrite-links requires --apply");
    }
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
        model: args.model.clone(),
        cwd: None,
        timeout: None,
    };
    Ok((prompt, pages, root))
}

/// Render `stdout` as a consolidate plan and apply (or preview)
/// against `pages`. Shared by tiered + non-tiered paths so the
/// post-LLM stdout handling is identical regardless of routing.
///
/// Internal note (v0.21.4): the actual rendering happens in
/// [`format_consolidate_output`] (returns a `String`); this thin
/// wrapper performs the side effects (apply-plan disk mutations,
/// `println!`). Tests reach for the formatter directly so the
/// byte-identity snapshot doesn't have to capture stdout.
fn render_or_apply_plan(
    stdout: &str,
    args: &ConsolidateArgs,
    pages: &[Page],
    root: &Path,
) -> Result<ExitCode> {
    if !args.apply {
        let formatted = format_preview_output(stdout);
        print!("{formatted}");
        return Ok(ExitCode::SUCCESS);
    }

    // Parse and apply.
    let plan = parse_consolidate_plan(stdout)
        .context("parsing consolidate YAML plan (LLM output below)")?;
    let report = apply_consolidate_plan(&plan, pages, root, args.rewrite_links)?;
    let formatted = format_apply_report(&report);
    print!("{formatted}");
    Ok(ExitCode::SUCCESS)
}

/// Pre-apply preview rendering. Shape MUST match v0.21.3 byte-for-byte:
/// header line, then the LLM stdout verbatim, then the help-line
/// nudging the user toward `--apply`. Each `println!` in v0.21.3 emits
/// `text + '\n'`; we mirror that exactly.
pub(crate) fn format_preview_output(stdout: &str) -> String {
    // v0.21.3 emitted:
    //   println!("# Consolidation suggestions (preview)\n");
    //   println!("{}", out.stdout);
    //   println!("\n_(pass `--apply` …)_");
    // i.e. literal sequence
    //   "# Consolidation suggestions (preview)\n\n"  // header + \n
    //   "<stdout>\n"
    //   "\n_(...)_\n"
    let mut out = String::new();
    out.push_str("# Consolidation suggestions (preview)\n\n");
    out.push_str(stdout);
    out.push('\n');
    out.push_str(
        "\n_(pass `--apply` to mark `retirements[]` slugs stale, materialize `merges[]` into a target page, and stub out `splits[]` targets.)_\n",
    );
    out
}

/// Post-apply rendering. Like `format_preview_output`, this MUST match
/// v0.21.3 byte-for-byte for AC-2 (snapshot byte-identity to v0.21.3).
pub(crate) fn format_apply_report(report: &ApplyReport) -> String {
    let mut out = String::new();
    out.push_str("# Consolidation applied\n\n");
    out.push_str(&format!("Retired: {} page(s)\n", report.retired.len()));
    for slug in &report.retired {
        out.push_str(&format!("- `{slug}` → status: stale\n"));
    }
    if !report.unknown_retirements.is_empty() {
        out.push_str(&format!(
            "\nWarning: retirements pointing at unknown slugs (skipped): {}\n",
            report.unknown_retirements.join(", ")
        ));
    }

    if !report.merged.is_empty() {
        out.push_str(&format!(
            "\nMerged: {} target page(s)\n",
            report.merged.len()
        ));
        for (target, sources) in &report.merged {
            let formatted_sources = sources
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("- `{target}` ← {formatted_sources}\n"));
        }
    }
    if !report.unknown_merge_targets.is_empty() {
        out.push_str(&format!(
            "\nWarning: merge entries skipped (unknown sources or empty source list): {}\n",
            report.unknown_merge_targets.join(", ")
        ));
    }

    if !report.split.is_empty() {
        out.push_str(&format!("\nSplit: {} source page(s)\n", report.split.len()));
        for (source, targets) in &report.split {
            let formatted_targets = targets
                .iter()
                .map(|t| format!("`{t}`"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("- `{source}` → {formatted_targets}\n"));
        }
    }
    if !report.unknown_split_sources.is_empty() {
        out.push_str(&format!(
            "\nWarning: split entries skipped (source slug unknown or no targets): {}\n",
            report.unknown_split_sources.join(", ")
        ));
    }

    if !report.rewrites.is_empty() {
        out.push_str(&format!(
            "\nRewrites: {} page(s) patched\n",
            report.rewrites.len()
        ));
        for entry in &report.rewrites {
            let edits = entry
                .from_to
                .iter()
                .map(|(from, to)| format!("`{from}` → `{to}`"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("- `{}` — {edits}\n", entry.page_slug));
        }
    }
    out
}

pub fn run_with_runner(
    args: ConsolidateArgs,
    wiki_root: Option<&Path>,
    runner: &dyn Runner,
) -> Result<ExitCode> {
    let (prompt, pages, root) = build_consolidate_prompt(&args, wiki_root)?;
    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("runner failed: {e}"))?;
    render_or_apply_plan(&out.stdout, &args, &pages, &root)
}

/// v0.21.4: tiered counterpart of `run_with_runner`. The reviewer's
/// `RunOutput.stdout` is the consolidate-plan parser input — i.e. AC-4
/// (reviewer stdout becomes the canonical YAML plan).
pub fn run_with_tiered_runner(
    args: ConsolidateArgs,
    wiki_root: Option<&Path>,
    runner: &dyn MultiStepRunner,
) -> Result<ExitCode> {
    let (prompt, pages, root) = build_consolidate_prompt(&args, wiki_root)?;
    let out = runner
        .run_tiered(&prompt)
        .map_err(|e| anyhow::anyhow!("tiered runner failed: {e}"))?;
    if args.verbose {
        // AC-15: --verbose prints one tracing::info summary line.
        tracing::info!(
            plan_calls = out.plan_calls.len(),
            execute_calls = out.execute_calls.len(),
            review_calls = out.review_calls.len(),
            tokens_used = out.tokens_used,
            "consolidate --tiered: tiered run summary"
        );
    }
    render_or_apply_plan(&out.final_output.stdout, &args, &pages, &root)
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
    /// Per-page summaries of outbound `[[wikilink]]` rewrites performed when
    /// `--rewrite-links` was set. Pages with zero rewrites are omitted.
    pub rewrites: Vec<RewriteSummary>,
}

/// One page's worth of `[[wikilink]]` rewrites, returned as part of
/// `ApplyReport.rewrites` when the caller passed `rewrite_links = true` to
/// `apply_consolidate_plan`.
#[derive(Debug, Clone)]
pub(crate) struct RewriteSummary {
    /// Slug of the page whose body was patched.
    pub page_slug: String,
    /// Pairs of `(old_target, new_target)` for each distinct slug rewrite that
    /// landed in this page. Order matches the rewrite map iteration; aliased
    /// and anchored forms (`[[X|alias]]`, `[[X#anchor]]`) collapse into one
    /// entry per source slug.
    pub from_to: Vec<(String, String)>,
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
    let trimmed = super::plan::strip_yaml_fence(stdout);
    Ok(serde_yaml_ng::from_str(trimmed)?)
}

/// Applies a consolidation plan against the on-disk wiki. Mutates pages on
/// disk for retirements, merges, and splits. Returns a report describing
/// what was applied and what was skipped.
///
/// When `rewrite_links` is `true`, after merges and splits land, every other
/// page in the wiki is scanned for outbound `[[wikilinks]]` pointing at a
/// retired source slug and rewritten to point at the merge target (or, for
/// splits, the FIRST split target). See
/// [`rewrite_outbound_links_to_merged_targets`] for details.
///
/// `wiki_root` is the absolute (or canonical relative) path to the
/// `.wiki/` directory. v0.19.4 made it explicit (was previously
/// inferred from the first page's path via `parent().parent()` —
/// see GitHub issue #21). Threading the root from the caller avoids
/// the empty-PathBuf foot-gun that surfaced when pages lived at
/// `<wiki>/<slug>.md` (one level deep) instead of the typical
/// `<wiki>/<type>/<slug>.md`.
pub(crate) fn apply_consolidate_plan(
    plan: &ConsolidatePlan,
    pages: &[Page],
    wiki_root: &Path,
    rewrite_links: bool,
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

    // Merges + splits need a wiki root for new-page creation. The caller
    // supplies it explicitly so we don't have to recover it from page
    // layout heuristics. v0.19.3 and earlier called the now-removed
    // `infer_wiki_root` here; that function did `pages.first().path.parent().parent()`
    // and silently produced an empty PathBuf for flat-layout wikis,
    // writing merge targets to `cwd` instead of `.wiki/`. See #21.
    let wiki_root_opt = if !plan.merges.is_empty() || !plan.splits.is_empty() {
        Some(wiki_root.to_path_buf())
    } else {
        None
    };
    let wiki_root = wiki_root_opt;
    let now = chrono::Utc::now().to_rfc3339();

    let mut merge_outcomes: Vec<MergeOutcome> = Vec::new();
    let mut split_outcomes: Vec<SplitOutcome> = Vec::new();

    if let Some(root) = wiki_root.as_deref() {
        for merge in &plan.merges {
            match apply_merge(merge, pages, &mut working_bodies, root, &now) {
                Ok(Some(outcome)) => {
                    report
                        .merged
                        .push((outcome.target_slug.clone(), outcome.source_slugs.clone()));
                    merge_outcomes.push(outcome);
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
                            .push((outcome.source_slug.clone(), outcome.created_targets.clone()));
                    }
                    if !outcome.skipped_targets.is_empty() {
                        eprintln!(
                            "warning: split source `{}` skipped existing targets: {}",
                            outcome.source_slug,
                            outcome.skipped_targets.join(", ")
                        );
                    }
                    split_outcomes.push(outcome);
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

    if rewrite_links {
        // Build the slug-rewrite map and the skip set from the successful
        // outcomes captured above. Every retired source becomes a key
        // pointing at its successor (merge target or first split target).
        // The skip set covers BOTH retired sources (now stale) and merge
        // targets / split targets (already handled by the merge body
        // concat / split stub creation).
        let mut rewrite_map: HashMap<String, String> = HashMap::new();
        let mut skip_slugs: HashSet<String> = HashSet::new();
        for outcome in &merge_outcomes {
            for src in &outcome.source_slugs {
                if src != &outcome.target_slug {
                    rewrite_map.insert(src.clone(), outcome.target_slug.clone());
                }
                skip_slugs.insert(src.clone());
            }
            skip_slugs.insert(outcome.target_slug.clone());
        }
        for outcome in &split_outcomes {
            if let Some(first_target) = outcome.created_targets.first() {
                rewrite_map.insert(outcome.source_slug.clone(), first_target.clone());
            }
            skip_slugs.insert(outcome.source_slug.clone());
            for tgt in &outcome.created_targets {
                skip_slugs.insert(tgt.clone());
            }
        }

        if !rewrite_map.is_empty() {
            report.rewrites =
                rewrite_outbound_links_to_merged_targets(pages, &rewrite_map, &skip_slugs)?;
        }
    }

    Ok(report)
}

/// Returns the cached compiled regex used to scan page bodies for wikilinks.
/// Mirrors the pattern from `coral_core::wikilinks::wikilink_re` — a flat
/// match over the inner text between `[[` and `]]`, with the alias / anchor
/// split handled by string ops in the replace closure.
fn outbound_wikilink_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    #[allow(
        clippy::expect_used,
        reason = "static regex literal; validity guarded by unit tests"
    )]
    RE.get_or_init(|| Regex::new(r"\[\[([^\]\n]+)\]\]").expect("valid wikilink regex"))
}

/// Scans every page in `pages` (other than those in `skip_slugs`) for outbound
/// `[[wikilinks]]` whose target appears as a key in `rewrites`, and rewrites
/// the link to the corresponding value while preserving any alias (`|alias`)
/// or anchor (`#anchor`) suffix. Pages whose body actually changes are
/// written back via `Page::write()` and surfaced in the returned summary;
/// untouched pages are omitted.
///
/// Idempotent: a second call with the same arguments will find no matching
/// links (the slugs to rewrite are already gone) and write nothing.
///
/// Pages listed in `skip_slugs` are explicitly NOT touched. Callers should
/// include both the retired sources (their bodies are stale and their
/// outbound links are moot) and the merge / split targets (their bodies
/// were just (re)built by the merge/split steps).
pub(crate) fn rewrite_outbound_links_to_merged_targets(
    pages: &[Page],
    rewrites: &HashMap<String, String>,
    skip_slugs: &HashSet<String>,
) -> Result<Vec<RewriteSummary>> {
    let re = outbound_wikilink_re();
    let mut summaries: Vec<RewriteSummary> = Vec::new();

    for page in pages {
        if skip_slugs.contains(&page.frontmatter.slug) {
            continue;
        }

        // Track which (old, new) slug pairs landed in THIS page so the
        // summary collapses `[[a]]` + `[[a|alias]]` + `[[a#anchor]]` into a
        // single `(a, ab)` row instead of three.
        let mut applied_pairs: Vec<(String, String)> = Vec::new();
        let mut applied_seen: HashSet<String> = HashSet::new();

        let new_body = re.replace_all(&page.body, |caps: &regex::Captures| {
            let inner = &caps[1];
            // Split the inner text into (target, suffix) where suffix is
            // either `|alias`, `#anchor`, or empty. Whitespace around the
            // target is trimmed (matching `coral_core::wikilinks::extract`).
            let (target_raw, suffix) = if let Some(idx) = inner.find('|') {
                (&inner[..idx], &inner[idx..])
            } else if let Some(idx) = inner.find('#') {
                (&inner[..idx], &inner[idx..])
            } else {
                (inner, "")
            };
            let target = target_raw.trim();
            match rewrites.get(target) {
                Some(new_target) => {
                    if applied_seen.insert(target.to_string()) {
                        applied_pairs.push((target.to_string(), new_target.clone()));
                    }
                    format!("[[{new_target}{suffix}]]")
                }
                None => caps[0].to_string(),
            }
        });

        if applied_pairs.is_empty() {
            continue;
        }

        let updated = Page {
            path: page.path.clone(),
            frontmatter: page.frontmatter.clone(),
            body: new_body.into_owned(),
        };
        updated
            .write()
            .with_context(|| format!("rewriting outbound links in `{}`", page.frontmatter.slug))?;
        summaries.push(RewriteSummary {
            page_slug: page.frontmatter.slug.clone(),
            from_to: applied_pairs,
        });
    }

    Ok(summaries)
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
    // v0.19.5 audit C5: refuse merge targets whose slug isn't safe for
    // direct path interpolation. Without this an LLM emitting
    // `target: ../etc/passwd` would write outside the wiki.
    if !coral_core::slug::is_safe_filename_slug(&merge.target) {
        tracing::warn!(slug = %merge.target, "skipping merge: unsafe target slug");
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
        valid_from: None,
        valid_to: None,
        superseded_by: None,
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
        // v0.19.5 audit C5: refuse split targets whose slug isn't safe
        // for direct path interpolation. An LLM emitting
        // `targets: [../etc/passwd]` would otherwise escape wiki_root.
        if !coral_core::slug::is_safe_filename_slug(target_slug) {
            tracing::warn!(slug = %target_slug, "skipping split target: unsafe slug");
            skipped_targets.push(target_slug.clone());
            continue;
        }
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
            valid_from: None,
            valid_to: None,
            superseded_by: None,
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
                valid_from: None,
                valid_to: None,
                superseded_by: None,
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
                valid_from: None,
                valid_to: None,
                superseded_by: None,
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
        let report = apply_consolidate_plan(&plan, &[a.clone(), b.clone()], &wiki, false).unwrap();

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
        let report = apply_consolidate_plan(&plan, &[a.clone(), b.clone()], &wiki, false).unwrap();
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
            apply_consolidate_plan(&plan, &[a.clone(), b.clone(), target.clone()], &wiki, false)
                .unwrap();
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
        let report =
            apply_consolidate_plan(&plan, std::slice::from_ref(&_seed), &wiki, false).unwrap();
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
        let report = apply_consolidate_plan(&plan, std::slice::from_ref(&a), &wiki, false).unwrap();
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
        let report =
            apply_consolidate_plan(&plan, std::slice::from_ref(&too_big), &wiki, false).unwrap();
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
        let report =
            apply_consolidate_plan(&plan, &[too_big.clone(), part_a.clone()], &wiki, false)
                .unwrap();
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
        let report =
            apply_consolidate_plan(&plan, std::slice::from_ref(&seed), &wiki, false).unwrap();
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
        let report =
            apply_consolidate_plan(&plan, std::slice::from_ref(&too_big), &wiki, false).unwrap();
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
            &wiki,
            false,
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

    // ---------- rewrite_outbound_links_to_merged_targets helper-level tests ----------

    /// Build a single-page fixture with a custom body for helper-level tests.
    fn linker_page(wiki: &Path, slug: &str, body: &str) -> Page {
        page_full(
            wiki,
            slug,
            Status::Reviewed,
            PageType::Module,
            0.7,
            body,
            vec![],
        )
    }

    #[test]
    fn rewrite_helper_plain_wikilink_rewrites_to_merge_target() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let p = linker_page(&wiki, "linker", "see [[a]]");
        let mut rewrites: HashMap<String, String> = HashMap::new();
        rewrites.insert("a".into(), "ab".into());
        let skip: HashSet<String> = HashSet::new();

        let summaries =
            rewrite_outbound_links_to_merged_targets(std::slice::from_ref(&p), &rewrites, &skip)
                .unwrap();

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].page_slug, "linker");
        assert_eq!(summaries[0].from_to, vec![("a".into(), "ab".into())]);
        let on_disk = read_back(&wiki, PageType::Module, "linker");
        assert!(
            on_disk.body.contains("see [[ab]]"),
            "body should contain rewritten link, got: {}",
            on_disk.body
        );
    }

    #[test]
    fn rewrite_helper_aliased_wikilink_preserves_alias() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let p = linker_page(&wiki, "linker", "see [[a|the order page]]");
        let mut rewrites: HashMap<String, String> = HashMap::new();
        rewrites.insert("a".into(), "ab".into());
        let skip: HashSet<String> = HashSet::new();

        let summaries =
            rewrite_outbound_links_to_merged_targets(std::slice::from_ref(&p), &rewrites, &skip)
                .unwrap();

        assert_eq!(summaries.len(), 1);
        let on_disk = read_back(&wiki, PageType::Module, "linker");
        assert!(
            on_disk.body.contains("see [[ab|the order page]]"),
            "alias must be preserved, got: {}",
            on_disk.body
        );
    }

    #[test]
    fn rewrite_helper_anchored_wikilink_preserves_anchor() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let p = linker_page(&wiki, "linker", "see [[a#step-3]]");
        let mut rewrites: HashMap<String, String> = HashMap::new();
        rewrites.insert("a".into(), "ab".into());
        let skip: HashSet<String> = HashSet::new();

        let summaries =
            rewrite_outbound_links_to_merged_targets(std::slice::from_ref(&p), &rewrites, &skip)
                .unwrap();

        assert_eq!(summaries.len(), 1);
        let on_disk = read_back(&wiki, PageType::Module, "linker");
        assert!(
            on_disk.body.contains("see [[ab#step-3]]"),
            "anchor must be preserved, got: {}",
            on_disk.body
        );
    }

    #[test]
    fn rewrite_helper_collapses_multiple_forms_to_one_summary_row() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let body = "plain [[a]], aliased [[a|alias text]], and anchored [[a#anchor]]";
        let p = linker_page(&wiki, "linker", body);
        let mut rewrites: HashMap<String, String> = HashMap::new();
        rewrites.insert("a".into(), "ab".into());
        let skip: HashSet<String> = HashSet::new();

        let summaries =
            rewrite_outbound_links_to_merged_targets(std::slice::from_ref(&p), &rewrites, &skip)
                .unwrap();

        assert_eq!(summaries.len(), 1);
        assert_eq!(
            summaries[0].from_to.len(),
            1,
            "all three forms must collapse into a single (a, ab) summary row"
        );
        assert_eq!(summaries[0].from_to[0], ("a".into(), "ab".into()));

        // All three forms in the body actually rewritten.
        let on_disk = read_back(&wiki, PageType::Module, "linker");
        assert!(on_disk.body.contains("plain [[ab]]"));
        assert!(on_disk.body.contains("aliased [[ab|alias text]]"));
        assert!(on_disk.body.contains("anchored [[ab#anchor]]"));
    }

    #[test]
    fn rewrite_helper_skip_set_actually_skips_the_page() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        // Page has a [[a]] link, but its slug `linker` is in skip_slugs.
        let p = linker_page(&wiki, "linker", "see [[a]]");
        let mut rewrites: HashMap<String, String> = HashMap::new();
        rewrites.insert("a".into(), "ab".into());
        let mut skip: HashSet<String> = HashSet::new();
        skip.insert("linker".into());

        let summaries =
            rewrite_outbound_links_to_merged_targets(std::slice::from_ref(&p), &rewrites, &skip)
                .unwrap();

        assert!(summaries.is_empty(), "skipped page should yield no summary");
        let on_disk = read_back(&wiki, PageType::Module, "linker");
        assert!(
            on_disk.body.contains("see [[a]]"),
            "skipped page body must remain untouched, got: {}",
            on_disk.body
        );
    }

    #[test]
    fn rewrite_helper_unrelated_wikilink_left_alone_and_no_write() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let p = linker_page(&wiki, "linker", "see [[unrelated]] only");
        // Capture mtime before the call so we can confirm Page::write was NOT
        // invoked for an unaffected page.
        let mtime_before = std::fs::metadata(&p.path).unwrap().modified().unwrap();
        // Sleep briefly so any potential write would yield a different mtime.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let mut rewrites: HashMap<String, String> = HashMap::new();
        rewrites.insert("a".into(), "ab".into());
        let skip: HashSet<String> = HashSet::new();

        let summaries =
            rewrite_outbound_links_to_merged_targets(std::slice::from_ref(&p), &rewrites, &skip)
                .unwrap();

        assert!(summaries.is_empty(), "no rewrites means no summary entries");
        let on_disk = read_back(&wiki, PageType::Module, "linker");
        assert!(on_disk.body.contains("see [[unrelated]] only"));
        let mtime_after = std::fs::metadata(&p.path).unwrap().modified().unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "Page::write must not be called for unaffected pages"
        );
    }

    #[test]
    fn rewrite_helper_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let p = linker_page(&wiki, "linker", "see [[a]] and [[a|alias]]");
        let mut rewrites: HashMap<String, String> = HashMap::new();
        rewrites.insert("a".into(), "ab".into());
        let skip: HashSet<String> = HashSet::new();

        // First pass rewrites.
        let first =
            rewrite_outbound_links_to_merged_targets(std::slice::from_ref(&p), &rewrites, &skip)
                .unwrap();
        assert_eq!(first.len(), 1);

        // Re-read the patched page from disk; second pass should be a no-op
        // because `a` is no longer referenced anywhere — only `ab` is.
        let p_after = read_back(&wiki, PageType::Module, "linker");
        let second = rewrite_outbound_links_to_merged_targets(
            std::slice::from_ref(&p_after),
            &rewrites,
            &skip,
        )
        .unwrap();
        assert!(
            second.is_empty(),
            "second pass should produce zero rewrites (source slugs are gone)"
        );
    }

    #[test]
    fn rewrite_helper_pages_with_zero_rewrites_omitted_from_result() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        // Two pages: one matches, one doesn't.
        let p_match = linker_page(&wiki, "matched", "see [[a]]");
        let p_no_match = linker_page(&wiki, "no-links-here", "no links here");
        let mut rewrites: HashMap<String, String> = HashMap::new();
        rewrites.insert("a".into(), "ab".into());
        let skip: HashSet<String> = HashSet::new();

        let summaries = rewrite_outbound_links_to_merged_targets(
            &[p_match.clone(), p_no_match.clone()],
            &rewrites,
            &skip,
        )
        .unwrap();

        assert_eq!(summaries.len(), 1, "only the matched page should appear");
        assert_eq!(summaries[0].page_slug, "matched");
        assert!(
            !summaries.iter().any(|s| s.page_slug == "no-links-here"),
            "page with zero rewrites must be omitted from the result vec"
        );
    }

    /// Smoke test that constructs a `RewriteSummary` directly to confirm its
    /// fields are reachable from inside the test module.
    #[test]
    fn mk_summary() {
        let s = RewriteSummary {
            page_slug: "linker".into(),
            from_to: vec![("a".into(), "ab".into())],
        };
        assert_eq!(s.page_slug, "linker");
        assert_eq!(s.from_to.len(), 1);
    }

    // ---------- end-to-end --apply --rewrite-links tests ----------

    #[test]
    fn apply_with_rewrite_links_patches_other_pages_for_merge() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let a = page_full(
            &wiki,
            "a",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "# a\n\nbody-a",
            vec![],
        );
        let b = page_full(
            &wiki,
            "b",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "# b\n\nbody-b",
            vec![],
        );
        let linker1 = page_full(
            &wiki,
            "linker1",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "refers to [[a]] and [[b]]",
            vec![],
        );
        let linker2 = page_full(
            &wiki,
            "linker2",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "only [[a]]",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![MergeOp {
                target: "ab".into(),
                sources: vec!["a".into(), "b".into()],
                rationale: "redundant".into(),
            }],
            retirements: vec![],
            splits: vec![],
        };
        let report = apply_consolidate_plan(
            &plan,
            &[a.clone(), b.clone(), linker1.clone(), linker2.clone()],
            &wiki,
            true,
        )
        .unwrap();

        assert_eq!(report.merged.len(), 1);
        assert_eq!(report.merged[0].0, "ab");
        // `ab` exists.
        let ab = read_back(&wiki, PageType::Module, "ab");
        assert_eq!(ab.frontmatter.slug, "ab");
        // `a` and `b` are stale.
        assert_eq!(
            read_back(&wiki, PageType::Module, "a").frontmatter.status,
            Status::Stale
        );
        assert_eq!(
            read_back(&wiki, PageType::Module, "b").frontmatter.status,
            Status::Stale
        );
        // linker1 contains [[ab]] twice (one from [[a]], one from [[b]]).
        let l1 = read_back(&wiki, PageType::Module, "linker1");
        assert_eq!(
            l1.body.matches("[[ab]]").count(),
            2,
            "linker1 should have two [[ab]] occurrences, body: {}",
            l1.body
        );
        assert!(
            !l1.body.contains("[[a]]") && !l1.body.contains("[[b]]"),
            "old slugs must be gone from linker1, body: {}",
            l1.body
        );
        // linker2 contains [[ab]].
        let l2 = read_back(&wiki, PageType::Module, "linker2");
        assert!(
            l2.body.contains("[[ab]]"),
            "linker2 must contain [[ab]], body: {}",
            l2.body
        );
        // Two pages were patched (linker1 + linker2).
        assert_eq!(report.rewrites.len(), 2);
    }

    #[test]
    fn apply_with_rewrite_links_uses_first_split_target() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let too_big = page_full(
            &wiki,
            "too-big",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "# too big\n\nbody",
            vec![],
        );
        let linker = page_full(
            &wiki,
            "linker",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "see [[too-big]]",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![],
            retirements: vec![],
            splits: vec![SplitOp {
                source: "too-big".into(),
                targets: vec!["part-a".into(), "part-b".into()],
                rationale: "split it".into(),
            }],
        };
        let report =
            apply_consolidate_plan(&plan, &[too_big.clone(), linker.clone()], &wiki, true).unwrap();

        assert_eq!(report.split.len(), 1);
        assert_eq!(report.split[0].1, vec!["part-a", "part-b"]);
        let l = read_back(&wiki, PageType::Module, "linker");
        assert!(
            l.body.contains("see [[part-a]]"),
            "linker must point at the FIRST split target (part-a), body: {}",
            l.body
        );
        assert!(
            !l.body.contains("[[too-big]]"),
            "old [[too-big]] must be gone, body: {}",
            l.body
        );
        assert_eq!(report.rewrites.len(), 1);
        assert_eq!(report.rewrites[0].page_slug, "linker");
    }

    #[test]
    fn apply_with_rewrite_links_skips_merge_sources() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        // Source `a` has its own [[b]] and [[external]] links, but since `a`
        // is in skip_slugs (it's a merge source), its body must NOT be
        // patched by the rewrite pass.
        let a = page_full(
            &wiki,
            "a",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "# a\n\nlinks to [[b]] and [[external]]",
            vec![],
        );
        let b = page_full(
            &wiki,
            "b",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "# b\n\nbody-b",
            vec![],
        );
        let linker = page_full(
            &wiki,
            "linker",
            Status::Reviewed,
            PageType::Module,
            0.7,
            "see [[a]] and [[b]]",
            vec![],
        );
        let plan = ConsolidatePlan {
            merges: vec![MergeOp {
                target: "ab".into(),
                sources: vec!["a".into(), "b".into()],
                rationale: "redundant".into(),
            }],
            retirements: vec![],
            splits: vec![],
        };
        let report =
            apply_consolidate_plan(&plan, &[a.clone(), b.clone(), linker.clone()], &wiki, true)
                .unwrap();

        // a is now stale (merge source), and its body's [[b]] reference is
        // NOT rewritten because `a` is in the skip set.
        let a_after = read_back(&wiki, PageType::Module, "a");
        assert_eq!(a_after.frontmatter.status, Status::Stale);
        assert!(
            a_after.body.contains("[[b]]"),
            "merge source `a` body must NOT be link-patched (a is in skip_slugs), body: {}",
            a_after.body
        );

        // linker IS patched.
        let l = read_back(&wiki, PageType::Module, "linker");
        assert!(
            l.body.contains("[[ab]]"),
            "linker must be patched to point at [[ab]], body: {}",
            l.body
        );
        assert!(
            !l.body.contains("[[a]]") && !l.body.contains("[[b]]"),
            "linker old slugs must be gone, body: {}",
            l.body
        );

        // Only `linker` appears in rewrites — `a` was skipped.
        assert_eq!(report.rewrites.len(), 1);
        assert_eq!(report.rewrites[0].page_slug, "linker");
    }

    #[test]
    fn rewrite_links_without_apply_returns_error() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        // Need at least one page so the wiki dir exists.
        let _seed = page(&wiki, "seed", Status::Reviewed);
        let runner = MockRunner::new();
        // Runner shouldn't even be invoked — the validation must fire first.
        let err = run_with_runner(
            ConsolidateArgs {
                apply: false,
                rewrite_links: true,
                ..Default::default()
            },
            Some(wiki.as_path()),
            &runner,
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("--apply"),
            "error message must mention --apply, got: {msg}"
        );
    }

    /// Regression for [#21](https://github.com/agustincbajo/Coral/issues/21):
    /// the now-removed `infer_wiki_root` walked
    /// `pages.first().path.parent().parent()` and silently produced an
    /// empty PathBuf for flat-layout wikis (pages at `<wiki>/<slug>.md`,
    /// no per-type subdir). Merge targets then landed at `cwd` instead
    /// of `<wiki>/`. v0.19.4 takes the wiki root as an explicit
    /// parameter, so flat layouts behave the same as nested.
    ///
    /// We construct a page directly under `<wiki>` (no `module/` subdir),
    /// run a merge that needs to materialize a brand-new target page,
    /// and assert the new file lands inside `<wiki>` (NOT cwd).
    #[test]
    fn apply_consolidate_plan_uses_explicit_wiki_root_for_flat_layout() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();

        // Hand-construct a flat-layout page: <wiki>/orphan.md
        // No per-type subdirectory. This is the exact shape that
        // pre-v0.19.4 caused infer_wiki_root to misbehave.
        let flat_page = Page {
            path: wiki.join("orphan.md"),
            frontmatter: Frontmatter {
                slug: "orphan".into(),
                page_type: PageType::Module,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(0.8).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Reviewed,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra: Default::default(),
            },
            body: "Original orphan body".into(),
        };
        flat_page.write().unwrap();

        // A merge plan whose target is a brand-new slug — forces
        // `apply_merge_create_new` to build a path against `wiki_root`.
        let plan = ConsolidatePlan {
            merges: vec![MergeOp {
                target: "consolidated".into(),
                sources: vec!["orphan".into()],
                rationale: "test".into(),
            }],
            retirements: vec![],
            splits: vec![],
        };

        // Pass the wiki root explicitly — pre-v0.19.4 this was
        // inferred (incorrectly) from the page path.
        let report =
            apply_consolidate_plan(&plan, std::slice::from_ref(&flat_page), &wiki, false).unwrap();
        assert_eq!(report.merged.len(), 1);

        // The merge target must NOT have landed at cwd.
        assert!(
            !std::path::Path::new("consolidated.md").exists(),
            "merge target leaked to cwd — apply_consolidate_plan should not write outside wiki_root",
        );

        // The merge target SHOULD land somewhere under `wiki/`.
        let in_wiki = walkdir::WalkDir::new(&wiki)
            .into_iter()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy() == "consolidated.md");
        assert!(
            in_wiki,
            "merge target was not written under wiki root {}",
            wiki.display(),
        );
    }

    // -----------------------------------------------------------------
    // v0.21.4 — MultiStepRunner opt-in: byte-identity + tiered routing.
    // -----------------------------------------------------------------

    /// AC-2: `coral consolidate` (no `--tiered`, no manifest opt-in)
    /// must produce stdout that's byte-identical to v0.21.3 against
    /// the same scripted `MockRunner`. We pin the actual byte string
    /// here (header + verbatim LLM output + footer) — anything that
    /// drifts the formatter will fail this test.
    ///
    /// This snapshot is also wired in v0.21.4 to ensure the new
    /// `format_preview_output` / `format_apply_report` helpers are
    /// not just refactored but byte-for-byte compatible with the
    /// pre-refactor `println!` calls.
    #[test]
    fn consolidate_no_tiered_flag_is_byte_identical_to_v0213() {
        // v0.21.3 byte string for the dry-run preview path. The
        // `\n\n` after the header line, the literal LLM stdout, the
        // `\n\n` separator, and the trailing `_(...)_\n` are all
        // load-bearing.
        let llm_yaml = "retirements:\n  - slug: obsolete\n    rationale: superseded\n";
        let formatted = format_preview_output(llm_yaml);
        let expected = "# Consolidation suggestions (preview)\n\n\
                        retirements:\n  - slug: obsolete\n    rationale: superseded\n\n\
                        \n_(pass `--apply` to mark `retirements[]` slugs stale, materialize `merges[]` into a target page, and stub out `splits[]` targets.)_\n";
        assert_eq!(
            formatted, expected,
            "preview output drifted from v0.21.3 byte string"
        );

        // For the `--apply` path, snapshot the format_apply_report
        // output against a hand-built `ApplyReport`.
        let report = ApplyReport {
            retired: vec!["obsolete".into()],
            unknown_retirements: Vec::new(),
            merged: Vec::new(),
            split: Vec::new(),
            unknown_merge_targets: Vec::new(),
            unknown_split_sources: Vec::new(),
            rewrites: Vec::new(),
        };
        let applied = format_apply_report(&report);
        let expected_applied = "# Consolidation applied\n\n\
                                Retired: 1 page(s)\n\
                                - `obsolete` → status: stale\n";
        assert_eq!(
            applied, expected_applied,
            "apply report output drifted from v0.21.3 byte string"
        );
    }

    /// Sanity check that the byte-identity snapshot above isn't a
    /// tautology — if we mutate the formatter, the snapshot test
    /// must fail. Builds the same `ApplyReport` the snapshot uses,
    /// hand-mutates the rendered text, and asserts INequality.
    #[test]
    fn consolidate_byte_identity_snapshot_actually_catches_drift() {
        let report = ApplyReport {
            retired: vec!["obsolete".into()],
            unknown_retirements: Vec::new(),
            merged: Vec::new(),
            split: Vec::new(),
            unknown_merge_targets: Vec::new(),
            unknown_split_sources: Vec::new(),
            rewrites: Vec::new(),
        };
        let applied = format_apply_report(&report);
        let expected_applied = "# Consolidation applied\n\n\
                                Retired: 1 page(s)\n\
                                - `obsolete` → status: stale\n";
        // The snapshot under test ↑↑.
        assert_eq!(applied, expected_applied);

        // Drift the snapshot in two ways and verify the equality fails.
        let drift_header = applied.replace("# Consolidation applied", "# Consolidation done");
        assert_ne!(
            drift_header, expected_applied,
            "snapshot must reject any header drift"
        );
        let drift_status = applied.replace("status: stale", "status: archived");
        assert_ne!(
            drift_status, expected_applied,
            "snapshot must reject any retired-bullet wording drift"
        );
    }

    /// AC-3 + AC-12: `consolidate --tiered` invokes the three sub-runners
    /// in order (planner → executor → reviewer), threads each tier's
    /// per-tier model into the `Prompt`, and parses the reviewer's
    /// stdout as the consolidate plan. AC-4 (reviewer stdout = parser
    /// input) is verified by pushing a YAML retirement plan into the
    /// reviewer mock and asserting `obsolete` ends up `status: stale`.
    #[test]
    fn consolidate_with_tiered_flag_invokes_three_runners() {
        use coral_runner::{
            BudgetConfig as RBudget, MockRunner, Runner, TierSpec, TieredConfig, TieredRunner,
        };
        use std::sync::Arc;

        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        // Page that'll be retired by the reviewer's plan.
        let p = page(&wiki, "obsolete", Status::Reviewed);

        // Each tier mock returns its scripted reply; the reviewer's
        // is the canonical YAML plan parser input.
        let planner = Arc::new(MockRunner::new());
        planner.push_ok("subtasks:\n  - id: t1\n    description: scan for duplicates\n");
        let executor = Arc::new(MockRunner::new());
        executor.push_ok("found candidates: obsolete is dead");
        let reviewer = Arc::new(MockRunner::new());
        reviewer.push_ok("retirements:\n  - slug: obsolete\n    rationale: superseded\n");

        // Tiny `ArcRunner` wrapper so we can keep call-introspection
        // handles alive after the tiered run consumes the boxed runners.
        struct ArcRunner(Arc<MockRunner>);
        impl Runner for ArcRunner {
            fn run(&self, p: &Prompt) -> coral_runner::RunnerResult<coral_runner::RunOutput> {
                self.0.run(p)
            }
        }

        let cfg = TieredConfig {
            planner: TierSpec {
                provider: "claude".into(),
                model: Some("haiku".into()),
            },
            executor: TierSpec {
                provider: "claude".into(),
                model: Some("sonnet".into()),
            },
            reviewer: TierSpec {
                provider: "claude".into(),
                model: Some("opus".into()),
            },
            budget: RBudget {
                max_tokens_per_run: 1_000_000,
            },
        };
        let tiered = TieredRunner::new(
            Box::new(ArcRunner(planner.clone())),
            Box::new(ArcRunner(executor.clone())),
            Box::new(ArcRunner(reviewer.clone())),
            cfg,
        );

        // Execute the apply path so we can assert the reviewer's
        // stdout actually drove disk mutation (== AC-4).
        let exit = run_with_tiered_runner(
            ConsolidateArgs {
                apply: true,
                tiered: true,
                ..Default::default()
            },
            Some(wiki.as_path()),
            &tiered,
        )
        .unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);

        // AC-3: each tier saw exactly one call.
        let p_calls = planner.calls();
        let e_calls = executor.calls();
        let r_calls = reviewer.calls();
        assert_eq!(p_calls.len(), 1, "planner called exactly once");
        assert_eq!(
            e_calls.len(),
            1,
            "executor called once per sub-task (1 here)"
        );
        assert_eq!(r_calls.len(), 1, "reviewer called exactly once");

        // Per-tier model overrides reach each Prompt.
        assert_eq!(p_calls[0].model.as_deref(), Some("haiku"));
        assert_eq!(e_calls[0].model.as_deref(), Some("sonnet"));
        assert_eq!(r_calls[0].model.as_deref(), Some("opus"));

        // AC-4: reviewer's stdout (the YAML plan) drove disk mutation —
        // `obsolete` is now status: stale on disk.
        let on_disk = std::fs::read_to_string(&p.path).unwrap();
        assert!(
            on_disk.contains("status: stale"),
            "reviewer's plan must have been parsed and applied; got:\n{on_disk}"
        );
    }
}
