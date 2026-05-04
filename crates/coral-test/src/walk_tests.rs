//! Recursive walker for `.coral/tests/**` shared by every test reader.
//!
//! ## Why this exists
//!
//! v0.19's `coral test-discover --commit` wrote candidate YAML files
//! to `.coral/tests/discovered/<id>.yaml`. The advertised workflow:
//! commit those files, then `coral test` reads + runs them. But every
//! reader (`UserDefinedRunner::discover_tests_dir`, `HurlRunner`'s
//! consumer scan, and `contract_check::parse_consumer_for_repo`) was
//! using non-recursive `read_dir` — it stopped at directory entries
//! and silently skipped everything in subdirectories. Files committed
//! by `--commit` were therefore invisible to the test runner.
//!
//! Centralizing the walk here ensures every reader sees the same set
//! of files. Subdirectory support is the explicit contract; callers
//! that DON'T want to recurse can keep their own non-recursive
//! `read_dir`.
//!
//! ## What gets visited
//!
//! - Files at any depth under `<root>/.coral/tests/`.
//! - Order: by sorted path (lexicographic). This is intentional so
//!   `coral test` runs the same suites in the same order on every
//!   developer's machine — required for snapshot tests + CI parity.
//!
//! ## What gets skipped
//!
//! - Hidden files (`.gitignore`, `.DS_Store`, etc.) — they never carry
//!   test specs and would clutter parse error messages.
//! - The `.coral/tests/` directory itself if it doesn't exist (returns
//!   an empty Vec, not an error — a project that hasn't authored any
//!   tests yet is a valid state, not a failure).
//! - Files whose extension doesn't match the requested filter list.

use std::path::{Path, PathBuf};

/// Walk `<project_root>/.coral/tests/` recursively and return every
/// file whose extension is in `extensions` (case-insensitive, no
/// leading dot — e.g. `["yaml", "yml"]`).
///
/// Returns paths sorted lexicographically so callers get deterministic
/// ordering across runs and platforms.
///
/// **Error model:** filesystem errors during the walk are surfaced as
/// `std::io::Error`. Missing `.coral/tests` is NOT an error — returns
/// an empty Vec.
///
/// **Hidden files (`.foo`) are skipped** because the only files
/// authored under `.coral/tests/` are user-written test specs; a
/// hidden file there is overwhelmingly likely to be a stray editor
/// swap file or `.DS_Store`, not a spec the user wants run.
pub fn walk_tests_recursive(
    project_root: &Path,
    extensions: &[&str],
) -> std::io::Result<Vec<PathBuf>> {
    let root = project_root.join(".coral/tests");
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    walk_dir(&root, extensions, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk_dir(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        // Skip dotfiles and hidden directories. `.git` would never live
        // here in practice but stay defensive.
        if file_name.starts_with('.') {
            continue;
        }
        let ftype = entry.file_type()?;
        if ftype.is_dir() {
            walk_dir(&path, extensions, out)?;
            continue;
        }
        if !ftype.is_file() {
            // Symlinks: skip. Following them risks cycles + leaking
            // outside the project; users with legitimate symlinks can
            // file an issue when the use case shows up.
            continue;
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        let matches = match ext.as_deref() {
            Some(e) => extensions
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(e)),
            None => false,
        };
        if matches {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn returns_empty_when_no_tests_dir() {
        let tmp = TempDir::new().unwrap();
        let found = walk_tests_recursive(tmp.path(), &["yaml", "yml"]).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn finds_yaml_at_root_level() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join(".coral/tests/api.yaml"));
        touch(&tmp.path().join(".coral/tests/worker.yml"));
        let found = walk_tests_recursive(tmp.path(), &["yaml", "yml"]).unwrap();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn finds_yaml_in_subdirectories_recursively() {
        // The exact case that v0.19 broke: `coral test-discover --commit`
        // writes to `.coral/tests/discovered/*.yaml`. The walker MUST
        // descend into the subdirectory.
        let tmp = TempDir::new().unwrap();
        touch(
            &tmp.path()
                .join(".coral/tests/discovered/openapi_GET__users.yaml"),
        );
        touch(&tmp.path().join(".coral/tests/manual/auth.yaml"));
        let found = walk_tests_recursive(tmp.path(), &["yaml", "yml"]).unwrap();
        assert_eq!(found.len(), 2, "got: {found:?}");
        // Sorted order — discovered/ first lexicographically.
        assert!(found[0].ends_with("discovered/openapi_GET__users.yaml"));
        assert!(found[1].ends_with("manual/auth.yaml"));
    }

    #[test]
    fn skips_files_whose_extension_does_not_match() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join(".coral/tests/case.yaml"));
        touch(&tmp.path().join(".coral/tests/notes.md"));
        touch(&tmp.path().join(".coral/tests/case.hurl"));
        let yaml_only = walk_tests_recursive(tmp.path(), &["yaml", "yml"]).unwrap();
        assert_eq!(yaml_only.len(), 1);
        assert!(yaml_only[0].ends_with("case.yaml"));
    }

    #[test]
    fn skips_hidden_files() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join(".coral/tests/case.yaml"));
        touch(&tmp.path().join(".coral/tests/.DS_Store"));
        touch(&tmp.path().join(".coral/tests/.case.yaml.swp"));
        let found = walk_tests_recursive(tmp.path(), &["yaml", "yml"]).unwrap();
        assert_eq!(found.len(), 1, "expected only case.yaml, got {found:?}");
    }

    #[test]
    fn skips_hidden_subdirectories() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join(".coral/tests/case.yaml"));
        // A hidden subdirectory shouldn't be descended into. Realistic
        // example: an editor's `.swap/` dir or a half-deleted `.git/`.
        touch(&tmp.path().join(".coral/tests/.staging/case.yaml"));
        let found = walk_tests_recursive(tmp.path(), &["yaml", "yml"]).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("case.yaml"));
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join(".coral/tests/case.YAML"));
        let found = walk_tests_recursive(tmp.path(), &["yaml"]).unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn returns_paths_in_sorted_order() {
        // Sorted ordering is part of the contract — snapshot tests
        // depend on stable test execution order.
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join(".coral/tests/c.yaml"));
        touch(&tmp.path().join(".coral/tests/a.yaml"));
        touch(&tmp.path().join(".coral/tests/sub/b.yaml"));
        let found = walk_tests_recursive(tmp.path(), &["yaml"]).unwrap();
        let names: Vec<&std::ffi::OsStr> = found.iter().map(|p| p.file_name().unwrap()).collect();
        // Lexicographic on the full path: ".coral/tests/a.yaml",
        // ".coral/tests/c.yaml", ".coral/tests/sub/b.yaml".
        assert_eq!(names, vec!["a.yaml", "c.yaml", "b.yaml"]);
    }
}
