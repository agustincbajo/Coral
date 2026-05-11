//! Storage abstraction traits for wiki pages and embeddings.
//!
//! v0.25 (M1.11): defines the trait boundary so future backends
//! (pgvector, Neo4j, Qdrant) can implement without touching core.
//! Default implementations remain filesystem-based (JSON + SQLite).

use crate::error::{CoralError, Result};
use crate::page::Page;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Trait for reading/writing wiki pages.
///
/// The default implementation is filesystem-based (`FsWikiStorage`),
/// which delegates to `walk::read_pages()`. Future backends (Git-native,
/// database-backed) implement this trait directly.
pub trait WikiStorage: Send + Sync {
    /// Read all pages from storage.
    fn read_all(&self) -> Result<Vec<Page>>;

    /// Read a single page by slug. Returns None if not found.
    fn read_by_slug(&self, slug: &str) -> Result<Option<Page>>;

    /// Write a page to storage (upsert by slug).
    fn write_page(&self, page: &Page) -> Result<()>;

    /// Delete a page by slug.
    fn delete_page(&self, slug: &str) -> Result<()>;

    /// List all slugs without loading full page content.
    fn list_slugs(&self) -> Result<Vec<String>>;

    /// Return the number of pages.
    fn count(&self) -> Result<usize> {
        Ok(self.list_slugs()?.len())
    }
}

/// Trait for storing and querying embeddings vectors.
///
/// Two implementations ship in-tree: `JsonEmbeddingsStorage` (the
/// `.coral-embeddings.json` file) and `SqliteEmbeddingsStorage` (the
/// `.coral-embeddings.db` file). External crates can add pgvector,
/// Qdrant, Weaviate, etc.
pub trait EmbeddingsStorage: Send + Sync {
    /// Provider name (e.g. "voyage-3", "text-embedding-3-small").
    fn provider(&self) -> &str;

    /// Vector dimensionality.
    fn dim(&self) -> usize;

    /// Check if a slug's embedding is fresh (mtime matches).
    fn is_fresh(&self, slug: &str, mtime: i64) -> Result<bool>;

    /// Upsert a vector for a slug.
    fn upsert(&mut self, slug: &str, mtime: i64, vector: Vec<f32>) -> Result<()>;

    /// Remove vectors for slugs not in the live set.
    fn prune(&mut self, live_slugs: &HashSet<String>) -> Result<()>;

    /// Cosine-similarity search. Returns (slug, score) pairs.
    fn search(&self, query_vector: &[f32], limit: usize) -> Result<Vec<(String, f32)>>;

    /// Persist to disk (for backends that buffer writes).
    fn flush(&self) -> Result<()>;
}

/// Filesystem-based wiki storage (the default).
///
/// Delegates to `walk::read_pages()` for reading and
/// `atomic::atomic_write_string()` for writing.
pub struct FsWikiStorage {
    wiki_root: PathBuf,
}

impl FsWikiStorage {
    pub fn new(wiki_root: impl Into<PathBuf>) -> Self {
        Self {
            wiki_root: wiki_root.into(),
        }
    }

    pub fn wiki_root(&self) -> &Path {
        &self.wiki_root
    }
}

impl WikiStorage for FsWikiStorage {
    fn read_all(&self) -> Result<Vec<Page>> {
        if !self.wiki_root.exists() {
            return Ok(Vec::new());
        }
        crate::walk::read_pages(&self.wiki_root)
    }

    fn read_by_slug(&self, slug: &str) -> Result<Option<Page>> {
        let pages = self.read_all()?;
        Ok(pages.into_iter().find(|p| p.frontmatter.slug == slug))
    }

    fn write_page(&self, page: &Page) -> Result<()> {
        let content = page.to_string()?;
        crate::atomic::atomic_write_string(&page.path, &content)
    }

    fn delete_page(&self, slug: &str) -> Result<()> {
        let pages = self.read_all()?;
        if let Some(page) = pages.iter().find(|p| p.frontmatter.slug == slug) {
            std::fs::remove_file(&page.path).map_err(|e| CoralError::Io {
                path: page.path.clone(),
                source: e,
            })?;
        }
        Ok(())
    }

    fn list_slugs(&self) -> Result<Vec<String>> {
        let pages = self.read_all()?;
        Ok(pages.into_iter().map(|p| p.frontmatter.slug).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn fs_wiki_storage_reads_pages() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(wiki.join("modules")).unwrap();
        std::fs::write(
            wiki.join("modules/test.md"),
            "---\nslug: test\ntype: module\nlast_updated_commit: abc\nconfidence: 0.8\nstatus: reviewed\n---\n\nTest body.\n",
        ).unwrap();

        let storage = FsWikiStorage::new(&wiki);
        let pages = storage.read_all().unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].frontmatter.slug, "test");
    }

    #[test]
    fn fs_wiki_storage_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let storage = FsWikiStorage::new(tmp.path());
        let pages = storage.read_all().unwrap();
        assert!(pages.is_empty());
    }

    #[test]
    fn fs_wiki_storage_nonexistent_dir() {
        let storage = FsWikiStorage::new("/tmp/coral-nonexistent-test-dir-xyz");
        let pages = storage.read_all().unwrap();
        assert!(pages.is_empty());
    }

    #[test]
    fn fs_wiki_storage_list_slugs() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(wiki.join("modules")).unwrap();
        std::fs::write(
            wiki.join("modules/a.md"),
            "---\nslug: a\ntype: module\nlast_updated_commit: x\nconfidence: 0.5\nstatus: draft\n---\n\nA\n",
        ).unwrap();
        std::fs::write(
            wiki.join("modules/b.md"),
            "---\nslug: b\ntype: module\nlast_updated_commit: y\nconfidence: 0.6\nstatus: draft\n---\n\nB\n",
        ).unwrap();

        let storage = FsWikiStorage::new(&wiki);
        let mut slugs = storage.list_slugs().unwrap();
        slugs.sort();
        assert_eq!(slugs, vec!["a", "b"]);
    }

    #[test]
    fn fs_wiki_storage_count() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(wiki.join("modules")).unwrap();
        std::fs::write(
            wiki.join("modules/a.md"),
            "---\nslug: a\ntype: module\nlast_updated_commit: x\nconfidence: 0.5\nstatus: draft\n---\n\nA\n",
        ).unwrap();

        let storage = FsWikiStorage::new(&wiki);
        assert_eq!(storage.count().unwrap(), 1);
    }

    #[test]
    fn fs_wiki_storage_read_by_slug() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(wiki.join("modules")).unwrap();
        std::fs::write(
            wiki.join("modules/alpha.md"),
            "---\nslug: alpha\ntype: module\nlast_updated_commit: z\nconfidence: 0.7\nstatus: draft\n---\n\nAlpha body.\n",
        ).unwrap();

        let storage = FsWikiStorage::new(&wiki);
        let found = storage.read_by_slug("alpha").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().frontmatter.slug, "alpha");

        let missing = storage.read_by_slug("nonexistent").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn fs_wiki_storage_write_and_delete() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(wiki.join("modules")).unwrap();

        let page_path = wiki.join("modules/new.md");
        let page = Page::from_content(
            "---\nslug: new\ntype: module\nlast_updated_commit: w\nconfidence: 0.9\nstatus: draft\n---\n\nNew page.\n",
            &page_path,
        ).unwrap();

        let storage = FsWikiStorage::new(&wiki);
        storage.write_page(&page).unwrap();
        assert!(page_path.exists());

        // Can read it back
        let found = storage.read_by_slug("new").unwrap();
        assert!(found.is_some());

        // Delete it
        storage.delete_page("new").unwrap();
        assert!(!page_path.exists());
    }
}
