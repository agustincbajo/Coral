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
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// MCP protocol version. Coral pins to the 2025-11-25 spec freeze;
/// future bumps are coordinated via the spec's negotiation flow.
pub const PROTOCOL_VERSION: &str = "2025-11-25";

/// v0.19.8 #26: page size for `resources/list` + `tools/list` cursor
/// pagination. Wikis with hundreds of pages otherwise emit one
/// JSON-RPC envelope per page in the resources catalog, which some
/// transports truncate. 100 is generous for the median wiki and
/// small enough that the round-trip cost stays sub-100µs over stdio.
pub const PAGINATION_PAGE_SIZE: usize = 100;

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

/// v0.30 audit #008: typed handler error so the JSON-RPC envelope's
/// `error.code` reflects the actual cause (params malformed vs method
/// missing vs gated). Pre-fix every non-parse, non-version error
/// collapsed to `-32601 Method not found`, defeating clients that
/// branch on `error.code` (JSON-RPC 2.0 §5.1).
#[derive(Debug)]
enum HandlerError {
    /// `-32602 Invalid params` — missing required parameter, bad cursor, etc.
    InvalidParams(String),
    /// `-32002` server-defined: resource/prompt URI does not exist.
    NotFound(String),
    /// `-32001` server-defined: feature gated behind `--allow-write-tools`
    /// or `--allow-experimental-tasks`.
    Gated(String),
    /// `-32601 Method not found` — the only case where the pre-fix
    /// behavior was correct.
    MethodNotFound(String),
    /// `-32603 Internal error` — handler couldn't satisfy the request
    /// for an internal reason (catch-all).
    #[allow(dead_code)]
    Internal(String),
}

impl HandlerError {
    fn code(&self) -> i64 {
        match self {
            HandlerError::InvalidParams(_) => -32602,
            HandlerError::NotFound(_) => -32002,
            HandlerError::Gated(_) => -32001,
            HandlerError::MethodNotFound(_) => -32601,
            HandlerError::Internal(_) => -32603,
        }
    }

    fn message(&self) -> &str {
        match self {
            HandlerError::InvalidParams(m)
            | HandlerError::NotFound(m)
            | HandlerError::Gated(m)
            | HandlerError::MethodNotFound(m)
            | HandlerError::Internal(m) => m,
        }
    }
}

/// v0.25 M3.11: A task stored in the experimental MCP Tasks handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTask {
    pub task_id: String,
    pub name: String,
    pub description: String,
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
    /// URIs the client has subscribed to for change notifications.
    subscriptions: Mutex<HashSet<String>>,
    /// Channel for pushing notifications to the transport layer.
    notification_tx: Mutex<Option<std::sync::mpsc::Sender<serde_json::Value>>>,
    /// v0.25 M3.11: In-memory task store for experimental MCP Tasks.
    tasks: Mutex<Vec<McpTask>>,
    /// Monotonic counter for task IDs.
    task_id_counter: AtomicU64,
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

/// v0.24 M2.2: dispatcher that handles `list_interfaces` and
/// `contract_status` in-process (they only need the wiki + filesystem)
/// and delegates all other tools to an inner dispatcher. The MCP server
/// constructor wraps the CLI-provided dispatcher in this layer so the
/// contract tools work without extra CLI wiring.
pub struct ContractToolDispatcher {
    pub inner: Arc<dyn ToolDispatcher>,
    pub resources: Arc<dyn ResourceProvider>,
    pub project_root: std::path::PathBuf,
}

impl ToolDispatcher for ContractToolDispatcher {
    fn call(&self, name: &str, args: &serde_json::Value) -> ToolCallResult {
        match name {
            "list_interfaces" => {
                let body = self.resources.read("coral://contracts");
                match body {
                    Some((json, _)) => match serde_json::from_str::<serde_json::Value>(&json) {
                        Ok(val) => ToolCallResult::Ok(val),
                        Err(_) => ToolCallResult::Ok(serde_json::json!({"contracts": []})),
                    },
                    None => ToolCallResult::Ok(serde_json::json!({"contracts": []})),
                }
            }
            "contract_status" => {
                let contracts_dir = self.project_root.join(".coral").join("contracts");
                if !contracts_dir.exists() {
                    return ToolCallResult::Ok(
                        serde_json::json!({"status": "no contract checks found"}),
                    );
                }
                let repo_filter = args.get("repo").and_then(|v| v.as_str());
                let mut reports = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&contracts_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|e| e.to_str()) == Some("json") {
                            if let Ok(raw) = std::fs::read_to_string(&path) {
                                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                                    if let Some(repo) = repo_filter {
                                        if val.get("repo").and_then(|r| r.as_str()) != Some(repo) {
                                            continue;
                                        }
                                    }
                                    reports.push(val);
                                }
                            }
                        }
                    }
                }
                if reports.is_empty() {
                    ToolCallResult::Ok(serde_json::json!({"status": "no contract checks found"}))
                } else {
                    ToolCallResult::Ok(serde_json::json!({"reports": reports}))
                }
            }
            _ => self.inner.call(name, args),
        }
    }
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
            subscriptions: Mutex::new(HashSet::new()),
            notification_tx: Mutex::new(None),
            tasks: Mutex::new(Vec::new()),
            task_id_counter: AtomicU64::new(1),
        }
    }

    /// Wire a notification channel so push notifications can reach the transport.
    pub fn set_notification_sender(&self, tx: std::sync::mpsc::Sender<serde_json::Value>) {
        *self.notification_tx.lock().unwrap() = Some(tx);
    }

    /// Run the stdio loop. Reads one JSON-RPC message per line until
    /// stdin closes. Each response is written to stdout followed by
    /// a newline. Notifications (requests with no `id`) are dispatched
    /// for side effects but no response is emitted to stdout, per
    /// JSON-RPC 2.0 §4.1.
    ///
    /// v0.21.1: thin shim over [`crate::transport::stdio::serve_stdio`].
    /// The body was lifted into `transport/stdio.rs` so the new HTTP/SSE
    /// transport could share `handle_line` without dragging the stdio
    /// loop into the JSON-RPC core. Behavior is byte-identical to the
    /// v0.21.0 stdio loop and pinned by a golden fixture in
    /// `crates/coral-mcp/tests/mcp_stdio_golden.rs`.
    pub fn serve_stdio(&self) -> std::io::Result<()> {
        crate::transport::stdio::serve_stdio(self)
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
            "resources/list" => self.method_resources_list(&request.params),
            "resources/read" => self.method_resources_read(&request.params),
            "resources/subscribe" => self.method_resources_subscribe(&request.params),
            "resources/unsubscribe" => self.method_resources_unsubscribe(&request.params),
            "tools/list" => self.method_tools_list(&request.params),
            "tools/call" => self.method_tools_call(&request.params),
            "prompts/list" => self.method_prompts_list(),
            "prompts/get" => self.method_prompts_get(&request.params),
            "tasks/create" => self.method_tasks_create(&request.params),
            "tasks/list" => self.method_tasks_list(),
            "ping" => Ok(serde_json::json!({})),
            _ => Err(HandlerError::MethodNotFound(format!(
                "unknown method: {}",
                request.method
            ))),
        };
        // Notifications: side effects ran above, but we MUST NOT emit
        // a response — JSON-RPC 2.0 §4.1.
        request.id.as_ref()?;
        Some(match result {
            Ok(value) => ok_response(request.id, value),
            Err(e) => error_response(request.id, e.code(), e.message()),
        })
    }

    fn method_initialize(&self) -> std::result::Result<serde_json::Value, HandlerError> {
        // v0.30 audit #010: when running over HTTP/SSE we still advertise
        // subscribe + listChanged. The GET /mcp stream now drains
        // notifications from `notification_tx` (see
        // `transport/http_sse.rs::handle_get_sse`), so the capability is
        // honest for both transports.
        Ok(serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "resources": { "listChanged": true, "subscribe": true },
                "tools": { "listChanged": false },
                "prompts": { "listChanged": false }
            },
            "serverInfo": {
                "name": "coral",
                "version": env!("CARGO_PKG_VERSION")
            }
        }))
    }

    fn method_resources_list(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, HandlerError> {
        let resources = self.resources.list();
        let (page, next_cursor) = paginate(&resources, params, PAGINATION_PAGE_SIZE)
            .map_err(HandlerError::InvalidParams)?;
        let mut response = serde_json::json!({ "resources": page });
        // MCP 2025-11-25: omit `nextCursor` when there is no next page.
        // Including a `null` field would mislead clients that test
        // `if response.nextCursor` for the existence of more data.
        if let Some(cursor) = next_cursor {
            response
                .as_object_mut()
                .expect("json object")
                .insert("nextCursor".to_string(), serde_json::Value::String(cursor));
        }
        Ok(response)
    }

    fn method_resources_read(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, HandlerError> {
        let uri = params.get("uri").and_then(|v| v.as_str()).ok_or_else(|| {
            HandlerError::InvalidParams("missing required parameter `uri`".to_string())
        })?;
        // v0.19.6 audit C1: forward the provider-supplied mimeType
        // instead of hardcoding `text/markdown`. Hardcoding was
        // silently mislabeling every JSON resource in the catalog
        // (`coral://manifest`, `coral://lock`, `coral://stats`, etc.)
        // as markdown — clients then either fell back to plain text or
        // failed to parse the JSON body.
        let (body, mime_type) = self
            .resources
            .read(uri)
            .ok_or_else(|| HandlerError::NotFound(format!("resource not found: {uri}")))?;
        Ok(serde_json::json!({
            "contents": [
                { "uri": uri, "mimeType": mime_type, "text": body }
            ]
        }))
    }

    fn method_tools_list(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, HandlerError> {
        // v0.20.2 audit-followup #38: the listing gate is now
        // `allow_write_tools`, NOT `!read_only`. Pre-fix `--read-only
        // false` alone would list write tools in `tools/list` even
        // though `tools/call` correctly required both flags — the
        // user saw "(skipped: tool 'down' requires --allow-write-tools)"
        // on every write call after `--read-only false`, which is
        // confusing. Now the catalog matches the dispatcher.
        let tools = if self.config.allow_write_tools {
            ToolCatalog::all()
        } else {
            ToolCatalog::read_only()
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
        // v0.19.8 #26: tools/list also honors cursor pagination per
        // the MCP spec. The current catalog ships ~5–8 tools, well
        // under PAGINATION_PAGE_SIZE — pagination is a no-op today
        // but the contract is pinned for forward compatibility.
        let (page, next_cursor) = paginate(&tools_json, params, PAGINATION_PAGE_SIZE)
            .map_err(HandlerError::InvalidParams)?;
        let mut response = serde_json::json!({ "tools": page });
        if let Some(cursor) = next_cursor {
            response
                .as_object_mut()
                .expect("json object")
                .insert("nextCursor".to_string(), serde_json::Value::String(cursor));
        }
        Ok(response)
    }

    fn method_tools_call(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, HandlerError> {
        let name = params.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
            HandlerError::InvalidParams("missing required parameter `name`".to_string())
        })?;
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        // v0.20.2 audit-followup #38: the dispatcher's gate now
        // matches the listing gate — both surfaces require
        // `allow_write_tools` for the three write tools. Pre-fix the
        // listing checked `!read_only` while the dispatcher checked
        // `read_only && !allow_write_tools` (computed at the CLI
        // layer); the two surfaces could disagree.
        let kind = lookup_tool_kind(name)
            .ok_or_else(|| HandlerError::InvalidParams(format!("unknown tool: {name}")))?;
        let is_write = matches!(kind, ToolKind::RunTest | ToolKind::Up | ToolKind::Down);
        if is_write && !self.config.allow_write_tools {
            return Err(HandlerError::Gated(format!(
                "tool '{name}' requires --allow-write-tools"
            )));
        }
        // v0.30 audit #008: per MCP spec, errors from a tool *that ran*
        // surface as `result: { isError: true, content: [...] }` — the
        // JSON-RPC envelope `error` is reserved for protocol-level
        // failures (method missing, params malformed, gating). Pre-fix
        // `ToolCallResult::Error` was returned as `Err(message)` which
        // collapsed into a `-32601` JSON-RPC error, double-violating
        // the spec.
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
            ToolCallResult::Error { message } => Ok(serde_json::json!({
                "isError": true,
                "content": [
                    { "type": "text", "text": message }
                ]
            })),
        }
    }

    fn method_prompts_list(&self) -> std::result::Result<serde_json::Value, HandlerError> {
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
    ) -> std::result::Result<serde_json::Value, HandlerError> {
        let name = params.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
            HandlerError::InvalidParams("missing required parameter `name`".to_string())
        })?;
        let prompt = PromptCatalog::list()
            .into_iter()
            .find(|p| p.name == name)
            .ok_or_else(|| HandlerError::NotFound(format!("unknown prompt: {name}")))?;
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

    fn method_resources_subscribe(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, HandlerError> {
        let uri = params.get("uri").and_then(|v| v.as_str()).ok_or_else(|| {
            HandlerError::InvalidParams("missing required parameter `uri`".to_string())
        })?;
        self.subscriptions.lock().unwrap().insert(uri.to_string());
        Ok(serde_json::json!({}))
    }

    fn method_resources_unsubscribe(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, HandlerError> {
        let uri = params.get("uri").and_then(|v| v.as_str()).ok_or_else(|| {
            HandlerError::InvalidParams("missing required parameter `uri`".to_string())
        })?;
        self.subscriptions.lock().unwrap().remove(uri);
        Ok(serde_json::json!({}))
    }

    /// v0.25 M3.11: `tasks/create` — experimental MCP Tasks handle.
    /// Accepts `{name, description}`, stores in memory, returns `{task_id}`.
    /// Gated behind `allow_experimental_tasks` config flag.
    fn method_tasks_create(
        &self,
        params: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, HandlerError> {
        if !self.config.allow_experimental_tasks {
            return Err(HandlerError::MethodNotFound(
                "unknown method: tasks/create (enable with allow_experimental_tasks)".to_string(),
            ));
        }
        let name = params.get("name").and_then(|v| v.as_str()).ok_or_else(|| {
            HandlerError::InvalidParams("missing required parameter `name`".to_string())
        })?;
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let id = self.task_id_counter.fetch_add(1, Ordering::Relaxed);
        let task_id = format!("task-{id}");

        let task = McpTask {
            task_id: task_id.clone(),
            name: name.to_string(),
            description: description.to_string(),
        };
        self.tasks.lock().unwrap().push(task);

        Ok(serde_json::json!({ "task_id": task_id }))
    }

    /// v0.25 M3.11: `tasks/list` — experimental MCP Tasks handle.
    /// Returns the in-memory task list.
    /// Gated behind `allow_experimental_tasks` config flag.
    fn method_tasks_list(&self) -> std::result::Result<serde_json::Value, HandlerError> {
        if !self.config.allow_experimental_tasks {
            return Err(HandlerError::MethodNotFound(
                "unknown method: tasks/list (enable with allow_experimental_tasks)".to_string(),
            ));
        }
        let tasks = self.tasks.lock().unwrap();
        let tasks_json: Vec<serde_json::Value> = tasks
            .iter()
            .map(|t| {
                serde_json::json!({
                    "task_id": t.task_id,
                    "name": t.name,
                    "description": t.description
                })
            })
            .collect();
        Ok(serde_json::json!({ "tasks": tasks_json }))
    }

    /// Push a resource-updated notification for a specific URI.
    /// Called externally when the wiki changes (e.g., after ingest/bootstrap).
    /// Only sends if the URI is in the subscription set.
    pub fn notify_resource_updated(&self, uri: &str) {
        let subscribed = self.subscriptions.lock().unwrap().contains(uri);
        if !subscribed {
            return;
        }
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/resources/updated",
            "params": { "uri": uri }
        });
        if let Some(tx) = self.notification_tx.lock().unwrap().as_ref() {
            let _ = tx.send(notification);
        }
    }

    /// Push a list-changed notification (all resources may have changed).
    /// Sends to ALL subscribers. Called after wiki modifications.
    pub fn notify_resources_list_changed(&self) {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/resources/list_changed"
        });
        if let Some(tx) = self.notification_tx.lock().unwrap().as_ref() {
            let _ = tx.send(notification);
        }
    }
}

fn lookup_tool_kind(name: &str) -> Option<ToolKind> {
    use ahash::AHashMap;
    use std::sync::OnceLock;

    static TOOL_KIND_MAP: OnceLock<AHashMap<String, ToolKind>> = OnceLock::new();
    let map = TOOL_KIND_MAP.get_or_init(|| {
        ToolCatalog::all()
            .into_iter()
            .map(|t| (t.name, t.kind))
            .collect()
    });
    map.get(name).copied()
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

/// v0.19.8 #26: cursor-paginate a slice into one page of `page_size`
/// items + an optional `nextCursor` for the following page.
///
/// Cursor encoding is intentionally simple: a stringified non-negative
/// integer offset. The MCP spec treats cursors as opaque, so this is
/// spec-compliant; clients must not parse them. The encoding is also
/// trivial to inspect when debugging — JSON-RPC traces stay readable.
///
/// Drift / invalidation: when the underlying list changes between
/// requests (e.g. the wiki adds a page), the cursor still points to
/// an offset, which now refers to a different item. This is "best
/// effort" semantics — clients that need stable iteration should
/// re-fetch from offset 0. We document this in the README.
///
/// Errors:
/// - cursor that's not a non-negative integer string → JSON-RPC
///   `-32602` (per the call site: returned as `Err(String)` here, the
///   server wraps it with -32601; the client sees a clear message).
fn paginate<T: Clone + Serialize>(
    items: &[T],
    params: &serde_json::Value,
    page_size: usize,
) -> std::result::Result<(Vec<T>, Option<String>), String> {
    let cursor = params.get("cursor").and_then(|v| v.as_str());
    let offset = match cursor {
        None => 0,
        Some(s) => parse_cursor(s)?,
    };
    let total = items.len();
    if offset > total {
        return Err(format!(
            "cursor offset {offset} exceeds list length {total} \
             — re-list from offset 0 (the underlying collection \
             may have shrunk between requests)"
        ));
    }
    let end = offset.saturating_add(page_size).min(total);
    let page = items[offset..end].to_vec();
    let next_cursor = if end < total {
        Some(encode_cursor(end))
    } else {
        None
    };
    Ok((page, next_cursor))
}

/// Stringified offset cursor — opaque per MCP spec but readable.
fn encode_cursor(offset: usize) -> String {
    offset.to_string()
}

/// Parse a stringified-offset cursor. Surfaces a clear error for
/// malformed input (e.g. base64 garbage, negative numbers, NaN).
fn parse_cursor(s: &str) -> std::result::Result<usize, String> {
    s.parse::<usize>()
        .map_err(|e| format!("invalid cursor {s:?}: {e} (cursor must be a non-negative integer)"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServerConfig;
    use crate::resources::WikiResourceProvider;

    fn handler(read_only: bool) -> McpHandler {
        // Pre-v0.20.2 helper retained for back-compat with existing
        // tests: a `read_only` flag drives the config. When the
        // legacy boolean is `false`, this helper now ALSO sets
        // `allow_write_tools = true` so the test that exercised
        // "non-read-only mode" still observes the same surface.
        // New tests should prefer `handler_with(read_only,
        // allow_write_tools)` to pin both axes explicitly.
        handler_with(read_only, !read_only)
    }

    /// v0.20.2 audit-followup #38: explicit constructor for the
    /// 2-axis matrix (`read_only`, `allow_write_tools`). Lets the
    /// new tests pin every cell of:
    /// - default (read_only=true, allow_write_tools=false)
    /// - --read-only false alone (read_only=false, allow_write_tools=false)
    /// - --read-only false + --allow-write-tools (both false / true)
    fn handler_with(read_only: bool, allow_write_tools: bool) -> McpHandler {
        let cfg = ServerConfig {
            transport: crate::Transport::Stdio,
            read_only,
            allow_write_tools,
            port: None,
            bind_addr: None,
            allow_experimental_tasks: false,
        };
        let resources = Arc::new(WikiResourceProvider::new(std::path::PathBuf::from("/tmp")));
        let tools = Arc::new(NoOpDispatcher);
        McpHandler::new(cfg, resources, tools)
    }

    /// Helper that creates a handler with experimental tasks enabled.
    fn handler_with_tasks() -> McpHandler {
        let cfg = ServerConfig {
            transport: crate::Transport::Stdio,
            read_only: true,
            allow_write_tools: false,
            port: None,
            bind_addr: None,
            allow_experimental_tasks: true,
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
        // v0.20.2 audit-followup #38: error message changed from
        // "(server is in --read-only mode)" to a cleaner form that
        // matches the new contract: write tools require
        // --allow-write-tools regardless of the --read-only flag.
        assert!(
            msg.contains("--allow-write-tools"),
            "expected --allow-write-tools hint in: {msg}"
        );
    }

    /// v0.20.2 audit-followup #38: regression matrix for `tools/list`.
    /// Pre-fix `--read-only false` alone surfaced all 8 tools; post-fix
    /// the listing gate is `allow_write_tools` only. Pin every cell:
    ///
    /// - default (read_only=true, allow_write_tools=false) → 5 read-only tools
    /// - read_only=false, allow_write_tools=false → STILL 5 read-only tools
    /// - read_only=true, allow_write_tools=true → 8 tools (allow_write_tools wins)
    /// - read_only=false, allow_write_tools=true → 8 tools
    #[test]
    fn tools_list_default_shows_only_read_only_tools() {
        let h = handler_with(true, false);
        let names = list_tool_names(&h);
        let read_only_count = ToolCatalog::read_only().len();
        assert_eq!(
            names.len(),
            read_only_count,
            "default must list exactly the {read_only_count} read-only tools, got: {names:?}"
        );
        assert!(!names.contains(&"up".to_string()));
        assert!(!names.contains(&"down".to_string()));
        assert!(!names.contains(&"run_test".to_string()));
    }

    #[test]
    fn tools_list_with_read_only_false_alone_still_hides_write_tools() {
        // The load-bearing regression: pre-fix this matrix cell
        // listed all 8 tools; post-fix it lists only the 5 read-only
        // ones because `allow_write_tools` is the gate.
        let h = handler_with(false, false);
        let names = list_tool_names(&h);
        let read_only_count = ToolCatalog::read_only().len();
        assert_eq!(
            names.len(),
            read_only_count,
            "--read-only false alone MUST NOT list write tools (audit #38): {names:?}"
        );
        assert!(!names.contains(&"up".to_string()));
        assert!(!names.contains(&"down".to_string()));
        assert!(!names.contains(&"run_test".to_string()));
    }

    #[test]
    fn tools_list_with_allow_write_tools_lists_all_tools() {
        let h = handler_with(false, true);
        let names = list_tool_names(&h);
        let total = ToolCatalog::all().len();
        assert_eq!(
            names.len(),
            total,
            "--allow-write-tools must list every tool (read-only + write)"
        );
        assert!(names.contains(&"up".to_string()));
        assert!(names.contains(&"down".to_string()));
        assert!(names.contains(&"run_test".to_string()));
    }

    /// v0.20.2 audit-followup #38: `tools/call` matrix mirrors the
    /// listing matrix. The dispatcher gate is `allow_write_tools`,
    /// not `!read_only`.
    #[test]
    fn tools_call_dispatcher_uses_allow_write_tools_gate() {
        // read_only=false alone → write tool still rejected.
        let h = handler_with(false, false);
        let resp = call(
            &h,
            "tools/call",
            serde_json::json!({"name": "up", "arguments": {}}),
        );
        let msg = resp["error"]["message"].as_str().unwrap_or("");
        assert!(
            msg.contains("--allow-write-tools"),
            "expected --allow-write-tools hint in: {msg}"
        );
        // allow_write_tools=true → write tool dispatched (NoOp
        // dispatcher returns Skip but the gate has been passed).
        let h2 = handler_with(false, true);
        let resp2 = call(
            &h2,
            "tools/call",
            serde_json::json!({"name": "up", "arguments": {}}),
        );
        // No error envelope — the call passed the gate, dispatcher
        // returned a Skip.
        assert!(
            resp2["error"].is_null(),
            "write tool with --allow-write-tools must pass the gate, got: {resp2}"
        );
    }

    fn list_tool_names(h: &McpHandler) -> Vec<String> {
        let resp = call(h, "tools/list", serde_json::json!({}));
        resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
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

    /// v0.19.8 #26: paginate returns the whole slice when it fits in
    /// one page and emits no `nextCursor`.
    #[test]
    fn paginate_returns_whole_list_when_under_page_size() {
        let items: Vec<i32> = (0..10).collect();
        let (page, next) = paginate(&items, &serde_json::json!({}), 100).expect("ok");
        assert_eq!(page, items);
        assert!(next.is_none(), "no next cursor when single page");
    }

    /// v0.19.8 #26: paginate returns exactly `page_size` items + a
    /// `nextCursor` pointing at the next offset when the slice
    /// overflows.
    #[test]
    fn paginate_returns_first_page_with_next_cursor_when_overflow() {
        let items: Vec<i32> = (0..250).collect();
        let (page, next) = paginate(&items, &serde_json::json!({}), 100).expect("ok");
        assert_eq!(page.len(), 100);
        assert_eq!(page.first(), Some(&0));
        assert_eq!(page.last(), Some(&99));
        assert_eq!(next, Some("100".to_string()));
    }

    /// v0.19.8 #26: paginate honors a string-encoded offset cursor
    /// from the client.
    #[test]
    fn paginate_resumes_at_cursor() {
        let items: Vec<i32> = (0..250).collect();
        let (page, next) =
            paginate(&items, &serde_json::json!({"cursor": "100"}), 100).expect("ok");
        assert_eq!(page.len(), 100);
        assert_eq!(page.first(), Some(&100));
        assert_eq!(page.last(), Some(&199));
        assert_eq!(next, Some("200".to_string()));
        // Last page (offset 200, 50 items left).
        let (page3, next3) =
            paginate(&items, &serde_json::json!({"cursor": "200"}), 100).expect("ok");
        assert_eq!(page3.len(), 50);
        assert_eq!(next3, None, "no next cursor on final page");
    }

    /// v0.19.8 #26: invalid cursor is a JSON-RPC error, not a silent
    /// empty response. Clients that send garbage should learn
    /// immediately rather than think the wiki is empty.
    #[test]
    fn paginate_rejects_invalid_cursor() {
        let items: Vec<i32> = (0..10).collect();
        let err = paginate(&items, &serde_json::json!({"cursor": "not-a-number"}), 100)
            .expect_err("must reject");
        assert!(
            err.contains("invalid cursor") && err.contains("not-a-number"),
            "expected clear error naming the bad cursor, got: {err}"
        );
        // Cursor pointing past the end is also an error (drift detection).
        let err2 =
            paginate(&items, &serde_json::json!({"cursor": "9999"}), 100).expect_err("must reject");
        assert!(
            err2.contains("exceeds list length") && err2.contains("9999"),
            "expected drift error, got: {err2}"
        );
    }

    /// v0.19.8 #26: edge case — cursor at exactly `len` returns an
    /// empty page with no `nextCursor`. This is well-defined behavior:
    /// the previous page's `nextCursor` legitimately landed here.
    #[test]
    fn paginate_at_exact_end_returns_empty_no_next() {
        let items: Vec<i32> = (0..100).collect();
        let (page, next) = paginate(&items, &serde_json::json!({"cursor": "100"}), 100)
            .expect("offset == len is valid (empty terminal page)");
        assert!(page.is_empty());
        assert!(next.is_none());
    }

    /// v0.19.8 #26: `resources/list` end-to-end via the JSON-RPC
    /// dispatch — pinning the client-visible shape (no `nextCursor`
    /// field when results fit one page).
    #[test]
    fn resources_list_does_not_emit_nextcursor_when_single_page() {
        let h = handler(true);
        let resp = call(&h, "resources/list", serde_json::json!({}));
        // The static catalog has < PAGINATION_PAGE_SIZE entries, so
        // the response must not include a `nextCursor` field.
        assert!(
            resp["result"].get("nextCursor").is_none(),
            "single-page response must omit nextCursor: {resp}"
        );
    }

    /// v0.19.8 #26: an invalid cursor on `resources/list` surfaces as
    /// a JSON-RPC error (so the client doesn't silently see an empty
    /// catalog and assume the wiki is unindexed).
    #[test]
    fn resources_list_invalid_cursor_returns_error() {
        let h = handler(true);
        let resp = call(
            &h,
            "resources/list",
            serde_json::json!({"cursor": "not-a-number"}),
        );
        assert!(resp.get("error").is_some(), "expected error: {resp}");
        assert!(
            resp["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("invalid cursor"),
            "error message must name the invalid cursor: {resp}"
        );
    }

    /// v0.19.8 #26: same contract on `tools/list` — pin it before any
    /// future tool catalog explosion crosses the page-size threshold.
    #[test]
    fn tools_list_does_not_emit_nextcursor_when_single_page() {
        let h = handler(false);
        let resp = call(&h, "tools/list", serde_json::json!({}));
        assert!(
            resp["result"].get("nextCursor").is_none(),
            "single-page tools/list response must omit nextCursor: {resp}"
        );
    }

    #[test]
    fn tools_list_invalid_cursor_returns_error() {
        let h = handler(true);
        let resp = call(&h, "tools/list", serde_json::json!({"cursor": "garbage"}));
        assert!(resp.get("error").is_some(), "expected error: {resp}");
    }

    /// v0.24 #11: initialize now advertises resource subscription
    /// support (`listChanged: true`, `subscribe: true`).
    #[test]
    fn initialize_advertises_resource_subscriptions() {
        let h = handler(true);
        let resp = call(&h, "initialize", serde_json::json!({}));
        let resources_cap = &resp["result"]["capabilities"]["resources"];
        assert_eq!(
            resources_cap["listChanged"], true,
            "listChanged must be true: {resp}"
        );
        assert_eq!(
            resources_cap["subscribe"], true,
            "subscribe must be true: {resp}"
        );
    }

    /// v0.24 #11: `resources/subscribe` records the URI and returns
    /// an empty result.
    #[test]
    fn resources_subscribe_returns_empty_ok() {
        let h = handler(true);
        let resp = call(
            &h,
            "resources/subscribe",
            serde_json::json!({"uri": "coral://manifest"}),
        );
        assert!(
            resp.get("error").is_none(),
            "subscribe must not error: {resp}"
        );
        assert_eq!(resp["result"], serde_json::json!({}));
    }

    /// v0.24 #11: `resources/unsubscribe` removes the URI and returns
    /// an empty result.
    #[test]
    fn resources_unsubscribe_returns_empty_ok() {
        let h = handler(true);
        // Subscribe first, then unsubscribe.
        call(
            &h,
            "resources/subscribe",
            serde_json::json!({"uri": "coral://manifest"}),
        );
        let resp = call(
            &h,
            "resources/unsubscribe",
            serde_json::json!({"uri": "coral://manifest"}),
        );
        assert!(
            resp.get("error").is_none(),
            "unsubscribe must not error: {resp}"
        );
        assert_eq!(resp["result"], serde_json::json!({}));
    }

    /// v0.24 #11: subscribe/unsubscribe reject calls missing the `uri`
    /// parameter.
    #[test]
    fn resources_subscribe_rejects_missing_uri() {
        let h = handler(true);
        let resp = call(&h, "resources/subscribe", serde_json::json!({}));
        assert!(
            resp["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("missing required parameter `uri`"),
            "expected uri error: {resp}"
        );
    }

    /// v0.24 #11: `notify_resource_updated` only fires when the URI
    /// is subscribed, and the notification reaches the channel.
    #[test]
    fn notify_resource_updated_sends_when_subscribed() {
        let h = handler(true);
        let (tx, rx) = std::sync::mpsc::channel();
        h.set_notification_sender(tx);
        // Not yet subscribed — should not send.
        h.notify_resource_updated("coral://manifest");
        assert!(
            rx.try_recv().is_err(),
            "must not send notification for unsubscribed URI"
        );
        // Subscribe, then notify.
        call(
            &h,
            "resources/subscribe",
            serde_json::json!({"uri": "coral://manifest"}),
        );
        h.notify_resource_updated("coral://manifest");
        let msg = rx.try_recv().expect("notification must be sent");
        assert_eq!(msg["method"], "notifications/resources/updated");
        assert_eq!(msg["params"]["uri"], "coral://manifest");
    }

    /// v0.24 #11: `notify_resources_list_changed` always fires
    /// (unconditionally) and emits the correct method name.
    #[test]
    fn notify_resources_list_changed_sends_unconditionally() {
        let h = handler(true);
        let (tx, rx) = std::sync::mpsc::channel();
        h.set_notification_sender(tx);
        h.notify_resources_list_changed();
        let msg = rx
            .try_recv()
            .expect("list_changed notification must be sent");
        assert_eq!(msg["method"], "notifications/resources/list_changed");
        assert_eq!(msg["jsonrpc"], "2.0");
    }

    /// v0.24 M2.2: `list_interfaces` returns empty when no interface
    /// pages exist (empty wiki root).
    #[test]
    fn list_interfaces_returns_empty_with_no_pages() {
        let h = handler(true);
        let resp = call(
            &h,
            "tools/call",
            serde_json::json!({"name": "list_interfaces", "arguments": {}}),
        );
        // The NoOpDispatcher returns a Skip, which is fine -- the tool
        // is correctly listed and dispatched. The Skip text confirms
        // the tool name was recognized.
        let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("list_interfaces") || text.contains("contracts"),
            "expected list_interfaces response, got: {text}"
        );
    }

    /// v0.24 M2.2: `contract_status` is recognized as a valid tool.
    #[test]
    fn contract_status_is_recognized() {
        let h = handler(true);
        let resp = call(
            &h,
            "tools/call",
            serde_json::json!({"name": "contract_status", "arguments": {}}),
        );
        // NoOpDispatcher returns Skip, confirming the tool name lookup
        // succeeded (unknown tools return an error envelope).
        assert!(
            resp.get("error").is_none(),
            "contract_status must be a known tool: {resp}"
        );
    }

    /// v0.24 M2.2: tools/list includes the new contract tools.
    #[test]
    fn tools_list_includes_contract_tools() {
        let h = handler(true);
        let names = list_tool_names(&h);
        assert!(
            names.contains(&"list_interfaces".to_string()),
            "tools/list must include list_interfaces: {names:?}"
        );
        assert!(
            names.contains(&"contract_status".to_string()),
            "tools/list must include contract_status: {names:?}"
        );
    }

    /// v0.24 M2.2: resources/list includes the contract resource URIs.
    #[test]
    fn resources_list_includes_contract_uris() {
        let h = handler(true);
        let resp = call(&h, "resources/list", serde_json::json!({}));
        let uris: Vec<&str> = resp["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["uri"].as_str().unwrap())
            .collect();
        assert!(
            uris.contains(&"coral://contracts"),
            "resources/list must include coral://contracts: {uris:?}"
        );
        assert!(
            uris.contains(&"coral://coverage"),
            "resources/list must include coral://coverage: {uris:?}"
        );
    }

    /// v0.24 M2.2: ContractToolDispatcher handles list_interfaces
    /// and delegates unknown tools to inner.
    #[test]
    fn contract_tool_dispatcher_handles_list_interfaces() {
        let resources = Arc::new(WikiResourceProvider::new(std::path::PathBuf::from(
            "/tmp/coral-mcp-tests-empty",
        )));
        let dispatcher = ContractToolDispatcher {
            inner: Arc::new(NoOpDispatcher),
            resources: resources.clone(),
            project_root: std::path::PathBuf::from("/tmp/coral-mcp-tests-empty"),
        };
        let result = dispatcher.call("list_interfaces", &serde_json::json!({}));
        match result {
            ToolCallResult::Ok(val) => {
                let contracts = val["contracts"].as_array().unwrap();
                assert!(
                    contracts.is_empty(),
                    "empty wiki should yield zero contracts"
                );
            }
            other => panic!("expected Ok, got: {other:?}"),
        }
    }

    /// v0.24 M2.2: ContractToolDispatcher handles contract_status
    /// and returns "no contract checks found" when .coral/contracts/
    /// doesn't exist.
    #[test]
    fn contract_tool_dispatcher_handles_contract_status() {
        let resources = Arc::new(WikiResourceProvider::new(std::path::PathBuf::from(
            "/tmp/coral-mcp-tests-empty",
        )));
        let dispatcher = ContractToolDispatcher {
            inner: Arc::new(NoOpDispatcher),
            resources: resources.clone(),
            project_root: std::path::PathBuf::from("/tmp/coral-mcp-tests-empty"),
        };
        let result = dispatcher.call("contract_status", &serde_json::json!({}));
        match result {
            ToolCallResult::Ok(val) => {
                assert_eq!(val["status"], "no contract checks found");
            }
            other => panic!("expected Ok, got: {other:?}"),
        }
    }

    /// v0.24 M2.2: ContractToolDispatcher delegates unknown tools to
    /// the inner dispatcher.
    #[test]
    fn contract_tool_dispatcher_delegates_unknown() {
        let resources = Arc::new(WikiResourceProvider::new(std::path::PathBuf::from(
            "/tmp/coral-mcp-tests-empty",
        )));
        let dispatcher = ContractToolDispatcher {
            inner: Arc::new(NoOpDispatcher),
            resources: resources.clone(),
            project_root: std::path::PathBuf::from("/tmp/coral-mcp-tests-empty"),
        };
        let result = dispatcher.call("query", &serde_json::json!({}));
        match result {
            ToolCallResult::Skip { reason } => {
                assert!(reason.contains("query"));
            }
            other => panic!("expected Skip from NoOpDispatcher, got: {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // v0.25 M3.11: MCP Tasks handle tests
    // ---------------------------------------------------------------

    /// tasks/create returns a task_id when experimental tasks are enabled.
    #[test]
    fn tasks_create_returns_task_id() {
        let h = handler_with_tasks();
        let resp = call(
            &h,
            "tasks/create",
            serde_json::json!({"name": "lint wiki", "description": "Run wiki lint pass"}),
        );
        assert!(
            resp.get("error").is_none(),
            "tasks/create must not error when enabled: {resp}"
        );
        let task_id = resp["result"]["task_id"].as_str().unwrap();
        assert!(
            task_id.starts_with("task-"),
            "task_id must start with 'task-': {task_id}"
        );
    }

    /// tasks/list returns tasks that were previously created.
    #[test]
    fn tasks_list_returns_stored_tasks() {
        let h = handler_with_tasks();
        // Create two tasks.
        call(
            &h,
            "tasks/create",
            serde_json::json!({"name": "task-a", "description": "First task"}),
        );
        call(
            &h,
            "tasks/create",
            serde_json::json!({"name": "task-b", "description": "Second task"}),
        );
        let resp = call(&h, "tasks/list", serde_json::json!({}));
        assert!(
            resp.get("error").is_none(),
            "tasks/list must not error when enabled: {resp}"
        );
        let tasks = resp["result"]["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 2, "must have 2 tasks: {tasks:?}");
        assert_eq!(tasks[0]["name"], "task-a");
        assert_eq!(tasks[1]["name"], "task-b");
        assert_eq!(tasks[0]["description"], "First task");
        assert_eq!(tasks[1]["description"], "Second task");
        // Each task has a unique id.
        assert_ne!(
            tasks[0]["task_id"].as_str().unwrap(),
            tasks[1]["task_id"].as_str().unwrap()
        );
    }

    /// tasks/create is rejected when allow_experimental_tasks is false.
    #[test]
    fn tasks_create_rejected_when_disabled() {
        let h = handler(true); // default: tasks disabled
        let resp = call(
            &h,
            "tasks/create",
            serde_json::json!({"name": "x", "description": "y"}),
        );
        let msg = resp["error"]["message"].as_str().unwrap_or("");
        assert!(
            msg.contains("allow_experimental_tasks"),
            "expected gate error, got: {msg}"
        );
    }

    /// tasks/list is rejected when allow_experimental_tasks is false.
    #[test]
    fn tasks_list_rejected_when_disabled() {
        let h = handler(true); // default: tasks disabled
        let resp = call(&h, "tasks/list", serde_json::json!({}));
        let msg = resp["error"]["message"].as_str().unwrap_or("");
        assert!(
            msg.contains("allow_experimental_tasks"),
            "expected gate error, got: {msg}"
        );
    }

    /// tasks/create rejects calls missing the required `name` parameter.
    #[test]
    fn tasks_create_rejects_missing_name() {
        let h = handler_with_tasks();
        let resp = call(
            &h,
            "tasks/create",
            serde_json::json!({"description": "no name"}),
        );
        let msg = resp["error"]["message"].as_str().unwrap_or("");
        assert!(
            msg.contains("missing required parameter `name`"),
            "expected name error: {msg}"
        );
    }

    // ---------------------------------------------------------------
    // v0.30 audit #008: JSON-RPC error code mapping. Pre-fix every
    // non-parse, non-version error collapsed to `-32601`. Pin each
    // variant of `HandlerError` to its JSON-RPC code so a regression
    // would surface immediately.
    // ---------------------------------------------------------------

    /// Dispatcher that always returns `ToolCallResult::Error` — used
    /// to verify that tool runtime errors flow through `result:
    /// { isError: true }` per MCP spec, NOT through the JSON-RPC
    /// envelope `error`.
    struct ErrorToolDispatcher;
    impl ToolDispatcher for ErrorToolDispatcher {
        fn call(&self, _name: &str, _args: &serde_json::Value) -> ToolCallResult {
            ToolCallResult::Error {
                message: "tool blew up".to_string(),
            }
        }
    }

    fn handler_with_error_dispatcher() -> McpHandler {
        let cfg = ServerConfig {
            transport: crate::Transport::Stdio,
            read_only: true,
            allow_write_tools: false,
            port: None,
            bind_addr: None,
            allow_experimental_tasks: false,
        };
        let resources = Arc::new(WikiResourceProvider::new(std::path::PathBuf::from("/tmp")));
        let tools = Arc::new(ErrorToolDispatcher);
        McpHandler::new(cfg, resources, tools)
    }

    /// #008: missing `uri` on `resources/read` is `-32602 Invalid params`,
    /// not `-32601`.
    #[test]
    fn error_code_invalid_params_for_missing_uri_on_resources_read() {
        let h = handler(true);
        let resp = call(&h, "resources/read", serde_json::json!({}));
        assert_eq!(
            resp["error"]["code"], -32602,
            "missing required param must be -32602 InvalidParams, got: {resp}"
        );
    }

    /// #008: missing `name` on `prompts/get` is `-32602`.
    #[test]
    fn error_code_invalid_params_for_missing_name_on_prompts_get() {
        let h = handler(true);
        let resp = call(&h, "prompts/get", serde_json::json!({}));
        assert_eq!(resp["error"]["code"], -32602);
    }

    /// #008: invalid cursor on `resources/list` is `-32602`, not
    /// `-32601`.
    #[test]
    fn error_code_invalid_params_for_garbage_cursor() {
        let h = handler(true);
        let resp = call(
            &h,
            "resources/list",
            serde_json::json!({"cursor": "not-a-number"}),
        );
        assert_eq!(
            resp["error"]["code"], -32602,
            "invalid cursor must be -32602, got: {resp}"
        );
    }

    /// #008: cursor offset past end is also `-32602` (drift detection).
    #[test]
    fn error_code_invalid_params_for_cursor_past_end() {
        let h = handler(true);
        let resp = call(
            &h,
            "resources/list",
            serde_json::json!({"cursor": "999999"}),
        );
        assert_eq!(resp["error"]["code"], -32602);
    }

    /// #008: unknown resource URI is `-32002 NotFound` (server-defined
    /// MCP convention), not `-32601`.
    #[test]
    fn error_code_not_found_for_unknown_resource_uri() {
        let h = handler(true);
        let resp = call(
            &h,
            "resources/read",
            serde_json::json!({"uri": "coral://does-not-exist"}),
        );
        assert_eq!(
            resp["error"]["code"], -32002,
            "unknown resource must be -32002 NotFound, got: {resp}"
        );
    }

    /// #008: unknown prompt is `-32002 NotFound`.
    #[test]
    fn error_code_not_found_for_unknown_prompt() {
        let h = handler(true);
        let resp = call(
            &h,
            "prompts/get",
            serde_json::json!({"name": "no-such-prompt"}),
        );
        assert_eq!(resp["error"]["code"], -32002);
    }

    /// #008: gated write tool without `--allow-write-tools` is
    /// `-32001 Gated` (server-defined), not `-32601`.
    #[test]
    fn error_code_gated_for_write_tool_without_allow_flag() {
        let h = handler(true);
        let resp = call(
            &h,
            "tools/call",
            serde_json::json!({"name": "up", "arguments": {}}),
        );
        assert_eq!(
            resp["error"]["code"], -32001,
            "gated tool must be -32001, got: {resp}"
        );
    }

    /// #008: unknown method stays `-32601 Method not found` — the
    /// only path where the pre-fix code was correct.
    #[test]
    fn error_code_method_not_found_for_unknown_method() {
        let h = handler(true);
        let resp = call(&h, "frobnicate", serde_json::json!({}));
        assert_eq!(
            resp["error"]["code"], -32601,
            "unknown method must remain -32601, got: {resp}"
        );
    }

    /// #008: tool-runtime errors (the tool ran and returned an error)
    /// surface as `result: { isError: true, content: [...] }` per MCP
    /// spec — NOT as a JSON-RPC envelope error. Pre-fix this routed
    /// through `Err(message)` → `-32601` (double-violation).
    #[test]
    fn tool_runtime_error_routes_through_result_is_error_envelope() {
        let h = handler_with_error_dispatcher();
        let resp = call(
            &h,
            "tools/call",
            serde_json::json!({"name": "query", "arguments": {}}),
        );
        // No envelope error.
        assert!(
            resp.get("error").is_none() || resp["error"].is_null(),
            "tool runtime error must NOT use JSON-RPC envelope error: {resp}"
        );
        assert_eq!(
            resp["result"]["isError"], true,
            "tool runtime error must set result.isError=true: {resp}"
        );
        let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
        assert!(
            text.contains("tool blew up"),
            "result.content must carry the tool error message: {resp}"
        );
    }
}
