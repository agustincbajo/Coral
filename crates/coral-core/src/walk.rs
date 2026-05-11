//! Parallel `.md` page discovery and reading under `.wiki/`.
//!
//! Skips hidden files, the `_archive/` directory, and the top-level
//! `SCHEMA.md` / `README.md` / `index.md` / `log.md` operator files.
//! Symlinks are NOT followed.

use crate::cache::WalkCache;
use crate::error::Result;
use crate::frontmatter::body_after_frontmatter;
use crate::page::Page;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Returns all `.md` page paths under `root`, deterministically sorted.
/// Skipped:
///   - hidden files (starting with `.`) at any depth
///   - directories named `_archive/` (and everything under)
///   - non-`.md` files
///   - the SCHEMA.md file at the top level (it's the contract, not a page)
///   - the README.md file at the top level (operator notes, not a page)
///   - the index.md file at the top level (master index, not a page)
///   - the log.md file at the top level (activity log, not a page)
///
/// Walks `root` even if `root` is `.wiki/SCHEMA.md`'s parent. Symlinks NOT followed.
pub fn list_page_paths(root: impl AsRef<Path>) -> Result<Vec<PathBuf>> {
    let root = root.as_ref();
    let mut paths: Vec<PathBuf> = Vec::new();

    for entry in WalkDir::new(root).follow_links(false).into_iter() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();

        // Skip hidden basenames at any depth.
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name.starts_with('.') {
            continue;
        }

        // Skip anything under an `_archive` directory.
        if path
            .components()
            .any(|c| c.as_os_str() == std::ffi::OsStr::new("_archive"))
        {
            continue;
        }

        // Only consider files with `.md` (lowercase) extension.
        if !entry.file_type().is_file() {
            continue;
        }
        let ext_ok = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "md")
            .unwrap_or(false);
        if !ext_ok {
            continue;
        }

        // Skip the top-level SCHEMA.md / README.md / index.md / log.md.
        if path == root.join("SCHEMA.md")
            || path == root.join("README.md")
            || path == root.join("index.md")
            || path == root.join("log.md")
        {
            continue;
        }

        paths.push(path.to_path_buf());
    }

    paths.sort();
    Ok(paths)
}

/// Reads and parses every page under `root` in parallel (rayon).
/// On any per-file error, the file is logged via `tracing::warn!` and skipped;
/// the function returns the successful pages. The order is deterministic
/// (sorted by path).
///
/// Reads the on-disk `WalkCache` (`<root>/.coral-cache.json`) and skips YAML
/// parsing for files whose mtime matches a cached entry. After the walk,
/// the cache is rebuilt with live entries and saved (best-effort — a write
/// failure here is logged but doesn't fail the walk).
pub fn read_pages(root: impl AsRef<Path>) -> Result<Vec<Page>> {
    let root = root.as_ref();
    let paths = list_page_paths(root)?;

    let cache_in = WalkCache::load(root).unwrap_or_default();

    // Build (page, mtime, rel_path, content_hash) tuples in parallel. The
    // cache fast-path skips YAML deserialization when the mtime AND content
    // hash both match; otherwise we fall back to a full Page::from_content
    // parse. The content hash defends against cache poisoning — see
    // C1 in the v0.20.1 cycle-4 audit fixes.
    let parsed: Vec<Option<(Page, i64, String, String)>> = paths
        .par_iter()
        .map(|p| {
            let rel = match p.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().into_owned(),
                Err(_) => p.to_string_lossy().into_owned(),
            };
            let mtime = WalkCache::mtime_of(p);
            // v0.19.5 audit N3: cap per-file read at 32 MiB. Wiki
            // pages are markdown, not large media; anything bigger is
            // either a mistake (a binary checked in) or a DoS vector.
            const MAX_PAGE_BYTES: u64 = 32 * 1024 * 1024;
            if let Ok(meta) = fs::metadata(p)
                && meta.len() > MAX_PAGE_BYTES
            {
                tracing::warn!(
                    path = %p.display(),
                    bytes = meta.len(),
                    cap = MAX_PAGE_BYTES,
                    "skipping page: file exceeds 32 MiB cap"
                );
                return None;
            }
            let content = match fs::read_to_string(p) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(path = %p.display(), error = %e, "skipping page");
                    return None;
                }
            };
            // v0.20.1 cycle-4 audit C1: hash the on-disk content and
            // pin the cache hit to (rel_path, mtime_secs, content_hash).
            // Without this, a poisoned `.coral-cache.json` could
            // short-circuit the `unreviewed-distilled` lint gate by
            // returning a `reviewed: true` frontmatter for a file whose
            // disk content actually says `reviewed: false`.
            let hash = crate::cache::WalkCache::hash_content(&content);
            // Cache fast-path: same mtime AND same hash → reuse parsed
            // frontmatter, only re-extract body. The hash check makes
            // the cache tamper-resistant; the mtime check still
            // shaves ~one comparison on the trivial-no-change path.
            if let Some(mt) = mtime
                && let Some(fm) = cache_in.get(&rel, mt, &hash)
            {
                let body = body_after_frontmatter(&content);
                let page = Page {
                    path: p.clone(),
                    frontmatter: fm.clone(),
                    body,
                };
                return Some((page, mt, rel, hash));
            }
            // Slow path: full parse.
            match Page::from_content(&content, p.clone()) {
                Ok(page) => Some((page, mtime.unwrap_or(0), rel, hash)),
                Err(e) => {
                    tracing::warn!(path = %p.display(), error = %e, "skipping page");
                    None
                }
            }
        })
        .collect();

    // Drop the failed entries (None), keep (page, mtime, rel, hash) for
    // cache rebuild.
    let mut live: Vec<(Page, i64, String, String)> = parsed.into_iter().flatten().collect();
    live.sort_by(|a, b| a.0.path.cmp(&b.0.path));

    // Rebuild a fresh cache from live entries; this naturally prunes anything
    // that disappeared since the last walk, and refreshes mtimes + hashes.
    let mut cache_out = WalkCache {
        version: WalkCache::SCHEMA_VERSION,
        ..WalkCache::default()
    };
    let mut live_paths: HashSet<String> = HashSet::with_capacity(live.len());
    for (page, mtime, rel, hash) in &live {
        if *mtime > 0 {
            cache_out.insert(rel.clone(), *mtime, hash.clone(), page.frontmatter.clone());
            live_paths.insert(rel.clone());
        }
    }
    let _ = cache_out.prune(&live_paths);
    if let Err(e) = cache_out.save(root) {
        // Best-effort: warn but don't fail the walk.
        tracing::warn!(error = %e, "failed to persist .coral-cache.json");
    }

    let pages: Vec<Page> = live.into_iter().map(|(p, _, _, _)| p).collect();
    Ok(pages)
}

/// Filter pages to only those valid at a given point in time.
/// Used by `coral query --at <timestamp>` for bi-temporal queries.
/// Pages with `superseded_by` set are excluded — they are effectively dead.
pub fn pages_valid_at<'a>(pages: &'a [Page], at: &str) -> Vec<&'a Page> {
    pages
        .iter()
        .filter(|p| p.frontmatter.superseded_by.is_none() && p.frontmatter.is_valid_at(at))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_valid_page(path: &Path, slug: &str) {
        let body = format!(
            "---\nslug: {slug}\ntype: module\nlast_updated_commit: abc\nconfidence: 0.5\nstatus: draft\n---\n\nbody\n"
        );
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn walk_lists_md_files_only() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("a.md"), "x").unwrap();
        fs::write(root.join("b.txt"), "x").unwrap();
        fs::write(root.join("c.md"), "x").unwrap();

        let paths = list_page_paths(root).expect("walk");
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], root.join("a.md"));
        assert_eq!(paths[1], root.join("c.md"));
    }

    #[test]
    fn walk_skips_hidden_files() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::write(root.join(".hidden.md"), "x").unwrap();
        fs::write(root.join("visible.md"), "x").unwrap();

        let paths = list_page_paths(root).expect("walk");
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], root.join("visible.md"));
    }

    #[test]
    fn walk_skips_archive_directory() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("modules")).unwrap();
        fs::write(root.join("modules/order.md"), "x").unwrap();
        fs::create_dir_all(root.join("_archive/nested")).unwrap();
        fs::write(root.join("_archive/old.md"), "x").unwrap();
        fs::write(root.join("_archive/nested/x.md"), "x").unwrap();

        let paths = list_page_paths(root).expect("walk");
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], root.join("modules/order.md"));
    }

    #[test]
    fn walk_skips_top_level_schema_and_readme() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("SCHEMA.md"), "x").unwrap();
        fs::write(root.join("README.md"), "x").unwrap();
        fs::create_dir_all(root.join("modules")).unwrap();
        fs::write(root.join("modules/SCHEMA.md"), "x").unwrap();

        let paths = list_page_paths(root).expect("walk");
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], root.join("modules/SCHEMA.md"));
    }

    #[test]
    fn walk_skips_top_level_index_and_log() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("index.md"), "x").unwrap();
        fs::write(root.join("log.md"), "x").unwrap();
        fs::create_dir_all(root.join("modules")).unwrap();
        fs::write(root.join("modules/index.md"), "x").unwrap();
        fs::write(root.join("modules/order.md"), "x").unwrap();

        let paths = list_page_paths(root).expect("walk");
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], root.join("modules/index.md"));
        assert_eq!(paths[1], root.join("modules/order.md"));
    }

    #[test]
    fn walk_does_not_skip_nested_index_or_log() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("concepts")).unwrap();
        fs::write(root.join("concepts/index.md"), "x").unwrap();
        fs::write(root.join("concepts/log.md"), "x").unwrap();

        let paths = list_page_paths(root).expect("walk");
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&root.join("concepts/index.md")));
        assert!(paths.contains(&root.join("concepts/log.md")));
    }

    #[test]
    fn walk_recursive_descends() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        let nested = root.join("a/b/c/page.md");
        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        fs::write(&nested, "x").unwrap();

        let paths = list_page_paths(root).expect("walk");
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], nested);
    }

    #[test]
    fn walk_returns_sorted() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("z.md"), "x").unwrap();
        fs::write(root.join("a.md"), "x").unwrap();
        fs::write(root.join("m.md"), "x").unwrap();

        let paths = list_page_paths(root).expect("walk");
        let names: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.md", "m.md", "z.md"]);
    }

    #[test]
    fn read_pages_parses_valid_pages() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        write_valid_page(&root.join("modules/a.md"), "a");
        write_valid_page(&root.join("modules/b.md"), "b");

        let pages = read_pages(root).expect("read");
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].frontmatter.slug, "a");
        assert_eq!(pages[1].frontmatter.slug, "b");
    }

    /// v0.20.1 cycle-4 audit C1: a poisoned `.coral-cache.json` whose
    /// frontmatter disagrees with the on-disk content (e.g. `reviewed:
    /// true` cached for a file whose body actually says `reviewed:
    /// false`) must NOT short-circuit `read_pages`. The disk wins, and
    /// the lint gate (`unreviewed-distilled`) sees the truthful state.
    ///
    /// Pre-fix this test failed: the cache hit returned the poisoned
    /// `reviewed: true` frontmatter and the slow-path full parse never
    /// ran. Post-fix the content-hash check forces a re-parse and the
    /// disk's `reviewed: false` lands in the returned Page.
    #[test]
    fn read_pages_rejects_poisoned_cache_via_hash_check() {
        use crate::cache::WalkCache;
        use crate::frontmatter::{Confidence, PageType, Status};
        use std::collections::BTreeMap;
        use std::time::SystemTime;
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        // Disk content: an unreviewed distilled page.
        let body = "---\n\
slug: poisoned\n\
type: module\n\
last_updated_commit: abc\n\
confidence: 0.5\n\
status: draft\n\
reviewed: false\n\
source:\n  runner: claude-sonnet-4-5\n\
---\n\nbody\n";
        let page_path = root.join("modules/poisoned.md");
        fs::create_dir_all(page_path.parent().unwrap()).unwrap();
        fs::write(&page_path, body).unwrap();

        // Build a poisoned cache that says `reviewed: true`. We honor
        // the file's actual mtime so the mtime check would pass — only
        // the content-hash check stops the poisoning.
        let mtime = WalkCache::mtime_of(&page_path).expect("mtime");
        let mut extra = BTreeMap::new();
        extra.insert("reviewed".into(), serde_yaml_ng::Value::Bool(true));
        let poisoned_fm = crate::frontmatter::Frontmatter {
            slug: "poisoned".to_string(),
            page_type: PageType::Module,
            last_updated_commit: "abc".to_string(),
            confidence: Confidence::try_new(0.5).unwrap(),
            sources: vec![],
            backlinks: vec![],
            status: Status::Draft,
            generated_at: None,
            valid_from: None,
            valid_to: None,
            superseded_by: None,
            extra,
        };
        let mut poisoned = WalkCache {
            version: WalkCache::SCHEMA_VERSION,
            ..WalkCache::default()
        };
        // Use a hash that matches the cache's idea of the body but
        // NOT the actual disk content — that's the poisoning vector.
        // (An attacker who can write the cache controls this string.)
        poisoned.insert("modules/poisoned.md", mtime, "deadbeef", poisoned_fm);
        poisoned.save(root).expect("save poisoned cache");

        // Sanity: the cache file must exist before the walk so the
        // fast path is reachable.
        let cache_file = root.join(WalkCache::FILENAME);
        assert!(cache_file.exists());
        // Touch to ensure mtime second is preserved across the test.
        let _ = SystemTime::now();

        let pages = read_pages(root).expect("read");
        assert_eq!(pages.len(), 1);
        let page = &pages[0];
        // Disk wins: `reviewed: false` from the body, not the
        // poisoned cache's `true`.
        let reviewed_flag = page
            .frontmatter
            .extra
            .get("reviewed")
            .and_then(|v| v.as_bool());
        assert_eq!(
            reviewed_flag,
            Some(false),
            "poisoned cache must not override on-disk frontmatter; got page.extra.reviewed = {reviewed_flag:?}"
        );
    }

    #[test]
    fn read_pages_skips_malformed_silently() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        write_valid_page(&root.join("modules/good.md"), "good");
        // Malformed: no frontmatter at all.
        fs::create_dir_all(root.join("modules")).unwrap();
        fs::write(root.join("modules/bad.md"), "no frontmatter here\n").unwrap();

        let pages = read_pages(root).expect("read");
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].frontmatter.slug, "good");
    }

    // ── pages_valid_at + superseded_by (M2.16) ──────────────────────────

    fn write_page_with_frontmatter(path: &Path, yaml: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let content = format!("---\n{yaml}---\n\nbody\n");
        fs::write(path, content).unwrap();
    }

    #[test]
    fn pages_valid_at_excludes_superseded_pages() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();

        // A normal page valid in the range
        write_page_with_frontmatter(
            &root.join("modules/current.md"),
            "slug: current\ntype: module\nlast_updated_commit: abc\nconfidence: 0.5\nstatus: draft\nvalid_from: 2024-01-01T00:00:00Z\n",
        );
        // A superseded page also valid in the range
        write_page_with_frontmatter(
            &root.join("modules/old.md"),
            "slug: old\ntype: module\nlast_updated_commit: abc\nconfidence: 0.5\nstatus: archived\nvalid_from: 2024-01-01T00:00:00Z\nsuperseded_by: current\n",
        );

        let pages = read_pages(root).expect("read");
        assert_eq!(pages.len(), 2, "both pages should be read");

        let filtered = pages_valid_at(&pages, "2024-06-15T00:00:00Z");
        assert_eq!(filtered.len(), 1, "superseded page should be excluded");
        assert_eq!(filtered[0].frontmatter.slug, "current");
    }

    #[test]
    fn pages_valid_at_includes_non_superseded_pages() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();

        write_page_with_frontmatter(
            &root.join("modules/a.md"),
            "slug: a\ntype: module\nlast_updated_commit: abc\nconfidence: 0.5\nstatus: draft\nvalid_from: 2024-01-01T00:00:00Z\n",
        );
        write_page_with_frontmatter(
            &root.join("modules/b.md"),
            "slug: b\ntype: module\nlast_updated_commit: abc\nconfidence: 0.5\nstatus: draft\nvalid_from: 2024-01-01T00:00:00Z\n",
        );

        let pages = read_pages(root).expect("read");
        let filtered = pages_valid_at(&pages, "2024-06-15T00:00:00Z");
        assert_eq!(filtered.len(), 2);
    }
}
