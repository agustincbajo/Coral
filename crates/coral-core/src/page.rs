//! `Page` — a wiki page (frontmatter + body) with disk persistence.

use crate::error::{CoralError, Result};
use crate::frontmatter::{Frontmatter, parse as parse_fm, serialize as serialize_fm};
use crate::wikilinks;
use std::fs;
use std::path::{Path, PathBuf};

/// A wiki page parsed from disk (or memory) — frontmatter + body + path.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub path: PathBuf,
    pub frontmatter: Frontmatter,
    pub body: String,
}

impl Page {
    /// Parses a page from disk. Returns `CoralError::Io` on read failure
    /// (path captured), `CoralError::MissingFrontmatter` / `UnterminatedFrontmatter`
    /// for malformed pages, `CoralError::Yaml` on YAML errors.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|source| CoralError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_content(&content, path.to_path_buf())
    }

    /// Parses a page from raw content + a path (path used only for error reporting).
    pub fn from_content(content: &str, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let (frontmatter, body) = parse_fm(content, path.clone())?;
        Ok(Self {
            path,
            frontmatter,
            body,
        })
    }

    /// Serializes back to a Markdown string suitable for writing to disk.
    pub fn to_string(&self) -> Result<String> {
        serialize_fm(&self.frontmatter, &self.body)
    }

    /// Writes the page to its `path`. Creates parent dirs if missing.
    pub fn write(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| CoralError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
        }
        let content = self.to_string()?;
        fs::write(&self.path, content).map_err(|source| CoralError::Io {
            path: self.path.clone(),
            source,
        })
    }

    /// Returns wikilinks discovered in the body (and in `backlinks` field, deduplicated).
    /// The result is sorted alphabetically + deduplicated for stable output.
    pub fn outbound_links(&self) -> Vec<String> {
        let mut all: Vec<String> = wikilinks::extract(&self.body);
        for bl in &self.frontmatter.backlinks {
            all.push(bl.clone());
        }
        all.sort();
        all.dedup();
        all
    }

    /// Bumps `last_updated_commit` to `commit` (string sha).
    /// Mutates the page in place; caller persists with `write()`.
    pub fn bump_last_commit(&mut self, commit: impl Into<String>) {
        self.frontmatter.last_updated_commit = commit.into();
    }

    /// Inserts a backlink entry if not already present. Idempotent.
    pub fn add_backlink(&mut self, slug: impl Into<String>) {
        let slug = slug.into();
        if !self.frontmatter.backlinks.contains(&slug) {
            self.frontmatter.backlinks.push(slug);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::PageType;
    use tempfile::TempDir;

    fn sample_content() -> &'static str {
        "\
---
slug: order
type: module
last_updated_commit: aaa
confidence: 0.8
status: draft
---

# Order

See [[customer]] and [[product]].
"
    }

    #[test]
    fn page_from_content_minimal() {
        let page = Page::from_content(sample_content(), "test.md").expect("parse");
        assert_eq!(page.frontmatter.slug, "order");
        assert_eq!(page.frontmatter.page_type, PageType::Module);
        assert_eq!(page.frontmatter.last_updated_commit, "aaa");
        assert!(page.body.starts_with("# Order"));
        assert_eq!(page.path, PathBuf::from("test.md"));
    }

    #[test]
    fn page_to_string_roundtrip() {
        let page = Page::from_content(sample_content(), "test.md").expect("parse");
        let serialized = page.to_string().expect("serialize");
        let page2 = Page::from_content(&serialized, "test.md").expect("re-parse");
        assert_eq!(page.frontmatter, page2.frontmatter);
        assert_eq!(page.body, page2.body);
    }

    #[test]
    fn page_from_file_io_error_captures_path() {
        let bogus = PathBuf::from("/definitely/does/not/exist/page-xyz-9999.md");
        let err = Page::from_file(&bogus).expect_err("must fail");
        match err {
            CoralError::Io { path, .. } => assert_eq!(path, bogus),
            other => panic!("expected Io error, got {other:?}"),
        }
    }

    #[test]
    fn page_write_creates_parent_dirs() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("a/b/c/page.md");
        let page = Page::from_content(sample_content(), &target).expect("parse");
        page.write().expect("write");
        assert!(target.exists(), "page should exist at {target:?}");
        // Re-parse from disk to confirm round trip.
        let reloaded = Page::from_file(&target).expect("reload");
        assert_eq!(reloaded.frontmatter, page.frontmatter);
    }

    #[test]
    fn page_outbound_links_combines_body_and_backlinks_dedup() {
        let content = "\
---
slug: order
type: module
last_updated_commit: aaa
confidence: 0.5
status: draft
backlinks:
  - y
  - z
---

[[x]] [[y]]
";
        let page = Page::from_content(content, "test.md").expect("parse");
        let links = page.outbound_links();
        assert_eq!(
            links,
            vec!["x".to_string(), "y".to_string(), "z".to_string()]
        );
    }

    #[test]
    fn page_outbound_links_skips_code_fences() {
        let content = "\
---
slug: order
type: module
last_updated_commit: aaa
confidence: 0.5
status: draft
---

[[real]]

```
[[in-fence]]
```
";
        let page = Page::from_content(content, "test.md").expect("parse");
        let links = page.outbound_links();
        assert!(links.contains(&"real".to_string()));
        assert!(
            !links.contains(&"in-fence".to_string()),
            "fenced wikilink leaked: {links:?}"
        );
    }

    #[test]
    fn page_bump_last_commit() {
        let mut page = Page::from_content(sample_content(), "test.md").expect("parse");
        assert_eq!(page.frontmatter.last_updated_commit, "aaa");
        page.bump_last_commit("bbb");
        assert_eq!(page.frontmatter.last_updated_commit, "bbb");
    }

    #[test]
    fn page_add_backlink_idempotent() {
        let content = "\
---
slug: order
type: module
last_updated_commit: aaa
confidence: 0.5
status: draft
backlinks:
  - a
---

body
";
        let mut page = Page::from_content(content, "test.md").expect("parse");
        page.add_backlink("a");
        page.add_backlink("a");
        page.add_backlink("b");
        assert_eq!(
            page.frontmatter.backlinks,
            vec!["a".to_string(), "b".to_string()]
        );
    }
}
