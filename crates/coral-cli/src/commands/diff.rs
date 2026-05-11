//! `coral diff <slugA> <slugB>` — compare two wiki pages structurally.
//!
//! Use cases:
//! - You suspect two pages overlap and want to see whether to merge them.
//! - You're considering retiring a page and want to spot what claims it
//!   shares with neighboring pages.
//! - You're reviewing a `wiki/auto-ingest` PR and want to see what changed
//!   between an existing page and its proposed replacement.
//!
//! v0.5 ships the **structural** diff: frontmatter delta, sources/backlinks
//! set arithmetic, wikilink overlap, and body length stats. The optional
//! `--semantic` flag layers an LLM pass on top — it asks the configured
//! provider to surface contradictions, overlap, and coverage gaps between
//! the two bodies, then appends the response to the structural output.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::page::Page;
use coral_core::walk;
use coral_core::wikilinks;
use coral_runner::{Prompt, Runner};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct DiffArgs {
    /// Slug of the first page.
    pub slug_a: String,
    /// Slug of the second page.
    pub slug_b: String,
    /// Output format: markdown (default) or json.
    #[arg(long, default_value = "markdown")]
    pub format: String,
    /// Layer an LLM pass on top of the structural diff. Asks the configured
    /// provider to surface contradictions, overlap, and coverage gaps
    /// between the two page bodies.
    #[arg(long)]
    pub semantic: bool,
    /// Model id passed to the runner (e.g. `sonnet`, `haiku`, or a full id).
    /// Only used with `--semantic`; silently ignored otherwise.
    #[arg(long)]
    pub model: Option<String>,
    /// LLM provider used by --semantic: claude (default) | gemini | local.
    /// Or set CORAL_PROVIDER env. Silently ignored without `--semantic`.
    #[arg(long)]
    pub provider: Option<String>,
}

/// Entry point wired to `Cmd::Diff`. Loads the two pages, runs the
/// structural diff, and (when `--semantic` is set) builds a runner and
/// invokes the LLM. Without `--semantic`, no runner is constructed —
/// `--model` and `--provider` are quietly ignored.
pub fn run(args: DiffArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
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
    let a = find_page(&pages, &args.slug_a)
        .with_context(|| format!("page `{}` not found in {}", args.slug_a, root.display()))?;
    let b = find_page(&pages, &args.slug_b)
        .with_context(|| format!("page `{}` not found in {}", args.slug_b, root.display()))?;

    let report = compute_diff(a, b);

    let semantic_analysis: Option<String> = if args.semantic {
        let provider = super::runner_helper::resolve_provider(args.provider.as_deref())
            .map_err(|e| anyhow::anyhow!(e))?;
        let runner = super::runner_helper::make_runner(provider);
        Some(run_semantic_analysis(
            a,
            b,
            runner.as_ref(),
            args.model.as_deref(),
        )?)
    } else {
        None
    };

    let model_label = args.model.as_deref().unwrap_or("default");
    match args.format.as_str() {
        "json" => {
            let semantic_pair = semantic_analysis
                .as_deref()
                .map(|analysis| (model_label, analysis));
            println!(
                "{}",
                serde_json::to_string_pretty(&report.as_json_with_semantic(semantic_pair))?
            );
        }
        _ => println!(
            "{}",
            report.as_markdown_with_semantic(semantic_analysis.as_deref())
        ),
    }
    Ok(ExitCode::SUCCESS)
}

fn find_page<'a>(pages: &'a [Page], slug: &str) -> Result<&'a Page> {
    pages
        .iter()
        .find(|p| p.frontmatter.slug == slug)
        .ok_or_else(|| anyhow::anyhow!("no page with slug `{slug}`"))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DiffReport {
    pub slug_a: String,
    pub slug_b: String,
    pub same_type: bool,
    pub type_a: String,
    pub type_b: String,
    pub same_status: bool,
    pub status_a: String,
    pub status_b: String,
    pub confidence_delta: f64,
    pub sources_common: BTreeSet<String>,
    pub sources_only_a: BTreeSet<String>,
    pub sources_only_b: BTreeSet<String>,
    pub wikilinks_common: BTreeSet<String>,
    pub wikilinks_only_a: BTreeSet<String>,
    pub wikilinks_only_b: BTreeSet<String>,
    pub body_chars_a: usize,
    pub body_chars_b: usize,
}

impl DiffReport {
    /// Render the structural diff as markdown. Thin delegate to
    /// [`Self::as_markdown_with_semantic`] with no LLM block. Retained as
    /// the structural-only entry point for callers (tests, future
    /// embedders) that don't want the semantic block.
    #[allow(dead_code)]
    pub fn as_markdown(&self) -> String {
        self.as_markdown_with_semantic(None)
    }

    /// Render the structural diff as markdown, optionally appending the
    /// LLM-produced semantic analysis as a `## Semantic analysis` section.
    /// An empty `semantic` string is rendered as `_(no semantic findings)_`
    /// so the section header is still meaningful.
    pub fn as_markdown_with_semantic(&self, semantic: Option<&str>) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# Diff: `{}` ↔ `{}`\n\n",
            self.slug_a, self.slug_b
        ));

        out.push_str("## Frontmatter\n\n");
        out.push_str(&format!(
            "| field | `{a}` | `{b}` |\n|---|---|---|\n",
            a = self.slug_a,
            b = self.slug_b
        ));
        out.push_str(&format!(
            "| type | `{}` | `{}` |{}\n",
            self.type_a,
            self.type_b,
            if self.same_type { "" } else { " ⚠️ differ" }
        ));
        out.push_str(&format!(
            "| status | `{}` | `{}` |{}\n",
            self.status_a,
            self.status_b,
            if self.same_status {
                ""
            } else {
                " ⚠️ differ"
            }
        ));
        out.push_str(&format!(
            "| confidence Δ | — | — | {:+.2} |\n",
            self.confidence_delta
        ));
        out.push_str(&format!(
            "| body chars | {} | {} |\n\n",
            self.body_chars_a, self.body_chars_b
        ));

        section(
            &mut out,
            "Sources",
            &self.sources_common,
            &self.sources_only_a,
            &self.sources_only_b,
            &self.slug_a,
            &self.slug_b,
        );
        section(
            &mut out,
            "Wikilinks",
            &self.wikilinks_common,
            &self.wikilinks_only_a,
            &self.wikilinks_only_b,
            &self.slug_a,
            &self.slug_b,
        );

        if let Some(analysis) = semantic {
            out.push_str("## Semantic analysis\n\n");
            let trimmed = analysis.trim();
            if trimmed.is_empty() {
                out.push_str("_(no semantic findings)_\n");
            } else {
                out.push_str(trimmed);
                out.push('\n');
            }
        }

        out
    }

    /// Render the structural diff as a JSON value. Thin delegate to
    /// [`Self::as_json_with_semantic`] without the `semantic` field.
    /// Retained as the structural-only entry point for callers (tests,
    /// future embedders) that don't want the semantic block.
    #[allow(dead_code)]
    pub fn as_json(&self) -> serde_json::Value {
        self.as_json_with_semantic(None)
    }

    /// Render the structural diff as a JSON value, optionally including a
    /// top-level `semantic: { model, analysis }` field. When `semantic` is
    /// `None` the field is omitted entirely.
    pub fn as_json_with_semantic(&self, semantic: Option<(&str, &str)>) -> serde_json::Value {
        let mut value = serde_json::json!({
            "slug_a": self.slug_a,
            "slug_b": self.slug_b,
            "type": {
                "a": self.type_a, "b": self.type_b, "same": self.same_type,
            },
            "status": {
                "a": self.status_a, "b": self.status_b, "same": self.same_status,
            },
            "confidence_delta": self.confidence_delta,
            "body_chars": { "a": self.body_chars_a, "b": self.body_chars_b },
            "sources": {
                "common": self.sources_common,
                "only_a": self.sources_only_a,
                "only_b": self.sources_only_b,
            },
            "wikilinks": {
                "common": self.wikilinks_common,
                "only_a": self.wikilinks_only_a,
                "only_b": self.wikilinks_only_b,
            },
        });
        if let Some((model, analysis)) = semantic
            && let Some(obj) = value.as_object_mut()
        {
            obj.insert(
                "semantic".to_string(),
                serde_json::json!({ "model": model, "analysis": analysis }),
            );
        }
        value
    }
}

/// Hardcoded last-resort system prompt for `coral diff --semantic`.
/// Power users can override with `<cwd>/prompts/diff-semantic.md`, and the
/// embedded `template/prompts/diff-semantic.md` (when present) takes
/// priority over this string. See [`crate::commands::prompt_loader`].
pub(crate) const DIFF_SEMANTIC_FALLBACK: &str = "You are the Coral wiki diff analyzer. Read the two page bodies below and identify:\n\
1. Contradictions — claims in one page that the other directly contradicts.\n\
2. Overlap — topics or facts both pages cover, suggesting a merge candidate.\n\
3. Coverage gaps — claims one makes that the other should but doesn't.\n\
\n\
Be terse. Use bullet points. Cite both pages by slug. If pages are clearly distinct\n\
and have no contradiction or meaningful overlap, say so in one line.";

/// Build the diff-semantic prompt, dispatch it to the supplied runner, and
/// return the trimmed stdout. The system prompt is loaded via
/// [`crate::commands::prompt_loader::load_or_fallback`] so user overrides
/// in `<cwd>/prompts/diff-semantic.md` win over the embedded template,
/// which in turn wins over [`DIFF_SEMANTIC_FALLBACK`].
fn run_semantic_analysis(
    a: &Page,
    b: &Page,
    runner: &dyn Runner,
    model: Option<&str>,
) -> Result<String> {
    let prompt_template =
        super::prompt_loader::load_or_fallback("diff-semantic", DIFF_SEMANTIC_FALLBACK);
    let user = build_semantic_user_prompt(a, b);
    // v0.20.1 cycle-4 audit H3: append the untrusted-content notice
    // to the system prompt so the LLM treats fenced page bodies as
    // data rather than instructions.
    let mut system = prompt_template.content;
    system.push_str(super::common::untrusted_fence::UNTRUSTED_CONTENT_NOTICE);
    let prompt = Prompt {
        system: Some(system),
        user,
        model: model.map(str::to_string),
        ..Default::default()
    };
    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("semantic diff runner failed: {e}"))?;
    Ok(out.stdout.trim().to_string())
}

/// Pure builder for the user-prompt string we send to the runner. Split
/// out so it can be unit-tested without standing up a runner.
///
/// v0.20.1 cycle-4 audit H3: bodies are fenced via
/// [`crate::commands::common::untrusted_fence::fence_body_annotated`]
/// so a poisoned body cannot inject instructions into the prompt.
/// `--semantic` uses the annotated form (vs `query`'s drop form)
/// because dropping a page would render the diff meaningless.
fn build_semantic_user_prompt(a: &Page, b: &Page) -> String {
    use super::common::untrusted_fence::fence_body_annotated;
    format!(
        "Page A — slug: {slug_a}\n\
type: {type_a}, status: {status_a}, confidence: {conf_a:.2}\n\
\n\
{fenced_a}\n\
\n\
---\n\
\n\
Page B — slug: {slug_b}\n\
type: {type_b}, status: {status_b}, confidence: {conf_b:.2}\n\
\n\
{fenced_b}\n\
\n\
---\n\
\n\
Analyze.",
        slug_a = a.frontmatter.slug,
        type_a = format!("{:?}", a.frontmatter.page_type).to_lowercase(),
        status_a = format!("{:?}", a.frontmatter.status).to_lowercase(),
        conf_a = a.frontmatter.confidence.as_f64(),
        fenced_a = fence_body_annotated(a),
        slug_b = b.frontmatter.slug,
        type_b = format!("{:?}", b.frontmatter.page_type).to_lowercase(),
        status_b = format!("{:?}", b.frontmatter.status).to_lowercase(),
        conf_b = b.frontmatter.confidence.as_f64(),
        fenced_b = fence_body_annotated(b),
    )
}

fn section(
    out: &mut String,
    label: &str,
    common: &BTreeSet<String>,
    only_a: &BTreeSet<String>,
    only_b: &BTreeSet<String>,
    slug_a: &str,
    slug_b: &str,
) {
    out.push_str(&format!("## {label}\n\n"));
    if common.is_empty() && only_a.is_empty() && only_b.is_empty() {
        out.push_str("_(none)_\n\n");
        return;
    }
    if !common.is_empty() {
        out.push_str(&format!("### Common ({})\n\n", common.len()));
        for s in common {
            out.push_str(&format!("- {s}\n"));
        }
        out.push('\n');
    }
    if !only_a.is_empty() {
        out.push_str(&format!("### Only in `{slug_a}` ({})\n\n", only_a.len()));
        for s in only_a {
            out.push_str(&format!("- {s}\n"));
        }
        out.push('\n');
    }
    if !only_b.is_empty() {
        out.push_str(&format!("### Only in `{slug_b}` ({})\n\n", only_b.len()));
        for s in only_b {
            out.push_str(&format!("- {s}\n"));
        }
        out.push('\n');
    }
}

pub(crate) fn compute_diff(a: &Page, b: &Page) -> DiffReport {
    let sources_a: BTreeSet<String> = a.frontmatter.sources.iter().cloned().collect();
    let sources_b: BTreeSet<String> = b.frontmatter.sources.iter().cloned().collect();
    let wikis_a: BTreeSet<String> = wikilinks::extract(&a.body).into_iter().collect();
    let wikis_b: BTreeSet<String> = wikilinks::extract(&b.body).into_iter().collect();

    DiffReport {
        slug_a: a.frontmatter.slug.clone(),
        slug_b: b.frontmatter.slug.clone(),
        same_type: a.frontmatter.page_type == b.frontmatter.page_type,
        type_a: format!("{:?}", a.frontmatter.page_type).to_lowercase(),
        type_b: format!("{:?}", b.frontmatter.page_type).to_lowercase(),
        same_status: a.frontmatter.status == b.frontmatter.status,
        status_a: format!("{:?}", a.frontmatter.status).to_lowercase(),
        status_b: format!("{:?}", b.frontmatter.status).to_lowercase(),
        confidence_delta: b.frontmatter.confidence.as_f64() - a.frontmatter.confidence.as_f64(),
        sources_common: sources_a.intersection(&sources_b).cloned().collect(),
        sources_only_a: sources_a.difference(&sources_b).cloned().collect(),
        sources_only_b: sources_b.difference(&sources_a).cloned().collect(),
        wikilinks_common: wikis_a.intersection(&wikis_b).cloned().collect(),
        wikilinks_only_a: wikis_a.difference(&wikis_b).cloned().collect(),
        wikilinks_only_b: wikis_b.difference(&wikis_a).cloned().collect(),
        body_chars_a: a.body.chars().count(),
        body_chars_b: b.body.chars().count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use coral_runner::MockRunner;
    use coral_runner::runner::RunnerError;

    fn page(
        slug: &str,
        ty: PageType,
        status: Status,
        conf: f64,
        body: &str,
        sources: &[&str],
    ) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/x/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.into(),
                page_type: ty,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(conf).unwrap(),
                sources: sources.iter().map(|s| s.to_string()).collect(),
                backlinks: vec![],
                status,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                extra: Default::default(),
            },
            body: body.to_string(),
        }
    }

    /// Minimal `DiffReport` constructor for testing the rendering helpers
    /// without going through `compute_diff`. Keeps every field at a
    /// deterministic, trivially-non-empty value so each test only needs
    /// to assert the bit it cares about (typically the `semantic` block).
    fn mk_report() -> DiffReport {
        DiffReport {
            slug_a: "a".into(),
            slug_b: "b".into(),
            same_type: true,
            type_a: "module".into(),
            type_b: "module".into(),
            same_status: true,
            status_a: "reviewed".into(),
            status_b: "reviewed".into(),
            confidence_delta: 0.0,
            sources_common: BTreeSet::new(),
            sources_only_a: BTreeSet::new(),
            sources_only_b: BTreeSet::new(),
            wikilinks_common: BTreeSet::new(),
            wikilinks_only_a: BTreeSet::new(),
            wikilinks_only_b: BTreeSet::new(),
            body_chars_a: 0,
            body_chars_b: 0,
        }
    }

    #[test]
    fn diff_finds_common_sources_and_wikilinks() {
        let a = page(
            "order",
            PageType::Module,
            Status::Reviewed,
            0.8,
            "See [[outbox]] and [[idempotency]].",
            &["src/order.rs", "docs/adr/0001.md"],
        );
        let b = page(
            "checkout",
            PageType::Flow,
            Status::Reviewed,
            0.7,
            "Goes through [[outbox]] then [[payment]].",
            &["src/checkout.rs", "docs/adr/0001.md"],
        );
        let r = compute_diff(&a, &b);
        assert_eq!(r.slug_a, "order");
        assert_eq!(r.slug_b, "checkout");
        assert!(!r.same_type);
        assert!(r.same_status);
        assert!((r.confidence_delta - (-0.1)).abs() < 1e-9);
        assert_eq!(r.sources_common, set(&["docs/adr/0001.md"]));
        assert_eq!(r.sources_only_a, set(&["src/order.rs"]));
        assert_eq!(r.sources_only_b, set(&["src/checkout.rs"]));
        assert_eq!(r.wikilinks_common, set(&["outbox"]));
        assert_eq!(r.wikilinks_only_a, set(&["idempotency"]));
        assert_eq!(r.wikilinks_only_b, set(&["payment"]));
    }

    #[test]
    fn diff_markdown_includes_each_section_only_when_non_empty() {
        let a = page(
            "a",
            PageType::Module,
            Status::Reviewed,
            0.7,
            "no links",
            &["src/a.rs"],
        );
        let b = page(
            "b",
            PageType::Module,
            Status::Reviewed,
            0.7,
            "also no links",
            &["src/a.rs"],
        );
        let md = compute_diff(&a, &b).as_markdown();
        assert!(md.contains("# Diff: `a` ↔ `b`"));
        assert!(md.contains("## Sources"));
        assert!(md.contains("Common (1)"));
        // Wikilinks empty on both sides → "_(none)_".
        assert!(md.contains("## Wikilinks"));
        assert!(md.contains("_(none)_"));
    }

    #[test]
    fn diff_json_round_trips_to_a_serde_value() {
        let a = page(
            "a",
            PageType::Module,
            Status::Reviewed,
            0.7,
            "[[x]]",
            &["s1"],
        );
        let b = page(
            "b",
            PageType::Module,
            Status::Reviewed,
            0.7,
            "[[y]]",
            &["s2"],
        );
        let v = compute_diff(&a, &b).as_json();
        assert_eq!(v["slug_a"], "a");
        assert_eq!(v["wikilinks"]["only_a"][0], "x");
        assert_eq!(v["sources"]["only_b"][0], "s2");
    }

    #[test]
    fn diff_confidence_delta_is_b_minus_a() {
        let a = page("a", PageType::Module, Status::Reviewed, 0.5, "", &[]);
        let b = page("b", PageType::Module, Status::Reviewed, 0.9, "", &[]);
        let r = compute_diff(&a, &b);
        assert!((r.confidence_delta - 0.4).abs() < 1e-9);
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // ---- --semantic flag: markdown rendering --------------------------

    #[test]
    fn markdown_with_semantic_appends_section_with_analysis() {
        let r = mk_report();
        let md = r.as_markdown_with_semantic(Some("**Findings**: pages overlap on outbox."));
        assert!(
            md.contains("## Semantic analysis"),
            "missing semantic section header: {md}"
        );
        assert!(
            md.contains("**Findings**: pages overlap on outbox."),
            "missing analysis body: {md}"
        );
        assert!(
            !md.contains("_(no semantic findings)_"),
            "should not show empty placeholder when analysis is present: {md}"
        );
    }

    #[test]
    fn markdown_with_semantic_renders_empty_string_as_placeholder() {
        let r = mk_report();
        let md = r.as_markdown_with_semantic(Some(""));
        assert!(md.contains("## Semantic analysis"));
        assert!(
            md.contains("_(no semantic findings)_"),
            "empty analysis must render placeholder: {md}"
        );
    }

    #[test]
    fn markdown_with_semantic_renders_whitespace_only_as_placeholder() {
        let r = mk_report();
        let md = r.as_markdown_with_semantic(Some("   \n  "));
        assert!(md.contains("## Semantic analysis"));
        assert!(
            md.contains("_(no semantic findings)_"),
            "whitespace-only analysis must render placeholder: {md}"
        );
    }

    #[test]
    fn markdown_with_semantic_none_matches_as_markdown() {
        let r = mk_report();
        assert_eq!(
            r.as_markdown(),
            r.as_markdown_with_semantic(None),
            "None semantic must produce identical output to as_markdown()"
        );
        assert!(
            !r.as_markdown_with_semantic(None)
                .contains("Semantic analysis"),
            "None semantic must not emit the section header"
        );
    }

    // ---- --semantic flag: JSON rendering ------------------------------

    #[test]
    fn json_with_semantic_includes_model_and_analysis() {
        let r = mk_report();
        let v = r.as_json_with_semantic(Some(("haiku", "**done**")));
        let semantic = v
            .get("semantic")
            .expect("semantic field must be present when Some(...) passed");
        assert_eq!(semantic["model"], "haiku");
        assert_eq!(semantic["analysis"], "**done**");
    }

    #[test]
    fn json_with_semantic_none_omits_field_entirely() {
        let r = mk_report();
        let v = r.as_json_with_semantic(None);
        assert!(
            v.get("semantic").is_none(),
            "semantic key must be absent when None is passed; got: {v}"
        );
        // And the public delegate matches.
        assert_eq!(r.as_json(), v);
    }

    // ---- --semantic flag: user-prompt builder -------------------------

    #[test]
    fn build_semantic_user_prompt_includes_all_frontmatter_and_bodies() {
        let a = page(
            "alpha-slug",
            PageType::Module,
            Status::Reviewed,
            0.80,
            "ALPHA BODY TEXT",
            &[],
        );
        let b = page(
            "beta-slug",
            PageType::Flow,
            Status::Draft,
            0.30,
            "BETA BODY TEXT",
            &[],
        );
        let s = build_semantic_user_prompt(&a, &b);

        // Both slugs.
        assert!(s.contains("alpha-slug"), "missing slug A: {s}");
        assert!(s.contains("beta-slug"), "missing slug B: {s}");
        // Both types (lowercased).
        assert!(s.contains("module"), "missing type A: {s}");
        assert!(s.contains("flow"), "missing type B: {s}");
        // Both statuses (lowercased).
        assert!(s.contains("reviewed"), "missing status A: {s}");
        assert!(s.contains("draft"), "missing status B: {s}");
        // Both confidences, formatted with 2 decimals.
        assert!(s.contains("0.80"), "missing confidence A (0.80): {s}");
        assert!(s.contains("0.30"), "missing confidence B (0.30): {s}");
        // Both bodies verbatim.
        assert!(s.contains("ALPHA BODY TEXT"), "missing body A: {s}");
        assert!(s.contains("BETA BODY TEXT"), "missing body B: {s}");
        // The `---` separator appears at least twice (between A/B and after B).
        assert!(
            s.matches("---").count() >= 2,
            "expected at least 2 `---` separators: {s}"
        );
        // Final line directs the LLM to analyze.
        assert!(
            s.trim_end().ends_with("Analyze."),
            "must end with 'Analyze.': {s}"
        );
    }

    // ---- --semantic flag: run_semantic_analysis with a mock runner ----

    #[test]
    fn run_semantic_analysis_dispatches_to_runner_and_trims() {
        let a = page("a", PageType::Module, Status::Reviewed, 0.7, "body a", &[]);
        let b = page("b", PageType::Module, Status::Reviewed, 0.7, "body b", &[]);
        let runner = MockRunner::new();
        // Surrounding whitespace must be trimmed by run_semantic_analysis.
        runner.push_ok("\n  **Contradiction**: A says X but B says Y.\n  \n");

        let out = run_semantic_analysis(&a, &b, &runner, Some("haiku"))
            .expect("mock runner must not fail");
        assert_eq!(out, "**Contradiction**: A says X but B says Y.");

        // Runner saw exactly one call, with the expected model + system prompt
        // shape and a user prompt that came from build_semantic_user_prompt.
        let calls = runner.calls();
        assert_eq!(calls.len(), 1, "runner must be invoked exactly once");
        let p = &calls[0];
        assert_eq!(p.model.as_deref(), Some("haiku"));
        let system = p.system.as_deref().unwrap_or("");
        assert!(
            system.contains("Coral wiki diff analyzer"),
            "system prompt must come from diff-semantic loader (fallback or override); got: {system}"
        );
        // The user prompt must include both slugs from our pages.
        assert!(
            p.user.contains("a"),
            "user prompt missing slug a: {}",
            p.user
        );
        assert!(
            p.user.contains("b"),
            "user prompt missing slug b: {}",
            p.user
        );
    }

    #[test]
    fn run_semantic_analysis_propagates_runner_errors() {
        let a = page("a", PageType::Module, Status::Reviewed, 0.7, "x", &[]);
        let b = page("b", PageType::Module, Status::Reviewed, 0.7, "y", &[]);
        let runner = MockRunner::new();
        runner.push_err(RunnerError::NotFound);

        let r = run_semantic_analysis(&a, &b, &runner, None);
        assert!(r.is_err(), "expected runner error to surface as Err");
        let msg = format!("{}", r.unwrap_err());
        assert!(
            msg.contains("semantic diff runner failed"),
            "error must be wrapped with diff context; got: {msg}"
        );
    }
}
