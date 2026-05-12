//! `GET /api/v1/manifest`, `/lock`, `/stats` — read-only project metadata.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use coral_core::frontmatter::{PageType, Status};
use coral_core::walk::read_pages;
use serde::Serialize;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Serialize)]
struct Envelope<T: Serialize> {
    data: T,
}

/// Resolve a sibling-of-wiki-root file. v0.32: `wiki_root.parent()` is
/// the repo root. We tolerate `wiki_root` being relative; fall back to
/// CWD when there's no parent.
fn repo_file(state: &AppState, name: &str) -> PathBuf {
    state
        .wiki_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join(name)
}

pub fn manifest(state: &Arc<AppState>) -> Result<Vec<u8>, ApiError> {
    let path = repo_file(state, "coral.toml");
    read_toml_as_json(&path, "manifest").and_then(|v| {
        serde_json::to_vec(&Envelope { data: v }).map_err(|e| anyhow::anyhow!(e).into())
    })
}

pub fn lock(state: &Arc<AppState>) -> Result<Vec<u8>, ApiError> {
    let path = repo_file(state, "coral.lock");
    read_toml_as_json(&path, "lock").and_then(|v| {
        serde_json::to_vec(&Envelope { data: v }).map_err(|e| anyhow::anyhow!(e).into())
    })
}

fn read_toml_as_json(path: &Path, label: &str) -> Result<serde_json::Value, ApiError> {
    if !path.exists() {
        // Log the absolute path for operators; only the label leaves the
        // process. Avoids leaking filesystem layout via 404 messages.
        tracing::debug!(label = %label, path = %path.display(), "manifest file not found");
        return Err(ApiError::NotFound(label.to_string()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!(e))?;
    let value: toml::Value = toml::from_str(&text).map_err(|e| anyhow::anyhow!(e))?;
    // toml::Value implements Serialize, so we can hand it to serde_json
    // through an intermediate JSON value. Avoids hand-walking the TOML
    // tree to JSON.
    let json = serde_json::to_value(value).map_err(|e| anyhow::anyhow!(e))?;
    Ok(json)
}

#[derive(Serialize)]
struct Stats {
    page_count: usize,
    status_breakdown: HashMap<String, usize>,
    page_type_breakdown: HashMap<String, usize>,
    avg_confidence: f64,
    total_backlinks: usize,
}

pub fn stats(state: &Arc<AppState>) -> Result<Vec<u8>, ApiError> {
    let pages = read_pages(&state.wiki_root).map_err(|e| anyhow::anyhow!(e))?;
    let mut status_breakdown: HashMap<String, usize> = HashMap::new();
    let mut page_type_breakdown: HashMap<String, usize> = HashMap::new();
    let mut total_conf = 0.0_f64;
    let mut total_backlinks = 0usize;
    for p in &pages {
        *status_breakdown
            .entry(status_str_static(p.frontmatter.status).to_string())
            .or_insert(0) += 1;
        *page_type_breakdown
            .entry(page_type_str_static(p.frontmatter.page_type).to_string())
            .or_insert(0) += 1;
        total_conf += p.frontmatter.confidence.as_f64();
        total_backlinks += p.frontmatter.backlinks.len();
    }
    let avg_confidence = if pages.is_empty() {
        0.0
    } else {
        total_conf / pages.len() as f64
    };
    let env = Envelope {
        data: Stats {
            page_count: pages.len(),
            status_breakdown,
            page_type_breakdown,
            avg_confidence,
            total_backlinks,
        },
    };
    serde_json::to_vec(&env).map_err(|e| anyhow::anyhow!(e).into())
}

fn status_str_static(s: Status) -> &'static str {
    super::pages::status_str(s)
}
fn page_type_str_static(pt: PageType) -> &'static str {
    super::pages::page_type_str(pt)
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
    fn manifest_missing_returns_not_found() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir(&wiki).unwrap();
        let s = state(wiki);
        let err = manifest(&s).unwrap_err();
        assert_eq!(err.code(), "NOT_FOUND");
    }

    #[test]
    fn manifest_parses_toml() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir(&wiki).unwrap();
        std::fs::write(
            tmp.path().join("coral.toml"),
            "[project]\nname = \"demo\"\n",
        )
        .unwrap();
        let s = state(wiki);
        let body = manifest(&s).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["data"]["project"]["name"], "demo");
    }

    #[test]
    fn stats_aggregates_pages() {
        let tmp = TempDir::new().unwrap();
        write_page(tmp.path(), "alpha", "alpha");
        write_page(tmp.path(), "beta", "beta");
        let s = state(tmp.path().to_path_buf());
        let body = stats(&s).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["data"]["page_count"], 2);
        assert_eq!(v["data"]["status_breakdown"]["draft"], 2);
        assert_eq!(v["data"]["page_type_breakdown"]["module"], 2);
    }
}
