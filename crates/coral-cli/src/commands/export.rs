//! Export the wiki to various target formats: Markdown bundle, raw JSON,
//! Notion API page-create bodies, JSONL for fine-tuning datasets.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::page::Page;
use coral_core::walk;
use coral_runner::{Prompt, RunOutput, Runner, RunnerError, RunnerResult};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Output format. Choose: markdown-bundle | json | notion-json | jsonl.
    #[arg(long, default_value = "markdown-bundle")]
    pub format: String,
    /// Optional output file. If absent, prints to stdout.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Filter by page type (repeatable). Example: --type module --type concept.
    /// If empty, exports all types.
    #[arg(long = "type", value_name = "TYPE")]
    pub types: Vec<String>,
    /// Generate LLM-driven Q/A pairs per page (jsonl format only). v0.3.
    #[arg(long)]
    pub qa: bool,
    /// Override model name passed to the runner (e.g. "haiku", "gemini-2.5-flash").
    #[arg(long)]
    pub model: Option<String>,
    /// LLM provider used by --qa: claude (default) | gemini. Or set CORAL_PROVIDER env.
    #[arg(long)]
    pub provider: Option<String>,
}

pub fn run(args: ExportArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    if args.qa {
        let provider = super::runner_helper::resolve_provider(args.provider.as_deref())
            .map_err(|e| anyhow::anyhow!(e))?;
        let runner = super::runner_helper::make_runner(provider);
        return run_with_runner(args, wiki_root, runner.as_ref());
    }
    run_with_runner(args, wiki_root, &NoopRunner)
}

/// Runner used as a placeholder when `--qa` isn't set. Calling it indicates
/// a misuse — every code path that needs a runner must set `args.qa = true`.
#[derive(Debug, Default)]
struct NoopRunner;
impl Runner for NoopRunner {
    fn run(&self, _prompt: &Prompt) -> RunnerResult<RunOutput> {
        Err(RunnerError::NotFound)
    }
}

pub fn run_with_runner(
    args: ExportArgs,
    wiki_root: Option<&Path>,
    runner: &dyn Runner,
) -> Result<ExitCode> {
    let root = wiki_root
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
    let pages: Vec<Page> = if args.types.is_empty() {
        pages
    } else {
        let allow: std::collections::HashSet<&str> =
            args.types.iter().map(String::as_str).collect();
        pages
            .into_iter()
            .filter(|p| allow.contains(page_type_name(&p.frontmatter)))
            .collect()
    };

    let output = match args.format.as_str() {
        "markdown-bundle" => render_markdown_bundle(&pages),
        "json" => render_json(&pages)?,
        "notion-json" => render_notion_json(&pages)?,
        "jsonl" => {
            if args.qa {
                render_jsonl_with_qa(&pages, runner, args.model.as_deref())?
            } else {
                render_jsonl(&pages)?
            }
        }
        other => anyhow::bail!(
            "unknown format: {other}. Choose: markdown-bundle | json | notion-json | jsonl"
        ),
    };

    if let Some(path) = &args.out {
        std::fs::write(path, &output).with_context(|| format!("writing {}", path.display()))?;
        eprintln!("Wrote {} bytes to {}.", output.len(), path.display());
    } else {
        print!("{output}");
    }
    Ok(ExitCode::SUCCESS)
}

fn page_type_name(fm: &coral_core::frontmatter::Frontmatter) -> &'static str {
    use coral_core::frontmatter::PageType::*;
    match fm.page_type {
        Module => "module",
        Concept => "concept",
        Entity => "entity",
        Flow => "flow",
        Decision => "decision",
        Synthesis => "synthesis",
        Operation => "operation",
        Source => "source",
        Gap => "gap",
        Index => "index",
        Log => "log",
        Schema => "schema",
        Readme => "readme",
        Reference => "reference",
    }
}

fn status_name(fm: &coral_core::frontmatter::Frontmatter) -> &'static str {
    use coral_core::frontmatter::Status::*;
    match fm.status {
        Draft => "draft",
        Reviewed => "reviewed",
        Verified => "verified",
        Stale => "stale",
        Archived => "archived",
        Reference => "reference",
    }
}

/// Public accessor for `page_type_name` so sibling command modules
/// (e.g. `notion_push`) can reuse the canonical type label.
pub fn page_type_name_pub(fm: &coral_core::frontmatter::Frontmatter) -> &'static str {
    page_type_name(fm)
}

/// Public accessor for `status_name` (see `page_type_name_pub`).
pub fn status_name_pub(fm: &coral_core::frontmatter::Frontmatter) -> &'static str {
    status_name(fm)
}

fn render_markdown_bundle(pages: &[Page]) -> String {
    let mut out = String::from(
        "# Wiki bundle\n\nGenerated by `coral export --format markdown-bundle`.\n\n---\n\n",
    );
    for p in pages {
        out.push_str(&format!(
            "## {} ({})\n\n_status: {}, confidence: {:.2}_\n\n{}\n\n---\n\n",
            p.frontmatter.slug,
            page_type_name(&p.frontmatter),
            status_name(&p.frontmatter),
            p.frontmatter.confidence.as_f64(),
            p.body.trim()
        ));
    }
    out
}

fn render_json(pages: &[Page]) -> Result<String> {
    let arr: Vec<_> = pages
        .iter()
        .map(|p| {
            json!({
                "slug": p.frontmatter.slug,
                "type": page_type_name(&p.frontmatter),
                "status": status_name(&p.frontmatter),
                "confidence": p.frontmatter.confidence.as_f64(),
                "sources": p.frontmatter.sources,
                "backlinks": p.frontmatter.backlinks,
                "body": p.body,
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&arr)?)
}

fn render_notion_json(pages: &[Page]) -> Result<String> {
    // Each entry follows the Notion `POST /v1/pages` request body shape.
    // The consumer fills in `parent.database_id` from their config.
    let arr: Vec<_> = pages
        .iter()
        .map(|p| {
            json!({
                "parent": { "database_id": "<set-from-config>" },
                "properties": {
                    "Name": {
                        "title": [{ "text": { "content": p.frontmatter.slug } }]
                    },
                    "Type": {
                        "select": { "name": page_type_name(&p.frontmatter) }
                    },
                    "Status": {
                        "select": { "name": status_name(&p.frontmatter) }
                    },
                    "Confidence": {
                        "number": p.frontmatter.confidence.as_f64()
                    }
                },
                "children": [{
                    "object": "block",
                    "type": "paragraph",
                    "paragraph": {
                        "rich_text": [{
                            "type": "text",
                            "text": { "content": p.body.chars().take(2000).collect::<String>() }
                        }]
                    }
                }]
            })
        })
        .collect();
    Ok(serde_json::to_string_pretty(&arr)?)
}

fn render_jsonl(pages: &[Page]) -> Result<String> {
    // One JSON object per line. v0.2 ships raw page data with a stub prompt.
    // Pass --qa for LLM-driven Q/A pairs (v0.3+).
    let mut out = String::new();
    for p in pages {
        let line = json!({
            "slug": p.frontmatter.slug,
            "body": p.body,
            "prompt": format!("Tell me about [[{}]] in this wiki.", p.frontmatter.slug),
        });
        out.push_str(&serde_json::to_string(&line)?);
        out.push('\n');
    }
    Ok(out)
}

/// Hardcoded fallback used when neither a local override nor an embedded
/// `template/prompts/qa-pairs.md` is available.
pub const QA_FALLBACK: &str = "\
You are a fine-tuning dataset generator. For the wiki page below, emit \
3 to 5 question/answer pairs that an engineer might ask about its content.

Output rules — IMPORTANT:
- One JSON object per line, no fences, no prose, no commentary.
- Each line must be valid JSON with EXACTLY two keys: \"prompt\" and \"completion\".
- The \"prompt\" is the question; the \"completion\" is the answer (terse but complete).
- Do NOT include any other keys. Do NOT wrap the lines in an array.
- Do NOT prefix or suffix with markdown, headings, or explanations.
";

fn render_jsonl_with_qa(
    pages: &[Page],
    runner: &dyn Runner,
    model: Option<&str>,
) -> Result<String> {
    let template = super::prompt_loader::load_or_fallback("qa-pairs", QA_FALLBACK);
    let mut out = String::new();
    for p in pages {
        let user_prompt = format!(
            "<page slug=\"{}\" type=\"{}\">\n{}\n</page>",
            p.frontmatter.slug,
            page_type_name(&p.frontmatter),
            p.body.trim()
        );
        let prompt = Prompt {
            system: Some(template.content.clone()),
            user: user_prompt,
            model: model.map(String::from),
            ..Default::default()
        };
        let result = match runner.run(&prompt) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(slug = %p.frontmatter.slug, error = %e, "qa runner failed; skipping page");
                continue;
            }
        };

        for raw_line in result.stdout.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            let value: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => {
                    tracing::warn!(slug = %p.frontmatter.slug, line, "skipping malformed qa line");
                    continue;
                }
            };
            let prompt_field = value.get("prompt").and_then(|v| v.as_str());
            let completion_field = value.get("completion").and_then(|v| v.as_str());
            let (q, a) = match (prompt_field, completion_field) {
                (Some(q), Some(a)) => (q, a),
                _ => {
                    tracing::warn!(slug = %p.frontmatter.slug, line, "qa line missing prompt/completion");
                    continue;
                }
            };
            let tagged = json!({
                "slug": p.frontmatter.slug,
                "prompt": q,
                "completion": a,
            });
            out.push_str(&serde_json::to_string(&tagged)?);
            out.push('\n');
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use std::path::PathBuf;

    fn page(slug: &str, page_type: PageType, body: &str) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/modules/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type,
                last_updated_commit: "abc".to_string(),
                confidence: Confidence::try_new(0.8).unwrap(),
                sources: vec!["src/x.rs".into()],
                backlinks: vec![],
                status: Status::Reviewed,
                generated_at: None,
                extra: Default::default(),
            },
            body: body.to_string(),
        }
    }

    #[test]
    fn markdown_bundle_includes_all_pages() {
        let pages = vec![
            page("order", PageType::Module, "Order body."),
            page("idempotency", PageType::Concept, "Idempotency body."),
        ];
        let out = render_markdown_bundle(&pages);
        assert!(out.contains("## order (module)"));
        assert!(out.contains("## idempotency (concept)"));
        assert!(out.contains("Order body."));
        assert!(out.contains("Idempotency body."));
    }

    #[test]
    fn json_format_is_valid() {
        let pages = vec![page("x", PageType::Module, "body")];
        let out = render_json(&pages).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed[0]["slug"], "x");
        assert_eq!(parsed[0]["type"], "module");
    }

    #[test]
    fn notion_json_has_expected_shape() {
        let pages = vec![page("x", PageType::Module, "body")];
        let out = render_notion_json(&pages).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(parsed[0]["parent"]["database_id"].is_string());
        assert_eq!(
            parsed[0]["properties"]["Name"]["title"][0]["text"]["content"],
            "x"
        );
        assert_eq!(parsed[0]["properties"]["Type"]["select"]["name"], "module");
    }

    #[test]
    fn jsonl_emits_one_line_per_page() {
        let pages = vec![
            page("a", PageType::Module, "body a"),
            page("b", PageType::Concept, "body b"),
        ];
        let out = render_jsonl(&pages).unwrap();
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            let _: serde_json::Value = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn jsonl_includes_stub_prompt() {
        let pages = vec![page("x", PageType::Module, "body")];
        let out = render_jsonl(&pages).unwrap();
        assert!(out.contains("Tell me about [[x]]"));
    }

    #[test]
    fn qa_jsonl_uses_runner_per_page_and_tags_slug() {
        use coral_runner::MockRunner;
        let pages = vec![
            page("a", PageType::Module, "body a"),
            page("b", PageType::Concept, "body b"),
        ];
        let runner = MockRunner::new();
        runner.push_ok(
            "{\"prompt\":\"q1\",\"completion\":\"a1\"}\n{\"prompt\":\"q2\",\"completion\":\"a2\"}\n",
        );
        runner.push_ok("{\"prompt\":\"q3\",\"completion\":\"a3\"}\n");

        let out = render_jsonl_with_qa(&pages, &runner, Some("haiku")).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3, "expected 2+1 pairs, got: {out}");
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["slug"], "a");
        assert_eq!(first["prompt"], "q1");
        assert_eq!(first["completion"], "a1");
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["slug"], "a");
        let third: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(third["slug"], "b");
        assert_eq!(third["prompt"], "q3");

        let calls = runner.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].model.as_deref(), Some("haiku"));
        assert!(calls[0].user.contains("slug=\"a\""));
        assert!(calls[1].user.contains("slug=\"b\""));
    }

    #[test]
    fn qa_jsonl_skips_malformed_runner_output() {
        use coral_runner::MockRunner;
        let pages = vec![page("x", PageType::Module, "body")];
        let runner = MockRunner::new();
        runner.push_ok(
            "not json\n{\"prompt\":\"q\",\"completion\":\"a\"}\n{\"prompt\":\"missing-completion\"}\n{not really json}\n",
        );

        let out = render_jsonl_with_qa(&pages, &runner, None).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "only the well-formed line should pass: {out}"
        );
        let v: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(v["slug"], "x");
        assert_eq!(v["prompt"], "q");
        assert_eq!(v["completion"], "a");
    }
}
