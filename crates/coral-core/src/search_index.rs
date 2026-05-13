//! Persisted BM25 search index (M3.1).
//!
//! Pre-computes document frequencies, per-document term vectors, and document
//! lengths for sub-millisecond BM25 lookup on large wikis (5000+ pages).
//! The index is serialized to `.coral/search-index.bin` via postcard
//! (varint serde format, RUSTSEC-2025-0141 clearance — see Cargo.toml)
//! and invalidated by a content hash — a SHA-256 digest of all page bodies
//! concatenated in slug-sorted order.

use crate::page::Page;
use crate::search::{self, SearchResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
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
    /// SHA-256 hex digest of all page bodies (concatenated in
    /// slug-sorted order with length prefixes). See
    /// [`compute_content_hash`] for the exact framing. 64-char
    /// lowercase hex.
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

            docs.insert(p.frontmatter.slug.clone(), DocEntry { tf, doc_len });
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
    ///
    /// Uses `atomic_write_bytes` (tmp + rename) so a SIGKILL or panic
    /// mid-write can never leave a torn / zero-length file on disk.
    /// Readers (incl. parallel `coral` processes) see either the OLD
    /// contents or the NEW contents, never garbage.
    ///
    /// Encoding: postcard 1.x via the serde integration (varint wire
    /// format). v0.39.0 swapped from bincode 2.x to postcard to clear
    /// RUSTSEC-2025-0141 (bincode upstream unmaintained since
    /// 2025-12-16). Older bincode-written files are not readable; see
    /// [`Self::load_index`] for the graceful-rebuild story.
    pub fn save_index(&self, path: &Path) -> std::io::Result<()> {
        let encoded = postcard::to_allocvec(self)
            .map_err(|e| std::io::Error::other(format!("postcard encode: {e}")))?;
        crate::atomic::atomic_write_bytes(path, &encoded).map_err(|e| match e {
            crate::error::CoralError::Io { source, .. } => source,
            other => std::io::Error::other(other.to_string()),
        })
    }

    /// Load the index from disk.
    ///
    /// Returns `Err(InvalidData)` on any decode failure — including
    /// format mismatches from earlier coral versions (bincode 1.x in
    /// pre-v0.34, bincode 2.x in v0.34..v0.38). Callers above
    /// [`search_with_index`] interpret the error as "cache is stale,
    /// rebuild from corpus" and never surface it to the user. We log
    /// via `tracing::warn!` so the migration is observable in
    /// `coral`'s default-INFO logs.
    pub fn load_index(path: &Path) -> std::io::Result<Self> {
        let mut file = fs::File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        postcard::from_bytes::<Self>(&buf).map_err(|e| {
            // The most common failure on first v0.39.x boot will be a
            // pre-existing `search-index.bin` written by bincode (1.x
            // from pre-v0.34, or 2.x from v0.34..v0.38). Surface it once
            // at WARN; `search_with_index` rebuilds transparently. Other
            // decode errors (truncated file, mid-write power loss before
            // tmp+rename swap) hit the same recovery path.
            tracing::warn!(
                target: "coral_core::search_index",
                path = %path.display(),
                error = %e,
                "search index decode failed — treating as stale and rebuilding (likely legacy bincode format from a pre-v0.39 install)"
            );
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("postcard decode: {e}"),
            )
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

    // Make sure the parent dir exists before we ask `with_exclusive_lock`
    // to create the `.lock` sibling next to a path that doesn't yet have
    // a parent on disk (first-run case in `tempdir/.coral/`).
    if let Some(parent) = index_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Wrap load + rebuild + save in an exclusive flock so two concurrent
    // ingest invocations against the same wiki_root can't race and have
    // one of them read a half-written file while the other is mid-save.
    // The lock is on `<index_path>.lock`; the actual data file is still
    // replaced atomically via tmp + rename inside `save_index`.
    let index_res: crate::error::Result<SearchIndex> =
        crate::atomic::with_exclusive_lock(&index_path, || {
            let idx = if force_rebuild {
                let i = SearchIndex::build(pages);
                let _ = i.save_index(&index_path);
                i
            } else {
                match SearchIndex::load_index(&index_path) {
                    Ok(i) if i.is_valid_for(pages) => i,
                    _ => {
                        let i = SearchIndex::build(pages);
                        let _ = i.save_index(&index_path);
                        i
                    }
                }
            };
            Ok(idx)
        });

    // If the lock itself failed (extremely unusual — typically a perms
    // problem creating the `.lock` file), fall back to an in-memory
    // build so the search still succeeds. The persistent cache is then
    // a perf regression on this call only.
    let index = match index_res {
        Ok(i) => i,
        Err(_) => SearchIndex::build(pages),
    };

    // BM25 scoring runs OUTSIDE the lock — it only needs a `&[Page]`
    // snapshot and the in-memory `SearchIndex` we just built/loaded,
    // so we don't block other ingests on it.
    index.search_bm25(query, limit, pages)
}

/// Compute a content hash for a set of pages.
///
/// SHA-256 of `(slug, body)` pairs concatenated in slug-sorted order,
/// with explicit length prefixes so distinct page sets can't collide
/// just by re-flowing bytes across a slug/body boundary. The output is
/// a 64-character lowercase hex digest.
///
/// Cryptographic collision resistance matters here because the hash is
/// the sole gate that decides whether `load_index` returns a stale
/// cached index or rebuilds. A 64-bit hash (the v0.30.0 implementation
/// — `DefaultHasher` / SipHash 1-3) would silently serve stale results
/// on collision; SHA-256 brings the collision probability to
/// cryptographically negligible.
pub fn compute_content_hash(pages: &[Page]) -> String {
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;

    // Sort by slug for determinism.
    let sorted: BTreeMap<&str, &str> = pages
        .iter()
        .map(|p| (p.frontmatter.slug.as_str(), p.body.as_str()))
        .collect();

    let mut hasher = Sha256::new();
    for (slug, body) in &sorted {
        // Length prefixes (u64-LE) frame each field so concatenation is
        // unambiguous — without them, ("ab", "c") and ("a", "bc") would
        // hash to the same bytes.
        hasher.update((slug.len() as u64).to_le_bytes());
        hasher.update(slug.as_bytes());
        hasher.update((body.len() as u64).to_le_bytes());
        hasher.update(body.as_bytes());
    }
    format!("{:x}", hasher.finalize())
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

    // ─── Regression: SHA-256 content hash format ───

    /// v0.30.0 audit (finding #006B): `compute_content_hash` previously
    /// used 64-bit `DefaultHasher` despite the doc claiming SHA-256.
    /// Pin the new contract: 64 lowercase hex chars (= 256 bits).
    #[test]
    fn content_hash_is_64_char_hex_sha256() {
        let pages = vec![page("outbox", "outbox dispatcher")];
        let h = compute_content_hash(&pages);
        assert_eq!(
            h.len(),
            64,
            "SHA-256 hex digest must be 64 chars, got {}: {h}",
            h.len()
        );
        assert!(
            h.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "digest must be lowercase hex: {h}"
        );
    }

    /// Length-prefix framing in `compute_content_hash` must prevent the
    /// classic concatenation collision: ("ab", "c") and ("a", "bc") must
    /// hash differently even though their unframed byte concatenation
    /// "abc" is identical. (Strictly speaking, slug-sorted pairs in
    /// production differ by slug, but the framing is the principled fix.)
    #[test]
    fn content_hash_framing_prevents_split_ambiguity() {
        let a = vec![page("ab", "c")];
        let b = vec![page("a", "bc")];
        assert_ne!(
            compute_content_hash(&a),
            compute_content_hash(&b),
            "length-prefix framing must distinguish (\"ab\",\"c\") from (\"a\",\"bc\")"
        );
    }

    // ─── Concurrency: search_with_index under contention ───

    /// v0.30.0 audit (finding #006A): two `coral ingest` processes
    /// could race the rebuild and produce torn writes or
    /// bincode-decode errors. The fix wraps load+rebuild+save in
    /// `with_exclusive_lock` and persists via tmp+rename. Pin it: N
    /// threads hammering the same wiki_root all return non-empty
    /// results, the final on-disk index decodes cleanly, no panics.
    #[test]
    fn search_with_index_is_safe_under_concurrent_writers() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wiki_root = tmp.path().to_path_buf();
        // Distinct page sets per thread so each one would invalidate
        // the other's cache and trigger a rebuild — maximises the
        // contention the lock is supposed to serialise.
        let datasets: Vec<Vec<Page>> = (0..4)
            .map(|i| {
                vec![
                    page("outbox", &format!("outbox dispatcher version {i}")),
                    page("order", &format!("order module references outbox {i}")),
                    page(
                        &format!("extra-{i}"),
                        &format!("extra page only present in thread {i}"),
                    ),
                ]
            })
            .collect();

        const ROUNDS: usize = 5;
        std::thread::scope(|s| {
            for ds in &datasets {
                let wiki_root = wiki_root.clone();
                let ds = ds.clone();
                s.spawn(move || {
                    for _ in 0..ROUNDS {
                        let r = search_with_index(&ds, "outbox", 5, &wiki_root, false);
                        // Each call must return at least one result; the
                        // important invariant is "no panic, no bincode
                        // garbage observed by a reader mid-rebuild".
                        assert!(!r.is_empty(), "concurrent search returned no results");
                    }
                });
            }
        });

        // After the storm the persisted index must still decode
        // cleanly — proving no torn write ever escaped to disk.
        let index_path = SearchIndex::default_path(&wiki_root);
        let loaded = SearchIndex::load_index(&index_path)
            .expect("post-contention index file must decode cleanly");
        assert!(
            loaded.n_docs > 0,
            "post-contention index must contain documents"
        );
    }

    // ─── postcard migration (RUSTSEC-2025-0141 clearance, v0.39.0) ───

    /// Pin the new on-disk codec: postcard 1.x via the serde
    /// integration. A roundtrip through `to_allocvec` +
    /// `take_from_bytes` MUST preserve every public field of
    /// `SearchIndex`, and MUST consume the entire buffer (no trailing
    /// garbage). This is regression insurance against an accidental
    /// codec swap (e.g. flipping back to bincode or to a fixed-int
    /// format).
    #[test]
    fn postcard_encode_decode_roundtrip() {
        let pages = vec![
            page("outbox", "outbox dispatcher polls"),
            page("order", "order references outbox"),
        ];
        let original = SearchIndex::build(&pages);

        // Encode via postcard 1.x serde integration.
        let bytes = postcard::to_allocvec(&original).unwrap();

        // Decode back via `take_from_bytes` so we can assert the byte
        // stream consumed matches the input length (no trailing
        // garbage). `from_bytes` would also work but discards the
        // remainder slice we need for that check.
        let (decoded, rest): (SearchIndex, &[u8]) = postcard::take_from_bytes(&bytes).unwrap();
        assert!(
            rest.is_empty(),
            "postcard should consume the entire encoded buffer (got {} trailing bytes)",
            rest.len()
        );

        // Field-by-field equality. We can't `#[derive(PartialEq)]` on
        // `SearchIndex` because `HashMap` field ordering is non-
        // deterministic across allocations, but content equality is
        // exactly what we need.
        assert_eq!(decoded.content_hash, original.content_hash);
        assert_eq!(decoded.n_docs, original.n_docs);
        assert!((decoded.avgdl - original.avgdl).abs() < 1e-12);
        assert_eq!(decoded.df, original.df);
        for (slug, orig_entry) in &original.docs {
            let dec_entry = decoded.docs.get(slug).expect("slug missing post-roundtrip");
            assert_eq!(dec_entry.tf, orig_entry.tf);
            assert_eq!(dec_entry.doc_len, orig_entry.doc_len);
        }
    }

    /// v0.39.0 migration: bincode 2.x → postcard 1.x changed the wire
    /// format. Old `search-index.bin` files written by v0.34.x..v0.38.x
    /// (bincode 2.x) or pre-v0.34 (bincode 1.x) CANNOT be decoded.
    ///
    /// Behaviour contract: `load_index` returns `Err(InvalidData)` on
    /// any decode failure (legacy file, truncated file, garbage); the
    /// `search_with_index` caller swallows the error and rebuilds the
    /// cache from the in-memory corpus. The user sees no error.
    ///
    /// We can't ship a real legacy bincode fixture file here without
    /// dragging the old crate back in as a dev-dep — instead we
    /// simulate the failure with deliberately-garbage bytes that
    /// definitely don't decode as a valid `SearchIndex`. The same
    /// failure mode covers truncated postcard payloads too.
    #[test]
    fn load_index_rebuilds_on_legacy_format_mismatch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wiki_root = tmp.path();
        let index_path = SearchIndex::default_path(wiki_root);

        // Pre-populate the cache file with bytes that look like an old
        // index but won't decode as bincode 2.x. The header pattern is
        // close enough that the failure mode mirrors what a v0.33 user
        // would actually hit on first v0.34 boot: a file exists, has
        // non-trivial size, parses to garbage.
        fs::create_dir_all(index_path.parent().unwrap()).unwrap();
        let garbage: Vec<u8> = (0..=255u8).cycle().take(1024).collect();
        fs::write(&index_path, &garbage).unwrap();

        // Direct `load_index` call: must report decode failure with
        // `InvalidData` so the upper layer recognises "stale cache".
        let err =
            SearchIndex::load_index(&index_path).expect_err("load_index must reject garbage bytes");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::InvalidData,
            "decode failure must surface as InvalidData (got {err:?})"
        );

        // High-level search call: must transparently rebuild and
        // return non-empty results despite the stale file on disk.
        let pages = vec![
            page("outbox", "outbox dispatcher polls every second"),
            page("order", "order references outbox"),
        ];
        let results = search_with_index(&pages, "outbox", 5, wiki_root, false);
        assert!(
            !results.is_empty(),
            "search must transparently rebuild over a stale-format cache file"
        );

        // After the rebuild, the cache file is a valid bincode-2.x
        // payload that decodes cleanly.
        let reloaded =
            SearchIndex::load_index(&index_path).expect("post-rebuild cache must decode cleanly");
        assert!(reloaded.n_docs > 0);
        assert_eq!(reloaded.n_docs, pages.len());
    }

    #[test]
    fn default_path_is_under_coral_dir() {
        let root = Path::new("/tmp/my-wiki");
        let path = SearchIndex::default_path(root);
        assert_eq!(path, PathBuf::from("/tmp/my-wiki/.coral/search-index.bin"));
    }
}
