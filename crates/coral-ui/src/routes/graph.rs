//! `GET /api/v1/graph` — wikilink graph for visualization.
//!
//! Returns `{nodes:[…], edges:[…]}`. Nodes carry the metadata the SPA
//! needs to render colored bubbles (page_type, status, confidence,
//! degree, valid_from/valid_to). Edges are directed source→target
//! pairs derived from `Page::outbound_links()`.
//!
//! `max_nodes` (default 500) caps the response. `valid_at` (ISO-8601)
//! filters to pages valid at that timestamp.

use std::collections::HashMap;
use std::sync::Arc;

use coral_core::page::Page;
use coral_core::walk::read_pages;
use serde::Serialize;

use super::pages::parse_query;
use crate::error::ApiError;
use crate::state::AppState;

const DEFAULT_MAX_NODES: usize = 500;

#[derive(Serialize)]
struct Node {
    id: String,
    label: String,
    page_type: String,
    status: String,
    confidence: f64,
    degree: usize,
    valid_from: Option<String>,
    valid_to: Option<String>,
}

#[derive(Serialize)]
struct Edge {
    source: String,
    target: String,
}

#[derive(Serialize)]
struct GraphData {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

#[derive(Serialize)]
struct Envelope {
    data: GraphData,
}

pub fn handle(state: &Arc<AppState>, query_string: &str) -> Result<Vec<u8>, ApiError> {
    let params = parse_query(query_string);
    let max_nodes = params
        .get("max_nodes")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_NODES);
    let valid_at = params.get("valid_at").cloned();

    let pages = read_pages(&state.wiki_root).map_err(|e| anyhow::anyhow!(e))?;

    // Pre-filter by `valid_at`.
    let visible: Vec<&Page> = pages
        .iter()
        .filter(|p| match &valid_at {
            Some(at) => p.frontmatter.is_valid_at(at),
            None => true,
        })
        .collect();

    // Compute degrees in a single pass over outbound links. We count both
    // out- and in-degrees per node so the SPA can size bubbles by total
    // connectivity. Nodes referenced only via wikilinks (no own page) are
    // skipped — we keep the node set closed.
    let visible_slugs: std::collections::HashSet<String> =
        visible.iter().map(|p| p.frontmatter.slug.clone()).collect();
    let mut degree: HashMap<String, usize> = HashMap::new();
    let mut edges: Vec<Edge> = Vec::new();
    for p in &visible {
        let src = p.frontmatter.slug.clone();
        for tgt in p.outbound_links() {
            if !visible_slugs.contains(&tgt) {
                continue;
            }
            edges.push(Edge {
                source: src.clone(),
                target: tgt.clone(),
            });
            *degree.entry(src.clone()).or_insert(0) += 1;
            *degree.entry(tgt).or_insert(0) += 1;
        }
        degree.entry(src).or_insert(0);
    }

    let mut nodes: Vec<Node> = visible
        .iter()
        .map(|p| Node {
            id: p.frontmatter.slug.clone(),
            label: p.frontmatter.slug.clone(),
            page_type: super::pages::page_type_str(p.frontmatter.page_type).to_string(),
            status: super::pages::status_str(p.frontmatter.status).to_string(),
            confidence: p.frontmatter.confidence.as_f64(),
            degree: degree.get(&p.frontmatter.slug).copied().unwrap_or(0),
            valid_from: p.frontmatter.valid_from.clone(),
            valid_to: p.frontmatter.valid_to.clone(),
        })
        .collect();

    // Cap by degree (highest first) so the visualization always
    // surfaces the densest hubs first when truncated.
    if nodes.len() > max_nodes {
        nodes.sort_by(|a, b| b.degree.cmp(&a.degree));
        nodes.truncate(max_nodes);
        let kept: std::collections::HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
        edges.retain(|e| kept.contains(&e.source) && kept.contains(&e.target));
    }

    serde_json::to_vec(&Envelope {
        data: GraphData { nodes, edges },
    })
    .map_err(|e| anyhow::anyhow!(e).into())
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
    fn graph_builds_nodes_and_edges() {
        let tmp = TempDir::new().unwrap();
        write_page(tmp.path(), "alpha", "links to [[beta]] and [[gamma]]");
        write_page(tmp.path(), "beta", "body");
        write_page(tmp.path(), "gamma", "body");
        let s = state(tmp.path().to_path_buf());
        let body = handle(&s, "").unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let nodes = v["data"]["nodes"].as_array().unwrap();
        let edges = v["data"]["edges"].as_array().unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(edges.len(), 2);
    }
}
