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
    /// Output format. Choose: markdown-bundle | json | notion-json | html | jsonl | llms-txt.
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
    /// HTML only: split output into multiple files (index.html + per-page
    /// `<type>/<slug>.html` + extracted `style.css`). Requires --out to be a
    /// directory. Designed for hosting on GitHub Pages or any static host.
    #[arg(long)]
    pub multi: bool,
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

    // --multi is HTML-only and writes a directory tree, so it short-circuits
    // the single-string render+write path used by every other format.
    if args.multi {
        if args.format != "html" {
            anyhow::bail!("--multi is only valid with --format html");
        }
        let out_dir = args
            .out
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("--multi requires --out <directory>"))?;
        let written = render_html_multi(&pages, out_dir)?;
        eprintln!("Wrote {written} files to {}.", out_dir.display());
        return Ok(ExitCode::SUCCESS);
    }

    let output = match args.format.as_str() {
        "markdown-bundle" => render_markdown_bundle(&pages),
        "json" => render_json(&pages)?,
        "notion-json" => render_notion_json(&pages)?,
        "html" => render_html(&pages),
        "jsonl" => {
            if args.qa {
                render_jsonl_with_qa(&pages, runner, args.model.as_deref())?
            } else {
                render_jsonl(&pages)?
            }
        }
        "llms-txt" => {
            let project_name = detect_project_name(&root);
            coral_core::llms_txt::generate(&pages, &project_name)
        }
        other => anyhow::bail!(
            "unknown format: {other}. Choose: markdown-bundle | json | notion-json | html | jsonl | llms-txt"
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
        Interface => "interface",
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

/// Detect the project name from `coral.toml` (parent of wiki root).
fn detect_project_name(wiki_root: &Path) -> String {
    let parent = wiki_root.parent().unwrap_or(wiki_root);
    let manifest_path = parent.join("coral.toml");
    if let Ok(raw) = std::fs::read_to_string(&manifest_path) {
        if let Ok(table) = raw.parse::<toml::Table>() {
            if let Some(name) = table.get("name").and_then(|v| v.as_str()) {
                return name.to_string();
            }
        }
    }
    parent
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("wiki")
        .to_string()
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

/// Translate `[[slug]]`, `[[slug|alias]]`, `[[slug#anchor]]` wikilinks into
/// CommonMark links to in-page anchors so pulldown-cmark renders them as
/// real `<a href="#slug">` elements. Pure for testability.
pub(crate) fn translate_wikilinks_to_anchors(body: &str) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re =
        RE.get_or_init(|| regex::Regex::new(r"\[\[([^\]\n]+)\]\]").expect("valid wikilink regex"));
    re.replace_all(body, |caps: &regex::Captures| {
        let inner = &caps[1];
        let (slug, label) = if let Some((s, a)) = inner.split_once('|') {
            (s.trim(), a.trim().to_string())
        } else {
            let s = inner
                .split_once('#')
                .map(|(s, _)| s)
                .unwrap_or(inner)
                .trim();
            (s, inner.trim().to_string())
        };
        format!("[{label}](#{slug})")
    })
    .into_owned()
}

fn render_html(pages: &[Page]) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    // v0.20.1 cycle-4 audit C2: filter out pages whose slug isn't a
    // safe identifier BEFORE building any HTML. The slug ends up in
    // both `id="..."` attributes and `<h1>` text in the single-bundle
    // export — even with `html_escape`, an `id` attribute has no
    // escape grammar by spec, so the only safe path is a strict
    // allowlist (the same one already used by `render_html_multi`).
    // A page with `slug: x"><script>alert(1)</script>` would otherwise
    // produce live XSS in the exported single-file HTML bundle.
    let safe_pages: Vec<&Page> = pages
        .iter()
        .filter(|p| {
            if coral_core::slug::is_safe_filename_slug(&p.frontmatter.slug) {
                true
            } else {
                tracing::warn!(slug = %p.frontmatter.slug, "skipping export: unsafe slug");
                false
            }
        })
        .collect();

    // Group pages by type for the sidebar TOC.
    let mut by_type: std::collections::BTreeMap<&str, Vec<&Page>> =
        std::collections::BTreeMap::new();
    for p in &safe_pages {
        by_type
            .entry(page_type_name(&p.frontmatter))
            .or_default()
            .push(*p);
    }

    let mut toc = String::from("<nav class=\"toc\">\n<h2>Pages</h2>\n");
    for (ty, ps) in &by_type {
        toc.push_str(&format!(
            "<details open><summary>{ty} ({})</summary>\n<ul>\n",
            ps.len()
        ));
        for p in ps {
            toc.push_str(&format!(
                "<li><a href=\"#{slug}\">{slug}</a></li>\n",
                slug = html_escape(&p.frontmatter.slug),
            ));
        }
        toc.push_str("</ul>\n</details>\n");
    }
    toc.push_str("</nav>\n");

    let mut sections = String::new();
    for p in &safe_pages {
        let translated = translate_wikilinks_to_anchors(&p.body);
        let mut html_body = String::new();
        // v0.19.8 #30 audit-gap conversion: pulldown-cmark passes raw
        // HTML through verbatim (CommonMark behavior). A wiki page body
        // with `<script>...</script>` would land in the rendered output
        // unchanged — XSS in any browser viewing the static export.
        // pulldown-cmark 0.13 has no `Options::ENABLE_HTML` flag, so we
        // sanitize at the Event level: every `Html` / `InlineHtml`
        // chunk is replaced with its HTML-escaped text equivalent.
        let parser = Parser::new_ext(&translated, opts).map(escape_raw_html_event);
        html::push_html(&mut html_body, parser);
        sections.push_str(&format!(
            "<section id=\"{slug}\" class=\"page page-{ty}\">\n  <header><h1>{slug_esc}</h1>\n  <p class=\"meta\">type: <code>{ty}</code> · status: <code>{status}</code> · confidence: <code>{conf:.2}</code> · last_commit: <code>{commit}</code></p></header>\n  <div class=\"body\">{html_body}</div>\n</section>\n",
            slug = p.frontmatter.slug,
            slug_esc = html_escape(&p.frontmatter.slug),
            ty = page_type_name(&p.frontmatter),
            status = status_name(&p.frontmatter),
            conf = p.frontmatter.confidence.as_f64(),
            commit = html_escape(&p.frontmatter.last_updated_commit),
            html_body = html_body,
        ));
    }

    format!(
        "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>Coral wiki</title>\n\
  <style>{css}</style>\n\
</head>\n\
<body>\n\
  <header class=\"site-header\"><h1>Coral wiki</h1>\n  <p>{count} pages · generated by <code>coral export --format html</code></p></header>\n\
  <main>\n{toc}    <article>\n{sections}    </article>\n  </main>\n\
</body>\n\
</html>\n",
        css = HTML_CSS,
        count = safe_pages.len(),
        toc = toc,
        sections = sections,
    )
}

/// Translate `[[slug]]`, `[[slug|alias]]`, `[[slug#anchor]]` wikilinks into
/// CommonMark links to *other files* in the multi-page export. Each target
/// page lives at `<type>/<slug>.html` relative to the output root, so a
/// link from one page to another resolves via `../<type>/<slug>.html`.
///
/// Pages whose slug is unknown to the index fall back to the in-page anchor
/// form (`#slug`) so we still produce valid markup; a stale wikilink is
/// preferable to an export error.
fn translate_wikilinks_to_multi(
    body: &str,
    slug_to_type: &std::collections::HashMap<String, String>,
) -> String {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re =
        RE.get_or_init(|| regex::Regex::new(r"\[\[([^\]\n]+)\]\]").expect("valid wikilink regex"));
    re.replace_all(body, |caps: &regex::Captures| {
        let inner = &caps[1];
        let (slug, label) = if let Some((s, a)) = inner.split_once('|') {
            (s.trim(), a.trim().to_string())
        } else {
            let s = inner
                .split_once('#')
                .map(|(s, _)| s)
                .unwrap_or(inner)
                .trim();
            (s, inner.trim().to_string())
        };
        match slug_to_type.get(slug) {
            Some(ty) => format!("[{label}](../{ty}/{slug}.html)"),
            None => format!("[{label}](#{slug})"),
        }
    })
    .into_owned()
}

/// Render the wiki as a directory tree of HTML files for static hosting.
///
/// Layout written under `out_dir`:
///
/// ```text
/// out_dir/
///   index.html                 — TOC of all pages, grouped by type
///   style.css                  — shared CSS (pulled out of the template)
///   <type>/<slug>.html         — one file per page
/// ```
///
/// Per-page files link to siblings via `../<type>/<slug>.html` and back to
/// the TOC via `../index.html`. Returns the number of files written
/// (`index.html` + `style.css` + N pages).
pub(crate) fn render_html_multi(pages: &[Page], out_dir: &Path) -> Result<usize> {
    use pulldown_cmark::{Options, Parser, html};

    // Reject `--out path/to/file.html`. We require a directory because we
    // create a tree under it and a stray file at that path would block the
    // tree creation later anyway — better to fail loud and early.
    if out_dir.exists() && !out_dir.is_dir() {
        anyhow::bail!(
            "--multi requires --out to be a directory; {} is a file",
            out_dir.display()
        );
    }
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("creating output directory {}", out_dir.display()))?;

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    // v0.20.1 cycle-4 audit C3: filter unsafe slugs BEFORE building
    // the TOC. Pre-fix, the TOC builder iterated `pages` directly,
    // baking unsafe slugs into `index.html` even though the per-page
    // write (line ~446) skipped them — leaving live XSS in the index
    // even though the per-page file was never created. The filter is
    // hoisted here so TOC and disk stay consistent: both skip the
    // unsafe page.
    let safe_pages: Vec<&Page> = pages
        .iter()
        .filter(|p| {
            if coral_core::slug::is_safe_filename_slug(&p.frontmatter.slug) {
                true
            } else {
                tracing::warn!(slug = %p.frontmatter.slug, "skipping export: unsafe slug");
                false
            }
        })
        .collect();

    // Build a slug -> type lookup so wikilinks resolve to the correct
    // `<type>/<slug>.html` file across pages.
    let mut slug_to_type: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for p in &safe_pages {
        slug_to_type.insert(
            p.frontmatter.slug.clone(),
            page_type_name(&p.frontmatter).to_string(),
        );
    }

    // Group pages by type for the TOC.
    let mut by_type: std::collections::BTreeMap<&str, Vec<&Page>> =
        std::collections::BTreeMap::new();
    for p in &safe_pages {
        by_type
            .entry(page_type_name(&p.frontmatter))
            .or_default()
            .push(*p);
    }

    let mut files_written = 0usize;

    // 1) shared style.css
    let css_path = out_dir.join("style.css");
    std::fs::write(&css_path, HTML_CSS)
        .with_context(|| format!("writing {}", css_path.display()))?;
    files_written += 1;

    // 2) index.html (TOC)
    let mut toc_html = String::new();
    toc_html.push_str("<nav class=\"toc\">\n<h2>Pages</h2>\n");
    for (ty, ps) in &by_type {
        toc_html.push_str(&format!(
            "<details open><summary>{ty} ({})</summary>\n<ul>\n",
            ps.len()
        ));
        for p in ps {
            let slug = &p.frontmatter.slug;
            toc_html.push_str(&format!(
                "<li><a href=\"{ty}/{slug}.html\">{slug_esc}</a></li>\n",
                ty = ty,
                slug = slug,
                slug_esc = html_escape(slug),
            ));
        }
        toc_html.push_str("</ul>\n</details>\n");
    }
    toc_html.push_str("</nav>\n");

    let index_html = format!(
        "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>Coral wiki</title>\n\
  <link rel=\"stylesheet\" href=\"style.css\">\n\
</head>\n\
<body>\n\
  <header class=\"site-header\"><h1>Coral wiki</h1>\n  <p>{count} pages · generated by <code>coral export --format html --multi</code></p></header>\n\
  <main>\n{toc}    <article>\n      <p>Browse pages in the sidebar.</p>\n    </article>\n  </main>\n\
</body>\n\
</html>\n",
        count = safe_pages.len(),
        toc = toc_html,
    );
    let index_path = out_dir.join("index.html");
    std::fs::write(&index_path, index_html)
        .with_context(|| format!("writing {}", index_path.display()))?;
    files_written += 1;

    // 3) one file per page under <type>/<slug>.html. The slug
    // safety check already happened above (see audit C3 comment) —
    // `safe_pages` is guaranteed not to contain unsafe slugs.
    for p in &safe_pages {
        let ty = page_type_name(&p.frontmatter);
        let slug = &p.frontmatter.slug;
        let type_dir = out_dir.join(ty);
        std::fs::create_dir_all(&type_dir)
            .with_context(|| format!("creating {}", type_dir.display()))?;

        let translated = translate_wikilinks_to_multi(&p.body, &slug_to_type);
        let mut html_body = String::new();
        // v0.19.8 #30: same Html-event sanitizer as `render_html`.
        // See the comment there for rationale.
        let parser = Parser::new_ext(&translated, opts).map(escape_raw_html_event);
        html::push_html(&mut html_body, parser);

        let page_html = format!(
            "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>{slug_esc} · Coral wiki</title>\n\
  <link rel=\"stylesheet\" href=\"../style.css\">\n\
</head>\n\
<body>\n\
  <header class=\"site-header\"><h1>{slug_esc}</h1>\n  <p class=\"meta\">type: <code>{ty}</code> · status: <code>{status}</code> · confidence: <code>{conf:.2}</code> · last_commit: <code>{commit}</code></p>\n  <p><a href=\"../index.html\">&larr; back to index</a></p></header>\n\
  <main><article class=\"body\">{html_body}</article></main>\n\
</body>\n\
</html>\n",
            slug_esc = html_escape(slug),
            ty = ty,
            status = status_name(&p.frontmatter),
            conf = p.frontmatter.confidence.as_f64(),
            commit = html_escape(&p.frontmatter.last_updated_commit),
            html_body = html_body,
        );

        let page_path = type_dir.join(format!("{slug}.html"));
        std::fs::write(&page_path, page_html)
            .with_context(|| format!("writing {}", page_path.display()))?;
        files_written += 1;
    }

    Ok(files_written)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// v0.19.8 #30 audit-gap conversion: turn a `pulldown_cmark::Event`
/// carrying raw HTML into its escaped-text equivalent and neutralize
/// unsafe URL schemes in link destinations.
///
/// Two failure modes the audit flagged:
///
///   1. CommonMark allows raw HTML in markdown; `pulldown-cmark` 0.13
///      emits `Event::Html(s)` (block-level HTML lines, e.g.
///      `<script>...</script>` on its own line) and `Event::InlineHtml(s)`
///      (HTML in a paragraph, e.g. `<img onerror=...>`). Both pass
///      through `html::push_html` verbatim by default. Re-emitting
///      them as `Event::Text` produces HTML-escaped output —
///      `<script>` becomes `&lt;script&gt;`, never reaching the
///      browser as a tag.
///
///   2. CommonMark allows arbitrary URL schemes in link destinations.
///      `[click me](javascript:alert(1))` would render as
///      `<a href="javascript:alert(1)">click me</a>` — XSS the
///      moment a reader clicks. We rewrite such hrefs to a benign
///      placeholder (`#`) and add a `data-coral-unsafe-href` attr
///      via the link-text trick: actually we substitute the
///      `dest_url` to `#` directly. The unsafe scheme list is the
///      common XSS surface: `javascript:`, `data:`, `vbscript:`,
///      `file:`. Everything else (relative paths, `http://`,
///      `https://`, `mailto:`, `tel:`, `#fragment`) is allowed.
///
/// This is the smallest possible XSS hardening. It does NOT attempt
/// HTML sanitization (allowlisting tags / attributes); a future
/// "rich HTML pages" feature would need a real sanitizer dep.
/// Coral's stance: wikis are markdown-first; raw HTML is never
/// rendered.
fn escape_raw_html_event(ev: pulldown_cmark::Event<'_>) -> pulldown_cmark::Event<'_> {
    use pulldown_cmark::{CowStr, Event, Tag};
    match ev {
        Event::Html(s) => Event::Text(CowStr::Boxed(s.to_string().into_boxed_str())),
        Event::InlineHtml(s) => Event::Text(CowStr::Boxed(s.to_string().into_boxed_str())),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) if is_unsafe_url_scheme(&dest_url) => Event::Start(Tag::Link {
            link_type,
            dest_url: CowStr::Borrowed("#"),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) if is_unsafe_url_scheme(&dest_url) => Event::Start(Tag::Image {
            link_type,
            dest_url: CowStr::Borrowed("#"),
            title,
            id,
        }),
        other => other,
    }
}

/// v0.19.8 #30: returns `true` for URL schemes that the static-HTML
/// export must NOT preserve (XSS sinks the moment a reader clicks
/// the link). Comparison is ASCII-case-insensitive over the prefix
/// before the first `:` — `JavaScript:` and `javascript:` both
/// match. Whitespace before the scheme is also stripped (some
/// browsers strip leading whitespace before parsing the scheme).
fn is_unsafe_url_scheme(href: &str) -> bool {
    let trimmed = href.trim_start();
    // Strip ASCII control bytes that some browsers ignore inside
    // schemes (`\t`, `\r`, `\n`, plus NUL through SP minus the ones
    // already trimmed). E.g. `java\tscript:` triggers in some
    // legacy parsers. Build a normalized prefix without those.
    let mut normalized = String::new();
    for b in trimmed.bytes() {
        if b > 0x20 {
            // Once we hit `:`, the scheme is over.
            if b == b':' {
                normalized.push(':');
                break;
            }
            normalized.push(b.to_ascii_lowercase() as char);
        }
    }
    matches!(
        normalized.as_str(),
        "javascript:" | "data:" | "vbscript:" | "file:"
    )
}

const HTML_CSS: &str = "\
:root { --fg: #1a1a1a; --bg: #fff; --muted: #6a6a6a; --link: #0366d6; --code-bg: #f4f4f4; --border: #ddd; }\
@media (prefers-color-scheme: dark) { :root { --fg: #e6e6e6; --bg: #161616; --muted: #888; --link: #58a6ff; --code-bg: #222; --border: #333; } }\
* { box-sizing: border-box; }\
body { margin: 0; font: 16px/1.55 -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; color: var(--fg); background: var(--bg); }\
.site-header { padding: 24px; border-bottom: 1px solid var(--border); }\
.site-header h1 { margin: 0 0 4px 0; font-size: 22px; }\
.site-header p { margin: 0; color: var(--muted); font-size: 13px; }\
main { display: grid; grid-template-columns: 260px 1fr; max-width: 1400px; margin: 0 auto; }\
nav.toc { padding: 24px; border-right: 1px solid var(--border); position: sticky; top: 0; max-height: 100vh; overflow-y: auto; }\
nav.toc h2 { margin: 0 0 12px 0; font-size: 14px; text-transform: uppercase; color: var(--muted); }\
nav.toc details { margin-bottom: 8px; }\
nav.toc summary { cursor: pointer; font-weight: 600; font-size: 14px; padding: 4px 0; }\
nav.toc ul { list-style: none; margin: 0; padding-left: 12px; }\
nav.toc li { padding: 2px 0; font-size: 13px; }\
nav.toc a { color: var(--link); text-decoration: none; }\
nav.toc a:hover { text-decoration: underline; }\
article { padding: 24px 32px; min-width: 0; }\
section.page { margin-bottom: 48px; padding-bottom: 24px; border-bottom: 1px solid var(--border); }\
section.page header h1 { margin: 0 0 4px 0; }\
section.page header .meta { margin: 0 0 16px 0; color: var(--muted); font-size: 13px; }\
section.page .body :first-child { margin-top: 0; }\
code { background: var(--code-bg); padding: 1px 5px; border-radius: 3px; font-size: 0.9em; }\
pre { background: var(--code-bg); padding: 12px; border-radius: 4px; overflow-x: auto; }\
pre code { background: none; padding: 0; }\
a { color: var(--link); }\
table { border-collapse: collapse; margin: 12px 0; }\
table th, table td { border: 1px solid var(--border); padding: 6px 12px; text-align: left; }\
@media (max-width: 800px) { main { grid-template-columns: 1fr; } nav.toc { position: static; max-height: none; border-right: none; border-bottom: 1px solid var(--border); } }\
";

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
                valid_from: None,
                valid_to: None,
                superseded_by: None,
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
    fn translate_wikilinks_handles_plain_alias_and_anchor_forms() {
        // Plain
        assert_eq!(
            translate_wikilinks_to_anchors("see [[order]]"),
            "see [order](#order)"
        );
        // Alias `target|label`: link points to target, text shows label
        assert_eq!(
            translate_wikilinks_to_anchors("see [[order|the order page]]"),
            "see [the order page](#order)"
        );
        // Anchor `target#section`: link points to target only (anchor stripped)
        assert_eq!(
            translate_wikilinks_to_anchors("see [[order#step-3]]"),
            "see [order#step-3](#order)"
        );
        // No wikilinks → unchanged
        assert_eq!(
            translate_wikilinks_to_anchors("plain markdown [link](https://x)"),
            "plain markdown [link](https://x)"
        );
    }

    #[test]
    fn html_render_emits_doc_skeleton_with_toc_and_sections() {
        let pages = vec![
            page(
                "order",
                PageType::Module,
                "# Order\n\nSee [[outbox]] for details.",
            ),
            page("outbox", PageType::Concept, "# Outbox\n\nA pattern."),
        ];
        let html = render_html(&pages);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<title>Coral wiki</title>"));
        // TOC mentions both pages, grouped under their type label.
        assert!(html.contains("module (1)"));
        assert!(html.contains("concept (1)"));
        assert!(html.contains("href=\"#order\""));
        assert!(html.contains("href=\"#outbox\""));
        // Sections have anchor ids.
        assert!(html.contains("<section id=\"order\""));
        assert!(html.contains("<section id=\"outbox\""));
        // Wikilink translated to a real anchor link in rendered HTML.
        assert!(html.contains("href=\"#outbox\""));
        // The body markdown turned into HTML headings.
        assert!(html.contains("<h1>Order</h1>") || html.contains("<h1>Outbox</h1>"));
    }

    /// v0.20.1 cycle-4 audit C2 changed this test's contract: a slug
    /// with an ampersand fails `is_safe_filename_slug` and is now
    /// filtered out entirely (rather than relying on `html_escape` to
    /// neutralize it). The strict allowlist is the right primitive
    /// for HTML `id="..."` attributes, which have no escape grammar.
    #[test]
    fn html_render_filters_unsafe_slug_chars_pre_render() {
        let pages = vec![
            page("safe", PageType::Module, "body"),
            page("a&b", PageType::Module, "body"),
        ];
        let html = render_html(&pages);
        // Unsafe slug never lands in the output — neither raw nor escaped.
        assert!(!html.contains("a&b"), "raw unsafe slug must not appear");
        assert!(
            !html.contains("a&amp;b"),
            "even escaped, unsafe slugs are filtered out by C2 contract: {html}"
        );
        // Safe slug went through normally.
        assert!(html.contains("<section id=\"safe\""));
    }

    #[test]
    fn export_html_multi_writes_index_and_per_page_files() {
        // 4 seed pages across 3 types -> we should get index.html, style.css,
        // and one file per page under <type>/<slug>.html.
        let pages = vec![
            page("order", PageType::Module, "# Order\n\nbody"),
            page("invoice", PageType::Module, "# Invoice\n\nbody"),
            page("idempotency", PageType::Concept, "# Idempotency\n\nbody"),
            page("retry", PageType::Decision, "# Retry\n\nbody"),
        ];
        let tmp = tempfile::TempDir::new().unwrap();
        let out_dir = tmp.path().join("public");
        let written = render_html_multi(&pages, &out_dir).unwrap();
        // 4 pages + index.html + style.css = 6 files.
        assert_eq!(written, 6, "should write 4 pages + index + css");
        assert!(out_dir.join("index.html").exists(), "index.html missing");
        assert!(out_dir.join("style.css").exists(), "style.css missing");
        assert!(out_dir.join("module/order.html").exists());
        assert!(out_dir.join("module/invoice.html").exists());
        assert!(out_dir.join("concept/idempotency.html").exists());
        assert!(out_dir.join("decision/retry.html").exists());

        // index.html links to each page via its <type>/<slug>.html path.
        let idx = std::fs::read_to_string(out_dir.join("index.html")).unwrap();
        assert!(idx.contains("href=\"module/order.html\""));
        assert!(idx.contains("href=\"concept/idempotency.html\""));
        // Per-page files reference the shared stylesheet via ../style.css.
        let order_html = std::fs::read_to_string(out_dir.join("module/order.html")).unwrap();
        assert!(order_html.contains("href=\"../style.css\""));
        assert!(order_html.contains("href=\"../index.html\""));
    }

    /// v0.19.5 audit C5: `coral export-multi` must refuse to write a
    /// page whose frontmatter slug isn't safe for path interpolation.
    /// Without the guard, a poisoned `slug: ../../etc/passwd` would
    /// escape `out_dir`.
    #[test]
    fn export_html_multi_skips_unsafe_slugs() {
        let pages = vec![
            page("legit", PageType::Module, "# Legit"),
            page("../escape", PageType::Module, "# Evil"),
        ];
        let tmp = tempfile::TempDir::new().unwrap();
        let out_dir = tmp.path().join("public");
        let written = render_html_multi(&pages, &out_dir).unwrap();
        assert!(out_dir.join("module/legit.html").exists());
        // Nothing escaped.
        assert!(!tmp.path().join("escape.html").exists());
        assert!(!out_dir.join("module/../escape.html").exists());
        // Only the legit page (1) + index.html + style.css = 3 files.
        assert_eq!(written, 3, "unsafe slug page must be skipped");
    }

    /// v0.20.1 cycle-4 audit C2: the single-bundle HTML export
    /// (`render_html`) interpolates the slug into both `id="…"`
    /// attributes and `<h1>` text. Pre-fix, an adversarial slug
    /// like `x"><script>alert(1)</script><span x="` produced a live
    /// XSS in the exported HTML. The fix filters such slugs
    /// out via `is_safe_filename_slug` before any HTML is built.
    #[test]
    fn render_html_skips_unsafe_slug_for_xss_in_id_attribute() {
        let evil_slug = "x\"><script>alert(1)</script><span x=\"";
        let pages = vec![
            page("legit", PageType::Module, "# Legit"),
            page(evil_slug, PageType::Module, "# Evil"),
        ];
        let html = render_html(&pages);
        // Defensive: no live <script> tags in the output, period.
        assert!(
            !html.contains("<script>"),
            "render_html must not emit raw <script> tags: {html}"
        );
        assert!(
            !html.contains("alert(1)"),
            "render_html must not emit unescaped slug payload: {html}"
        );
        // The legit page still made it.
        assert!(html.contains("<section id=\"legit\""));
        // Page count in the header reflects the filter.
        assert!(
            html.contains("1 pages"),
            "count should be 1 (only safe page)"
        );
    }

    /// v0.20.1 cycle-4 audit C3: the multi-page HTML export's
    /// `index.html` (TOC) used to interpolate raw slugs into `href`
    /// attributes BEFORE the per-page slug-safety filter ran. The
    /// per-page write would skip the unsafe page, but the TOC
    /// already contained the live `<script>`. Hoisted filter ensures
    /// TOC and disk are consistent.
    #[test]
    fn render_html_multi_skips_unsafe_slug_in_toc() {
        let evil_slug = "x\"><script>alert(1)</script><span x=\"";
        let pages = vec![
            page("legit", PageType::Module, "# Legit"),
            page(evil_slug, PageType::Module, "# Evil"),
        ];
        let tmp = tempfile::TempDir::new().unwrap();
        let out_dir = tmp.path().join("public");
        let _ = render_html_multi(&pages, &out_dir).unwrap();
        let idx = std::fs::read_to_string(out_dir.join("index.html")).unwrap();
        assert!(
            !idx.contains("<script>"),
            "index.html TOC must not contain raw <script>: {idx}"
        );
        assert!(
            !idx.contains("alert(1)"),
            "index.html TOC must not contain raw alert(1): {idx}"
        );
        // The legit page still appears.
        assert!(idx.contains("href=\"module/legit.html\""));
        // Page count in the index reflects the filter (1, not 2).
        assert!(
            idx.contains("1 pages"),
            "count should be 1 (only safe page)"
        );
    }

    #[test]
    fn export_html_multi_requires_directory_out() {
        // Pre-create a regular file at the --out path: render_html_multi must
        // refuse to clobber it / write a tree underneath it.
        let pages = vec![page("a", PageType::Module, "body")];
        let tmp = tempfile::TempDir::new().unwrap();
        let bogus = tmp.path().join("file.html");
        std::fs::write(&bogus, "not a dir").unwrap();
        let err = render_html_multi(&pages, &bogus).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("directory") && msg.contains("file"),
            "error should mention directory requirement: {msg}"
        );
    }

    #[test]
    fn export_html_multi_links_resolve() {
        // A wikilink `[[outbox]]` from a module page must rewrite to
        // ../<type>/<slug>.html where the linked page lives.
        let pages = vec![
            page("order", PageType::Module, "See [[outbox]] for details."),
            page("outbox", PageType::Concept, "# Outbox\n\nA pattern."),
        ];
        let tmp = tempfile::TempDir::new().unwrap();
        let out_dir = tmp.path().join("public");
        render_html_multi(&pages, &out_dir).unwrap();

        let order_html = std::fs::read_to_string(out_dir.join("module/order.html")).unwrap();
        // outbox is a concept page, so the link target is ../concept/outbox.html.
        assert!(
            order_html.contains("href=\"../concept/outbox.html\""),
            "wikilink should rewrite to ../concept/outbox.html: {order_html}"
        );
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

    // -------------------------------------------------------------
    // v0.19.8 #30 audit-gap conversion: HTML export XSS surface.
    //
    // The audit explicitly noted "didn't construct adversarial
    // bodies for `coral export --format html`". These tests build
    // every adversarial fixture the issue body enumerated and pin
    // the v0.19.8 hardening:
    //
    //   - Raw `<script>` in body -> escaped, never reaches output
    //     as an HTML tag.
    //   - `[click me](javascript:alert(1))` -> `<a href="#">` (the
    //     unsafe scheme is stripped).
    //   - Frontmatter slug / commit interpolated into HTML attrs ->
    //     already HTML-escaped (`html_escape` call sites pinned by
    //     the existing `html_render_escapes_special_chars_in_slug_and_meta`
    //     test, plus a new fixture for breakout-attempt slugs).
    //   - `<img src=x onerror=...>` inline HTML -> escaped, attrs
    //     don't reach the parser as a tag.
    // -------------------------------------------------------------

    /// #30 (1) — A page body containing `<script>...</script>` must
    /// NOT land in the rendered HTML as a real script tag.
    #[test]
    fn xss_script_tag_in_body_is_escaped() {
        let pages = vec![page(
            "p",
            PageType::Module,
            "# P\n\n<script>document.body.innerText='owned'</script>\n",
        )];
        let html = render_html(&pages);
        assert!(
            !html.contains("<script>document.body.innerText='owned'</script>"),
            "raw <script> tag leaked: {html}"
        );
        // The escaped form should be present so the page reader still
        // sees what the wiki author wrote (as text).
        assert!(
            html.contains("&lt;script&gt;"),
            "expected <script> to be HTML-escaped: {html}"
        );
    }

    /// #30 (2) — Inline HTML attrs (`<img src=x onerror=alert(1)>`)
    /// must also be escaped, not passed through as inline tags.
    #[test]
    fn xss_inline_img_onerror_is_escaped() {
        let pages = vec![page(
            "p",
            PageType::Module,
            "para with <img src=x onerror=alert(1)> embedded",
        )];
        let html = render_html(&pages);
        assert!(
            !html.contains("<img src=x onerror=alert(1)>"),
            "inline <img onerror=...> leaked: {html}"
        );
        assert!(
            html.contains("&lt;img"),
            "inline HTML must be escaped: {html}"
        );
    }

    /// #30 (3) — A markdown link with a `javascript:` href must
    /// rewrite to a benign `#` href, not preserve the unsafe scheme.
    #[test]
    fn xss_javascript_link_scheme_is_neutralized() {
        let pages = vec![page(
            "p",
            PageType::Module,
            "# P\n\n[click me](javascript:alert(1))\n",
        )];
        let html = render_html(&pages);
        assert!(
            !html.contains("href=\"javascript:alert(1)\""),
            "javascript: href leaked: {html}"
        );
        // Implementation detail: pulldown-cmark may percent-encode
        // characters in the rewritten href, but the SCHEME `javascript:`
        // must not appear anywhere in any href.
        assert!(
            !html.to_lowercase().contains("href=\"javascript:"),
            "any javascript:-prefixed href is unsafe: {html}"
        );
    }

    /// #30 — same hardening for `data:` (commonly used to embed
    /// inline payloads) and `vbscript:`.
    #[test]
    fn xss_other_unsafe_schemes_are_neutralized() {
        for unsafe_url in [
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
            "file:///etc/passwd",
            // Case-insensitive check.
            "JavaScript:alert(1)",
            // Whitespace-prefix bypass.
            "  javascript:alert(1)",
        ] {
            let pages = vec![page("p", PageType::Module, &format!("[c]({unsafe_url})"))];
            let html = render_html(&pages);
            // Lowercase scheme should not appear anywhere in any href.
            let lc = html.to_lowercase();
            assert!(
                !lc.contains("href=\"javascript:")
                    && !lc.contains("href=\"data:")
                    && !lc.contains("href=\"vbscript:")
                    && !lc.contains("href=\"file:"),
                "unsafe scheme {unsafe_url:?} leaked: {html}"
            );
        }
    }

    /// #30 (4) — A wikilink with a `javascript:`-like target lands
    /// in the markdown as `[label](#javascript:...)`. The leading `#`
    /// is a fragment, NOT a scheme — so this is benign by virtue of
    /// the wikilink translator's contract. Pinning it here so a
    /// future translator change can't regress.
    #[test]
    fn xss_wikilink_with_jsalert_target_renders_as_fragment_only() {
        let pages = vec![page(
            "p",
            PageType::Module,
            "# P\n\n[[javascript:alert(1)]]\n",
        )];
        let html = render_html(&pages);
        // The target becomes a `#javascript:alert(1)` fragment URL.
        // Browsers don't interpret fragment URLs as scripts, but
        // belt-and-braces: any `href="javascript:...` (without the `#`
        // prefix) IS bad. Confirm only `#javascript:...` form appears.
        let lc = html.to_lowercase();
        assert!(
            !lc.contains(r#"href="javascript:"#),
            "wikilink leaked unsafe absolute javascript: href: {html}"
        );
    }

    /// #30 (5) — Frontmatter values that try to break out of
    /// HTML attribute context are escaped at every interpolation
    /// site. The audit specifically called out the title field.
    /// Coral's HTML export interpolates slug + commit + status +
    /// type into the meta line; everything passes through
    /// `html_escape` before reaching the template.
    #[test]
    fn xss_frontmatter_breakout_attempt_is_escaped_in_attrs() {
        // We can't easily set last_updated_commit to a payload via
        // `page()` — but the existing `html_render_escapes_special_chars_in_slug_and_meta`
        // test pins the slug path. Add a parallel test for commit.
        let mut p = page("breakout-attempt", PageType::Module, "# normal body");
        p.frontmatter.last_updated_commit = r#""><script>alert(1)</script>"#.to_string();
        let html = render_html(&[p]);
        assert!(
            !html.contains(r#""><script>alert(1)</script>"#),
            "frontmatter commit field leaked breakout payload: {html}"
        );
        // The escaped form must be present.
        assert!(
            html.contains("&quot;&gt;&lt;script&gt;"),
            "expected escaped breakout payload: {html}"
        );
    }

    /// #30 multi-export equivalent of the body-script test. The
    /// adversarial body must NOT render as a real script tag in any
    /// of the per-page files.
    #[test]
    fn xss_script_tag_in_body_is_escaped_in_multi_export() {
        let pages = vec![page(
            "p",
            PageType::Module,
            "# P\n\n<script>alert(1)</script>\n",
        )];
        let tmp = tempfile::TempDir::new().unwrap();
        let out_dir = tmp.path().join("public");
        render_html_multi(&pages, &out_dir).unwrap();
        let body = std::fs::read_to_string(out_dir.join("module/p.html")).unwrap();
        assert!(
            !body.contains("<script>alert(1)</script>"),
            "raw <script> leaked into multi-export: {body}"
        );
        assert!(body.contains("&lt;script&gt;"));
    }

    /// #30 sanity: legitimate inline HTML constructs are also
    /// escaped (they were never rendered safely anyway, but this
    /// pins the new contract). A future "rich HTML" feature would
    /// need a real sanitizer.
    #[test]
    fn xss_legitimate_inline_html_is_also_escaped_under_new_contract() {
        let pages = vec![page("p", PageType::Module, "# P\n\n<em>emph</em>\n")];
        let html = render_html(&pages);
        assert!(
            !html.contains("<em>emph</em>"),
            "inline <em> would leak under the old policy; new policy escapes it: {html}"
        );
        assert!(html.contains("&lt;em&gt;"));
    }

    /// #30 helper: `is_unsafe_url_scheme` direct unit checks. Pinning
    /// the matrix so a future relaxation can't silently regress.
    #[test]
    fn is_unsafe_url_scheme_matrix() {
        for bad in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "  javascript:alert(1)",
            "data:text/html,<x>",
            "vbscript:msgbox",
            "file:///etc/passwd",
            "JaVaScRiPt:foo",
        ] {
            assert!(is_unsafe_url_scheme(bad), "should reject {bad:?}");
        }
        for ok in [
            "https://example.com",
            "http://example.com",
            "mailto:a@b.c",
            "tel:+15551234",
            "../foo/bar.html",
            "/abs/path",
            "#fragment",
            "?query=1",
            "",
        ] {
            assert!(!is_unsafe_url_scheme(ok), "should accept {ok:?}");
        }
    }
}
