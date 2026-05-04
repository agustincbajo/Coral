//! MCP `Resource` model and the `ResourceProvider` trait.
//!
//! In MCP, a resource is a URI-addressable piece of text the server
//! exposes for the client (the LLM agent) to read on demand. Coral's
//! resource catalog is a curated subset of the wiki + manifest:
//!
//! - `coral://wiki/<repo>/<slug>` — markdown raw + frontmatter parsed
//! - `coral://wiki/<repo>/_index` — listing of slugs in a repo
//! - `coral://wiki/_index` — aggregated listing across repos
//! - `coral://manifest` — coral.toml as JSON
//! - `coral://lock` — coral.lock as JSON
//! - `coral://stats` — `coral stats --format json`

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    pub uri: String,
    pub name: String,
    pub description: String,
    // v0.19.5 audit: MCP wire format uses camelCase (`mimeType`).
    // Without the rename `resources/list` would emit `mime_type`,
    // which clients silently ignore — they then fall back to
    // `text/plain` which loses our application/json hint.
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

/// The trait the MCP server delegates to. Lives behind a trait so the
/// CLI can swap in a `MockResourceProvider` for tests without
/// standing up a real wiki.
pub trait ResourceProvider: Send + Sync {
    /// List all resources the server is willing to expose.
    fn list(&self) -> Vec<Resource>;
    /// Read the body of one resource by URI. `None` when the URI
    /// doesn't match any known resource.
    fn read(&self, uri: &str) -> Option<String>;
}

/// Production implementation: reads the wiki + manifest + lock off
/// disk on every call. v0.19.5 wired the `read()` body materialization
/// (previously `None`); per-page resources are now enumerated by
/// `list()` so agents can `resources/read` an exact slug.
pub struct WikiResourceProvider {
    pub project_root: std::path::PathBuf,
}

impl WikiResourceProvider {
    pub fn new(project_root: std::path::PathBuf) -> Self {
        Self { project_root }
    }

    /// The static catalog that's always exposed — the per-page
    /// resources are listed dynamically by `list()` so only existing
    /// pages show up.
    pub fn static_catalog() -> Vec<Resource> {
        vec![
            Resource {
                uri: "coral://manifest".into(),
                name: "Project manifest".into(),
                description: "coral.toml parsed to JSON".into(),
                mime_type: "application/json".into(),
            },
            Resource {
                uri: "coral://lock".into(),
                name: "Project lockfile".into(),
                description: "coral.lock parsed to JSON".into(),
                mime_type: "application/json".into(),
            },
            Resource {
                uri: "coral://graph".into(),
                name: "Repo dependency graph".into(),
                description: "[[repos]] depends_on graph as JSON".into(),
                mime_type: "application/json".into(),
            },
            Resource {
                uri: "coral://wiki/_index".into(),
                name: "Wiki index (aggregated)".into(),
                description: "All wiki slugs across all repos".into(),
                mime_type: "application/json".into(),
            },
            Resource {
                uri: "coral://stats".into(),
                name: "Wiki health stats".into(),
                description: "`coral stats --format json` output".into(),
                mime_type: "application/json".into(),
            },
            Resource {
                uri: "coral://test-report/latest".into(),
                name: "Last test report".into(),
                description: "Most recent `coral test` run (JUnit + JSON)".into(),
                mime_type: "application/json".into(),
            },
        ]
    }
}

impl WikiResourceProvider {
    fn wiki_root(&self) -> std::path::PathBuf {
        self.project_root.join(".wiki")
    }

    /// Read pages from the wiki root, returning an empty vec if the
    /// root doesn't exist (a freshly-initialized project).
    fn read_pages(&self) -> Vec<coral_core::page::Page> {
        let root = self.wiki_root();
        if !root.exists() {
            return Vec::new();
        }
        coral_core::walk::read_pages(&root).unwrap_or_default()
    }

    /// Render `coral://manifest` as JSON.
    fn render_manifest(&self) -> Option<String> {
        let manifest_path = self.project_root.join("coral.toml");
        if !manifest_path.exists() {
            // No manifest → synthesize legacy + emit summary so v0.15
            // single-repo users still see something useful.
            return Some(
                serde_json::json!({
                    "kind": "legacy_single_repo",
                    "root": self.project_root.display().to_string()
                })
                .to_string(),
            );
        }
        let project = coral_core::project::Project::load_from_manifest(&manifest_path).ok()?;
        let summary = serde_json::json!({
            "api_version": project.api_version,
            "name": project.name,
            "root": project.root.display().to_string(),
            "is_multi_repo": project.is_multi_repo(),
            "repos": project.repos.iter().map(|r| serde_json::json!({
                "name": r.name,
                "url": r.url,
                "path": r.path.as_ref().map(|p| p.display().to_string()),
                "tags": r.tags,
                "depends_on": r.depends_on,
            })).collect::<Vec<_>>()
        });
        Some(summary.to_string())
    }

    fn render_lock(&self) -> Option<String> {
        let lock_path = self.project_root.join("coral.lock");
        if !lock_path.exists() {
            return Some(serde_json::json!({"repos": []}).to_string());
        }
        let raw = std::fs::read_to_string(&lock_path).ok()?;
        let parsed: toml::Value = toml::from_str(&raw).ok()?;
        // Re-serialize as JSON for client compatibility.
        serde_json::to_string(&parsed).ok()
    }

    fn render_stats(&self) -> Option<String> {
        let pages = self.read_pages();
        let report = coral_stats::StatsReport::new(&pages);
        report.as_json().ok()
    }

    fn render_aggregate_index(&self) -> Option<String> {
        let pages = self.read_pages();
        let entries: Vec<serde_json::Value> = pages
            .iter()
            .map(|p| {
                serde_json::json!({
                    "slug": p.frontmatter.slug,
                    "type": p.frontmatter.page_type,
                    "status": p.frontmatter.status,
                    "confidence": p.frontmatter.confidence.as_f64(),
                })
            })
            .collect();
        Some(serde_json::json!({ "pages": entries }).to_string())
    }

    fn render_repo_index(&self, repo: &str) -> Option<String> {
        let pages = self.read_pages();
        let prefix = format!("{repo}/");
        let entries: Vec<serde_json::Value> = pages
            .iter()
            .filter(|p| p.frontmatter.slug.starts_with(&prefix) || repo == "_default")
            .map(|p| {
                serde_json::json!({
                    "slug": p.frontmatter.slug,
                    "type": p.frontmatter.page_type,
                    "status": p.frontmatter.status,
                })
            })
            .collect();
        Some(serde_json::json!({ "repo": repo, "pages": entries }).to_string())
    }

    fn render_page(&self, slug: &str) -> Option<String> {
        // v0.19.5 audit C4: validate the slug before any path
        // interpolation. The slug arrives over MCP from an untrusted
        // agent — even the local Claude Code can be fed a poisoned
        // page. Rejecting unsafe slugs here is the last line of
        // defense before fs::read.
        if !slug_is_safe_segments(slug) {
            return None;
        }
        let pages = self.read_pages();
        let page = pages.iter().find(|p| p.frontmatter.slug == slug)?;
        let json = serde_json::json!({
            "slug": page.frontmatter.slug,
            "type": page.frontmatter.page_type,
            "status": page.frontmatter.status,
            "confidence": page.frontmatter.confidence.as_f64(),
            "last_updated_commit": page.frontmatter.last_updated_commit,
            "sources": page.frontmatter.sources,
            "backlinks": page.frontmatter.backlinks,
            "body": page.body,
        });
        Some(json.to_string())
    }
}

/// Helper for `coral://wiki/<repo>/<slug>` segments. The slug may be
/// either bare (`order`) or namespaced (`api/order`); we run each
/// segment through the safe-filename allowlist so an attacker can't
/// sneak `..` into either side.
fn slug_is_safe_segments(slug: &str) -> bool {
    if slug.is_empty() {
        return false;
    }
    slug.split('/').all(coral_core::slug::is_safe_filename_slug)
}

impl ResourceProvider for WikiResourceProvider {
    fn list(&self) -> Vec<Resource> {
        let mut out = Self::static_catalog();
        // Append per-page resources so agents can `resources/read`
        // a specific slug directly. v0.19.5 audit: previously empty.
        let pages = self.read_pages();
        for p in &pages {
            out.push(Resource {
                uri: format!("coral://wiki/{}", p.frontmatter.slug),
                name: p.frontmatter.slug.clone(),
                description: format!("{:?} page", p.frontmatter.page_type),
                mime_type: "text/markdown".into(),
            });
        }
        out
    }

    fn read(&self, uri: &str) -> Option<String> {
        // v0.19.5 audit C1: previous wave 1 stub returned None for
        // every URI, which broke every MCP client trying to actually
        // consume the resources. The dispatch below is data-driven —
        // each URI maps to a single render_* helper.
        match uri {
            "coral://manifest" => self.render_manifest(),
            "coral://lock" => self.render_lock(),
            "coral://graph" => {
                // The graph is a thin slice of the manifest; lift it
                // out so consumers don't have to re-deserialize.
                let manifest_path = self.project_root.join("coral.toml");
                let project = if manifest_path.exists() {
                    coral_core::project::Project::load_from_manifest(&manifest_path).ok()?
                } else {
                    coral_core::project::Project::synthesize_legacy(&self.project_root)
                };
                let nodes: Vec<serde_json::Value> = project
                    .repos
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "name": r.name,
                            "depends_on": r.depends_on,
                        })
                    })
                    .collect();
                Some(serde_json::json!({ "nodes": nodes }).to_string())
            }
            "coral://wiki/_index" => self.render_aggregate_index(),
            "coral://stats" => self.render_stats(),
            "coral://test-report/latest" => {
                let report_path = self.project_root.join(".coral").join("test-report.json");
                std::fs::read_to_string(&report_path).ok().or_else(|| {
                    Some(serde_json::json!({"status": "no test report yet"}).to_string())
                })
            }
            other => {
                // `coral://wiki/<rest>` — rest may be `_index`, a
                // bare slug, or `<repo>/<slug>` etc.
                let rest = other.strip_prefix("coral://wiki/")?;
                if let Some(repo) = rest.strip_suffix("/_index") {
                    return self.render_repo_index(repo);
                }
                self.render_page(rest)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_catalog_includes_six_canonical_uris() {
        let cat = WikiResourceProvider::static_catalog();
        let uris: Vec<&str> = cat.iter().map(|r| r.uri.as_str()).collect();
        assert!(uris.contains(&"coral://manifest"));
        assert!(uris.contains(&"coral://lock"));
        assert!(uris.contains(&"coral://graph"));
        assert!(uris.contains(&"coral://wiki/_index"));
        assert!(uris.contains(&"coral://stats"));
        assert!(uris.contains(&"coral://test-report/latest"));
    }

    #[test]
    fn provider_list_returns_static_catalog_plus_pages() {
        let p = WikiResourceProvider::new(std::path::PathBuf::from("/tmp/coral-mcp-tests-empty"));
        // No wiki on disk — list() should at least include the static catalog.
        let n = p.list().len();
        assert!(n >= WikiResourceProvider::static_catalog().len());
    }

    /// v0.19.5 audit: serde rename to `mimeType`.
    #[test]
    fn resource_serializes_with_camelcase_mime_type() {
        let r = Resource {
            uri: "coral://manifest".into(),
            name: "n".into(),
            description: "d".into(),
            mime_type: "application/json".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"mimeType\""), "got: {json}");
        assert!(!json.contains("\"mime_type\""), "got: {json}");
    }

    /// v0.19.5 audit C4: rejecting path traversal in slug segments.
    #[test]
    fn slug_segments_reject_dot_dot() {
        assert!(!slug_is_safe_segments("../etc"));
        assert!(!slug_is_safe_segments("api/../etc"));
        assert!(!slug_is_safe_segments("api/.."));
        assert!(slug_is_safe_segments("api/order"));
        assert!(slug_is_safe_segments("order"));
    }
}
