//! Filesystem-mtime-keyed cache for parsed `Frontmatter`. Reduces redundant
//! YAML parsing on `coral lint` / `coral stats` invocations when the
//! underlying file hasn't changed.
//!
//! Storage: `<wiki_root>/.coral-cache.json`. Auto-ignored if `coral init`
//! creates `<wiki_root>/.gitignore` (it does, in v0.3+).

use crate::error::{CoralError, Result};
use crate::frontmatter::Frontmatter;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WalkCache {
    #[serde(default = "default_version")]
    pub version: u32,
    pub entries: BTreeMap<String, CacheEntry>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub mtime_secs: i64,
    /// v0.20.1 cycle-4 audit C1: cache entries used to be keyed only by
    /// `(rel_path, mtime_secs)`. A poisoned `.coral-cache.json` (writable
    /// by anyone with repo access — including a CI step or a malicious
    /// `coral env` shim) could return a `reviewed: true` frontmatter for
    /// a file whose disk content actually says `reviewed: false`,
    /// short-circuiting the v0.20 `unreviewed-distilled` lint gate.
    /// Pinning the entry to a content hash means `read_pages` re-reads
    /// the file (cheap) and only trusts the cached parse if hash agrees.
    /// Missing in v1 entries → treated as a miss (forces a re-parse).
    #[serde(default)]
    pub content_hash: String,
    pub frontmatter: Frontmatter,
}

impl WalkCache {
    pub const FILENAME: &'static str = ".coral-cache.json";
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn load(wiki_root: &Path) -> Result<Self> {
        let path = wiki_root.join(Self::FILENAME);
        if !path.exists() {
            return Ok(Self::default_v1());
        }
        let content = fs::read_to_string(&path).map_err(|e| CoralError::Io {
            path: path.clone(),
            source: e,
        })?;
        let parsed: Self = serde_json::from_str(&content)
            .map_err(|e| CoralError::Walk(format!("cache parse error: {e}")))?;
        if parsed.version != Self::SCHEMA_VERSION {
            return Ok(Self::default_v1());
        }
        Ok(parsed)
    }

    fn default_v1() -> Self {
        Self {
            version: Self::SCHEMA_VERSION,
            entries: BTreeMap::new(),
        }
    }

    pub fn save(&self, wiki_root: &Path) -> Result<PathBuf> {
        let path = wiki_root.join(Self::FILENAME);
        // v0.19.6 audit N1: use the shared atomic-write helper so a
        // crash mid-save can't leave a half-written `.coral-cache.json`
        // — the next `coral lint` / `coral stats` would then either
        // bail on JSON parse error or, worse, treat the truncated
        // entries as authoritative and re-parse every page.
        // `atomic_write_string` itself creates parent dirs, so we
        // skip the explicit `create_dir_all` here.
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| CoralError::Walk(format!("cache serialize error: {e}")))?;
        crate::atomic::atomic_write_string(&path, &content)?;
        Ok(path)
    }

    pub fn get(&self, rel_path: &str, mtime_secs: i64, content_hash: &str) -> Option<&Frontmatter> {
        self.entries
            .get(rel_path)
            .filter(|e| e.mtime_secs == mtime_secs)
            // v0.20.1 cycle-4 audit C1: only honor a cache hit when
            // the disk content's hash matches what we recorded last
            // time. Empty `content_hash` (legacy entries from v0.20.0
            // and earlier) is treated as a miss to force a re-parse.
            .filter(|e| !e.content_hash.is_empty() && e.content_hash == content_hash)
            .map(|e| &e.frontmatter)
    }

    pub fn insert(
        &mut self,
        rel_path: impl Into<String>,
        mtime_secs: i64,
        content_hash: impl Into<String>,
        frontmatter: Frontmatter,
    ) {
        self.entries.insert(
            rel_path.into(),
            CacheEntry {
                mtime_secs,
                content_hash: content_hash.into(),
                frontmatter,
            },
        );
    }

    /// FNV-1a 64-bit, then truncated to 32 bits and hex-encoded. Same
    /// shape as `coral_env::compose_yaml::content_hash`. We don't pull
    /// that helper across the crate boundary because `coral-env`
    /// already depends on `coral-core` — pulling it back would loop.
    /// Keeping the FNV math here costs ~10 lines and zero deps.
    pub fn hash_content(content: &str) -> String {
        const FNV_OFFSET: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;
        let mut hash = FNV_OFFSET;
        for byte in content.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        format!("{:08x}", hash & 0xffff_ffff)
    }

    pub fn prune(&mut self, live_paths: &std::collections::HashSet<String>) -> usize {
        let before = self.entries.len();
        self.entries.retain(|k, _| live_paths.contains(k));
        before - self.entries.len()
    }

    pub fn mtime_of(path: &Path) -> Option<i64> {
        let meta = fs::metadata(path).ok()?;
        let mt = meta.modified().ok()?;
        let dur = mt.duration_since(SystemTime::UNIX_EPOCH).ok()?;
        Some(dur.as_secs() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{Confidence, PageType, Status};
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn sample_fm(slug: &str) -> Frontmatter {
        Frontmatter {
            slug: slug.to_string(),
            page_type: PageType::Module,
            last_updated_commit: "abc".to_string(),
            confidence: Confidence::try_new(0.7).unwrap(),
            sources: vec![],
            backlinks: vec![],
            status: Status::Draft,
            generated_at: None,
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn load_returns_default_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        let cache = WalkCache::load(tmp.path()).expect("load empty");
        assert_eq!(cache.version, WalkCache::SCHEMA_VERSION);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let mut cache = WalkCache {
            version: WalkCache::SCHEMA_VERSION,
            ..WalkCache::default()
        };
        cache.insert("modules/order.md", 1234567, "deadbeef", sample_fm("order"));
        let path = cache.save(tmp.path()).expect("save");
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), WalkCache::FILENAME);

        let reloaded = WalkCache::load(tmp.path()).expect("reload");
        assert_eq!(reloaded.entries.len(), 1);
        let fm = reloaded
            .get("modules/order.md", 1234567, "deadbeef")
            .expect("hit");
        assert_eq!(fm.slug, "order");
    }

    #[test]
    fn get_returns_none_on_mtime_mismatch() {
        let mut cache = WalkCache::default_v1();
        cache.insert("a.md", 1000, "h0", sample_fm("a"));
        assert!(cache.get("a.md", 1000, "h0").is_some());
        assert!(cache.get("a.md", 1001, "h0").is_none());
        assert!(cache.get("nonexistent.md", 1000, "h0").is_none());
    }

    /// v0.20.1 cycle-4 audit C1: a cache entry whose `content_hash`
    /// disagrees with the current disk content must be a miss, even
    /// when the mtime second matches. This is the property that
    /// stops a poisoned cache from short-circuiting the
    /// `unreviewed-distilled` lint gate.
    #[test]
    fn get_returns_none_on_content_hash_mismatch() {
        let mut cache = WalkCache::default_v1();
        cache.insert("a.md", 1000, "abc12345", sample_fm("a"));
        // Same path, same mtime, *different* hash → must miss.
        assert!(cache.get("a.md", 1000, "ffff0000").is_none());
        // Empty hash (legacy v1 entry shape) also misses, even if both
        // sides are empty — we never trust an unhashed entry.
        let mut legacy = WalkCache::default_v1();
        legacy.entries.insert(
            "old.md".to_string(),
            CacheEntry {
                mtime_secs: 1,
                content_hash: String::new(),
                frontmatter: sample_fm("old"),
            },
        );
        assert!(legacy.get("old.md", 1, "").is_none());
        assert!(legacy.get("old.md", 1, "anyhash").is_none());
    }

    #[test]
    fn prune_drops_dead_entries() {
        let mut cache = WalkCache::default_v1();
        cache.insert("a.md", 1, "h", sample_fm("a"));
        cache.insert("b.md", 2, "h", sample_fm("b"));
        cache.insert("c.md", 3, "h", sample_fm("c"));

        let mut live = HashSet::new();
        live.insert("a.md".to_string());
        live.insert("c.md".to_string());

        let removed = cache.prune(&live);
        assert_eq!(removed, 1);
        assert_eq!(cache.entries.len(), 2);
        assert!(cache.entries.contains_key("a.md"));
        assert!(!cache.entries.contains_key("b.md"));
        assert!(cache.entries.contains_key("c.md"));
    }

    #[test]
    fn stale_schema_version_starts_fresh() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(WalkCache::FILENAME);
        // Write a cache file with a future schema version that v0.3 doesn't recognize.
        let bogus = r#"{
          "version": 999,
          "entries": {
            "old.md": {
              "mtime_secs": 1,
              "frontmatter": {
                "slug": "old", "type": "module", "last_updated_commit": "x",
                "confidence": 0.5, "status": "draft"
              }
            }
          }
        }"#;
        fs::write(&path, bogus).unwrap();

        let cache = WalkCache::load(tmp.path()).expect("load");
        assert_eq!(cache.version, WalkCache::SCHEMA_VERSION);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn mtime_of_real_file_returns_some() {
        let tmp = TempDir::new().unwrap();
        let p = tmp.path().join("file.txt");
        fs::write(&p, "x").unwrap();
        let mtime = WalkCache::mtime_of(&p);
        assert!(mtime.is_some());
        assert!(mtime.unwrap() > 0);
    }

    #[test]
    fn mtime_of_missing_file_returns_none() {
        let p = PathBuf::from("/definitely/does/not/exist/file-xyz.md");
        assert!(WalkCache::mtime_of(&p).is_none());
    }

    /// v0.19.6 audit N1: under N concurrent `save` calls, every reader
    /// must see a parseable cache — never an empty / torn file. This
    /// is the same property that `atomic_write_string` provides; the
    /// test pins the migration to it by hammering save concurrently
    /// and checking the on-disk shape after every write storm.
    #[test]
    fn save_concurrent_never_writes_torn_cache() {
        let tmp = TempDir::new().unwrap();
        let wiki_root = tmp.path().to_path_buf();

        const N: usize = 10;
        std::thread::scope(|s| {
            for i in 0..N {
                let wiki_root = wiki_root.clone();
                s.spawn(move || {
                    let mut cache = WalkCache::default_v1();
                    cache.insert(format!("file-{i}.md"), 1000 + i as i64, "h", sample_fm("x"));
                    cache.save(&wiki_root).expect("save");
                });
            }
        });

        // Every successive read must parse cleanly. A torn write would
        // surface as a JSON parse error on `WalkCache::load`.
        let cache = WalkCache::load(&wiki_root).expect("load after storm must parse");
        assert_eq!(cache.version, WalkCache::SCHEMA_VERSION);
    }
}
