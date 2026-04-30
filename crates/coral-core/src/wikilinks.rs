//! Extraction of `[[wikilinks]]` from Markdown content.

use ahash::AHashSet;
use regex::Regex;
use std::sync::OnceLock;

fn wikilink_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
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
            let m = cap.get(0).expect("group 0 always present");
            let abs_start = line_start + m.start();
            // Escape check: if char immediately before is a backslash, skip.
            if abs_start > 0 && bytes[abs_start - 1] == b'\\' {
                continue;
            }
            let target_raw = cap.get(1).expect("group 1 captured").as_str();
            // Strip alias (after `|`) and anchor (after `#`).
            let mut target = target_raw;
            if let Some(idx) = target.find('|') {
                target = &target[..idx];
            }
            if let Some(idx) = target.find('#') {
                target = &target[..idx];
            }
            let target = target.trim();
            if target.is_empty() {
                continue;
            }
            if seen.insert(target.to_string()) {
                out.push(target.to_string());
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
}
