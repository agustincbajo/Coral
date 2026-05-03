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
//! - `coral://graph` — depends_on graph as JSON
//! - `coral://test-report/latest` — last JUnit/JSON test run
//! - `coral://stats` — `coral stats --format json`

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resource {
    pub uri: String,
    pub name: String,
    pub description: String,
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

/// The production implementation: reads the wiki + manifest + lock
/// and the last test report off disk. v0.19 wave 1 supplies the
/// catalog (`list()`); the `read()` body materialization lands in
/// wave 2 with the rmcp wiring.
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

impl ResourceProvider for WikiResourceProvider {
    fn list(&self) -> Vec<Resource> {
        // v0.19 wave 1: static catalog only. Wave 2 enumerates
        // `coral://wiki/<repo>/<slug>` by walking the wiki index.
        Self::static_catalog()
    }

    fn read(&self, _uri: &str) -> Option<String> {
        // v0.19 wave 1 stub. Wave 2 reads the actual file and returns
        // its contents.
        None
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
    fn provider_list_returns_static_catalog() {
        let p = WikiResourceProvider::new(std::path::PathBuf::from("/tmp"));
        assert_eq!(p.list().len(), WikiResourceProvider::static_catalog().len());
    }
}
