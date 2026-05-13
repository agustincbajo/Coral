//! `GET /api/v1/pages` (list) and `GET /api/v1/pages/:repo/:slug` (single).
//!
//! The list endpoint applies the documented query-string filters
//! (`q`, `page_type`, `status`, `confidence_min`, `confidence_max`,
//! `repo`, `valid_at`, `limit`, `offset`) and returns a JSON envelope
//! with `data: [...]` and `meta: {total, limit, offset, next_offset}`.
//!
//! The single-page endpoint validates `repo` and `slug` through
//! `coral_core::slug::is_safe_*` before any I/O happens, then loads the
//! requested page, computes inbound backlinks across the rest of the
//! wiki, and returns frontmatter + body + backlinks.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use coral_core::frontmatter::{PageType, Status};
use coral_core::page::Page;
use coral_core::slug::{is_safe_filename_slug, is_safe_repo_name};
use coral_core::walk::read_pages;
use serde::Serialize;

use crate::error::ApiError;
use crate::state::AppState;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;

#[derive(Serialize)]
struct PageSummary {
    slug: String,
    page_type: String,
    status: String,
    confidence: f64,
    generated_at: Option<String>,
    valid_from: Option<String>,
    valid_to: Option<String>,
    backlinks_count: usize,
    sources_count: usize,
    path: String,
}

#[derive(Serialize)]
struct ListMeta {
    total: usize,
    limit: usize,
    offset: usize,
    next_offset: Option<usize>,
}

#[derive(Serialize)]
struct ListEnvelope {
    data: Vec<PageSummary>,
    meta: ListMeta,
}

pub fn list(state: &Arc<AppState>, query_string: &str) -> Result<Vec<u8>, ApiError> {
    let params = parse_query(query_string);
    let q = params.get("q").map(|s| s.as_str());
    let page_types = parse_page_types(params.get("page_type").map(|s| s.as_str()))?;
    let statuses = parse_statuses(params.get("status").map(|s| s.as_str()))?;
    let conf_min = parse_f64(params.get("confidence_min"))?;
    let conf_max = parse_f64(params.get("confidence_max"))?;
    let valid_at = params.get("valid_at").cloned();
    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_LIMIT)
        .min(MAX_LIMIT);
    let offset = params
        .get("offset")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    // NOTE(coral-ui spec): the `repo` filter is currently a no-op because
    // v0.32 ships single-repo only. We still validate the value through
    // `is_safe_repo_name` so a future multi-repo refactor doesn't change
    // the API surface; today any string but the single repo simply
    // returns the full list.
    if let Some(repo) = params.get("repo")
        && !is_safe_repo_name(repo)
    {
        return Err(ApiError::InvalidFilter(format!(
            "repo: {repo:?} not allowed"
        )));
    }

    let pages = read_pages(&state.wiki_root).map_err(|e| anyhow::anyhow!(e))?;

    let mut filtered: Vec<&Page> = pages
        .iter()
        .filter(|p| {
            if let Some(qs) = q {
                let needle = qs.to_lowercase();
                let hay_slug = p.frontmatter.slug.to_lowercase();
                let hay_body = p.body.to_lowercase();
                if !hay_slug.contains(&needle) && !hay_body.contains(&needle) {
                    return false;
                }
            }
            if !page_types.is_empty() && !page_types.contains(&p.frontmatter.page_type) {
                return false;
            }
            if !statuses.is_empty() && !statuses.contains(&p.frontmatter.status) {
                return false;
            }
            if let Some(min) = conf_min
                && p.frontmatter.confidence.as_f64() < min
            {
                return false;
            }
            if let Some(max) = conf_max
                && p.frontmatter.confidence.as_f64() > max
            {
                return false;
            }
            if let Some(at) = &valid_at
                && !p.frontmatter.is_valid_at(at)
            {
                return false;
            }
            true
        })
        .collect();

    filtered.sort_by(|a, b| a.frontmatter.slug.cmp(&b.frontmatter.slug));

    let total = filtered.len();
    let next_offset = if offset + limit < total {
        Some(offset + limit)
    } else {
        None
    };

    let data: Vec<PageSummary> = filtered
        .iter()
        .skip(offset)
        .take(limit)
        .map(|p| PageSummary {
            slug: p.frontmatter.slug.clone(),
            page_type: page_type_str(p.frontmatter.page_type).to_string(),
            status: status_str(p.frontmatter.status).to_string(),
            confidence: p.frontmatter.confidence.as_f64(),
            generated_at: p.frontmatter.generated_at.clone(),
            valid_from: p.frontmatter.valid_from.clone(),
            valid_to: p.frontmatter.valid_to.clone(),
            backlinks_count: p.frontmatter.backlinks.len(),
            sources_count: p.frontmatter.sources.len(),
            path: p.path.to_string_lossy().to_string(),
        })
        .collect();

    let env = ListEnvelope {
        data,
        meta: ListMeta {
            total,
            limit,
            offset,
            next_offset,
        },
    };
    serde_json::to_vec(&env).map_err(|e| anyhow::anyhow!(e).into())
}

#[derive(Serialize)]
struct SingleEnvelope {
    data: SinglePage,
}

#[derive(Serialize)]
struct SinglePage {
    frontmatter: serde_json::Value,
    body: String,
    backlinks: Vec<String>,
}

/// `tail` is everything after `/api/v1/pages/`; expected shape is
/// `<repo>/<slug>`.
pub fn single(state: &Arc<AppState>, tail: &str) -> Result<Vec<u8>, ApiError> {
    let parts: Vec<&str> = tail.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(ApiError::InvalidFilter(
            "expected /api/v1/pages/<repo>/<slug>".into(),
        ));
    }
    let repo = parts[0];
    let slug = parts[1];
    if !is_safe_repo_name(repo) {
        return Err(ApiError::InvalidFilter(format!("repo {repo:?}")));
    }
    if !is_safe_filename_slug(slug) {
        return Err(ApiError::InvalidFilter(format!("slug {slug:?}")));
    }

    let pages = read_pages(&state.wiki_root).map_err(|e| anyhow::anyhow!(e))?;
    let Some(page) = pages.iter().find(|p| p.frontmatter.slug == slug) else {
        return Err(ApiError::NotFound(format!("page {slug:?}")));
    };

    // Inbound backlinks: any other page whose outbound links include this slug.
    let mut backlinks = HashSet::new();
    for other in &pages {
        if other.frontmatter.slug == slug {
            continue;
        }
        for link in other.outbound_links() {
            if link == slug {
                backlinks.insert(other.frontmatter.slug.clone());
                break;
            }
        }
    }
    let mut backlinks: Vec<String> = backlinks.into_iter().collect();
    backlinks.sort();

    let frontmatter_json =
        serde_json::to_value(&page.frontmatter).map_err(|e| anyhow::anyhow!(e))?;
    let env = SingleEnvelope {
        data: SinglePage {
            frontmatter: frontmatter_json,
            body: page.body.clone(),
            backlinks,
        },
    };
    serde_json::to_vec(&env).map_err(|e| anyhow::anyhow!(e).into())
}

// ── Query-string parsing helpers ─────────────────────────────────────────

/// Minimal `application/x-www-form-urlencoded` parser. Decodes `+` to
/// space and `%XX` hex escapes. Sufficient for our filter parameters;
/// we deliberately avoid pulling in a urlencoding crate.
pub(crate) fn parse_query(qs: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if qs.is_empty() {
        return out;
    }
    for pair in qs.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k.to_string(), v.to_string()),
            None => (pair.to_string(), String::new()),
        };
        out.insert(percent_decode(&k), percent_decode(&v));
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'+' {
            out.push(b' ');
            i += 1;
        } else if b == b'%' && i + 2 < bytes.len() {
            let h = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok());
            if let Some(decoded) = h {
                out.push(decoded);
                i += 3;
            } else {
                out.push(b);
                i += 1;
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_f64(value: Option<&String>) -> Result<Option<f64>, ApiError> {
    let Some(v) = value else {
        return Ok(None);
    };
    v.parse::<f64>()
        .map(Some)
        .map_err(|_| ApiError::InvalidFilter(format!("not a number: {v:?}")))
}

fn parse_page_types(csv: Option<&str>) -> Result<Vec<PageType>, ApiError> {
    let Some(csv) = csv else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    for tok in csv.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let pt = page_type_from_str(tok)
            .ok_or_else(|| ApiError::InvalidFilter(format!("page_type {tok:?}")))?;
        out.push(pt);
    }
    Ok(out)
}

fn parse_statuses(csv: Option<&str>) -> Result<Vec<Status>, ApiError> {
    let Some(csv) = csv else {
        return Ok(vec![]);
    };
    let mut out = Vec::new();
    for tok in csv.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let st = status_from_str(tok)
            .ok_or_else(|| ApiError::InvalidFilter(format!("status {tok:?}")))?;
        out.push(st);
    }
    Ok(out)
}

// PageType / Status string conversions. PageType derives serde
// rename_all = "snake_case", but the public surface should accept
// only those snake_case names — so we have an explicit map. Adding
// a new variant requires updating both directions.
fn page_type_from_str(s: &str) -> Option<PageType> {
    Some(match s {
        "module" => PageType::Module,
        "concept" => PageType::Concept,
        "entity" => PageType::Entity,
        "flow" => PageType::Flow,
        "decision" => PageType::Decision,
        "synthesis" => PageType::Synthesis,
        "operation" => PageType::Operation,
        "source" => PageType::Source,
        "gap" => PageType::Gap,
        "index" => PageType::Index,
        "log" => PageType::Log,
        "schema" => PageType::Schema,
        "readme" => PageType::Readme,
        "reference" => PageType::Reference,
        "interface" => PageType::Interface,
        _ => return None,
    })
}

pub(crate) fn page_type_str(pt: PageType) -> &'static str {
    match pt {
        PageType::Module => "module",
        PageType::Concept => "concept",
        PageType::Entity => "entity",
        PageType::Flow => "flow",
        PageType::Decision => "decision",
        PageType::Synthesis => "synthesis",
        PageType::Operation => "operation",
        PageType::Source => "source",
        PageType::Gap => "gap",
        PageType::Index => "index",
        PageType::Log => "log",
        PageType::Schema => "schema",
        PageType::Readme => "readme",
        PageType::Reference => "reference",
        PageType::Interface => "interface",
    }
}

fn status_from_str(s: &str) -> Option<Status> {
    Some(match s {
        "draft" => Status::Draft,
        "reviewed" => Status::Reviewed,
        "verified" => Status::Verified,
        "stale" => Status::Stale,
        "archived" => Status::Archived,
        "reference" => Status::Reference,
        _ => return None,
    })
}

pub(crate) fn status_str(st: Status) -> &'static str {
    match st {
        Status::Draft => "draft",
        Status::Reviewed => "reviewed",
        Status::Verified => "verified",
        Status::Stale => "stale",
        Status::Archived => "archived",
        Status::Reference => "reference",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_page(dir: &std::path::Path, slug: &str, body: &str) {
        let content = format!(
            "---\nslug: {slug}\ntype: module\nlast_updated_commit: abc\nconfidence: 0.8\nstatus: draft\n---\n\n{body}\n"
        );
        let path = dir.join(format!("{slug}.md"));
        std::fs::write(path, content).unwrap();
    }

    fn state_with_wiki(root: PathBuf) -> Arc<AppState> {
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
    fn parse_query_decodes_percent_and_plus() {
        let q = parse_query("q=hello+world&page_type=module%2Cflow");
        assert_eq!(q.get("q").unwrap(), "hello world");
        assert_eq!(q.get("page_type").unwrap(), "module,flow");
    }

    #[test]
    fn list_returns_envelope_with_meta() {
        let tmp = TempDir::new().unwrap();
        write_page(tmp.path(), "alpha", "alpha body");
        write_page(tmp.path(), "beta", "beta body");

        let s = state_with_wiki(tmp.path().to_path_buf());
        let body = list(&s, "").unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v["data"].is_array());
        assert_eq!(v["data"].as_array().unwrap().len(), 2);
        assert_eq!(v["meta"]["total"], 2);
        assert_eq!(v["meta"]["offset"], 0);
    }

    #[test]
    fn list_filters_by_q() {
        let tmp = TempDir::new().unwrap();
        write_page(tmp.path(), "alpha", "talks about widgets");
        write_page(tmp.path(), "beta", "talks about gizmos");
        let s = state_with_wiki(tmp.path().to_path_buf());
        let body = list(&s, "q=widget").unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = v["data"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["slug"], "alpha");
    }

    #[test]
    fn single_rejects_bad_repo() {
        let s = state_with_wiki(PathBuf::from("does-not-exist"));
        let err = single(&s, "../escape/slug").unwrap_err();
        assert_eq!(err.code(), "INVALID_FILTER");
    }

    #[test]
    fn single_rejects_bad_slug() {
        let s = state_with_wiki(PathBuf::from("does-not-exist"));
        let err = single(&s, "default/../escape").unwrap_err();
        assert_eq!(err.code(), "INVALID_FILTER");
    }

    #[test]
    fn single_returns_frontmatter_and_body() {
        let tmp = TempDir::new().unwrap();
        write_page(tmp.path(), "alpha", "alpha body links to [[beta]]");
        write_page(tmp.path(), "beta", "beta body");
        let s = state_with_wiki(tmp.path().to_path_buf());
        let body = single(&s, "default/beta").unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["data"]["frontmatter"]["slug"], "beta");
        // alpha links to beta, so beta should have alpha as inbound.
        let bl = v["data"]["backlinks"].as_array().unwrap();
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0], "alpha");
    }

    #[test]
    fn _references_use_unused_helpers() {
        // Keep `Frontmatter` import live for future shape assertions.
        let _ = std::mem::size_of::<Frontmatter>();
        let _ = Confidence::try_new(0.5).unwrap();
    }
}
