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

use std::sync::OnceLock;

use coral_core::page::Page;
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
    /// Read the body of one resource by URI plus the MIME type the
    /// server should advertise to the client. `None` when the URI
    /// doesn't match any known resource.
    ///
    /// v0.19.6 audit C1: the second tuple field replaces a previous
    /// hardcoded `text/markdown` in `server.rs`. JSON resources
    /// (`coral://manifest`, `coral://lock`, `coral://stats`, etc.)
    /// declare `application/json` in their `Resource` catalog entry —
    /// the dispatcher MUST forward that hint to the client so it
    /// parses the body correctly instead of treating every payload as
    /// markdown.
    fn read(&self, uri: &str) -> Option<(String, String)>;
}

/// Production implementation: reads the wiki + manifest + lock off
/// disk on every call. v0.19.5 wired the `read()` body materialization
/// (previously `None`); per-page resources are now enumerated by
/// `list()` so agents can `resources/read` an exact slug.
pub struct WikiResourceProvider {
    pub project_root: std::path::PathBuf,
    /// v0.20.2 audit-followup #37: when `false` (default), pages whose
    /// frontmatter declares `reviewed: false` AND carry a populated
    /// `source.runner` field — i.e. LLM-distilled output that no
    /// human has signed off on — are filtered out of every resource
    /// listing AND made unreadable via `coral://wiki/<repo>/<slug>`.
    /// Mirrors the v0.20.1 H2 lint qualifier exactly.
    ///
    /// Set to `true` only via `coral mcp serve --include-unreviewed`,
    /// which is intended for users debugging distill flows.
    pub include_unreviewed: bool,
    /// v0.24: in-memory page cache. The MCP server is long-lived, so
    /// we load pages from disk once and serve from memory on all
    /// subsequent requests. Avoids ~200ms filesystem walks per request
    /// for large wikis.
    pages_cache: OnceLock<Vec<Page>>,
}

impl WikiResourceProvider {
    /// Build a default-deny provider: unreviewed distilled pages are
    /// hidden from `resources/list` and unreadable via
    /// `resources/read`. Use [`Self::with_include_unreviewed`] to opt
    /// into surfacing them.
    pub fn new(project_root: std::path::PathBuf) -> Self {
        Self {
            project_root,
            include_unreviewed: false,
            pages_cache: OnceLock::new(),
        }
    }

    /// Builder-style opt-in to surface `reviewed: false` distilled
    /// pages. Used by `coral mcp serve --include-unreviewed`.
    ///
    /// v0.20.2 audit-followup #37.
    pub fn with_include_unreviewed(mut self, include: bool) -> Self {
        self.include_unreviewed = include;
        self
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

    /// Read pages from the wiki root, returning an empty slice if the
    /// root doesn't exist (a freshly-initialized project).
    ///
    /// v0.24: pages are loaded once on first access and cached in
    /// `self.pages_cache` for the lifetime of the provider. This
    /// eliminates repeated filesystem walks (~200ms each for large
    /// wikis) on every MCP request.
    ///
    /// v0.20.2 audit-followup #37: when `include_unreviewed` is
    /// false, pages flagged by [`is_unreviewed_distilled`] are
    /// filtered out HERE — at the source — so every downstream
    /// renderer (`render_page`, `render_aggregate_index`,
    /// `render_repo_index`, the per-page enumeration in `list`)
    /// inherits the same qualifier with no extra filter calls.
    fn read_pages(&self) -> &[Page] {
        self.pages_cache.get_or_init(|| {
            let root = self.wiki_root();
            if !root.exists() {
                return Vec::new();
            }
            let pages = coral_core::walk::read_pages(&root).unwrap_or_default();
            if self.include_unreviewed {
                return pages;
            }
            pages
                .into_iter()
                .filter(|p| !p.is_unreviewed_distilled())
                .collect()
        })
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
        let report = coral_stats::StatsReport::new(pages);
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
        // v0.19.6 audit N4: validate the `<repo>` segment before it
        // ever gets reflected back in the response. Without this an
        // attacker who controls the URI (e.g. via an MCP client
        // they've coaxed Claude into invoking) could embed arbitrary
        // text in the URI's repo segment and have it echoed back as
        // the `repo` field — useful as part of a chained injection.
        // `_default` is the legacy single-repo wildcard and stays
        // allowlisted explicitly.
        if repo != "_default" && !coral_core::slug::is_safe_filename_slug(repo) {
            return None;
        }
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

// v0.20.2 audit-followup #37: the `reviewed: false` distilled-page
// qualifier lives in `coral_core::page::Page::is_unreviewed_distilled`
// so the MCP filter and the v0.20.1 H2 lint
// (`coral_lint::structural::check_unreviewed_distilled`) share one
// implementation. If this qualifier evolves, both call sites
// inherit the change.

impl ResourceProvider for WikiResourceProvider {
    fn list(&self) -> Vec<Resource> {
        let mut out = Self::static_catalog();
        // Append per-page resources so agents can `resources/read`
        // a specific slug directly. v0.19.5 audit: previously empty.
        // v0.19.6 audit C1: tag as `application/json` so it matches
        // the actual envelope body shape `render_page` returns
        // (slug + frontmatter + body fields). Earlier `text/markdown`
        // hint was inconsistent with the JSON-encoded payload and
        // confused clients trying to parse the response.
        let pages = self.read_pages();
        for p in pages {
            out.push(Resource {
                uri: format!("coral://wiki/{}", p.frontmatter.slug),
                name: p.frontmatter.slug.clone(),
                description: format!("{:?} page", p.frontmatter.page_type),
                mime_type: "application/json".into(),
            });
        }
        out
    }

    fn read(&self, uri: &str) -> Option<(String, String)> {
        // v0.19.5 audit C1: previous wave 1 stub returned None for
        // every URI, which broke every MCP client trying to actually
        // consume the resources. The dispatch below is data-driven —
        // each URI maps to a single render_* helper.
        //
        // v0.19.6 audit C1: returns `(body, mime_type)`. The mime
        // type is sourced ONCE from the same catalog `list()`
        // advertises, so `resources/list` and `resources/read` can
        // never disagree — the previous server was hardcoding
        // `text/markdown` for every read, silently relabeling
        // `application/json` resources as markdown.
        //
        // Each render_* helper returns just the body string; the
        // mime type is looked up via `mime_for_uri` below. Per-page
        // URIs aren't in the static catalog (they're discovered at
        // list-time); their JSON envelope payload is tagged
        // `application/json` to match what `render_page` actually
        // returns.
        let body = match uri {
            "coral://manifest" => self.render_manifest()?,
            "coral://lock" => self.render_lock()?,
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
                serde_json::json!({ "nodes": nodes }).to_string()
            }
            "coral://wiki/_index" => self.render_aggregate_index()?,
            "coral://stats" => self.render_stats()?,
            "coral://test-report/latest" => {
                let report_path = self.project_root.join(".coral").join("test-report.json");
                std::fs::read_to_string(&report_path)
                    .ok()
                    .unwrap_or_else(|| {
                        serde_json::json!({"status": "no test report yet"}).to_string()
                    })
            }
            other => {
                // `coral://wiki/<rest>` — rest may be `_index`, a
                // bare slug, or `<repo>/<slug>` etc.
                let rest = other.strip_prefix("coral://wiki/")?;
                if let Some(repo) = rest.strip_suffix("/_index") {
                    return Some((
                        self.render_repo_index(repo)?,
                        "application/json".to_string(),
                    ));
                }
                // Per-page payload is the JSON envelope built in
                // `render_page` (slug + frontmatter + body). It IS
                // JSON, so we tag it as such — clients that want the
                // raw markdown extract the `body` field.
                return Some((self.render_page(rest)?, "application/json".to_string()));
            }
        };
        // Look up the catalog entry for the canonical URIs above so
        // `resources/list` and `resources/read` agree on mimeType.
        let mime = Self::static_catalog()
            .into_iter()
            .find(|r| r.uri == uri)
            .map(|r| r.mime_type)
            .unwrap_or_else(|| "application/json".to_string());
        Some((body, mime))
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
