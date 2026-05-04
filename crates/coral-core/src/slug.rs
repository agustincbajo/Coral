//! Filesystem-safe slug allowlist.
//!
//! v0.19.5 audit identified three sites where an attacker-controlled
//! slug could escape the wiki root: LLM-emitted plan entries, page
//! frontmatter on round-trip, and `coral export-multi` rendering.
//! All three now route through [`is_safe_filename_slug`] before the
//! slug is interpolated into a path.
//!
//! v0.19.6 audit H1 added [`is_safe_repo_name`] for repo names from
//! `coral.toml` — a sibling check with the SAME allowlist (kebab /
//! snake / ASCII alphanum). Before this, `name = "../escape"` in a
//! `[[repos]]` block produced `<project_root>/repos/../escape` for
//! `resolved_path`, and `coral project sync` then `git clone`d into
//! that escaped path.
//!
//! The allowlist is intentionally tighter than POSIX would require —
//! we only accept what kebab/snake-cased slugs need (`[a-zA-Z0-9_-]`)
//! so any path-traversal or shell-metachar surprise is rejected at
//! the source.

/// Returns `true` when `s` is a safe filename slug suitable for direct
/// path interpolation (`wiki_root.join(format!("{slug}.md"))`).
///
/// Allowed: ASCII alphanumeric + `_` + `-`, length 1..=200, no leading
/// `.` or `-`.
///
/// Rejected: empty strings, `..`, paths containing `/` or `\`,
/// whitespace, NUL bytes, leading dot/hyphen, or anything > 200 chars.
pub fn is_safe_filename_slug(s: &str) -> bool {
    // Empty + length cap. 200 chars is generous for kebab-case slugs;
    // anything longer is almost certainly an attempt to overflow some
    // downstream buffer or produce a path that looks short visually.
    if s.is_empty() || s.len() > 200 {
        return false;
    }
    // Reject leading `.` (hidden file) and leading `-` (looks like a
    // CLI flag, breaks `git`/`rm` invocations downstream).
    let first = s.as_bytes()[0];
    if first == b'.' || first == b'-' {
        return false;
    }
    // Path-traversal sentinel: explicit `..` rejection in addition to
    // the byte-level checks below (a slug that's exactly `..` would
    // still pass the byte check on `.`, but we already rejected `.`-leading).
    if s == ".." {
        return false;
    }
    // Per-byte allowlist. Reject anything outside `[a-zA-Z0-9_-]`.
    for &b in s.as_bytes() {
        let ok = b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
        if !ok {
            return false;
        }
    }
    true
}

/// Returns `true` when `s` is a safe repo name suitable for direct
/// path interpolation in `<project_root>/<path_template>` (where
/// `path_template` substitutes `{name}` with this string).
///
/// Same allowlist as [`is_safe_filename_slug`]: ASCII alphanumeric
/// plus `_`/`-`, length 1..=200, no leading `.` or `-`.
///
/// Kept as a separate function (with its own name) so future
/// divergence (e.g. allowing `/` in scoped names like `scope/repo`)
/// doesn't have to thread through every slug call site.
///
/// v0.19.6 audit H1: `coral project sync` would otherwise run
/// `git clone <url> <project_root>/repos/<name>`, and
/// `<name> = "../escape"` would write outside the project root.
pub fn is_safe_repo_name(s: &str) -> bool {
    is_safe_filename_slug(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_kebab_and_snake_case() {
        for s in ["foo", "foo-bar", "Foo_42", "x", "a-b-c-d", "X"] {
            assert!(is_safe_filename_slug(s), "should accept {s:?}");
        }
    }

    #[test]
    fn rejects_path_traversal_and_separators() {
        for s in ["../etc", "foo/bar", "foo\\bar", "..", "."] {
            assert!(!is_safe_filename_slug(s), "should reject {s:?}");
        }
    }

    #[test]
    fn rejects_leading_dot_or_hyphen() {
        for s in [".hidden", "-flag", ".bashrc"] {
            assert!(!is_safe_filename_slug(s), "should reject {s:?}");
        }
    }

    #[test]
    fn rejects_whitespace_and_control() {
        for s in ["foo bar", "foo\tbar", "foo\nbar", "foo\0bar"] {
            assert!(!is_safe_filename_slug(s), "should reject {s:?}");
        }
    }

    #[test]
    fn rejects_empty_and_overlong() {
        assert!(!is_safe_filename_slug(""));
        let max = "a".repeat(200);
        assert!(is_safe_filename_slug(&max));
        let over = "a".repeat(201);
        assert!(!is_safe_filename_slug(&over));
    }

    #[test]
    fn rejects_unicode_lookalikes() {
        // Non-ASCII alphanumerics aren't in the allowlist; rejecting
        // them is cheaper and safer than running NFKC normalization
        // and a re-check.
        for s in ["café", "naïve", "日本"] {
            assert!(!is_safe_filename_slug(s), "should reject {s:?}");
        }
    }

    #[test]
    fn rejects_meta_characters() {
        // Common shell/glob metas — none of them are in the allowlist
        // but we pin them explicitly so a future relaxation can't
        // silently regress.
        for s in [
            "foo;bar", "foo|bar", "foo$bar", "foo*bar", "foo?bar", "foo`bar`", "foo&bar",
            "foo'bar", "foo\"bar",
        ] {
            assert!(!is_safe_filename_slug(s), "should reject {s:?}");
        }
    }

    /// v0.19.6 audit H1: the repo-name allowlist must reject
    /// path-traversal segments like `..`, `../escape`, `foo/bar`.
    #[test]
    fn repo_name_rejects_traversal_and_separators() {
        for bad in [
            "../escape",
            "..",
            "foo/bar",
            "foo\\bar",
            ".hidden",
            "-flag",
            "",
            "foo bar",
        ] {
            assert!(!is_safe_repo_name(bad), "should reject {bad:?}");
        }
    }

    #[test]
    fn repo_name_accepts_typical_names() {
        for ok in ["api", "worker", "shared-types", "Foo_42", "x"] {
            assert!(is_safe_repo_name(ok), "should accept {ok:?}");
        }
    }
}
