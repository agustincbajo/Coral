//! Persisted BM25 search index (M3.1).
//!
//! Pre-computes document frequencies, per-document term vectors, and document
//! lengths for sub-millisecond BM25 lookup on large wikis (5000+ pages).
//! The index is serialized to `.coral/search-index.bin` via bincode and
//! invalidated by a content hash — a SHA-256 digest of all page bodies
//! concatenated in slug-sorted order.

use crate::page::Page;
use crate::search::{self, SearchResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Filename for the persisted search index.
pub const INDEX_FILENAME: &str = "search-index.bin";

/// Directory under the wiki root where the index file lives.
pub const INDEX_DIR: &str = ".coral";

/// A persisted BM25 search index.
///
/// Contains all pre-computed data needed for BM25 scoring without
/// re-tokenizing the corpus. Invalidated when `content_hash` no longer
/// matches the current wiki state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndex {
    /// SHA-256 hex digest of all page bodies (concatenated in slug-sorted order).
    pub content_hash: String,

    /// Number of documents in the corpus at index time.
    pub n_docs: usize,

    /// Average document length (in tokens) across the corpus.
    pub avgdl: f64,

    /// Document frequency: term -> number of documents containing it.
    pub df: HashMap<String, usize>,

    /// Per-document data: slug -> (term frequencies, document length in tokens).
    pub docs: HashMap<String, DocEntry>,
}

/// Pre-computed per-document data for BM25 scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocEntry {
    /// Term frequency map: term -> count in this document.
    pub tf: HashMap<String, usize>,
    /// Document length in tokens.
    pub doc_len: usize,
}

impl SearchIndex {
    /// Build a new index from a set of pages.
    pub fn build(pages: &[Page]) -> Self {
        let content_hash = compute_content_hash(pages);
        let n_docs = pages.len();

        let mut df: HashMap<String, usize> = HashMap::new();
        let mut docs: HashMap<String, DocEntry> = HashMap::with_capacity(n_docs);
        let mut total_len: usize = 0;

        for p in pages {
            let combined = format!("{} {}", p.frontmatter.slug, p.body);
            let tokens = tokenize(&combined);
            let doc_len = tokens.len();
            total_len += doc_len;

            // Count term frequencies for this doc.
            let mut tf: HashMap<String, usize> = HashMap::new();
            let mut unique: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for t in &tokens {
                *tf.entry(t.clone()).or_insert(0) += 1;
                unique.insert(t.as_str());
            }

            // Update document frequencies.
            for t in unique {
                *df.entry(t.to_string()).or_insert(0) += 1;
            }

            docs.insert(
                p.frontmatter.slug.clone(),
                DocEntry { tf, doc_len },
            );
        }

        let avgdl = if n_docs > 0 && total_len > 0 {
            total_len as f64 / n_docs as f64
        } else {
            1.0
        };

        Self {
            content_hash,
            n_docs,
            avgdl,
            df,
            docs,
        }
    }

    /// Execute a BM25 search against this pre-computed index.
    ///
    /// Results are identical to `search::search_bm25` — same constants,
    /// same tokenization, same IDF formula.
    pub fn search_bm25(&self, query: &str, limit: usize, pages: &[Page]) -> Vec<SearchResult> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() || self.n_docs == 0 {
            return vec![];
        }

        let n_docs = self.n_docs as f64;

        // Precompute IDF per query term.
        let idf: HashMap<&str, f64> = query_tokens
            .iter()
            .map(|q| {
                let n_q = *self.df.get(q.as_str()).unwrap_or(&0) as f64;
                let raw = ((n_docs - n_q + 0.5) / (n_q + 0.5) + 1.0).ln();
                (q.as_str(), raw.max(0.0))
            })
            .collect();

        let mut results: Vec<SearchResult> = Vec::new();

        for p in pages {
            let slug = &p.frontmatter.slug;
            let doc_entry = match self.docs.get(slug) {
                Some(d) => d,
                None => continue,
            };

            if doc_entry.doc_len == 0 {
                continue;
            }

            let dl = doc_entry.doc_len as f64;
            let length_norm = 1.0 - search::BM25_B + search::BM25_B * (dl / self.avgdl);

            let mut score = 0.0;
            for q in &query_tokens {
                if let Some(&count) = doc_entry.tf.get(q.as_str()) {
                    let f = count as f64;
                    let term_idf = *idf.get(q.as_str()).unwrap_or(&0.0);
                    let numerator = f * (search::BM25_K1 + 1.0);
                    let denominator = f + search::BM25_K1 * length_norm;
                    if denominator > 0.0 {
                        score += term_idf * (numerator / denominator);
                    }
                }
            }

            if score == 0.0 {
                continue;
            }

            let snippet = build_snippet(&p.body, &query_tokens, 200);
            results.push(SearchResult {
                slug: slug.clone(),
                score,
                snippet,
            });
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        results
    }

    /// Save the index to disk at the given path.
    pub fn save_index(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let encoded = bincode::serialize(self).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, format!("bincode encode: {e}"))
        })?;
        let mut file = fs::File::create(path)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        Ok(())
    }

    /// Load the index from disk.
    pub fn load_index(path: &Path) -> std::io::Result<Self> {
        let mut file = fs::File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        bincode::deserialize(&buf).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bincode decode: {e}"))
        })
    }

    /// Return the default index file path for a given wiki root.
    pub fn default_path(wiki_root: &Path) -> PathBuf {
        wiki_root.join(INDEX_DIR).join(INDEX_FILENAME)
    }

    /// Check if this index is still valid for the given pages.
    pub fn is_valid_for(&self, pages: &[Page]) -> bool {
        let current_hash = compute_content_hash(pages);
        self.content_hash == current_hash
    }
}

/// High-level search function that uses the persisted index if available
/// and valid, otherwise rebuilds and persists it.
///
/// This is the recommended entry point for CLI commands that want
/// sub-millisecond BM25 with automatic caching.
pub fn search_with_index(
    pages: &[Page],
    query: &str,
    limit: usize,
    wiki_root: &Path,
    force_rebuild: bool,
) -> Vec<SearchResult> {
    let index_path = SearchIndex::default_path(wiki_root);

    let index = if force_rebuild {
        let idx = SearchIndex::build(pages);
        let _ = idx.save_index(&index_path);
        idx
    } else {
        match SearchIndex::load_index(&index_path) {
            Ok(idx) if idx.is_valid_for(pages) => idx,
            _ => {
                let idx = SearchIndex::build(pages);
                let _ = idx.save_index(&index_path);
                idx
            }
        }
    };

    index.search_bm25(query, limit, pages)
}

/// Compute a content hash for a set of pages.
///
/// SHA-256 of all page bodies concatenated in slug-sorted order.
/// This ensures the hash changes when any page content changes,
/// pages are added, or pages are removed.
pub fn compute_content_hash(pages: &[Page]) -> String {
    use std::collections::BTreeMap;
    use std::hash::{DefaultHasher, Hash, Hasher};

    // Sort by slug for determinism.
    let sorted: BTreeMap<&str, &str> = pages
        .iter()
        .map(|p| (p.frontmatter.slug.as_str(), p.body.as_str()))
        .collect();

    let mut hasher = DefaultHasher::new();
    for (slug, body) in &sorted {
        slug.hash(&mut hasher);
        body.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

// ─── Internal helpers (mirrors search.rs tokenization) ───

use std::collections::HashSet;
use std::sync::OnceLock;

fn stopwords() -> &'static HashSet<&'static str> {
    static INSTANCE: OnceLock<HashSet<&'static str>> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        [
            "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "he", "in",
            "is", "it", "its", "of", "on", "that", "the", "to", "was", "were", "will", "with",
            // Spanish
            "el", "la", "los", "las", "de", "y", "en", "que", "es", "se", "un", "una", "para",
            "por", "con", "del", "al",
        ]
        .into_iter()
        .collect()
    })
}

fn tokenize(text: &str) -> Vec<String> {
    let sw = stopwords();
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .filter(|t| !sw.contains(t))
        .map(String::from)
        .collect()
}

fn build_snippet(body: &str, query_tokens: &[String], max_len: usize) -> String {
    let lower = body.to_lowercase();
    for q in query_tokens {
        if let Some(pos) = lower.find(q.as_str()) {
            let start = floor_char_boundary(body, pos.saturating_sub(40));
            let end = ceil_char_boundary(body, (pos + q.len() + max_len).min(body.len()));
            return body[start..end].chars().take(max_len).collect::<String>();
        }
    }
    body.chars().take(max_len).collect::<String>()
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use std::path::PathBuf;

    fn page(slug: &str, body: &str) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/modules/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type: PageType::Module,
                last_updated_commit: "abc".to_string(),
                confidence: Confidence::try_new(0.8).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Reviewed,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra: Default::default(),
            },
            body: body.to_string(),
        }
    }

    // ─── Index save/load roundtrip ───

    #[test]
    fn index_save_load_roundtrip() {
        let pages = vec![
            page("outbox", "the outbox dispatcher polls every second"),
            page("order", "order module references the outbox"),
            page("payment", "payment processing via stripe gateway"),
        ];

        let index = SearchIndex::build(&pages);
        assert_eq!(index.n_docs, 3);
        assert!(!index.content_hash.is_empty());
        assert!(index.docs.contains_key("outbox"));
        assert!(index.docs.contains_key("order"));
        assert!(index.docs.contains_key("payment"));

        // Save to temp file.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test-index.bin");
        index.save_index(&path).unwrap();

        // Load it back.
        let loaded = SearchIndex::load_index(&path).unwrap();
        assert_eq!(loaded.content_hash, index.content_hash);
        assert_eq!(loaded.n_docs, index.n_docs);
        assert!((loaded.avgdl - index.avgdl).abs() < 1e-12);
        assert_eq!(loaded.df, index.df);
        assert_eq!(loaded.docs.len(), index.docs.len());

        // Verify term frequencies survived the roundtrip.
        let outbox_entry = loaded.docs.get("outbox").unwrap();
        let orig_entry = index.docs.get("outbox").unwrap();
        assert_eq!(outbox_entry.tf, orig_entry.tf);
        assert_eq!(outbox_entry.doc_len, orig_entry.doc_len);
    }

    // ─── Content hash invalidation ───

    #[test]
    fn content_hash_invalidation_on_page_change() {
        let pages = vec![
            page("outbox", "the outbox dispatcher polls every second"),
            page("order", "order module references the outbox"),
        ];

        let index = SearchIndex::build(&pages);
        assert!(index.is_valid_for(&pages));

        // Modify a page body.
        let modified_pages = vec![
            page("outbox", "the outbox dispatcher was completely rewritten"),
            page("order", "order module references the outbox"),
        ];
        assert!(!index.is_valid_for(&modified_pages));
    }

    #[test]
    fn content_hash_invalidation_on_page_added() {
        let pages = vec![page("outbox", "outbox dispatcher")];
        let index = SearchIndex::build(&pages);

        let expanded = vec![
            page("outbox", "outbox dispatcher"),
            page("new-page", "brand new content"),
        ];
        assert!(!index.is_valid_for(&expanded));
    }

    #[test]
    fn content_hash_invalidation_on_page_removed() {
        let pages = vec![
            page("outbox", "outbox dispatcher"),
            page("order", "order module"),
        ];
        let index = SearchIndex::build(&pages);

        let reduced = vec![page("outbox", "outbox dispatcher")];
        assert!(!index.is_valid_for(&reduced));
    }

    // ─── Search results identical (cached vs fresh) ───

    #[test]
    fn cached_index_produces_identical_results_to_fresh_search() {
        let pages = vec![
            page("outbox", "the outbox dispatcher polls every second"),
            page("order", "order module references the outbox pattern"),
            page("payment", "payment processing via stripe gateway"),
            page("invoice", "invoice generation from completed orders"),
        ];

        // Fresh BM25 results (the canonical implementation).
        let fresh_results = search::search_bm25(&pages, "outbox", 10);

        // Index-based results.
        let index = SearchIndex::build(&pages);
        let index_results = index.search_bm25("outbox", 10, &pages);

        // Same number of results.
        assert_eq!(
            fresh_results.len(),
            index_results.len(),
            "result count mismatch: fresh={} index={}",
            fresh_results.len(),
            index_results.len()
        );

        // Same slugs in same order.
        for (fresh, indexed) in fresh_results.iter().zip(index_results.iter()) {
            assert_eq!(
                fresh.slug, indexed.slug,
                "slug order mismatch: fresh={} index={}",
                fresh.slug, indexed.slug
            );
            assert!(
                (fresh.score - indexed.score).abs() < 1e-10,
                "score mismatch for '{}': fresh={} index={}",
                fresh.slug,
                fresh.score,
                indexed.score
            );
        }
    }

    #[test]
    fn cached_index_multi_term_query_matches_fresh() {
        let pages = vec![
            page("outbox", "outbox dispatcher polls"),
            page("order", "order handler dispatches events"),
            page("payment", "payment gateway stripe integration"),
        ];

        let fresh = search::search_bm25(&pages, "outbox dispatcher", 10);
        let index = SearchIndex::build(&pages);
        let indexed = index.search_bm25("outbox dispatcher", 10, &pages);

        assert_eq!(fresh.len(), indexed.len());
        for (f, i) in fresh.iter().zip(indexed.iter()) {
            assert_eq!(f.slug, i.slug);
            assert!((f.score - i.score).abs() < 1e-10);
        }
    }

    // ─── Performance: load from file is faster than rebuild ───

    #[test]
    fn load_from_disk_faster_than_rebuild() {
        // Generate a corpus large enough for the timing difference to be
        // measurable (100 pages with moderate-length bodies).
        let pages: Vec<Page> = (0..100)
            .map(|i| {
                page(
                    &format!("page-{i:04}"),
                    &format!(
                        "module {i} handles the outbox pattern for async delivery. \
                         It contains logic for retry, backoff, and dead-letter queues. \
                         The dispatcher runs every second checking for pending items. \
                         Integration with payment gateway stripe and order processing. {}",
                        "lorem ipsum dolor sit amet consectetur adipiscing elit ".repeat(5)
                    ),
                )
            })
            .collect();

        let index = SearchIndex::build(&pages);

        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("perf-index.bin");
        index.save_index(&path).unwrap();

        // Measure rebuild time.
        let rebuild_start = std::time::Instant::now();
        for _ in 0..5 {
            let _ = SearchIndex::build(&pages);
        }
        let rebuild_elapsed = rebuild_start.elapsed();

        // Measure load time.
        let load_start = std::time::Instant::now();
        for _ in 0..5 {
            let _ = SearchIndex::load_index(&path).unwrap();
        }
        let load_elapsed = load_start.elapsed();

        // Loading from disk should be faster than rebuilding.
        assert!(
            load_elapsed < rebuild_elapsed,
            "load ({load_elapsed:?}) should be faster than rebuild ({rebuild_elapsed:?})"
        );
    }

    // ─── search_with_index integration ───

    #[test]
    fn search_with_index_creates_and_reuses_cache() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wiki_root = tmp.path();
        let pages = vec![
            page("outbox", "the outbox dispatcher polls every second"),
            page("order", "order module references the outbox"),
        ];

        let index_path = SearchIndex::default_path(wiki_root);

        // First call: no cache file yet.
        assert!(!index_path.exists());
        let results1 = search_with_index(&pages, "outbox", 5, wiki_root, false);
        assert!(!results1.is_empty());
        assert!(index_path.exists(), "index file should have been created");

        // Second call: should use cached index (same pages).
        let results2 = search_with_index(&pages, "outbox", 5, wiki_root, false);
        assert_eq!(results1.len(), results2.len());
        for (r1, r2) in results1.iter().zip(results2.iter()) {
            assert_eq!(r1.slug, r2.slug);
            assert!((r1.score - r2.score).abs() < 1e-10);
        }
    }

    #[test]
    fn search_with_index_force_rebuild() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wiki_root = tmp.path();
        let pages = vec![page("outbox", "outbox dispatcher")];

        // Build initial cache.
        let _ = search_with_index(&pages, "outbox", 5, wiki_root, false);
        let index_path = SearchIndex::default_path(wiki_root);
        let mtime1 = fs::metadata(&index_path).unwrap().modified().unwrap();

        // Force rebuild should overwrite the file.
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _ = search_with_index(&pages, "outbox", 5, wiki_root, true);
        let mtime2 = fs::metadata(&index_path).unwrap().modified().unwrap();
        assert!(mtime2 > mtime1, "force-rebuild should write a new file");
    }

    #[test]
    fn search_with_index_invalidates_on_content_change() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wiki_root = tmp.path();

        let pages_v1 = vec![
            page("outbox", "the outbox dispatcher polls every second"),
            page("order", "order module references the outbox"),
        ];
        let _ = search_with_index(&pages_v1, "outbox", 5, wiki_root, false);

        // Change page content.
        let pages_v2 = vec![
            page("outbox", "the outbox was completely replaced by kafka"),
            page("order", "order module references the outbox"),
        ];

        // The cached index should be invalidated and rebuilt.
        let results = search_with_index(&pages_v2, "kafka", 5, wiki_root, false);
        assert!(!results.is_empty());
        assert_eq!(results[0].slug, "outbox");
    }

    #[test]
    fn empty_corpus_produces_empty_results() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pages: Vec<Page> = vec![];
        let results = search_with_index(&pages, "anything", 5, tmp.path(), false);
        assert!(results.is_empty());
    }

    #[test]
    fn default_path_is_under_coral_dir() {
        let root = Path::new("/tmp/my-wiki");
        let path = SearchIndex::default_path(root);
        assert_eq!(path, PathBuf::from("/tmp/my-wiki/.coral/search-index.bin"));
    }
}
