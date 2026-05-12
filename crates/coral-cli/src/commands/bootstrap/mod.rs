use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use coral_core::config;
use coral_core::cost::{
    PlanEntryEstimate, Provider, estimate_cost_from_tokens, estimate_tokens_for_entry,
};
use coral_core::frontmatter::PageType;
use coral_core::gitdiff;
use coral_core::index::{IndexEntry, WikiIndex};
use coral_core::log::WikiLog;
use coral_core::symbols::{self, Symbol, SymbolKind};
use coral_runner::{Prompt, Runner, RunnerError};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::plan::{Action, Plan, PlanEntry, build_page, page_type_subdir};

pub mod estimate;
pub mod state;

use estimate::{plan_cost_estimate, print_estimate};
use state::{BootstrapLock, BootstrapState, PageStatus};

/// Exit code reserved for "partial run — `--max-cost` halted bootstrap
/// mid-flight". PRD FR-ONB-29: distinct from 0 (success), 1
/// (findings), 2 (usage error), 3 (internal). Picked 2 per the PRD.
pub const EXIT_MAX_COST_REACHED: u8 = 2;

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
    /// Use symbol extraction (regex-based) instead of an LLM to generate
    /// draft wiki pages. Generates one page per module/significant struct
    /// with confidence 0.4.
    #[arg(long)]
    pub from_symbols: bool,
    /// Directory to scan for symbols (defaults to current directory).
    /// Only used with `--from-symbols`.
    #[arg(long)]
    pub path: Option<PathBuf>,
    /// v0.34.0 (FR-ONB-12): show cost estimate (upper-bound + margin)
    /// without running the bootstrap. Implies --dry-run for the
    /// page-write phase.
    #[arg(long, conflicts_with = "apply")]
    pub estimate: bool,
    /// v0.34.0 (FR-ONB-29): abort mid-flight (and pre-flight) if the
    /// running cost exceeds this USD value. Pre-flight: if estimate's
    /// upper-bound exceeds this, exit before any LLM call. Mid-flight:
    /// page-by-page gate using real Runner.usage cost (or heuristic
    /// fallback when usage is unavailable). On abort, the checkpoint
    /// at .wiki/.bootstrap-state.json is marked partial=true and
    /// `coral bootstrap --resume` continues.
    #[arg(long, value_name = "USD")]
    pub max_cost: Option<f64>,
    /// v0.34.0 (FR-ONB-30): resume from .wiki/.bootstrap-state.json
    /// checkpoint. Skips planner re-call (the persisted plan is
    /// re-used verbatim) and re-tries every page that is NOT
    /// Completed. Conflicts with --dry-run / --estimate.
    #[arg(long, conflicts_with_all = ["dry_run", "estimate"])]
    pub resume: bool,
    /// v0.34.0 (FR-ONB-12 large-repo hint): limit to the first N
    /// pages by plan order. Useful for very large repos where the
    /// full bootstrap would exceed --max-cost.
    #[arg(long, value_name = "N")]
    pub max_pages: Option<usize>,
}

pub fn run(args: BootstrapArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    if args.from_symbols {
        return run_from_symbols(args, wiki_root);
    }
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

    let cwd = std::env::current_dir().context("getting cwd")?;

    // Resolve provider name once — used by cost model + state.provider.
    let provider_name = super::runner_helper::resolve_provider(args.provider.as_deref())
        .map_err(|e| anyhow::anyhow!(e))?;
    let cost_provider = match provider_name {
        super::runner_helper::ProviderName::Claude => Provider::Claude,
        super::runner_helper::ProviderName::Gemini => Provider::Gemini,
        super::runner_helper::ProviderName::Http => Provider::Http,
        super::runner_helper::ProviderName::Local => Provider::Local,
    };
    // Bootstrap thresholds from `.coral/config.toml` (FR-ONB-14).
    let cfg = config::load_from_repo(&cwd).unwrap_or_default();
    let big_repo_threshold = cfg.bootstrap.big_repo_threshold_usd;

    // ---- --resume path: skip plan generation entirely --------------------
    if args.resume {
        let state = BootstrapState::load(&root)?.ok_or_else(|| {
            anyhow::anyhow!(
                "--resume requires an existing checkpoint at {}; \
                     none found. Run `coral bootstrap --apply` first.",
                BootstrapState::path(&root).display()
            )
        })?;
        let _lock = BootstrapLock::acquire(&root)?;
        let resume_max_cost = args.max_cost.or(state.max_cost_usd);
        return apply_pages(&root, &cwd, runner, cost_provider, state, resume_max_cost);
    }

    // ---- Plan generation (one runner call) -------------------------------
    let plan = generate_plan(runner, &cwd, args.model.clone())?;
    let mut plan = plan.plan;

    // FR-ONB-12 large-repo limit.
    if let Some(n) = args.max_pages {
        plan.truncate(n);
    }

    // ---- --estimate path -------------------------------------------------
    if args.estimate {
        let (loc, files_count) = repo_size(&cwd).unwrap_or((0, 0));
        print_estimate(&plan, cost_provider, loc, files_count, big_repo_threshold)?;
        return Ok(ExitCode::SUCCESS);
    }

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
        // Render the plan back as YAML-ish output so dry-run is still
        // useful as a preview. Existing tests check for slug content.
        println!("plan:");
        for e in &plan {
            println!("  - slug: {}", e.slug);
            if let Some(t) = e.r#type {
                println!("    type: {t:?}");
            }
            if let Some(c) = e.confidence {
                println!("    confidence: {c}");
            }
            if !e.rationale.is_empty() {
                println!("    rationale: {}", e.rationale);
            }
            if let Some(b) = &e.body {
                let trimmed = b.lines().next().unwrap_or("").trim();
                println!("    body: |");
                println!("      {trimmed}");
            }
        }
        println!("\n# (run with --apply to write pages, update index and append log)");
        return Ok(ExitCode::SUCCESS);
    }

    // ---- --apply path: per-page loop + checkpoints + cost gating ---------
    // FR-ONB-29 pre-flight gate: if upper_bound > max_cost, abort
    // before paying for any LLM call.
    if let Some(max_cost) = args.max_cost {
        let est = plan_cost_estimate(&plan, cost_provider);
        if est.usd_upper_bound > max_cost {
            anyhow::bail!(
                "Estimated upper bound (${:.2}) exceeds --max-cost (${:.2}). \
                 Try: --max-pages=N or remove --max-cost.",
                est.usd_upper_bound,
                max_cost
            );
        }
    }

    let _lock = BootstrapLock::acquire(&root)?;
    let max_cost = args.max_cost;
    let state = BootstrapState::fresh(plan, cost_provider.label().into(), max_cost);
    state.save_atomic(&root)?;
    apply_pages(&root, &cwd, runner, cost_provider, state, max_cost)
}

/// FR-ONB-30 + FR-ONB-29: walk the plan one page at a time. Each
/// page gets its own runner call (or, when the LLM-emitted plan
/// already carries a `body`, we reuse it for free). Cost accumulates
/// across the loop using real `Runner.usage` when present, falling
/// back to the heuristic when `None`. `--max-cost` gates before
/// every page; an exceeded budget marks the state `partial=true`
/// and exits 2 with a resume hint.
fn apply_pages(
    root: &Path,
    cwd: &Path,
    runner: &dyn Runner,
    cost_provider: Provider,
    mut state: BootstrapState,
    max_cost: Option<f64>,
) -> Result<ExitCode> {
    let head = match gitdiff::head_sha(cwd) {
        Ok(sha) => sha,
        Err(e) => {
            tracing::warn!(
                error = %e,
                cwd = %cwd.display(),
                "bootstrap: head_sha failed; pages will record `HEAD` as last_updated_commit"
            );
            "HEAD".to_string()
        }
    };
    let idx_path = root.join("index.md");
    let idx_content = std::fs::read_to_string(&idx_path).context("reading .wiki/index.md")?;
    let mut index = WikiIndex::parse(&idx_content)?;

    let mut created = 0usize;
    let mut skipped: Vec<String> = Vec::new();
    let mut partial = false;

    // Iterate by index so we can update state.pages[i] in-place.
    let plan_len = state.plan.len();
    for i in 0..plan_len {
        // Skip pages already completed (resume).
        if matches!(state.pages[i].status, PageStatus::Completed) {
            continue;
        }
        let entry = state.plan[i].clone();

        if entry.action != Action::Create {
            skipped.push(format!(
                "{} (action={:?} not supported in bootstrap)",
                entry.slug, entry.action
            ));
            state.pages[i].status = PageStatus::Failed;
            state.pages[i].error = Some(format!("unsupported action {:?}", entry.action));
            state.save_atomic(root)?;
            continue;
        }

        // FR-ONB-29 mid-flight gate. Projected cost for THIS page
        // uses the heuristic; if we've already exceeded, halt.
        let projected_page_cost = project_page_cost(&entry, cost_provider);
        if let Some(cap) = max_cost {
            if state.cost_spent_usd + projected_page_cost > cap {
                eprintln!(
                    "Stopped at ${:.2} (cap ${cap:.2}). Run `coral bootstrap --resume` to continue.",
                    state.cost_spent_usd
                );
                state.partial = true;
                state.save_atomic(root)?;
                partial = true;
                break;
            }
        }

        // ---- Mark InProgress + checkpoint --------------------------------
        state.pages[i].status = PageStatus::InProgress;
        state.save_atomic(root)?;

        // ---- Generate body if absent -------------------------------------
        let mut entry_with_body = entry.clone();
        let mut real_usage: Option<coral_runner::TokenUsage> = None;
        if entry.body.is_none() {
            // One runner call per page (FR-ONB-30).
            let page_prompt = build_page_prompt(&entry, cwd);
            match runner.run(&page_prompt) {
                Ok(out) => {
                    entry_with_body.body = Some(out.stdout.clone());
                    real_usage = out.usage;
                }
                Err(e) => {
                    eprintln!("warn: per-page runner failed for `{}`: {e}", entry.slug);
                    state.pages[i].status = PageStatus::Failed;
                    state.pages[i].error = Some(format!("{e}"));
                    state.save_atomic(root)?;
                    skipped.push(entry.slug.clone());
                    // Auth/not-found failures are terminal — surface them.
                    if matches!(e, RunnerError::AuthFailed(_) | RunnerError::NotFound) {
                        return Err(anyhow::anyhow!("runner failed: {e}"));
                    }
                    continue;
                }
            }
        }

        // ---- Write page + update index -----------------------------------
        let page = match build_page(&entry_with_body, &head, root) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("warn: skipping `{}`: {e}", entry.slug);
                state.pages[i].status = PageStatus::Failed;
                state.pages[i].error = Some(format!("{e}"));
                state.save_atomic(root)?;
                skipped.push(entry.slug.clone());
                continue;
            }
        };
        page.write()?;
        let rel_path = page_relative_path(root, page.frontmatter.page_type, &page.frontmatter.slug);
        index.upsert(IndexEntry {
            slug: page.frontmatter.slug.clone(),
            page_type: page.frontmatter.page_type,
            path: rel_path,
            confidence: page.frontmatter.confidence,
            status: page.frontmatter.status,
            last_updated_commit: page.frontmatter.last_updated_commit.clone(),
        });

        // ---- Record cost + mark Completed --------------------------------
        let (input, output, cost_usd) = if let Some(u) = real_usage {
            let est = estimate_cost_from_tokens(u.input_tokens, u.output_tokens, cost_provider);
            (u.input_tokens, u.output_tokens, est.usd_estimate)
        } else {
            // No body call (pre-existing body in plan, e.g. bootstrap
            // shape used by every v0.33 test fixture) → zero cost for
            // this page. If we did call the runner but it returned
            // None usage, fall back to the heuristic for the same
            // entry.
            if entry.body.is_some() {
                (0, 0, 0.0)
            } else {
                let est = plan_cost_estimate(std::slice::from_ref(&entry), cost_provider);
                (est.input_tokens, est.output_tokens, est.usd_estimate)
            }
        };
        state.pages[i].input_tokens = input;
        state.pages[i].output_tokens = output;
        state.pages[i].cost_usd = cost_usd;
        state.pages[i].status = PageStatus::Completed;
        state.pages[i].completed_at = Some(Utc::now());
        state.pages[i].error = None;
        state.cost_spent_usd += cost_usd;
        state.save_atomic(root)?;
        created += 1;
    }

    // Lock-protected write — see ingest.rs for rationale.
    index.bump_last_commit(head.clone());
    coral_core::atomic::with_exclusive_lock(&idx_path, || {
        coral_core::atomic::atomic_write_string(&idx_path, &index.to_string()?)
    })
    .context("writing .wiki/index.md")?;

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

    if partial {
        return Ok(ExitCode::from(EXIT_MAX_COST_REACHED));
    }
    // v0.30.x audit #B7: if the LLM produced a plan but every entry was
    // skipped, exit non-zero so CI / scripts can detect the no-op-on-
    // failure case.
    if created == 0 && !skipped.is_empty() {
        eprintln!(
            "bootstrap: no pages created; {} skipped — surfacing as failure",
            skipped.len()
        );
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

/// FR-ONB-30: planner call — generate just the plan skeleton from the
/// repo file listing. Returns the parsed `Plan` (with entries that
/// may or may not have inline `body` content depending on what the
/// model emitted).
fn generate_plan(runner: &dyn Runner, cwd: &Path, model: Option<String>) -> Result<Plan> {
    let files = collect_repo_files(cwd)?;
    let listing = files
        .iter()
        .take(200)
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
        model,
        cwd: None,
        timeout: None,
    };
    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("runner failed: {e}"))?;
    match Plan::parse(&out.stdout) {
        Ok(p) => Ok(p),
        Err(e) => {
            println!("# Raw runner output (failed to parse as YAML):\n");
            println!("{}", out.stdout);
            anyhow::bail!("failed to parse plan: {e}")
        }
    }
}

/// Build the per-page prompt for the page-body call (FR-ONB-30). Kept
/// small — system text is the same bootstrap template, user content
/// names the slug + rationale + page type so the LLM writes one
/// focused Markdown body.
fn build_page_prompt(entry: &PlanEntry, _cwd: &Path) -> Prompt {
    let system = format!(
        "You are the Coral wiki bibliotecario. Write ONE Markdown page body for the wiki slug \
         provided. Output the Markdown body directly — no YAML envelope, no code fence, no \
         meta-commentary. Aim for ~3 KB. The page type is `{:?}`.",
        entry.r#type.unwrap_or(PageType::Module)
    );
    let user = format!(
        "Slug: {}\nRationale: {}\n\nWrite the Markdown body now.",
        entry.slug, entry.rationale
    );
    Prompt {
        system: Some(system),
        user,
        model: None,
        cwd: None,
        timeout: None,
    }
}

/// Cheap pre-page cost projection for the mid-flight gate. Uses the
/// same coral_core::cost heuristic the upfront estimate does.
fn project_page_cost(entry: &PlanEntry, provider: Provider) -> f64 {
    let est = PlanEntryEstimate {
        body_len_chars: entry.body.as_deref().map(str::len).unwrap_or(0),
        rationale_len_chars: entry.rationale.len(),
    };
    let (input, output) = estimate_tokens_for_entry(&est);
    estimate_cost_from_tokens(input, output, provider).usd_estimate
}

/// Best-effort `(LOC, file_count)` snapshot of the repo for the
/// `--estimate` first line. We approximate LOC as the sum of file
/// sizes / 50 (a rough character-to-line ratio); the real count is
/// not worth a second walk. Returns `(0, 0)` on any error.
fn repo_size(root: &Path) -> Result<(usize, usize)> {
    let files = collect_repo_files(root)?;
    let mut total_bytes: u64 = 0;
    for rel in &files {
        let abs = root.join(rel);
        if let Ok(md) = std::fs::metadata(&abs) {
            total_bytes = total_bytes.saturating_add(md.len());
        }
    }
    let loc = (total_bytes / 50) as usize;
    Ok((loc, files.len()))
}

// ─── From-symbols path ─────────────────────────────────────────────────────

/// Default source extensions to scan when `--from-symbols` is used.
const SYMBOL_EXTENSIONS: &[&str] = &["rs", "ts", "tsx", "py", "go"];

/// Run bootstrap using symbol extraction instead of an LLM.
///
/// Groups extracted symbols by module (parent directory / explicit module_path),
/// generates one wiki page per module (and one per significant struct/class),
/// all with `confidence: 0.4`.
pub fn run_from_symbols(args: BootstrapArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let root: PathBuf = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));
    if !root.exists() {
        anyhow::bail!(
            "wiki root not found: {}. Run `coral init` first.",
            root.display()
        );
    }

    let scan_dir = args
        .path
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"));

    let all_symbols = symbols::extract_from_dir(&scan_dir, SYMBOL_EXTENSIONS);
    if all_symbols.is_empty() {
        eprintln!(
            "No symbols found under {}. Nothing to bootstrap.",
            scan_dir.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Group symbols by module. Module key = parent directory relative to scan_dir,
    // or `module_path` if set. Flatten to a simple string key.
    let grouped = group_symbols_by_module(&all_symbols, &scan_dir);

    // Build plan entries from grouped symbols.
    let entries = build_symbol_plan_entries(&grouped);

    // Resolve mode.
    let apply = args.apply;
    let dry_run = args.dry_run || !apply;
    if !args.dry_run && !apply {
        eprintln!(
            "No --dry-run / --apply flag passed; defaulting to --dry-run. Pass --apply to mutate disk.",
        );
    }

    if dry_run {
        println!(
            "# Bootstrap from symbols — {} modules, {} total symbols\n",
            entries.len(),
            all_symbols.len()
        );
        for entry in &entries {
            println!("  - slug: {}", entry.slug);
            println!("    type: {:?}", entry.r#type.unwrap_or(PageType::Module));
            println!("    confidence: 0.4");
            println!("    symbols: (see body)");
            println!();
        }
        println!("# (run with --apply to write pages, update index and append log)");
        return Ok(ExitCode::SUCCESS);
    }

    // Apply path.
    let cwd = std::env::current_dir().context("getting cwd")?;
    let head = match gitdiff::head_sha(&cwd) {
        Ok(sha) => sha,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "bootstrap --from-symbols: head_sha failed; using HEAD literal"
            );
            "HEAD".to_string()
        }
    };

    let idx_path = root.join("index.md");
    let idx_content = std::fs::read_to_string(&idx_path).context("reading .wiki/index.md")?;
    let mut index = WikiIndex::parse(&idx_content)?;

    let mut created = 0usize;
    let mut skipped: Vec<String> = Vec::new();

    for entry in &entries {
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
    coral_core::atomic::with_exclusive_lock(&idx_path, || {
        coral_core::atomic::atomic_write_string(&idx_path, &index.to_string()?)
    })
    .context("writing .wiki/index.md")?;

    let log_path = root.join("log.md");
    let summary = if skipped.is_empty() {
        format!(
            "{created} pages created from symbols ({} symbols extracted)",
            all_symbols.len()
        )
    } else {
        format!(
            "{created} pages created from symbols, skipped: {}",
            skipped.join(", ")
        )
    };
    WikiLog::append_atomic(&log_path, "bootstrap-from-symbols", &summary)?;

    println!(
        "Created {created} pages from {} symbols, updated index, appended log entry.{}",
        all_symbols.len(),
        if skipped.is_empty() {
            String::new()
        } else {
            format!(" Skipped: {}.", skipped.join(", "))
        }
    );
    // v0.30.x audit #B7: same skip-everything-still-SUCCESS bug as the
    // LLM bootstrap path above.
    if created == 0 && !skipped.is_empty() {
        eprintln!(
            "bootstrap --from-symbols: no pages created; {} skipped — surfacing as failure",
            skipped.len()
        );
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

/// Group symbols by their module key. The key is derived from the parent
/// directory relative to `scan_dir` (e.g., `src/auth` -> `auth`), or the
/// explicit `module_path` if present. Files directly in `scan_dir` map to
/// their file stem.
pub(crate) fn group_symbols_by_module<'a>(
    symbols: &'a [Symbol],
    scan_dir: &Path,
) -> BTreeMap<String, Vec<&'a Symbol>> {
    let mut groups: BTreeMap<String, Vec<&Symbol>> = BTreeMap::new();
    for sym in symbols {
        let key = module_key_for_symbol(sym, scan_dir);
        groups.entry(key).or_default().push(sym);
    }
    groups
}

/// Derive a module key for a symbol based on file path.
fn module_key_for_symbol(sym: &Symbol, scan_dir: &Path) -> String {
    // Prefer explicit module_path if available.
    if let Some(ref mp) = sym.module_path {
        // Use last segment(s) after "crate::" prefix.
        let trimmed = mp.strip_prefix("crate::").unwrap_or(mp);
        let parts: Vec<&str> = trimmed.split("::").collect();
        if parts.len() > 1 {
            return parts[..parts.len() - 1].join("_");
        }
        return parts[0].to_string();
    }

    // Fall back to parent directory relative to scan_dir.
    let rel = sym.file.strip_prefix(scan_dir).unwrap_or(&sym.file);
    let parent = rel.parent().unwrap_or(Path::new(""));
    let parent_str = parent.to_string_lossy().replace(['/', '\\'], "_");

    if parent_str.is_empty() || parent_str == "." {
        // Use file stem as module key.
        rel.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("root")
            .to_string()
    } else {
        // Strip common prefixes like "src_" for cleaner slugs.
        let cleaned = parent_str.strip_prefix("src_").unwrap_or(&parent_str);
        if cleaned.is_empty() {
            "src".to_string()
        } else {
            cleaned.to_string()
        }
    }
}

/// Convert a module key to a safe slug (lowercase, hyphens).
fn module_key_to_slug(key: &str) -> String {
    key.to_lowercase()
        .replace(['_', ' '], "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}

/// Build plan entries from grouped symbols.
pub(crate) fn build_symbol_plan_entries(
    grouped: &BTreeMap<String, Vec<&Symbol>>,
) -> Vec<super::plan::PlanEntry> {
    use super::plan::PlanEntry;

    let mut entries = Vec::new();

    for (module_key, syms) in grouped {
        let slug = module_key_to_slug(module_key);
        if slug.is_empty() {
            continue;
        }

        // Determine page type: if the module contains mostly structs/classes,
        // use Entity; if it has a trait/interface, use Interface; else Module.
        let page_type = infer_page_type(syms);

        let body = render_symbol_page_body(&slug, syms);

        entries.push(PlanEntry {
            slug,
            action: Action::Create,
            r#type: Some(page_type),
            confidence: Some(0.4),
            rationale: format!(
                "auto-generated from {} symbols in {}",
                syms.len(),
                module_key
            ),
            body: Some(body),
        });
    }

    entries
}

/// Infer the best page type for a group of symbols.
fn infer_page_type(syms: &[&Symbol]) -> PageType {
    let mut structs = 0usize;
    let mut traits = 0usize;
    let mut functions = 0usize;

    for sym in syms {
        match sym.kind {
            SymbolKind::Struct | SymbolKind::Class => structs += 1,
            SymbolKind::Trait | SymbolKind::Interface => traits += 1,
            SymbolKind::Function | SymbolKind::Method => functions += 1,
            _ => {}
        }
    }

    if traits > 0 && traits >= structs {
        PageType::Interface
    } else if structs > functions && structs > 0 {
        PageType::Entity
    } else {
        PageType::Module
    }
}

/// Render the markdown body for a symbol-based wiki page.
fn render_symbol_page_body(slug: &str, syms: &[&Symbol]) -> String {
    let mut body = String::new();
    body.push_str(&format!("# {}\n\n", slug));
    body.push_str("_Auto-generated from symbol extraction. Review and expand._\n\n");
    body.push_str("## Symbols\n\n");
    body.push_str("| Symbol | Kind | File | Line |\n");
    body.push_str("|--------|------|------|------|\n");
    for sym in syms {
        let file_display = sym.file.file_name().and_then(|f| f.to_str()).unwrap_or("?");
        body.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            sym.name, sym.kind, file_display, sym.line
        ));
    }
    body.push_str("\n## Overview\n\n");
    body.push_str("_TODO: describe the purpose and responsibilities of this module._\n");
    body
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

    /// v0.30.x audit #B7 regression: when every plan entry is skipped
    /// (here, every entry has a non-Create action that bootstrap rejects),
    /// the command must exit FAILURE so CI catches the no-op-on-failure
    /// case. Pre-fix the exit was SUCCESS.
    #[test]
    fn bootstrap_apply_all_skipped_returns_failure() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        // All entries use action=update — bootstrap only supports Create
        // and will push every one into `skipped`.
        runner.push_ok(
            "plan:\n  - slug: a\n    action: update\n    rationale: x\n  - slug: b\n    action: update\n    rationale: y",
        );
        let exit = run_with_runner(
            BootstrapArgs {
                apply: true,
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        )
        .expect("bootstrap must not bail; it should return FAILURE explicitly");
        std::env::set_current_dir(&cur).unwrap();
        assert_eq!(
            exit,
            ExitCode::FAILURE,
            "all-skipped bootstrap must exit non-zero"
        );
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

    // ─── --from-symbols tests ────────────────────────────────────────────────

    #[test]
    fn from_symbols_dry_run_does_not_mutate() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);

        // Create a Rust source file with symbols.
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            src_dir.join("lib.rs"),
            "pub struct Config {}\npub fn handle() {}\n",
        )
        .unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let exit = run_from_symbols(
            BootstrapArgs {
                from_symbols: true,
                dry_run: true,
                path: Some(tmp.path().to_path_buf()),
                ..Default::default()
            },
            Some(&wiki),
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();

        assert_eq!(exit, ExitCode::SUCCESS);
        // No pages written.
        assert!(
            !wiki.join("modules").exists(),
            "dry run must not create module pages"
        );
        assert!(
            !wiki.join("entities").exists(),
            "dry run must not create entity pages"
        );
    }

    #[test]
    fn from_symbols_apply_writes_pages_with_correct_confidence() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);

        // Create source with symbols in different directories.
        let auth_dir = tmp.path().join("src").join("auth");
        std::fs::create_dir_all(&auth_dir).unwrap();
        std::fs::write(
            auth_dir.join("handler.rs"),
            "pub fn login() {}\npub fn logout() {}\n",
        )
        .unwrap();

        let models_dir = tmp.path().join("src").join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(
            models_dir.join("user.rs"),
            "pub struct User {}\npub struct Session {}\n",
        )
        .unwrap();

        std::env::set_current_dir(tmp.path()).unwrap();

        let exit = run_from_symbols(
            BootstrapArgs {
                from_symbols: true,
                apply: true,
                path: Some(tmp.path().to_path_buf()),
                ..Default::default()
            },
            Some(&wiki),
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();

        assert_eq!(exit, ExitCode::SUCCESS);

        // The auth module should produce a Module page (functions).
        assert!(
            wiki.join("modules").join("auth.md").exists(),
            "expected modules/auth.md to be created"
        );

        // The models module should produce an Entity page (structs dominate).
        assert!(
            wiki.join("entities").join("models.md").exists(),
            "expected entities/models.md to be created"
        );

        // Verify confidence in the frontmatter.
        let auth_content = std::fs::read_to_string(wiki.join("modules").join("auth.md")).unwrap();
        assert!(
            auth_content.contains("confidence: 0.4"),
            "auth page must have confidence 0.4: {auth_content}"
        );

        // Index should mention both slugs.
        let idx = std::fs::read_to_string(wiki.join("index.md")).unwrap();
        assert!(idx.contains("auth"), "index missing auth: {idx}");
        assert!(idx.contains("models"), "index missing models: {idx}");

        // Log should mention from-symbols.
        let log = std::fs::read_to_string(wiki.join("log.md")).unwrap();
        assert!(
            log.contains("from-symbols"),
            "log missing from-symbols entry: {log}"
        );
    }

    #[test]
    fn from_symbols_groups_by_module_correctly() {
        let scan_dir = Path::new("/project");
        let symbols = vec![
            Symbol {
                name: "handle_request".to_string(),
                kind: SymbolKind::Function,
                file: PathBuf::from("/project/src/auth/handler.rs"),
                line: 1,
                module_path: None,
            },
            Symbol {
                name: "validate".to_string(),
                kind: SymbolKind::Function,
                file: PathBuf::from("/project/src/auth/validate.rs"),
                line: 5,
                module_path: None,
            },
            Symbol {
                name: "User".to_string(),
                kind: SymbolKind::Struct,
                file: PathBuf::from("/project/src/models/user.rs"),
                line: 1,
                module_path: None,
            },
        ];

        let grouped = group_symbols_by_module(&symbols, scan_dir);

        // Two groups: auth (2 symbols) and models (1 symbol).
        assert_eq!(grouped.len(), 2);
        assert!(grouped.contains_key("auth"), "expected 'auth' group");
        assert!(grouped.contains_key("models"), "expected 'models' group");
        assert_eq!(grouped["auth"].len(), 2);
        assert_eq!(grouped["models"].len(), 1);
    }

    #[test]
    fn from_symbols_infers_page_types() {
        // Functions -> Module
        let f1 = Symbol {
            name: "foo".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("a.rs"),
            line: 1,
            module_path: None,
        };
        let f2 = Symbol {
            name: "bar".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("a.rs"),
            line: 5,
            module_path: None,
        };
        let func_syms: Vec<&Symbol> = vec![&f1, &f2];
        assert_eq!(infer_page_type(&func_syms), PageType::Module);

        // Structs dominate -> Entity
        let s1 = Symbol {
            name: "User".to_string(),
            kind: SymbolKind::Struct,
            file: PathBuf::from("b.rs"),
            line: 1,
            module_path: None,
        };
        let s2 = Symbol {
            name: "Session".to_string(),
            kind: SymbolKind::Struct,
            file: PathBuf::from("b.rs"),
            line: 10,
            module_path: None,
        };
        let s3 = Symbol {
            name: "new".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("b.rs"),
            line: 20,
            module_path: None,
        };
        let struct_syms: Vec<&Symbol> = vec![&s1, &s2, &s3];
        assert_eq!(infer_page_type(&struct_syms), PageType::Entity);

        // Traits -> Interface
        let t1 = Symbol {
            name: "Handler".to_string(),
            kind: SymbolKind::Trait,
            file: PathBuf::from("c.rs"),
            line: 1,
            module_path: None,
        };
        let t2 = Symbol {
            name: "handle".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("c.rs"),
            line: 5,
            module_path: None,
        };
        let trait_syms: Vec<&Symbol> = vec![&t1, &t2];
        assert_eq!(infer_page_type(&trait_syms), PageType::Interface);
    }

    // ─── v0.34.0 (M1) — FR-ONB-12, 29, 30 ─────────────────────────────────

    /// FR-ONB-12: `--estimate` prints the cost projection to stdout
    /// and does NOT write any pages. The runner is invoked exactly
    /// once (for plan generation).
    #[test]
    fn estimate_does_not_write_pages() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok(
            "plan:\n  - slug: alpha\n    type: module\n    confidence: 0.6\n    rationale: anchor\n",
        );
        let exit = run_with_runner(
            BootstrapArgs {
                estimate: true,
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();

        assert_eq!(exit, ExitCode::SUCCESS);
        assert!(
            !wiki.join("modules").join("alpha.md").exists(),
            "--estimate must not write pages"
        );
        // Planner called once.
        assert_eq!(runner.calls().len(), 1);
    }

    /// FR-ONB-29 pre-flight gate: `--max-cost` smaller than the
    /// estimated upper bound aborts BEFORE any page-body call. The
    /// runner is called once (for the plan), then we bail.
    #[test]
    fn max_cost_preflight_aborts_when_upper_bound_exceeds_cap() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        // 100-page plan against Claude → upper bound > $2 (way more
        // than the $0.01 cap below).
        let mut plan_yaml = String::from("plan:\n");
        for i in 0..100 {
            plan_yaml.push_str(&format!(
                "  - slug: slug-{i}\n    type: module\n    confidence: 0.6\n    rationale: r\n"
            ));
        }
        let runner = MockRunner::new();
        runner.push_ok(plan_yaml);

        let res = run_with_runner(
            BootstrapArgs {
                apply: true,
                max_cost: Some(0.01),
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        );
        std::env::set_current_dir(&cur).unwrap();
        let err = res.expect_err("must abort pre-flight");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Estimated upper bound") && msg.contains("exceeds"),
            "unexpected error: {msg}"
        );
        // Plan call only; no per-page calls.
        assert_eq!(runner.calls().len(), 1);
    }

    /// FR-ONB-29 mid-flight gate: 3-page plan, --max-cost=$1.50,
    /// real per-page usage of 100k input + 100k output = $1.80 cost
    /// per page. Page 1 lands ($1.80 spent), Page 2 gate triggers,
    /// state is marked partial, exit code is 2.
    #[test]
    fn max_cost_midflight_halts_and_marks_partial() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        // Plan with NO body — forces per-page LLM calls.
        let plan = "plan:\n  \
            - slug: alpha\n    type: module\n    confidence: 0.6\n    rationale: a\n  \
            - slug: beta\n    type: module\n    confidence: 0.6\n    rationale: b\n  \
            - slug: gamma\n    type: module\n    confidence: 0.6\n    rationale: c\n";
        let runner = MockRunner::new();
        runner.push_ok(plan);
        // Page 1 body call: 100k input + 100k output = $0.30 (Claude).
        // We pick a max_cost of $0.50 so page 1 lands (gate uses
        // projected cost, ~$0.014 for an empty-body entry; well
        // under the cap). After page 1 lands ($0.30 spent), page 2
        // gate: $0.30 + $0.014 > $0.50? Pick the cost numbers + cap
        // so the gate fires on page 2.
        let big_usage = coral_runner::TokenUsage {
            input_tokens: 100_000,
            output_tokens: 100_000,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        runner.push_ok_with_usage("# alpha\n\nbody", big_usage);
        // Should never be called.
        runner.push_ok("# beta\n\nbody");

        // 3 empty-body entries: pre-flight estimate ≈ 3 * (1500
        // input × $3/MTok + 800 output × $15/MTok) ≈ $0.05, upper
        // bound ≈ $0.07. Pre-flight passes with cap=$1.00.
        // Mid-flight: page 1 real cost = 100k × $3/MTok + 100k ×
        // $15/MTok = $0.30 + $1.50 = $1.80 → exceeds cap of $1.00
        // when page 2 gate fires ($1.80 + projected > $1.00 → halt).
        let exit = run_with_runner(
            BootstrapArgs {
                apply: true,
                max_cost: Some(1.0),
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        )
        .expect("must complete with partial exit code");
        std::env::set_current_dir(&cur).unwrap();

        // Exit code 2 = partial / max-cost reached.
        assert_eq!(format!("{exit:?}"), format!("{:?}", ExitCode::from(2)));

        // State on disk is `partial: true` with page 1 Completed.
        let state = BootstrapState::load(&wiki).unwrap().expect("state file");
        assert!(state.partial, "state must be marked partial");
        assert_eq!(state.pages.len(), 3);
        assert_eq!(state.pages[0].status, PageStatus::Completed);
        assert!(state.cost_spent_usd > 0.0);
    }

    /// FR-ONB-30: `--resume` re-uses the persisted plan and skips
    /// `Completed` pages. We seed a state with page[0] already
    /// Completed and page[1] Pending; one runner call (page 2 body)
    /// is enough.
    #[test]
    fn resume_skips_completed_pages_and_finishes_pending() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        // Build a state with two pages, page 0 already Completed.
        let plan = vec![
            PlanEntry {
                slug: "alpha".into(),
                action: Action::Create,
                r#type: Some(PageType::Module),
                confidence: Some(0.6),
                rationale: "first".into(),
                body: Some("# alpha".into()),
            },
            PlanEntry {
                slug: "beta".into(),
                action: Action::Create,
                r#type: Some(PageType::Module),
                confidence: Some(0.6),
                rationale: "second".into(),
                body: None,
            },
        ];
        let mut state = BootstrapState::fresh(plan, "claude-sonnet-4-5".into(), None);
        state.pages[0].status = PageStatus::Completed;
        state.save_atomic(&wiki).unwrap();

        let runner = MockRunner::new();
        // Only one runner call expected (page 2 body).
        runner.push_ok("# beta\n\nbody");

        let exit = run_with_runner(
            BootstrapArgs {
                apply: true,
                resume: true,
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();

        assert_eq!(exit, ExitCode::SUCCESS);
        // Page 2 wrote.
        assert!(wiki.join("modules").join("beta.md").exists());
        // Runner called exactly once (the body call for page 2 —
        // the planner is NOT re-called on --resume).
        assert_eq!(runner.calls().len(), 1);
        // State now has both pages Completed.
        let final_state = BootstrapState::load(&wiki).unwrap().unwrap();
        assert_eq!(final_state.pages[0].status, PageStatus::Completed);
        assert_eq!(final_state.pages[1].status, PageStatus::Completed);
    }

    /// FR-ONB-30: `--resume` without a checkpoint errors out
    /// actionably.
    #[test]
    fn resume_without_checkpoint_errors_actionably() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        let res = run_with_runner(
            BootstrapArgs {
                apply: true,
                resume: true,
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        );
        std::env::set_current_dir(&cur).unwrap();
        let err = res.expect_err("must error without state file");
        let msg = format!("{err:#}");
        assert!(msg.contains("--resume requires"), "unexpected error: {msg}");
    }

    /// FR-ONB-30: `--apply` writes the checkpoint with a fingerprint
    /// + the full plan persisted, and on completion every page is
    /// Completed.
    #[test]
    fn apply_writes_checkpoint_with_completed_pages() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let cur = std::env::current_dir().unwrap();
        let wiki = tmp.path().join(".wiki");
        seed_wiki_with_index(&wiki);
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        // Plan with body inline = per-page calls skipped; cost = 0.
        runner.push_ok(
            "plan:\n  - slug: alpha\n    type: module\n    confidence: 0.6\n    rationale: a\n    body: |\n      # alpha\n",
        );

        let exit = run_with_runner(
            BootstrapArgs {
                apply: true,
                ..Default::default()
            },
            Some(&wiki),
            &runner,
        )
        .unwrap();
        std::env::set_current_dir(&cur).unwrap();

        assert_eq!(exit, ExitCode::SUCCESS);
        let state = BootstrapState::load(&wiki).unwrap().expect("state");
        assert_eq!(state.pages.len(), 1);
        assert_eq!(state.pages[0].status, PageStatus::Completed);
        assert!(!state.plan_fingerprint.is_empty());
    }
}
