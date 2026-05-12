//! `GET /api/v1/interfaces` — list wiki pages whose `frontmatter.page_type`
//! is `Interface`.
//!
//! Returns `{data: [...], meta: {total: N}}`. Each entry carries the
//! minimum surface area the M2 contract dashboard needs: slug, repo
//! (currently always `"default"` because v0.32 is single-repo — the
//! field is reserved so the multi-repo refactor doesn't change the
//! response shape), status (lowercase snake_case), confidence, sources,
//! validity window, and inbound backlink count.
//!
//! The handler reads the full wiki via `read_pages`, then filters to
//! `PageType::Interface` in memory. v0.32 wikis are O(hundreds) of
//! pages so a linear scan is fine; if that ever changes we can swap
//! in a `coral_core::search_index`-backed lookup.

use std::sync::Arc;

use coral_core::frontmatter::PageType;
use coral_core::walk::read_pages;
use serde::Serialize;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Serialize)]
struct InterfaceEntry {
    slug: String,
    repo: String,
    status: String,
    confidence: f64,
    sources: Vec<String>,
    valid_from: Option<String>,
    valid_to: Option<String>,
    backlinks_count: usize,
}

pub fn handle(state: &Arc<AppState>) -> Result<Vec<u8>, ApiError> {
    let pages = read_pages(&state.wiki_root).map_err(|e| anyhow::anyhow!(e))?;
    let entries: Vec<InterfaceEntry> = pages
        .iter()
        .filter(|p| matches!(p.frontmatter.page_type, PageType::Interface))
        .map(|p| InterfaceEntry {
            slug: p.frontmatter.slug.clone(),
            repo: "default".into(),
            // Status doesn't have a `Display` impl, so we round-trip
            // through Debug + lowercase. Matches the snake_case the
            // rest of the API exposes for status filters.
            status: format!("{:?}", p.frontmatter.status).to_lowercase(),
            confidence: p.frontmatter.confidence.as_f64(),
            sources: p.frontmatter.sources.clone(),
            valid_from: p.frontmatter.valid_from.clone(),
            valid_to: p.frontmatter.valid_to.clone(),
            backlinks_count: p.frontmatter.backlinks.len(),
        })
        .collect();
    let total = entries.len();
    let body = serde_json::json!({"data": entries, "meta": {"total": total}});
    serde_json::to_vec(&body).map_err(|e| anyhow::anyhow!(e).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_page(dir: &std::path::Path, slug: &str, page_type: &str) {
        let content = format!(
            "---\nslug: {slug}\ntype: {page_type}\nlast_updated_commit: abc\nconfidence: 0.7\nstatus: reviewed\nsources:\n  - {slug}.md\n---\n\nbody for {slug}\n"
        );
        std::fs::write(dir.join(format!("{slug}.md")), content).unwrap();
    }

    fn state(root: PathBuf) -> Arc<AppState> {
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
    fn filters_to_interface_pages_only() {
        let tmp = TempDir::new().unwrap();
        write_page(tmp.path(), "users-api", "interface");
        write_page(tmp.path(), "auth", "module");
        write_page(tmp.path(), "orders-api", "interface");

        let s = state(tmp.path().to_path_buf());
        let body = handle(&s).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = v["data"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "only two interface pages expected");
        assert_eq!(v["meta"]["total"], 2);
        let mut slugs: Vec<String> = arr
            .iter()
            .map(|e| e["slug"].as_str().unwrap().to_string())
            .collect();
        slugs.sort();
        assert_eq!(slugs, vec!["orders-api", "users-api"]);
        // repo should default to "default" in M1 single-repo.
        for entry in arr {
            assert_eq!(entry["repo"], "default");
            assert_eq!(entry["status"], "reviewed");
        }
    }

    #[test]
    fn empty_when_no_interfaces() {
        let tmp = TempDir::new().unwrap();
        write_page(tmp.path(), "alpha", "module");
        let s = state(tmp.path().to_path_buf());
        let body = handle(&s).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["data"].as_array().unwrap().len(), 0);
        assert_eq!(v["meta"]["total"], 0);
    }
}
