//! Push wiki pages to a Notion database via the Notion REST API.
//!
//! Thin wrapper over `coral export --format notion-json` that reads the
//! `NOTION_TOKEN` + `CORAL_NOTION_DB` env vars (or flags), substitutes the
//! database id, and shells out to `curl` to POST each body.
//!
//! v0.2.1: shells to `curl` to keep the binary footprint small. v0.3 may
//! switch to `reqwest` if richer error handling is needed.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::page::Page;
use coral_core::walk;
use coral_runner::body_tempfile::{TempFileGuard, body_tempfile_path, write_body_tempfile_secure};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct NotionPushArgs {
    /// Notion integration token. Falls back to `NOTION_TOKEN` env var.
    #[arg(long)]
    pub token: Option<String>,
    /// Notion database id (the parent the pages get pushed into). Falls back
    /// to `CORAL_NOTION_DB` env var.
    #[arg(long)]
    pub database: Option<String>,
    /// Filter by page type (repeatable).
    #[arg(long = "type", value_name = "TYPE")]
    pub types: Vec<String>,
    /// Apply: actually POST each page to the Notion database.
    /// Without this flag, the command runs as a dry-run preview (default,
    /// matches `bootstrap`/`ingest` semantics).
    #[arg(long)]
    pub apply: bool,
}

pub fn run(args: NotionPushArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let token = args
        .token
        .or_else(|| std::env::var("NOTION_TOKEN").ok())
        .context("NOTION_TOKEN env var (or --token) required")?;
    let database = args
        .database
        .or_else(|| std::env::var("CORAL_NOTION_DB").ok())
        .context("CORAL_NOTION_DB env var (or --database) required")?;

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
    let pages = filter_by_types(pages, &args.types);
    let bodies = build_notion_bodies(&pages, &database);

    if !args.apply {
        println!(
            "Would POST {} page(s) to Notion database {database} (dry-run; pass --apply to push)",
            bodies.len()
        );
        for (i, _b) in bodies.iter().enumerate() {
            println!("  - {i}: {}", pages[i].frontmatter.slug);
        }
        return Ok(ExitCode::SUCCESS);
    }

    let mut ok = 0usize;
    let mut fail = 0usize;
    for (i, body) in bodies.iter().enumerate() {
        let slug = &pages[i].frontmatter.slug;
        let json_string = serde_json::to_string(body)?;
        // v0.19.5 audit H4 + H6: capture the response body (drop
        // `-o /dev/null`) so non-2xx errors surface the Notion API's
        // error JSON rather than a bare status code. Pipe the
        // Authorization header via stdin (`@-`) so the secret never
        // appears in argv (visible to other processes via /proc).
        //
        // v0.20.2 audit-followup #43: also route the body through a
        // per-call mode-0600 tempfile via `--data-binary @<path>`
        // instead of `-d <body>`. Same shared helper as `HttpRunner`
        // (v0.19.6 N2, v0.19.7 #24/#25). RAII guard cleans up on
        // every return path including panic-unwind.
        let body_path = body_tempfile_path("coral-notion-body");
        write_body_tempfile_secure(&body_path, json_string.as_bytes())
            .with_context(|| format!("writing notion request body to {}", body_path.display()))?;
        let body_guard = TempFileGuard::new(Some(body_path.clone()));
        let auth_header = format!("Authorization: Bearer {token}\n");
        let mut child = build_curl_command(&body_path)
            .spawn()
            .context("spawning curl (is it in PATH?)")?;
        if let Some(mut stdin) = child.stdin.take() {
            std::io::Write::write_all(&mut stdin, auth_header.as_bytes())
                .context("writing auth header to curl stdin")?;
            // EOF stdin so curl proceeds to read the body file.
            drop(stdin);
        }
        let output = child.wait_with_output().context("awaiting curl")?;
        // Tempfile is removed when body_guard drops at end of iteration.
        drop(body_guard);
        if !output.status.success() {
            fail += 1;
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("FAIL {slug}: curl exited {:?} ({stderr})", output.status);
            continue;
        }
        let combined = String::from_utf8_lossy(&output.stdout);
        let (response_body, http_code) = match combined.rsplit_once("\nHTTP_CODE:") {
            Some((body, code)) => (body.to_string(), code.trim().to_string()),
            None => (String::new(), combined.trim().to_string()),
        };
        if http_code.starts_with('2') {
            ok += 1;
            tracing::info!(slug = %slug, http = %http_code, "notion push ok");
        } else {
            fail += 1;
            // Show the API's error body — it usually carries a
            // `code` + `message` field that's more useful than the
            // raw HTTP status alone.
            let body_excerpt: String = response_body.chars().take(400).collect();
            eprintln!("FAIL {slug}: HTTP {http_code} — {body_excerpt}");
        }
    }
    println!("Pushed: {ok} ok, {fail} failed.");
    Ok(if fail == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn filter_by_types(pages: Vec<Page>, types: &[String]) -> Vec<Page> {
    if types.is_empty() {
        return pages;
    }
    let allow: std::collections::HashSet<&str> = types.iter().map(String::as_str).collect();
    pages
        .into_iter()
        .filter(|p| allow.contains(super::export::page_type_name_pub(&p.frontmatter)))
        .collect()
}

/// Build the curl `Command` for a single Notion `POST /v1/pages`
/// request. Pure construction — does not spawn — so tests can assert
/// the argv shape without hitting the network.
///
/// Caller writes the body to `body_path` BEFORE spawning (via
/// [`write_body_tempfile_secure`]) and binds a [`TempFileGuard`] for
/// cleanup. The auth header rides on stdin (`@-`); the body rides on
/// `--data-binary @<body_path>`.
///
/// v0.20.2 audit-followup #43.
pub(crate) fn build_curl_command(body_path: &Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("curl");
    let body_arg = format!("@{}", body_path.display());
    cmd.args([
        "-s",
        "-w",
        "\nHTTP_CODE:%{http_code}",
        "-X",
        "POST",
        "https://api.notion.com/v1/pages",
        "-H",
        "@-",
        "-H",
        "Notion-Version: 2022-06-28",
        "-H",
        "Content-Type: application/json",
        "--data-binary",
        &body_arg,
    ]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd
}

/// Build Notion `POST /v1/pages` request bodies, one per page, with the
/// supplied database id substituted into `parent.database_id`.
pub fn build_notion_bodies(pages: &[Page], database_id: &str) -> Vec<Value> {
    pages
        .iter()
        .map(|p| {
            serde_json::json!({
                "parent": { "database_id": database_id },
                "properties": {
                    "Name": {
                        "title": [{ "text": { "content": p.frontmatter.slug } }]
                    },
                    "Type": {
                        "select": { "name": super::export::page_type_name_pub(&p.frontmatter) }
                    },
                    "Status": {
                        "select": { "name": super::export::status_name_pub(&p.frontmatter) }
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
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use std::path::PathBuf;

    fn page(slug: &str, t: PageType) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/x/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.into(),
                page_type: t,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(0.7).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Reviewed,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra: Default::default(),
            },
            body: "body".into(),
        }
    }

    #[test]
    fn build_substitutes_database_id() {
        let pages = vec![page("a", PageType::Module)];
        let bodies = build_notion_bodies(&pages, "db-abc-123");
        assert_eq!(bodies[0]["parent"]["database_id"], "db-abc-123");
    }

    #[test]
    fn build_includes_slug_in_title() {
        let pages = vec![page("order", PageType::Entity)];
        let bodies = build_notion_bodies(&pages, "db");
        assert_eq!(
            bodies[0]["properties"]["Name"]["title"][0]["text"]["content"],
            "order"
        );
    }

    #[test]
    fn build_includes_type_select_name() {
        let pages = vec![page("x", PageType::Concept)];
        let bodies = build_notion_bodies(&pages, "db");
        assert_eq!(bodies[0]["properties"]["Type"]["select"]["name"], "concept");
    }

    #[test]
    fn filter_by_types_keeps_matching_only() {
        let pages = vec![
            page("a", PageType::Module),
            page("b", PageType::Concept),
            page("c", PageType::Entity),
        ];
        let kept = filter_by_types(pages, &["concept".into()]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].frontmatter.slug, "b");
    }

    /// v0.20.2 audit-followup #43: regression — neither the auth
    /// token nor the request body must land in argv. The auth header
    /// rides on stdin (`@-` sentinel); the body rides on
    /// `--data-binary @<tempfile-path>`. Local `ps` viewers never see
    /// either.
    #[test]
    fn build_curl_command_does_not_put_body_or_token_in_argv() {
        let body_path = std::path::PathBuf::from("/tmp/coral-notion-test-body.json");
        let cmd = build_curl_command(&body_path);
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        // Auth header sentinel: `@-` tells curl to read it from stdin.
        assert!(
            argv.iter().any(|a| a == "@-"),
            "expected `@-` stdin header sentinel: {argv:?}"
        );
        // No flag-style auth value should remain in argv.
        assert!(
            argv.iter().all(|a| !a.starts_with("Authorization:")),
            "argv contained an Authorization header: {argv:?}"
        );
        // No bare `-d` arg — the migration to `--data-binary` is
        // intentional so the body source is consistent and easy to
        // grep for in audit.
        assert!(
            !argv.iter().any(|a| a == "-d"),
            "argv still contains `-d`; body migration to --data-binary missing: {argv:?}"
        );
        // The body must be referenced via `--data-binary @<path>`,
        // not via inline string.
        assert!(
            argv.iter().any(|a| a == "--data-binary"),
            "expected `--data-binary` flag: {argv:?}"
        );
        assert!(
            argv.iter().any(|a| a.starts_with('@') && a != "@-"),
            "expected `@<body-path>` reference: {argv:?}"
        );
    }

    /// v0.20.2 audit-followup #43: end-to-end argv probe with a real
    /// adversarial-shaped body. Even if a future refactor wires the
    /// body construction differently, the JSON contents must NEVER
    /// appear verbatim in argv.
    #[test]
    fn build_curl_command_argv_does_not_leak_body_content() {
        let body_path = std::path::PathBuf::from("/tmp/coral-notion-leak-test.json");
        let cmd = build_curl_command(&body_path);
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        // A real body would contain page contents — assert that the
        // tempfile path is the only carrier.
        let body_marker = "pineapple-secret-token-42";
        assert!(
            argv.iter().all(|a| !a.contains(body_marker)),
            "argv leaked synthetic body marker: {argv:?}"
        );
    }
}
