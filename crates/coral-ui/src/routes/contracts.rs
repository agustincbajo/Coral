//! `GET /api/v1/contract_status` — surface the latest
//! `coral contract check` JSON reports.
//!
//! Reads every `*.json` in `<repo>/.coral/contracts/` (next to the wiki
//! root) and returns them as an array under `data`. The reports are
//! treated as opaque `serde_json::Value`s here — the dashboard knows
//! their internal shape. If the directory doesn't exist (no contract
//! checks ever ran), we return `{"data": [], "meta": {"total": 0}}`
//! rather than 404; M2's contract dashboard is always renderable.

use std::sync::Arc;

use crate::error::ApiError;
use crate::state::AppState;

pub fn handle(state: &Arc<AppState>) -> Result<Vec<u8>, ApiError> {
    // The repo root is the parent of `.wiki/` — same heuristic that
    // `coral` CLI uses to locate `.coral/`. If `wiki_root` happens to
    // be the filesystem root (no parent), fall back to it.
    let repo_root: std::path::PathBuf = state
        .wiki_root
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| state.wiki_root.clone());
    let contracts_dir = repo_root.join(".coral").join("contracts");

    let mut reports: Vec<serde_json::Value> = Vec::new();
    if contracts_dir.is_dir() {
        let entries = std::fs::read_dir(&contracts_dir).map_err(|e| anyhow::anyhow!(e))?;
        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| anyhow::anyhow!(e))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                paths.push(path);
            }
        }
        // Sort for deterministic order — easier to test and friendlier
        // for clients that don't sort themselves.
        paths.sort();
        for path in paths {
            let text = std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!(e))?;
            let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                anyhow::anyhow!("failed to parse contract report {}: {e}", path.display())
            })?;
            reports.push(v);
        }
    }
    let total = reports.len();
    let body = serde_json::json!({"data": reports, "meta": {"total": total}});
    serde_json::to_vec(&body).map_err(|e| anyhow::anyhow!(e).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn state(root: std::path::PathBuf) -> Arc<AppState> {
        Arc::new(AppState {
            bind: "127.0.0.1".into(),
            port: 3838,
            wiki_root: root,
            token: None,
            allow_write_tools: false,
            runner: None,
        })
    }

    #[test]
    fn missing_contracts_dir_returns_empty_data() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        // No .coral/contracts/ at all.
        let s = state(wiki);
        let body = handle(&s).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["data"].as_array().unwrap().len(), 0);
        assert_eq!(v["meta"]["total"], 0);
    }

    #[test]
    fn reads_all_json_reports() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        let contracts = tmp.path().join(".coral").join("contracts");
        std::fs::create_dir_all(&contracts).unwrap();
        std::fs::write(
            contracts.join("a-users.json"),
            r#"{"contract":"users","status":"green"}"#,
        )
        .unwrap();
        std::fs::write(
            contracts.join("b-orders.json"),
            r#"{"contract":"orders","status":"red"}"#,
        )
        .unwrap();
        // Non-json should be ignored.
        std::fs::write(contracts.join("readme.txt"), "ignored").unwrap();

        let s = state(wiki);
        let body = handle(&s).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = v["data"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        // Sorted by filename: a-users first.
        assert_eq!(arr[0]["contract"], "users");
        assert_eq!(arr[1]["contract"], "orders");
    }

    #[test]
    fn malformed_json_surfaces_internal_error() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        let contracts = tmp.path().join(".coral").join("contracts");
        std::fs::create_dir_all(&contracts).unwrap();
        std::fs::write(contracts.join("broken.json"), "not json{{{").unwrap();

        let s = state(wiki);
        let err = handle(&s).unwrap_err();
        assert_eq!(err.code(), "INTERNAL");
    }
}
