//! SQLite-backed `EmbeddingsIndex`.
//!
//! Storage: `<wiki_root>/.coral-embeddings.db` — opt-in alternative to the
//! JSON backend (see [`crate::embeddings`] and ADR 0006). Uses a single
//! `embeddings(slug, mtime_secs, vector BLOB)` table plus a small `meta`
//! key/value table for `provider`, `dim`, and `schema_version`.
//!
//! The actual `sqlite-vec` extension is **not** used: it requires a C
//! dependency and complicates cross-platform builds. Instead, vectors are
//! stored as little-endian f32 BLOBs and cosine similarity is computed in
//! Rust (same code path as the JSON backend). This still buys us durable
//! storage and O(1) per-slug lookup without a JSON parse on load — the two
//! benefits the ADR called out as motivation for migrating off JSON once a
//! wiki crosses ~5k pages.
//!
//! API mirrors [`crate::embeddings::EmbeddingsIndex`] so the CLI can switch
//! between backends via the `CORAL_EMBEDDINGS_BACKEND` env var without any
//! call-site changes beyond the construction site.
//!
//! `bundled` rusqlite ships the SQLite source in-tree, so users do not need a
//! system SQLite. This adds ~1 MB to the release binary.

use crate::error::{CoralError, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Filename used inside `<wiki_root>` for the on-disk SQLite database.
pub const SQLITE_FILENAME: &str = ".coral-embeddings.db";

/// Schema version stored in the `meta` table. Bump on incompatible layouts.
pub const SCHEMA_VERSION: i64 = 1;

pub struct SqliteEmbeddingsIndex {
    conn: Connection,
    pub provider: String,
    pub dim: usize,
}

impl SqliteEmbeddingsIndex {
    /// Open or create the on-disk database under `wiki_root`.
    ///
    /// On first open, the schema is created and the `meta` table is seeded with
    /// default `provider = "voyage-3"` and `dim = 1024` so the index behaves
    /// identically to a freshly-loaded JSON index.
    pub fn open(wiki_root: &Path) -> Result<Self> {
        let path = wiki_root.join(SQLITE_FILENAME);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CoralError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        let conn = Connection::open(&path).map_err(map_sqlite)?;
        let mut me = Self {
            conn,
            provider: "voyage-3".to_string(),
            dim: 1024,
        };
        me.init_schema()?;
        me.load_meta()?;
        Ok(me)
    }

    /// In-memory database for tests. Provider/dim are taken verbatim and stored
    /// into `meta` so behaviour matches an on-disk open.
    pub fn empty(provider: &str, dim: usize) -> Self {
        let conn = Connection::open_in_memory().expect("in-memory sqlite open");
        let mut me = Self {
            conn,
            provider: provider.to_string(),
            dim,
        };
        me.init_schema().expect("init in-memory schema");
        me.write_meta_provider_dim()
            .expect("seed in-memory provider/dim");
        me
    }

    fn init_schema(&mut self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS embeddings (
                    slug TEXT PRIMARY KEY,
                    mtime_secs INTEGER NOT NULL,
                    vector BLOB NOT NULL
                );
                CREATE TABLE IF NOT EXISTS meta (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );",
            )
            .map_err(map_sqlite)?;
        // Always (re)assert schema_version so re-opens see it.
        self.conn
            .execute(
                "INSERT INTO meta(key, value) VALUES('schema_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![SCHEMA_VERSION.to_string()],
            )
            .map_err(map_sqlite)?;
        Ok(())
    }

    /// Read provider/dim from `meta`. Falls back to the struct defaults set in
    /// [`Self::open`] if the keys aren't present yet (fresh database).
    fn load_meta(&mut self) -> Result<()> {
        if let Some(p) = self.read_meta("provider")? {
            self.provider = p;
        }
        if let Some(d) = self.read_meta("dim")? {
            self.dim = d.parse::<usize>().unwrap_or(self.dim);
        }
        // Ensure the meta row exists so subsequent reopens are stable.
        self.write_meta_provider_dim()?;
        Ok(())
    }

    fn read_meta(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .map_err(map_sqlite)
    }

    fn write_meta_provider_dim(&mut self) -> Result<()> {
        let tx = self.conn.transaction().map_err(map_sqlite)?;
        tx.execute(
            "INSERT INTO meta(key, value) VALUES('provider', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![self.provider],
        )
        .map_err(map_sqlite)?;
        tx.execute(
            "INSERT INTO meta(key, value) VALUES('dim', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![self.dim.to_string()],
        )
        .map_err(map_sqlite)?;
        tx.commit().map_err(map_sqlite)?;
        Ok(())
    }

    /// Override provider/dim and persist the change to the `meta` table.
    /// Used when the CLI detects a provider/model switch and needs to mark
    /// the on-disk index for the new schema. Existing rows are left intact;
    /// the caller is expected to follow up with [`Self::prune`] or fresh
    /// upserts if the new dimension differs.
    pub fn set_provider_dim(&mut self, provider: &str, dim: usize) -> Result<()> {
        self.provider = provider.to_string();
        self.dim = dim;
        self.write_meta_provider_dim()
    }

    /// Insert or replace the vector for a slug.
    pub fn upsert(&mut self, slug: &str, mtime_secs: i64, vector: Vec<f32>) -> Result<()> {
        // v0.19.5 audit M2: refuse a vector whose length doesn't
        // match `self.dim` so the caller fails loudly instead of
        // shipping a corrupt SQLite cache that produces zero search
        // results forever.
        if vector.len() != self.dim {
            return Err(crate::error::CoralError::Sqlite(format!(
                "embeddings dim mismatch on upsert(`{slug}`): expected {}, got {}",
                self.dim,
                vector.len()
            )));
        }
        let bytes = encode_vec(&vector);
        self.conn
            .execute(
                "INSERT INTO embeddings(slug, mtime_secs, vector) VALUES(?1, ?2, ?3)
                 ON CONFLICT(slug) DO UPDATE SET
                    mtime_secs = excluded.mtime_secs,
                    vector = excluded.vector",
                params![slug, mtime_secs, bytes],
            )
            .map_err(map_sqlite)?;
        Ok(())
    }

    /// `true` iff `slug` is present and stored mtime equals `mtime_secs`.
    pub fn is_fresh(&self, slug: &str, mtime_secs: i64) -> Result<bool> {
        let stored: Option<i64> = self
            .conn
            .query_row(
                "SELECT mtime_secs FROM embeddings WHERE slug = ?1",
                params![slug],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sqlite)?;
        Ok(stored == Some(mtime_secs))
    }

    /// Cosine-similarity search. Returns top-`limit` entries sorted desc.
    /// Empty result if the query length doesn't match `self.dim`, the limit is
    /// 0, the query is the zero vector, or the index is empty.
    pub fn search(&self, query: &[f32], limit: usize) -> Result<Vec<(String, f32)>> {
        if query.len() != self.dim || limit == 0 {
            return Ok(vec![]);
        }
        let q_norm = norm(query);
        if q_norm == 0.0 {
            return Ok(vec![]);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT slug, vector FROM embeddings")
            .map_err(map_sqlite)?;
        let rows = stmt
            .query_map([], |r| {
                let slug: String = r.get(0)?;
                let blob: Vec<u8> = r.get(1)?;
                Ok((slug, blob))
            })
            .map_err(map_sqlite)?;

        let mut scored: Vec<(String, f32)> = Vec::new();
        for row in rows {
            let (slug, blob) = row.map_err(map_sqlite)?;
            let v = decode_vec(&blob);
            if v.len() != self.dim {
                continue;
            }
            let v_norm = norm(&v);
            if v_norm == 0.0 {
                continue;
            }
            let dot: f32 = query.iter().zip(v.iter()).map(|(a, b)| a * b).sum();
            scored.push((slug, dot / (q_norm * v_norm)));
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored)
    }

    /// Drop entries whose slug is not in `live_slugs`. Returns count removed.
    pub fn prune(&mut self, live_slugs: &HashSet<String>) -> Result<usize> {
        let before: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
            .map_err(map_sqlite)?;
        // Collect all slugs, drop the dead ones in a transaction.
        let mut stmt = self
            .conn
            .prepare("SELECT slug FROM embeddings")
            .map_err(map_sqlite)?;
        let slugs: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(map_sqlite)?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);

        let dead: Vec<&String> = slugs.iter().filter(|s| !live_slugs.contains(*s)).collect();
        let tx = self.conn.transaction().map_err(map_sqlite)?;
        for s in &dead {
            tx.execute("DELETE FROM embeddings WHERE slug = ?1", params![s])
                .map_err(map_sqlite)?;
        }
        tx.commit().map_err(map_sqlite)?;

        let after: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
            .map_err(map_sqlite)?;
        Ok((before - after).max(0) as usize)
    }

    /// SQLite auto-commits each statement; this is a no-op stub kept for API
    /// parity with the JSON backend. Returns the database path so callers can
    /// log "saved to X" messages without branching on backend.
    pub fn save(&self, wiki_root: &Path) -> Result<PathBuf> {
        Ok(wiki_root.join(SQLITE_FILENAME))
    }
}

fn map_sqlite(e: rusqlite::Error) -> CoralError {
    CoralError::Sqlite(e.to_string())
}

fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

fn encode_vec(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

fn decode_vec(bytes: &[u8]) -> Vec<f32> {
    if !bytes.len().is_multiple_of(4) {
        return vec![];
    }
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn sqlite_round_trip_in_memory() {
        // Open in-memory, upsert 3 entries, retrieve via search. The closest
        // match (vector aligned with [1,0,0]) must come first.
        let mut idx = SqliteEmbeddingsIndex::empty("voyage-3", 3);
        idx.upsert("a", 100, vec![1.0, 0.0, 0.0]).unwrap();
        idx.upsert("b", 100, vec![0.0, 1.0, 0.0]).unwrap();
        idx.upsert("c", 100, vec![0.9, 0.1, 0.0]).unwrap();
        let results = idx.search(&[1.0, 0.0, 0.0], 3).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, "a");
        assert_eq!(results[1].0, "c");
        assert_eq!(results[2].0, "b");
    }

    #[test]
    fn sqlite_round_trip_on_disk() {
        // Open in tempdir, upsert, drop the connection, re-open, verify
        // entries persist. This proves SQLite auto-commit per statement
        // and that `meta` is restored across opens.
        let tmp = TempDir::new().unwrap();
        {
            let mut idx = SqliteEmbeddingsIndex::open(tmp.path()).unwrap();
            assert_eq!(idx.dim, 1024);
            let v: Vec<f32> = (0..1024).map(|i| i as f32 / 1024.0).collect();
            idx.upsert("page-a", 42, v).unwrap();
            idx.upsert("page-b", 99, vec![0.1; 1024]).unwrap();
        }
        let idx = SqliteEmbeddingsIndex::open(tmp.path()).unwrap();
        assert_eq!(idx.dim, 1024);
        assert_eq!(idx.provider, "voyage-3");
        assert!(idx.is_fresh("page-a", 42).unwrap());
        assert!(idx.is_fresh("page-b", 99).unwrap());
        assert!(!idx.is_fresh("page-a", 0).unwrap());
        // path returned by save() points into the tempdir.
        let p = idx.save(tmp.path()).unwrap();
        assert!(p.ends_with(SQLITE_FILENAME));
    }

    #[test]
    fn sqlite_upsert_replaces() {
        // Inserting twice under the same slug must keep only the latest
        // vector — the storage layer is a map, not an append log.
        let mut idx = SqliteEmbeddingsIndex::empty("voyage-3", 3);
        idx.upsert("a", 100, vec![1.0, 0.0, 0.0]).unwrap();
        idx.upsert("a", 200, vec![0.0, 1.0, 0.0]).unwrap();
        assert!(idx.is_fresh("a", 200).unwrap());
        assert!(!idx.is_fresh("a", 100).unwrap());
        // Search aligned with the *new* vector ranks "a" first; with the old
        // direction it would still return "a" (only entry) but we mainly
        // assert that the second upsert wasn't a no-op.
        let r = idx.search(&[0.0, 1.0, 0.0], 1).unwrap();
        assert_eq!(r[0].0, "a");
        assert!((r[0].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn sqlite_is_fresh_compares_mtime() {
        let mut idx = SqliteEmbeddingsIndex::empty("voyage-3", 1);
        idx.upsert("a", 100, vec![0.5]).unwrap();
        assert!(idx.is_fresh("a", 100).unwrap());
        assert!(!idx.is_fresh("a", 200).unwrap());
        assert!(!idx.is_fresh("missing", 100).unwrap());
    }

    #[test]
    fn sqlite_prune_removes_dead_slugs() {
        let mut idx = SqliteEmbeddingsIndex::empty("voyage-3", 1);
        idx.upsert("a", 1, vec![0.1]).unwrap();
        idx.upsert("b", 1, vec![0.2]).unwrap();
        idx.upsert("c", 1, vec![0.3]).unwrap();
        let mut live = HashSet::new();
        live.insert("a".to_string());
        live.insert("c".to_string());
        let dropped = idx.prune(&live).unwrap();
        assert_eq!(dropped, 1);
        assert!(idx.is_fresh("a", 1).unwrap());
        assert!(!idx.is_fresh("b", 1).unwrap());
        assert!(idx.is_fresh("c", 1).unwrap());
    }

    #[test]
    fn sqlite_search_orders_by_cosine_descending() {
        // Each result's score should be greater-or-equal to the next. This is
        // the contract the CLI depends on for "top-k semantic search."
        let mut idx = SqliteEmbeddingsIndex::empty("voyage-3", 3);
        idx.upsert("perp", 1, vec![0.0, 1.0, 0.0]).unwrap();
        idx.upsert("close", 1, vec![0.99, 0.01, 0.0]).unwrap();
        idx.upsert("opposite", 1, vec![-1.0, 0.0, 0.0]).unwrap();
        idx.upsert("exact", 1, vec![1.0, 0.0, 0.0]).unwrap();
        let r = idx.search(&[1.0, 0.0, 0.0], 4).unwrap();
        assert_eq!(r.len(), 4);
        for w in r.windows(2) {
            assert!(
                w[0].1 >= w[1].1,
                "expected descending cosine, got {} then {}",
                w[0].1,
                w[1].1
            );
        }
        assert_eq!(r[0].0, "exact");
        assert_eq!(r[3].0, "opposite");
    }

    #[test]
    fn sqlite_provider_dim_persisted_in_meta_table() {
        // The meta table is the source of truth for provider+dim across
        // reopens. Verify both keys round-trip through the on-disk DB.
        let tmp = TempDir::new().unwrap();
        {
            let _idx = SqliteEmbeddingsIndex::open(tmp.path()).unwrap();
        }
        let conn = Connection::open(tmp.path().join(SQLITE_FILENAME)).unwrap();
        let provider: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'provider'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let dim: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'dim'", [], |r| r.get(0))
            .unwrap();
        let schema: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(provider, "voyage-3");
        assert_eq!(dim, "1024");
        assert_eq!(schema, SCHEMA_VERSION.to_string());
    }

    #[test]
    fn sqlite_search_rejects_dimension_mismatch() {
        let mut idx = SqliteEmbeddingsIndex::empty("voyage-3", 3);
        idx.upsert("a", 1, vec![1.0, 0.0, 0.0]).unwrap();
        let r = idx.search(&[1.0, 0.0], 5).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn sqlite_search_handles_zero_query() {
        let mut idx = SqliteEmbeddingsIndex::empty("voyage-3", 3);
        idx.upsert("a", 1, vec![1.0, 0.0, 0.0]).unwrap();
        let r = idx.search(&[0.0, 0.0, 0.0], 5).unwrap();
        assert!(r.is_empty());
    }

    #[test]
    fn encode_decode_roundtrip() {
        let v: Vec<f32> = vec![0.0, 1.0, -1.5, 42.125, 1e-10];
        let bytes = encode_vec(&v);
        let back = decode_vec(&bytes);
        assert_eq!(back.len(), v.len());
        for (a, b) in v.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-9);
        }
    }
}
