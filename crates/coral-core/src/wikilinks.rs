//! Extraction of `[[wikilinks]]` from Markdown content.

use ahash::AHashSet;
use regex::Regex;
use std::sync::OnceLock;

fn wikilink_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Static regex literal; verified at compile-time by the parser via
    // the unit tests in this module. The `expect` is a sentinel for
    // future refactors — if you change the pattern and it stops
    // compiling, the panic message tells you to fix it.
    #[allow(
        clippy::expect_used,
        reason = "static regex; compile validity guarded by unit tests"
    )]
    RE.get_or_init(|| Regex::new(r"\[\[([^\]\n]+)\]\]").expect("valid wikilink regex"))
}

/// Extracts unique wikilinks from `content` in document order.
///
/// A wikilink has the syntax `[[target]]` or `[[target|alias]]` or `[[target#anchor]]`.
/// The function returns the *target* portion (everything before `|` or `#`), trimmed.
///
/// Wikilinks inside fenced code blocks (```` ``` ````) are ignored.
/// Wikilinks with a leading backslash (`\[[...]]`) are ignored.
/// Empty targets (`[[]]`) are ignored.
/// Duplicates are removed but document order is preserved.
pub fn extract(content: &str) -> Vec<String> {
    let re = wikilink_re();
    let mut out: Vec<String> = Vec::new();
    let mut seen: AHashSet<String> = AHashSet::new();
    let mut inside_fence = false;
    let bytes = content.as_bytes();

    // We need byte offsets relative to the FULL content for the escape check, so we
    // reconstruct cumulative offset while iterating split_inclusive('\n').
    let mut line_start: usize = 0;
    for line in content.split_inclusive('\n') {
        let line_len = line.len();
        let trimmed = line.trim_start();

        // Toggle fence on lines that start with ``` (after optional leading whitespace).
        if trimmed.starts_with("```") {
            inside_fence = !inside_fence;
            line_start += line_len;
            continue;
        }
        if inside_fence {
            line_start += line_len;
            continue;
        }

        for cap in re.captures_iter(line) {
            // `cap.get(0)` is `Some` whenever a match exists (the iterator
            // only yields matched captures). `cap.get(1)` is `Some`
            // because group 1 in the pattern `\[\[([^\]\n]+)\]\]` requires
            // at least one non-`]` char to match. Both are unreachable
            // failure modes — `if let` keeps clippy quiet without a panic.
            let Some(m) = cap.get(0) else { continue };
            let abs_start = line_start + m.start();
            // Escape check: if char immediately before is a backslash, skip.
            if abs_start > 0 && bytes[abs_start - 1] == b'\\' {
                continue;
            }
            let Some(target_match) = cap.get(1) else {
                continue;
            };
            let target_raw = target_match.as_str();
            // Honor `\|` as an escaped pipe inside the link body — Obsidian
            // semantics (#27). Substitute a sentinel byte (UNIT SEPARATOR,
            // U+001F) that cannot legally occur in slugs before alias-splitting,
            // then restore it as a literal `|` afterward.
            const PIPE_SENTINEL: char = '\u{1f}';
            let escaped = target_raw.replace(r"\|", &PIPE_SENTINEL.to_string());
            // Strip alias (after `|`) and anchor (after `#`).
            let mut target: &str = &escaped;
            if let Some(idx) = target.find('|') {
                target = &target[..idx];
            }
            if let Some(idx) = target.find('#') {
                target = &target[..idx];
            }
            // Restore escaped pipes back to literal `|`.
            let target = target.trim().replace(PIPE_SENTINEL, "|");
            if target.is_empty() {
                continue;
            }
            // Reject any leftover backslash in the resulting slug — keeps the
            // existing slug allowlist behavior strict.
            if target.contains('\\') {
                continue;
            }
            if seen.insert(target.clone()) {
                out.push(target);
            }
        }

        line_start += line_len;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_single() {
        assert_eq!(extract("[[order]]"), vec!["order"]);
    }

    #[test]
    fn extract_multiple_in_text() {
        assert_eq!(
            extract("see [[order]] and [[outbox-pattern]]"),
            vec!["order", "outbox-pattern"]
        );
    }

    #[test]
    fn extract_dedupes_preserving_order() {
        assert_eq!(extract("[[a]] [[b]] [[a]] [[c]]"), vec!["a", "b", "c"]);
    }

    #[test]
    fn extract_strips_anchor() {
        assert_eq!(extract("[[order#status-machine]]"), vec!["order"]);
    }

    #[test]
    fn extract_strips_alias() {
        assert_eq!(
            extract("[[order|the canonical Order entity]]"),
            vec!["order"]
        );
    }

    #[test]
    fn extract_skips_inside_code_fence() {
        let content = "\
[[before]]

```
let x = [[in-code]];
```

[[after]]
";
        let result = extract(content);
        assert_eq!(result, vec!["before", "after"]);
        assert!(!result.contains(&"in-code".to_string()));
    }

    #[test]
    fn extract_skips_escaped() {
        assert!(extract(r"\[[escaped]]").is_empty());
    }

    #[test]
    fn extract_skips_empty_target() {
        assert!(extract("[[]]").is_empty());
        assert!(extract("[[   ]]").is_empty());
    }

    #[test]
    fn extract_handles_multiline() {
        let content = "\
# Page

- bullet with [[link-one]]
- another [[link-two]]

## Section

paragraph mentioning [[link-three]] and [[link-one]] again.
";
        let result = extract(content);
        assert_eq!(result, vec!["link-one", "link-two", "link-three"]);
    }

    #[test]
    fn extract_trims_whitespace() {
        assert_eq!(extract("[[  order  ]]"), vec!["order"]);
    }

    #[test]
    fn extract_empty_content() {
        assert!(extract("").is_empty());
    }

    #[test]
    fn extract_no_wikilinks() {
        assert!(extract("plain markdown with no links at all").is_empty());
    }

    /// #27 — escaped pipe `\|` is preserved as a literal `|` in the target.
    /// Regression: previously the regex saw `\|` as a literal char, so the
    /// alias split picked the `\|` and the target became `a\` (broken slug).
    #[test]
    fn extract_honors_escaped_pipe() {
        let result = extract(r"see [[a\|b]]");
        assert_eq!(result, vec!["a|b"]);
    }

    /// #27 — plain alias `[[a|b]]` (unescaped) keeps the existing semantics:
    /// alias is stripped, target is `a`.
    #[test]
    fn extract_unescaped_pipe_still_strips_alias() {
        let result = extract("see [[a|b]]");
        assert_eq!(result, vec!["a"]);
    }

    /// #27 — multiple escaped pipes inside one wikilink body.
    #[test]
    fn extract_multiple_escaped_pipes() {
        let result = extract(r"see [[a\|b\|c]]");
        assert_eq!(result, vec!["a|b|c"]);
    }

    /// #27 — escaped pipe BEFORE an unescaped one: split on the unescaped
    /// pipe (alias starts there), keep the escape.
    #[test]
    fn extract_escaped_then_unescaped_pipe() {
        let result = extract(r"see [[a\|b|alias]]");
        assert_eq!(result, vec!["a|b"]);
    }

    /// #27 — an unrelated trailing backslash in the slug must be rejected
    /// (escape was for the pipe, not the rest of the slug).
    #[test]
    fn extract_rejects_backslash_in_slug() {
        // No `\|` here — just a stray backslash. Old behavior dropped this
        // through to the slug; new behavior rejects it.
        let result = extract(r"see [[a\b]]");
        assert!(result.is_empty(), "stray backslash leaked: {result:?}");
    }
}
