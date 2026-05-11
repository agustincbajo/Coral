//! `coral wiki serve` — local HTTP wiki browser (M3.9).
//!
//! Starts a tiny_http server that serves:
//! - `GET /`         → index page listing all wiki pages with links
//! - `GET /page/<slug>` → rendered page (preformatted markdown)
//! - `GET /graph`    → Mermaid graph of page relationships (wikilinks)
//! - `GET /health`   → `{"status": "ok"}`
//!
//! Graceful shutdown on SIGINT/SIGTERM via signal-hook (same pattern as
//! `coral monitor up`).

use anyhow::{Context, Result};
use clap::Args;
use std::net::SocketAddr;
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use coral_core::page::Page;
use coral_core::walk::read_pages;

#[derive(Args, Debug, Clone)]
pub struct ServeArgs {
    /// Port to listen on.
    #[arg(long, default_value = "3838")]
    pub port: u16,

    /// Address to bind to.
    #[arg(long, default_value = "127.0.0.1")]
    pub bind: String,
}

pub fn run(args: ServeArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let wiki_dir = wiki_root
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(".wiki"));

    if !wiki_dir.exists() {
        anyhow::bail!(
            "wiki directory '{}' does not exist; run `coral init` first",
            wiki_dir.display()
        );
    }

    let pages = read_pages(&wiki_dir).context("failed to read wiki pages")?;
    eprintln!(
        "coral wiki serve: loaded {} pages from {}",
        pages.len(),
        wiki_dir.display()
    );

    let addr: SocketAddr = format!("{}:{}", args.bind, args.port)
        .parse()
        .with_context(|| format!("invalid bind address: {}:{}", args.bind, args.port))?;

    let server = tiny_http::Server::http(addr)
        .map_err(|e| anyhow::anyhow!("failed to start HTTP server on {}: {}", addr, e))?;

    eprintln!("listening on http://{}", addr);

    // Install shutdown handler (SIGINT/SIGTERM).
    let shutdown = Arc::new(AtomicBool::new(false));
    install_shutdown_handler(shutdown.clone())?;

    // Serve loop — check shutdown flag after each request (with 250ms
    // timeout on recv so Ctrl-C latency is bounded).
    loop {
        if shutdown.load(Ordering::Relaxed) {
            eprintln!("\nshutting down...");
            break;
        }

        // Use recv_timeout so the shutdown flag gets checked periodically.
        let request = match server.recv_timeout(std::time::Duration::from_millis(250)) {
            Ok(Some(req)) => req,
            Ok(None) => continue,    // timeout, re-check shutdown
            Err(_) => continue,      // transient error, keep going
        };

        handle_request(request, &pages);
    }

    Ok(ExitCode::SUCCESS)
}

fn handle_request(request: tiny_http::Request, pages: &[Page]) {
    let url = request.url().to_string();
    let response = route(&url, pages);

    let header = tiny_http::Header::from_bytes(
        b"Content-Type" as &[u8],
        response.content_type.as_bytes(),
    )
    .expect("valid header");

    let resp = tiny_http::Response::from_string(response.body)
        .with_status_code(response.status)
        .with_header(header);

    let _ = request.respond(resp);
}

struct HttpResponse {
    status: u16,
    content_type: String,
    body: String,
}

fn route(url: &str, pages: &[Page]) -> HttpResponse {
    // Strip query string if present.
    let path = url.split('?').next().unwrap_or(url);

    match path {
        "/" => index_page(pages),
        "/health" => health_page(),
        "/graph" => graph_page(pages),
        p if p.starts_with("/page/") => {
            let slug = &p[6..]; // strip "/page/"
            page_view(slug, pages)
        }
        _ => HttpResponse {
            status: 404,
            content_type: "text/html; charset=utf-8".into(),
            body: "<h1>404 — Not Found</h1><p><a href=\"/\">Back to index</a></p>"
                .into(),
        },
    }
}

fn health_page() -> HttpResponse {
    HttpResponse {
        status: 200,
        content_type: "application/json".into(),
        body: r#"{"status": "ok"}"#.into(),
    }
}

pub fn render_index(pages: &[Page]) -> String {
    let mut html = String::from(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Coral Wiki</title>
<style>
body { font-family: system-ui, sans-serif; max-width: 800px; margin: 2rem auto; padding: 0 1rem; }
a { color: #0066cc; }
h1 { border-bottom: 2px solid #333; padding-bottom: 0.5rem; }
ul { list-style: none; padding: 0; }
li { padding: 0.3rem 0; border-bottom: 1px solid #eee; }
nav { margin-bottom: 1rem; }
nav a { margin-right: 1rem; }
</style>
</head><body>
<h1>Coral Wiki</h1>
<nav><a href="/">Index</a><a href="/graph">Graph</a></nav>
<ul>
"#,
    );

    let mut sorted_pages: Vec<&Page> = pages.iter().collect();
    sorted_pages.sort_by(|a, b| a.frontmatter.slug.cmp(&b.frontmatter.slug));

    for page in &sorted_pages {
        let slug = &page.frontmatter.slug;
        html.push_str(&format!(
            "<li><a href=\"/page/{}\">{}</a></li>\n",
            slug, slug
        ));
    }

    html.push_str("</ul>\n</body></html>");
    html
}

fn index_page(pages: &[Page]) -> HttpResponse {
    HttpResponse {
        status: 200,
        content_type: "text/html; charset=utf-8".into(),
        body: render_index(pages),
    }
}

pub fn render_page_view(slug: &str, pages: &[Page]) -> Option<String> {
    let page = pages.iter().find(|p| p.frontmatter.slug == slug)?;

    let escaped_body = page
        .body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    Some(format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{slug} — Coral Wiki</title>
<style>
body {{ font-family: system-ui, sans-serif; max-width: 800px; margin: 2rem auto; padding: 0 1rem; }}
a {{ color: #0066cc; }}
h1 {{ border-bottom: 2px solid #333; padding-bottom: 0.5rem; }}
pre {{ background: #f5f5f5; padding: 1rem; overflow-x: auto; white-space: pre-wrap; word-wrap: break-word; }}
nav {{ margin-bottom: 1rem; }}
nav a {{ margin-right: 1rem; }}
</style>
</head><body>
<nav><a href="/">Index</a><a href="/graph">Graph</a></nav>
<h1>{slug}</h1>
<pre>{escaped_body}</pre>
</body></html>"#,
        slug = slug,
        escaped_body = escaped_body,
    ))
}

fn page_view(slug: &str, pages: &[Page]) -> HttpResponse {
    match render_page_view(slug, pages) {
        Some(html) => HttpResponse {
            status: 200,
            content_type: "text/html; charset=utf-8".into(),
            body: html,
        },
        None => HttpResponse {
            status: 404,
            content_type: "text/html; charset=utf-8".into(),
            body: format!(
                "<h1>404 — Page not found: {}</h1><p><a href=\"/\">Back to index</a></p>",
                slug
            ),
        },
    }
}

/// Generate a Mermaid graph definition from page wikilinks.
pub fn render_mermaid_graph(pages: &[Page]) -> String {
    let mut lines = Vec::new();
    lines.push("graph LR".to_string());

    for page in pages {
        let from = &page.frontmatter.slug;
        let links = page.outbound_links();
        if links.is_empty() {
            // Isolated node — still show it.
            lines.push(format!("    {}", mermaid_id(from)));
        } else {
            for target in &links {
                lines.push(format!(
                    "    {} --> {}",
                    mermaid_id(from),
                    mermaid_id(target)
                ));
            }
        }
    }

    lines.join("\n")
}

/// Sanitize a slug for Mermaid node IDs. Mermaid IDs can't contain
/// spaces or special chars, so we replace non-alphanumeric with `_`.
fn mermaid_id(slug: &str) -> String {
    slug.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

fn graph_page(pages: &[Page]) -> HttpResponse {
    let mermaid_def = render_mermaid_graph(pages);

    let html = format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Wiki Graph — Coral Wiki</title>
<style>
body {{ font-family: system-ui, sans-serif; max-width: 1200px; margin: 2rem auto; padding: 0 1rem; }}
a {{ color: #0066cc; }}
h1 {{ border-bottom: 2px solid #333; padding-bottom: 0.5rem; }}
nav {{ margin-bottom: 1rem; }}
nav a {{ margin-right: 1rem; }}
.mermaid {{ background: #fafafa; padding: 1rem; border: 1px solid #ddd; border-radius: 4px; }}
</style>
<script src="https://cdn.jsdelivr.net/npm/mermaid@10/dist/mermaid.min.js"></script>
<script>mermaid.initialize({{ startOnLoad: true }});</script>
</head><body>
<nav><a href="/">Index</a><a href="/graph">Graph</a></nav>
<h1>Wiki Graph</h1>
<div class="mermaid">
{mermaid_def}
</div>
</body></html>"#,
        mermaid_def = mermaid_def
    );

    HttpResponse {
        status: 200,
        content_type: "text/html; charset=utf-8".into(),
        body: html,
    }
}

fn install_shutdown_handler(flag: Arc<AtomicBool>) -> Result<()> {
    use signal_hook::consts::{SIGINT, SIGTERM};
    signal_hook::flag::register(SIGINT, flag.clone())
        .map_err(|e| anyhow::anyhow!("failed to register SIGINT handler: {e}"))?;
    signal_hook::flag::register(SIGTERM, flag)
        .map_err(|e| anyhow::anyhow!("failed to register SIGTERM handler: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use coral_core::page::Page;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn make_page(slug: &str, body: &str) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/{}.md", slug)),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type: PageType::Module,
                last_updated_commit: "abc1234".to_string(),
                confidence: Confidence::try_new(0.8).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Draft,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra: BTreeMap::new(),
            },
            body: body.to_string(),
        }
    }

    #[test]
    fn serve_arg_defaults() {
        // Verify default port and bind via clap parsing.
        use clap::Parser;

        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            serve: ServeArgs,
        }

        let w = Wrapper::parse_from(["test"]);
        assert_eq!(w.serve.port, 3838);
        assert_eq!(w.serve.bind, "127.0.0.1");

        let w = Wrapper::parse_from(["test", "--port", "9090", "--bind", "0.0.0.0"]);
        assert_eq!(w.serve.port, 9090);
        assert_eq!(w.serve.bind, "0.0.0.0");
    }

    #[test]
    fn index_page_lists_all_pages() {
        let pages = vec![
            make_page("auth-service", "auth stuff"),
            make_page("api-gateway", "gateway"),
        ];
        let html = render_index(&pages);
        assert!(html.contains("/page/api-gateway"), "missing api-gateway link");
        assert!(html.contains("/page/auth-service"), "missing auth-service link");
        // Pages should be sorted alphabetically.
        let pos_api = html.find("api-gateway").unwrap();
        let pos_auth = html.find("auth-service").unwrap();
        assert!(pos_api < pos_auth, "pages not sorted alphabetically");
    }

    #[test]
    fn page_route_renders_correct_slug() {
        let pages = vec![
            make_page("auth-service", "# Auth\nHandles [[tokens]]"),
            make_page("tokens", "token details"),
        ];
        let html = render_page_view("auth-service", &pages).expect("page should exist");
        assert!(html.contains("auth-service"), "slug not in title");
        assert!(html.contains("# Auth"), "body not rendered");
        assert!(html.contains("[[tokens]]"), "wikilinks preserved in pre");
        // Nonexistent slug returns None.
        assert!(render_page_view("nonexistent", &pages).is_none());
    }

    #[test]
    fn graph_produces_valid_mermaid_syntax() {
        let pages = vec![
            make_page("auth-service", "depends on [[tokens]] and [[users]]"),
            make_page("tokens", "a token page"),
            make_page("users", "links to [[auth-service]]"),
        ];
        let graph = render_mermaid_graph(&pages);
        assert!(graph.starts_with("graph LR"), "must start with graph directive");
        assert!(graph.contains("auth_service --> tokens"), "missing auth->tokens edge");
        assert!(graph.contains("auth_service --> users"), "missing auth->users edge");
        assert!(graph.contains("users --> auth_service"), "missing users->auth edge");
        // Isolated node (tokens has no outbound links).
        assert!(graph.contains("tokens"), "isolated node missing");
    }

    #[test]
    fn health_endpoint_returns_ok_json() {
        let resp = health_page();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.content_type, "application/json");
        let v: serde_json::Value = serde_json::from_str(&resp.body).expect("valid JSON");
        assert_eq!(v["status"], "ok");
    }

    #[test]
    fn route_dispatches_correctly() {
        let pages = vec![make_page("hello", "world")];
        let resp = route("/", &pages);
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("hello"));

        let resp = route("/health", &pages);
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("ok"));

        let resp = route("/page/hello", &pages);
        assert_eq!(resp.status, 200);
        assert!(resp.body.contains("world"));

        let resp = route("/page/missing", &pages);
        assert_eq!(resp.status, 404);

        let resp = route("/nonexistent", &pages);
        assert_eq!(resp.status, 404);
    }
}
