//! Hand-rolled JSON-RPC 2.0 server for the MCP protocol.
//!
//! v0.19 wave 2 keeps the dep tree slim by implementing the minimal
//! MCP surface (initialize, resources/list, resources/read,
//! tools/list, tools/call, prompts/list, prompts/get) directly. If
//! the user demands the full official `rmcp = "1.6"` surface
//! (notifications, streaming, advanced lifecycle), we swap this
//! implementation in v0.20 — the trait-based catalog model means
//! callers don't notice.
//!
//! The server reads JSON-RPC requests from stdin (one JSON object
//! per line per the MCP stdio transport spec) and writes responses
//! to stdout. Stderr is reserved for server-side logs.

use crate::ServerConfig;
use crate::prompts::PromptCatalog;
use crate::resources::ResourceProvider;
use crate::tools::{ToolCatalog, ToolKind};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::sync::Arc;

/// MCP protocol version. Coral pins to the 2025-11-25 spec freeze;
/// future bumps are coordinated via the spec's negotiation flow.
pub const PROTOCOL_VERSION: &str = "2025-11-25";

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

/// Per-request handler that uses the configured `ResourceProvider`
/// (which the CLI fills in with a real wiki reader) and the static
/// tool/prompt catalogs.
pub struct McpHandler {
    pub config: ServerConfig,
    pub resources: Arc<dyn ResourceProvider>,
    /// Tool dispatcher. Each tool name → callable that takes a JSON
    /// argument object and returns a JSON result. The CLI populates
    /// this with thin adapters over `coral query` etc.
    pub tools: Arc<dyn ToolDispatcher>,
}

/// Tools are dispatched through a trait so the catalog (data) and the
/// runtime behavior (CLI subprocess / library calls) can evolve
/// independently. v0.19 wave 2 ships a `NoOpDispatcher` for tests and
/// a CLI-side adapter that reuses `coral query` / `coral search` /
/// `coral verify`.
pub trait ToolDispatcher: Send + Sync {
    fn call(&self, name: &str, args: &serde_json::Value) -> ToolCallResult;
}

/// Outcome of a tool call. Errors get wrapped in the JSON-RPC error
/// envelope; `Skip` becomes a friendly text result so the agent can
/// route around feature gaps.
#[derive(Debug, Clone)]
pub enum ToolCallResult {
    Ok(serde_json::Value),
    Skip { reason: String },
    Error { message: String },
}

/// In-memory dispatcher returning canned responses. Used by tests +
/// `coral mcp serve --no-tools` (an undocumented dev flag).
pub struct NoOpDispatcher;

impl ToolDispatcher for NoOpDispatcher {
    fn call(&self, name: &str, _args: &serde_json::Value) -> ToolCallResult {
        ToolCallResult::Skip {
            reason: format!("tool '{name}' is not wired in this build"),
        }
    }
}

impl McpHandler {
    pub fn new(
        config: ServerConfig,
        resources: Arc<dyn ResourceProvider>,
        tools: Arc<dyn ToolDispatcher>,
    ) -> Self {
        Self {
            config,
            resources,
            tools,
        }
    }

    /// Run the stdio loop. Reads one JSON-RPC message per line until
    /// stdin closes. Each response is written to stdout followed by
    /// a newline. Notifications (requests with no `id`) are dispatched
    /// for side effects but no response is emitted to stdout, per
    /// JSON-RPC 2.0 §4.1.
    pub fn serve_stdio(&self) -> std::io::Result<()> {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.trim().is_empty() {
                continue;
            }
            // v0.19.6 audit M3: `handle_line` now returns `Option<…>`
            // — `None` for JSON-RPC notifications (no `id` field).
            // Skip emitting anything for notifications so we don't
            // confuse strict JSON-RPC clients that don't expect a
            // response.
            if let Some(response) = self.handle_line(&line) {
                let serialized = serde_json::to_string(&response).unwrap_or_else(|_| "{}".into());
                writeln!(handle, "{serialized}")?;
                handle.flush()?;
            }
        }
        Ok(())
    }

    /// Public for tests: handle a single JSON-RPC line and return the
    /// response value (so we can exercise the dispatch matrix without
    /// stdin/stdout).
    ///
    /// v0.19.6 audit M3: returns `None` for JSON-RPC notifications
    /// (requests without an `id` field). Per JSON-RPC 2.0 §4.1 the
    /// server MUST NOT reply to a notification — even with an error.
    /// Side effects (the actual handler dispatch) still run.
    /// Parse errors and version errors STILL produce a response with
    /// `id: null` because there's no way to know whether the malformed
    /// payload was intended as a notification.
    pub fn handle_line(&self, line: &str) -> Option<serde_json::Value> {
        let request: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                return Some(
                    serde_json::to_value(JsonRpcResponse {
                        jsonrpc: "2.0",
                        id: None,
                        result: None,
                        error: Some(JsonRpcError {
                            code: -32700,
                            message: format!("parse error: {e}"),
                        }),
                    })
                    .unwrap(),
                );
            }
        };
        if request.jsonrpc != "2.0" {
            return Some(error_response(request.id, -32600, "jsonrpc must be '2.0'"));
        }
        let result = match request.method.as_str() {
            "initialize" => self.method_initialize(),
            "resources/list" => self.method_resources_list(),
            "resources/read" => self.method_resources_read(&request.params),
            "tools/list" => self.method_tools_list(),
            "tools/call" => self.method_tools_call(&request.params),
            "prompts/list" => self.method_prompts_list(),
            "prompts/get" => self.method_prompts_get(&request.params),
            "ping" => Ok(serde_json::json!({})),
            _ => Err(format!("unknown method: {}", request.method)),
        };
        // Notifications: side effects ran above, but we MUST NOT emit
        // a response — JSON-RPC 2.0 §4.1.
        request.id.as_ref()?;
        Some(match result {
            Ok(value) => ok_response(request.id, value),
            Err(message) => error_response(request.id, -32601, &message),
        })
    }

    fn method_initialize(&self) -> std::result::Result<serde_json::Value, String> {
        Ok(serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "resources": { "listChanged": false },
                "tools": { "listChanged": false },
                "prompts": { "listChanged": false }
            },
            "serverInfo": {
                "name": "coral",
                "version": env!("CARGO_PKG_VERSION")
            }
        }))
    }

    fn method_resources_list(&self) -> std::result::Result<serde_json::Value, String> {
        let resources = self.resources.list();
        Ok(serde_json::json!({
            "resources": resources
        }))
    }

    fn method_resources_read(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required parameter `uri`".to_string())?;
        // v0.19.6 audit C1: forward the provider-supplied mimeType
        // instead of hardcoding `text/markdown`. Hardcoding was
        // silently mislabeling every JSON resource in the catalog
        // (`coral://manifest`, `coral://lock`, `coral://stats`, etc.)
        // as markdown — clients then either fell back to plain text or
        // failed to parse the JSON body.
        let (body, mime_type) = self
            .resources
            .read(uri)
            .ok_or_else(|| format!("resource not found: {uri}"))?;
        Ok(serde_json::json!({
            "contents": [
                { "uri": uri, "mimeType": mime_type, "text": body }
            ]
        }))
    }

    fn method_tools_list(&self) -> std::result::Result<serde_json::Value, String> {
        let tools = if self.config.read_only {
            ToolCatalog::read_only()
        } else {
            ToolCatalog::all()
        };
        let tools_json: Vec<serde_json::Value> = tools
            .into_iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": serde_json::from_str::<serde_json::Value>(&t.input_schema_json).unwrap_or(serde_json::Value::Null)
                })
            })
            .collect();
        Ok(serde_json::json!({ "tools": tools_json }))
    }

    fn method_tools_call(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required parameter `name`".to_string())?;
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        // Enforce read-only.
        let kind = lookup_tool_kind(name).ok_or_else(|| format!("unknown tool: {name}"))?;
        let is_write = matches!(kind, ToolKind::RunTest | ToolKind::Up | ToolKind::Down);
        if is_write && self.config.read_only {
            return Err(format!(
                "tool '{name}' requires --allow-write-tools (server is in --read-only mode)"
            ));
        }
        match self.tools.call(name, &args) {
            ToolCallResult::Ok(value) => Ok(serde_json::json!({
                "content": [
                    { "type": "text", "text": value.to_string() }
                ]
            })),
            ToolCallResult::Skip { reason } => Ok(serde_json::json!({
                "content": [
                    { "type": "text", "text": format!("(skipped: {reason})") }
                ]
            })),
            ToolCallResult::Error { message } => Err(message),
        }
    }

    fn method_prompts_list(&self) -> std::result::Result<serde_json::Value, String> {
        let prompts = PromptCatalog::list();
        let json: Vec<serde_json::Value> = prompts
            .into_iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "description": p.description,
                    "arguments": p.arguments
                })
            })
            .collect();
        Ok(serde_json::json!({ "prompts": json }))
    }

    fn method_prompts_get(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required parameter `name`".to_string())?;
        let prompt = PromptCatalog::list()
            .into_iter()
            .find(|p| p.name == name)
            .ok_or_else(|| format!("unknown prompt: {name}"))?;
        let mut content = prompt.template.clone();
        if let Some(args_obj) = params.get("arguments").and_then(|v| v.as_object()) {
            for (key, val) in args_obj {
                let placeholder = format!("{{{{{key}}}}}");
                let str_val = val
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| val.to_string());
                content = content.replace(&placeholder, &str_val);
            }
        }
        Ok(serde_json::json!({
            "description": prompt.description,
            "messages": [
                {
                    "role": "user",
                    "content": { "type": "text", "text": content }
                }
            ]
        }))
    }
}

fn lookup_tool_kind(name: &str) -> Option<ToolKind> {
    ToolCatalog::all()
        .into_iter()
        .find(|t| t.name == name)
        .map(|t| t.kind)
}

fn ok_response(id: Option<serde_json::Value>, result: serde_json::Value) -> serde_json::Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    })
    .unwrap()
}

fn error_response(id: Option<serde_json::Value>, code: i64, message: &str) -> serde_json::Value {
    serde_json::to_value(JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
        }),
    })
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServerConfig;
    use crate::resources::WikiResourceProvider;

    fn handler(read_only: bool) -> McpHandler {
        let cfg = ServerConfig {
            transport: crate::Transport::Stdio,
            read_only,
            port: None,
        };
        let resources = Arc::new(WikiResourceProvider::new(std::path::PathBuf::from("/tmp")));
        let tools = Arc::new(NoOpDispatcher);
        McpHandler::new(cfg, resources, tools)
    }

    fn call(h: &McpHandler, method: &str, params: serde_json::Value) -> serde_json::Value {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });
        h.handle_line(&req.to_string())
            .expect("requests with `id` must produce a response")
    }

    #[test]
    fn initialize_returns_protocol_version_and_server_info() {
        let h = handler(true);
        let resp = call(&h, "initialize", serde_json::json!({}));
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(resp["result"]["serverInfo"]["name"], "coral");
    }

    #[test]
    fn resources_list_returns_static_catalog_uris() {
        let h = handler(true);
        let resp = call(&h, "resources/list", serde_json::json!({}));
        let resources = &resp["result"]["resources"];
        assert!(resources.is_array());
        let uris: Vec<&str> = resources
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["uri"].as_str().unwrap())
            .collect();
        assert!(uris.contains(&"coral://manifest"));
        assert!(uris.contains(&"coral://lock"));
    }

    #[test]
    fn tools_list_filters_to_read_only_when_configured() {
        let h = handler(true);
        let resp = call(&h, "tools/list", serde_json::json!({}));
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        // Read-only tools present.
        assert!(names.contains(&"query"));
        assert!(names.contains(&"verify"));
        // Write tools NOT present.
        assert!(!names.contains(&"up"));
        assert!(!names.contains(&"run_test"));
    }

    #[test]
    fn tools_list_includes_write_tools_when_not_read_only() {
        let h = handler(false);
        let resp = call(&h, "tools/list", serde_json::json!({}));
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"up"));
        assert!(names.contains(&"down"));
    }

    #[test]
    fn tools_call_rejects_write_tool_in_read_only_mode() {
        let h = handler(true);
        let resp = call(
            &h,
            "tools/call",
            serde_json::json!({"name": "up", "arguments": {}}),
        );
        let msg = resp["error"]["message"].as_str().unwrap_or("");
        assert!(msg.contains("read-only"));
    }

    #[test]
    fn prompts_list_returns_three_prompts() {
        let h = handler(true);
        let resp = call(&h, "prompts/list", serde_json::json!({}));
        let prompts = resp["result"]["prompts"].as_array().unwrap();
        assert_eq!(prompts.len(), 3);
    }

    #[test]
    fn prompts_get_substitutes_arguments() {
        let h = handler(true);
        let resp = call(
            &h,
            "prompts/get",
            serde_json::json!({
                "name": "onboard",
                "arguments": {"profile": "backend", "slugs": "x,y,z"}
            }),
        );
        let text = resp["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("backend"));
    }

    #[test]
    fn unknown_method_returns_error() {
        let h = handler(true);
        let resp = call(&h, "frobnicate", serde_json::json!({}));
        assert!(
            resp["error"]["message"]
                .as_str()
                .unwrap()
                .contains("unknown method")
        );
    }

    #[test]
    fn invalid_jsonrpc_version_returns_error() {
        let h = handler(true);
        let line = r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#;
        let resp = h.handle_line(line).expect("expected response");
        assert!(
            resp["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("jsonrpc must be")
        );
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let h = handler(true);
        let resp = h.handle_line("not json").expect("expected response");
        assert_eq!(resp["error"]["code"], -32700);
    }

    /// v0.19.6 audit M3: a JSON-RPC request without `id` is a
    /// notification — server MUST NOT reply (JSON-RPC 2.0 §4.1).
    /// Verifies handle_line returns `None` even on a known method.
    #[test]
    fn notification_without_id_produces_no_response() {
        let h = handler(true);
        // ping with no `id` → notification.
        let line = r#"{"jsonrpc":"2.0","method":"ping"}"#;
        assert!(
            h.handle_line(line).is_none(),
            "notification must produce no response"
        );
    }

    /// v0.19.6 audit M3: notifications for unknown methods also stay
    /// silent. The dispatcher records the side-effect-free error
    /// internally but the wire stays empty — a misnamed `tools/call`
    /// notification shouldn't spam stdout with an error envelope the
    /// client never asked for.
    #[test]
    fn notification_for_unknown_method_produces_no_response() {
        let h = handler(true);
        let line = r#"{"jsonrpc":"2.0","method":"frobnicate","params":{}}"#;
        assert!(
            h.handle_line(line).is_none(),
            "unknown-method notification must produce no response"
        );
    }
}
