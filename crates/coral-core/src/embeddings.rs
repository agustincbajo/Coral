//! Local embeddings cache + cosine-similarity search.
//!
//! Storage: `<wiki_root>/.coral-embeddings.json` — schema-versioned, mtime-keyed
//! per slug. Auto-ignored if `coral init` wrote the `<wiki_root>/.gitignore`.
//!
//! v0.3.1: ships a single provider — Voyage AI `voyage-3` via curl (see
//! `coral-cli::commands::search`). Future versions may add a `Provider` trait
//! and OpenAI / Anthropic / local backends.

use crate::error::{CoralError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EmbeddingsIndex {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Provider id, e.g. "voyage-3". Mismatched providers ⇒ stale index.
    pub provider: String,
    /// Embedding dimension (e.g. 1024 for voyage-3).
    pub dim: usize,
    /// slug → IndexedVector
    pub entries: BTreeMap<String, IndexedVector>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedVector {
    pub mtime_secs: i64,
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbedResult {
    pub slug: String,
    pub score: f32,
    pub snippet: String,
}

impl EmbeddingsIndex {
    pub const FILENAME: &'static str = ".coral-embeddings.json";
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn load(wiki_root: &Path) -> Result<Self> {
        let path = wiki_root.join(Self::FILENAME);
        if !path.exists() {
            return Ok(Self::empty("voyage-3", 1024));
        }
        let content = fs::read_to_string(&path).map_err(|e| CoralError::Io {
            path: path.clone(),
            source: e,
        })?;
        let parsed: Self = serde_json::from_str(&content)
            .map_err(|e| CoralError::Walk(format!("embeddings parse error: {e}")))?;
        if parsed.version != Self::SCHEMA_VERSION {
            return Ok(Self::empty(&parsed.provider, parsed.dim));
        }
        Ok(parsed)
    }

    pub fn empty(provider: &str, dim: usize) -> Self {
        Self {
            version: Self::SCHEMA_VERSION,
            provider: provider.to_string(),
            dim,
            entries: BTreeMap::new(),
        }
    }

    pub fn save(&self, wiki_root: &Path) -> Result<PathBuf> {
        let path = wiki_root.join(Self::FILENAME);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| CoralError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| CoralError::Walk(format!("embeddings serialize error: {e}")))?;
        fs::write(&path, content).map_err(|e| CoralError::Io {
            path: path.clone(),
            source: e,
        })?;
        Ok(path)
    }

    pub fn upsert(&mut self, slug: impl Into<String>, mtime_secs: i64, vector: Vec<f32>) {
        self.entries
            .insert(slug.into(), IndexedVector { mtime_secs, vector });
    }

    /// Returns true if the entry's mtime matches `mtime_secs`.
    pub fn is_fresh(&self, slug: &str, mtime_secs: i64) -> bool {
        self.entries
            .get(slug)
            .is_some_and(|e| e.mtime_secs == mtime_secs)
    }

    /// Search by cosine similarity. Returns top-`limit` results sorted desc.
    /// `query_vec` must have length == `self.dim`; if not, returns empty.
    pub fn search(&self, query_vec: &[f32], limit: usize) -> Vec<(String, f32)> {
        if query_vec.len() != self.dim || limit == 0 || self.entries.is_empty() {
            return vec![];
        }
        let q_norm = norm(query_vec);
        if q_norm == 0.0 {
            return vec![];
        }
        let mut scored: Vec<(String, f32)> = self
            .entries
            .iter()
            .filter_map(|(slug, iv)| {
                let v_norm = norm(&iv.vector);
                if v_norm == 0.0 {
                    return None;
                }
                let dot: f32 = query_vec
                    .iter()
                    .zip(iv.vector.iter())
                    .map(|(a, b)| a * b)
                    .sum();
                Some((slug.clone(), dot / (q_norm * v_norm)))
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    /// Drop entries whose slug is no longer in `live_slugs`.
    pub fn prune(&mut self, live_slugs: &std::collections::HashSet<String>) -> usize {
        let before = self.entries.len();
        self.entries.retain(|k, _| live_slugs.contains(k));
        before - self.entries.len()
    }
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn empty_index_search_returns_empty() {
        let idx = EmbeddingsIndex::empty("voyage-3", 3);
        assert!(idx.search(&[1.0, 0.0, 0.0], 5).is_empty());
    }

    #[test]
    fn search_ranks_by_cosine_similarity() {
        let mut idx = EmbeddingsIndex::empty("voyage-3", 3);
        idx.upsert("a", 100, vec![1.0, 0.0, 0.0]);
        idx.upsert("b", 100, vec![0.0, 1.0, 0.0]);
        idx.upsert("c", 100, vec![0.9, 0.1, 0.0]);
        let results = idx.search(&[1.0, 0.0, 0.0], 3);
        assert_eq!(results[0].0, "a");
        assert_eq!(results[1].0, "c");
        assert_eq!(results[2].0, "b");
    }

    #[test]
    fn search_handles_zero_vector_query() {
        let mut idx = EmbeddingsIndex::empty("voyage-3", 3);
        idx.upsert("a", 100, vec![1.0, 0.0, 0.0]);
        let results = idx.search(&[0.0, 0.0, 0.0], 3);
        assert!(results.is_empty());
    }

    #[test]
    fn search_respects_limit() {
        let mut idx = EmbeddingsIndex::empty("voyage-3", 2);
        for i in 0..10 {
            idx.upsert(format!("p{i}"), 100, vec![1.0, i as f32 / 10.0]);
        }
        let results = idx.search(&[1.0, 0.5], 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn search_rejects_dimension_mismatch() {
        let mut idx = EmbeddingsIndex::empty("voyage-3", 3);
        idx.upsert("a", 100, vec![1.0, 0.0, 0.0]);
        let results = idx.search(&[1.0, 0.0], 3); // wrong dim
        assert!(results.is_empty());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let mut idx = EmbeddingsIndex::empty("voyage-3", 2);
        idx.upsert("a", 1, vec![0.1, 0.2]);
        idx.save(tmp.path()).unwrap();
        let loaded = EmbeddingsIndex::load(tmp.path()).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.dim, 2);
        assert_eq!(loaded.provider, "voyage-3");
    }

    #[test]
    fn is_fresh_matches_mtime() {
        let mut idx = EmbeddingsIndex::empty("voyage-3", 1);
        idx.upsert("a", 100, vec![0.5]);
        assert!(idx.is_fresh("a", 100));
        assert!(!idx.is_fresh("a", 200));
        assert!(!idx.is_fresh("b", 100));
    }

    #[test]
    fn prune_drops_dead_entries() {
        let mut idx = EmbeddingsIndex::empty("voyage-3", 1);
        idx.upsert("a", 1, vec![0.1]);
        idx.upsert("b", 1, vec![0.2]);
        let mut live = std::collections::HashSet::new();
        live.insert("a".to_string());
        let dropped = idx.prune(&live);
        assert_eq!(dropped, 1);
        assert!(!idx.entries.contains_key("b"));
    }

    #[test]
    fn stale_schema_returns_empty() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join(EmbeddingsIndex::FILENAME),
            r#"{"version": 999, "provider": "x", "dim": 3, "entries": {}}"#,
        )
        .unwrap();
        let loaded = EmbeddingsIndex::load(tmp.path()).unwrap();
        assert_eq!(loaded.version, 1);
        assert!(loaded.entries.is_empty());
    }
}
