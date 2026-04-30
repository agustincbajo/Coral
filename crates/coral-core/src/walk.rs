//! Parallel `.md` page discovery and reading under `.wiki/`.
//!
//! Skips hidden files, the `_archive/` directory, and the top-level
//! `SCHEMA.md` / `README.md` operator files. Symlinks are NOT followed.

use crate::error::Result;
use crate::page::Page;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Returns all `.md` page paths under `root`, deterministically sorted.
/// Skipped:
///   - hidden files (starting with `.`) at any depth
///   - directories named `_archive/` (and everything under)
///   - non-`.md` files
///   - the SCHEMA.md file at the top level (it's the contract, not a page)
///   - the README.md file at the top level (operator notes, not a page)
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

        // Skip the top-level SCHEMA.md / README.md.
        if path == root.join("SCHEMA.md") || path == root.join("README.md") {
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
pub fn read_pages(root: impl AsRef<Path>) -> Result<Vec<Page>> {
    let paths = list_page_paths(root)?;

    let mut pages: Vec<Page> = paths
        .par_iter()
        .filter_map(|p| match Page::from_file(p) {
            Ok(page) => Some(page),
            Err(e) => {
                tracing::warn!(path = %p.display(), error = %e, "skipping page");
                None
            }
        })
        .collect();

    pages.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(pages)
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
}
