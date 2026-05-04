//! `coral mcp serve [--transport stdio] [--read-only]`
//!
//! Exposes the wiki + manifest as a Model Context Protocol server.
//! v0.19 wave 2 ships stdio only; HTTP/SSE follows in v0.19.x once the
//! security model (audit log + rate limit) is pinned.
//!
//! v0.19.5 audit C1 wired the real `ToolDispatcher` and resource
//! `read()` paths — wave 1 had stub implementations that always
//! returned None, so MCP clients couldn't actually consume anything.

use anyhow::Result;
use clap::{Args, Subcommand};
use coral_core::{search, walk};
use coral_mcp::{
    McpHandler, ResourceProvider, ServerConfig, ToolCallResult, ToolDispatcher, Transport,
    WikiResourceProvider,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

#[derive(Args, Debug)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: McpCmd,
}

#[derive(Subcommand, Debug)]
pub enum McpCmd {
    /// Serve the MCP protocol over stdin/stdout.
    Serve(ServeArgs),
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Transport. v0.19 only supports `stdio`.
    #[arg(long, default_value = "stdio")]
    pub transport: TransportArg,

    /// Default-deny: write-tools (`up`, `down`, `run_test`) are
    /// disabled unless `--allow-write-tools` is also passed.
    ///
    /// v0.19.5 audit H3: previous `ArgAction::SetTrue` made
    /// `--read-only false` an error. Switched to explicit
    /// `default_value_t = true` + plain bool so `--read-only false`
    /// parses (clap derives `clap::Action::Set` for bool by default).
    #[arg(long, default_value_t = true, num_args = 0..=1, default_missing_value = "true", action = clap::ArgAction::Set)]
    pub read_only: bool,

    /// Enable write tools. Mutually exclusive with `--read-only true`
    /// (clap's `default_value_t = true` means `--read-only false` is
    /// the explicit opt-out).
    #[arg(long)]
    pub allow_write_tools: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum TransportArg {
    Stdio,
}

pub fn run(args: McpArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        McpCmd::Serve(a) => serve(a),
    }
}

fn serve(args: ServeArgs) -> Result<ExitCode> {
    let cwd = std::env::current_dir()?;
    let resources: Arc<dyn ResourceProvider> = Arc::new(WikiResourceProvider::new(cwd.clone()));
    let tools: Arc<dyn ToolDispatcher> = Arc::new(CoralToolDispatcher::new(cwd));
    let read_only = args.read_only && !args.allow_write_tools;
    let config = ServerConfig {
        transport: match args.transport {
            TransportArg::Stdio => Transport::Stdio,
        },
        read_only,
        port: None,
    };
    let handler = McpHandler::new(config, resources, tools);
    eprintln!(
        "coral mcp serve — transport={:?}, read_only={}",
        match args.transport {
            TransportArg::Stdio => "stdio",
        },
        read_only
    );
    handler
        .serve_stdio()
        .map_err(|e| anyhow::anyhow!("MCP stdio loop failed: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

#[allow(dead_code)]
fn _ensure_tool_call_result_used(_t: ToolCallResult) {}

/// Real dispatcher backed by the same library APIs the CLI uses.
///
/// v0.19.5 audit C1: replaces `NoOpDispatcher` so MCP clients get real
/// answers from `tools/call`. Each handler is intentionally simple —
/// load pages from `<cwd>/.wiki`, run the existing core helper,
/// serialize. Write tools (`run_test` / `up` / `down`) are gated by
/// the server's `read_only` flag at the McpHandler layer; we still
/// return `Skip` for them here so a misconfigured handler doesn't
/// silently turn into a remote-exec foothold.
pub struct CoralToolDispatcher {
    project_root: PathBuf,
}

impl CoralToolDispatcher {
    pub fn new(project_root: PathBuf) -> Self {
        Self { project_root }
    }

    fn wiki_root(&self) -> PathBuf {
        self.project_root.join(".wiki")
    }

    fn read_pages(&self) -> Vec<coral_core::page::Page> {
        let root = self.wiki_root();
        if !root.exists() {
            return Vec::new();
        }
        walk::read_pages(&root).unwrap_or_default()
    }

    fn append_audit(&self, tool: &str, args: &serde_json::Value, summary: &str) {
        // v0.19.5 audit M5: per-tool audit trail. Best-effort append;
        // the .coral dir might not exist yet (legacy projects), in
        // which case we skip silently rather than break the call.
        let dir = self.project_root.join(".coral");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let path = dir.join("audit.log");
        let entry = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "tool": tool,
            "args": args,
            "result_summary": summary,
        });
        let line = format!("{entry}\n");
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    }
}

impl ToolDispatcher for CoralToolDispatcher {
    fn call(&self, name: &str, args: &serde_json::Value) -> ToolCallResult {
        let result = match name {
            "search" => self.tool_search(args),
            "find_backlinks" => self.tool_find_backlinks(args),
            "affected_repos" => self.tool_affected_repos(args),
            "verify" => self.tool_verify(args),
            "query" => {
                // `query` requires an LLM provider key — defer to the
                // CLI rather than reach out from inside the MCP loop
                // (no streaming, no key handling here).
                ToolCallResult::Skip {
                    reason: "use the CLI `coral query` for LLM-augmented queries; MCP `query` is not wired in this build".into(),
                }
            }
            "run_test" | "up" | "down" => ToolCallResult::Skip {
                reason: format!(
                    "tool '{name}' requires --allow-write-tools and is not implemented over MCP yet"
                ),
            },
            other => ToolCallResult::Error {
                message: format!("unknown tool: {other}"),
            },
        };
        let summary = match &result {
            ToolCallResult::Ok(_) => "ok",
            ToolCallResult::Skip { .. } => "skip",
            ToolCallResult::Error { .. } => "error",
        };
        self.append_audit(name, args, summary);
        result
    }
}

impl CoralToolDispatcher {
    fn tool_search(&self, args: &serde_json::Value) -> ToolCallResult {
        let q = match args.get("q").and_then(|v| v.as_str()) {
            Some(q) => q,
            None => {
                return ToolCallResult::Error {
                    message: "missing required arg 'q'".into(),
                };
            }
        };
        let pages = self.read_pages();
        let hits = search::search(&pages, q, 10);
        let json: Vec<serde_json::Value> = hits
            .into_iter()
            .map(|h| {
                serde_json::json!({
                    "slug": h.slug,
                    "score": h.score,
                })
            })
            .collect();
        ToolCallResult::Ok(serde_json::json!({ "hits": json }))
    }

    fn tool_find_backlinks(&self, args: &serde_json::Value) -> ToolCallResult {
        let slug = match args.get("slug").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolCallResult::Error {
                    message: "missing required arg 'slug'".into(),
                };
            }
        };
        let pages = self.read_pages();
        let backlinks: Vec<String> = pages
            .iter()
            .filter(|p| p.outbound_links().iter().any(|l| l == slug))
            .map(|p| p.frontmatter.slug.clone())
            .collect();
        ToolCallResult::Ok(serde_json::json!({ "backlinks": backlinks }))
    }

    fn tool_affected_repos(&self, args: &serde_json::Value) -> ToolCallResult {
        let _since = args.get("since").and_then(|v| v.as_str()).unwrap_or("");
        // Reading the manifest gives us the repo list; without a
        // running git context here we can only return the configured
        // names — the CLI does the real --since walk. Return a thin
        // listing so agents at least see what's in the project.
        let manifest = self.project_root.join("coral.toml");
        let project = if manifest.exists() {
            match coral_core::project::Project::load_from_manifest(&manifest) {
                Ok(p) => p,
                Err(e) => {
                    return ToolCallResult::Error {
                        message: format!("manifest parse failed: {e}"),
                    };
                }
            }
        } else {
            coral_core::project::Project::synthesize_legacy(&self.project_root)
        };
        let repos: Vec<&str> = project.repos.iter().map(|r| r.name.as_str()).collect();
        ToolCallResult::Ok(serde_json::json!({ "repos": repos }))
    }

    fn tool_verify(&self, _args: &serde_json::Value) -> ToolCallResult {
        // Verify against running services touches the env layer; we
        // don't want to import `coral-env` from here for now. Surface
        // a useful Skip so the agent knows to fall back.
        ToolCallResult::Skip {
            reason: "verify is wired via the CLI; run `coral verify` in a shell".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_project(dir: &Path) {
        fs::create_dir_all(dir.join(".wiki/modules")).unwrap();
        fs::write(
            dir.join(".wiki/modules/order.md"),
            "---\nslug: order\ntype: module\nlast_updated_commit: abc\nconfidence: 0.6\nstatus: draft\n---\n\n# Order\n\nbody [[invoice]]\n",
        )
        .unwrap();
        fs::write(
            dir.join(".wiki/modules/invoice.md"),
            "---\nslug: invoice\ntype: module\nlast_updated_commit: abc\nconfidence: 0.6\nstatus: draft\n---\n\n# Invoice\n\nbody\n",
        )
        .unwrap();
    }

    #[test]
    fn search_returns_hits() {
        let dir = TempDir::new().unwrap();
        make_project(dir.path());
        let d = CoralToolDispatcher::new(dir.path().to_path_buf());
        let r = d.call("search", &serde_json::json!({"q": "Order"}));
        match r {
            ToolCallResult::Ok(v) => {
                let hits = v["hits"].as_array().unwrap();
                assert!(!hits.is_empty(), "expected hits");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn find_backlinks_returns_pages_linking_to_slug() {
        let dir = TempDir::new().unwrap();
        make_project(dir.path());
        let d = CoralToolDispatcher::new(dir.path().to_path_buf());
        let r = d.call("find_backlinks", &serde_json::json!({"slug": "invoice"}));
        match r {
            ToolCallResult::Ok(v) => {
                let bls = v["backlinks"].as_array().unwrap();
                assert!(bls.iter().any(|b| b == "order"), "got {:?}", bls);
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn unknown_tool_returns_error() {
        let dir = TempDir::new().unwrap();
        make_project(dir.path());
        let d = CoralToolDispatcher::new(dir.path().to_path_buf());
        match d.call("frobnicate", &serde_json::json!({})) {
            ToolCallResult::Error { message } => assert!(message.contains("unknown tool")),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    /// v0.19.5 audit M5: every tool call appends an audit line.
    #[test]
    fn tool_call_appends_audit_line() {
        let dir = TempDir::new().unwrap();
        make_project(dir.path());
        let d = CoralToolDispatcher::new(dir.path().to_path_buf());
        let _ = d.call("search", &serde_json::json!({"q": "Order"}));
        let log = std::fs::read_to_string(dir.path().join(".coral/audit.log"))
            .expect("audit log written");
        assert!(log.contains("\"tool\":\"search\""), "log was: {log}");
    }
}
