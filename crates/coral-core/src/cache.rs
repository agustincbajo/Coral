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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| CoralError::Io {
                path: parent.to_path_buf(),
                source: e,
            })?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| CoralError::Walk(format!("cache serialize error: {e}")))?;
        fs::write(&path, content).map_err(|e| CoralError::Io {
            path: path.clone(),
            source: e,
        })?;
        Ok(path)
    }

    pub fn get(&self, rel_path: &str, mtime_secs: i64) -> Option<&Frontmatter> {
        self.entries
            .get(rel_path)
            .filter(|e| e.mtime_secs == mtime_secs)
            .map(|e| &e.frontmatter)
    }

    pub fn insert(
        &mut self,
        rel_path: impl Into<String>,
        mtime_secs: i64,
        frontmatter: Frontmatter,
    ) {
        self.entries.insert(
            rel_path.into(),
            CacheEntry {
                mtime_secs,
                frontmatter,
            },
        );
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
        cache.insert("modules/order.md", 1234567, sample_fm("order"));
        let path = cache.save(tmp.path()).expect("save");
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), WalkCache::FILENAME);

        let reloaded = WalkCache::load(tmp.path()).expect("reload");
        assert_eq!(reloaded.entries.len(), 1);
        let fm = reloaded.get("modules/order.md", 1234567).expect("hit");
        assert_eq!(fm.slug, "order");
    }

    #[test]
    fn get_returns_none_on_mtime_mismatch() {
        let mut cache = WalkCache::default_v1();
        cache.insert("a.md", 1000, sample_fm("a"));
        assert!(cache.get("a.md", 1000).is_some());
        assert!(cache.get("a.md", 1001).is_none());
        assert!(cache.get("nonexistent.md", 1000).is_none());
    }

    #[test]
    fn prune_drops_dead_entries() {
        let mut cache = WalkCache::default_v1();
        cache.insert("a.md", 1, sample_fm("a"));
        cache.insert("b.md", 2, sample_fm("b"));
        cache.insert("c.md", 3, sample_fm("c"));

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
}
