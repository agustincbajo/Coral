//! `GET /api/v1/search?q=...&limit=...` — TF-IDF search over wiki pages.

use std::sync::Arc;

use coral_core::search;
use coral_core::walk::read_pages;
use serde::Serialize;

use super::pages::parse_query;
use crate::error::ApiError;
use crate::state::AppState;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 200;

#[derive(Serialize)]
struct Hit {
    slug: String,
    score: f64,
    snippet: String,
}

#[derive(Serialize)]
struct Envelope {
    data: Vec<Hit>,
}

pub fn handle(state: &Arc<AppState>, query_string: &str) -> Result<Vec<u8>, ApiError> {
    let params = parse_query(query_string);
    let q = params
        .get("q")
        .ok_or_else(|| ApiError::InvalidFilter("missing `q` parameter".into()))?;
    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_LIMIT)
        .min(MAX_LIMIT);

    let pages = read_pages(&state.wiki_root).map_err(|e| anyhow::anyhow!(e))?;
    let results = search::search(&pages, q, limit);

    let hits: Vec<Hit> = results
        .into_iter()
        .map(|r| Hit {
            slug: r.slug,
            score: r.score,
            snippet: r.snippet,
        })
        .collect();

    serde_json::to_vec(&Envelope { data: hits }).map_err(|e| anyhow::anyhow!(e).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_page(dir: &std::path::Path, slug: &str, body: &str) {
        let content = format!(
            "---\nslug: {slug}\ntype: module\nlast_updated_commit: abc\nconfidence: 0.8\nstatus: draft\n---\n\n{body}\n"
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
    fn search_requires_q() {
        let tmp = TempDir::new().unwrap();
        write_page(tmp.path(), "alpha", "body");
        let s = state(tmp.path().to_path_buf());
        let err = handle(&s, "").unwrap_err();
        assert_eq!(err.code(), "INVALID_FILTER");
    }

    #[test]
    fn search_returns_envelope() {
        let tmp = TempDir::new().unwrap();
        write_page(tmp.path(), "alpha", "widgets and gadgets together");
        write_page(
            tmp.path(),
            "beta",
            "completely different topic about plumbing",
        );
        let s = state(tmp.path().to_path_buf());
        let body = handle(&s, "q=widgets").unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let hits = v["data"].as_array().unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0]["slug"], "alpha");
    }
}
