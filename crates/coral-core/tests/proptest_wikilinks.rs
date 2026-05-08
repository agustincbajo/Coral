//! Property-based tests for `coral_core::wikilinks::extract`.
//!
//! Same harness pattern as the lint + search proptest files
//! (ProptestConfig::with_cases(64), shared vocabulary).
//!
//! Properties checked:
//! 1. `extract_never_panics` — totality on arbitrary input.
//! 2. `extract_returns_no_duplicates` — even with N copies of the same
//!    `[[X]]` in the body, the output Vec has each target at most once.
//! 3. `extract_preserves_document_order` — the output Vec is the
//!    first-encountered ordering of unique targets, not sorted or
//!    reversed.
//! 4. `extract_strips_alias_and_anchor` — `[[X|Y]]` and `[[X#anchor]]`
//!    return `X`. Trimmed.
//! 5. `extract_returns_only_alphanumeric_safe_targets` — every returned
//!    target is non-empty and free of newlines (the regex enforces it,
//!    but the property guards against future regex tweaks).
//! 6. `extract_skips_inside_code_fences` — wikilinks fenced with ```
//!    are not extracted.
//! 7. `extract_skips_escaped` — `\[[X]]` (with a leading backslash) is
//!    not extracted.

use coral_core::wikilinks::extract;
use proptest::prelude::*;

/// A small slug-shaped string strategy. 1-12 chars, lowercase
/// alphanumeric or `-`. Avoids the regex special chars that break
/// inside `[[...]]`.
fn slug_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,11}".prop_map(|s| s)
}

/// A short body strategy. 0–500 chars of arbitrary printable text plus
/// 0–6 wikilinks (mix of plain, alias, anchor) interleaved with prose.
fn body_with_wikilinks_strategy() -> impl Strategy<Value = (String, Vec<String>)> {
    (
        prop::collection::vec(
            (
                slug_strategy(),
                prop::sample::select(vec!["plain", "alias", "anchor"]),
            ),
            0..=6,
        ),
        "[a-z .,!?-]{0,200}",
    )
        .prop_map(|(slugs_with_form, prose)| {
            let mut body = String::new();
            let mut targets = Vec::new();
            for (slug, form) in &slugs_with_form {
                let link = match *form {
                    "plain" => format!("[[{slug}]]"),
                    "alias" => format!("[[{slug}|some alias]]"),
                    "anchor" => format!("[[{slug}#section]]"),
                    _ => unreachable!(),
                };
                body.push_str(&link);
                body.push(' ');
                body.push_str(&prose);
                body.push(' ');
                targets.push(slug.clone());
            }
            (body, targets)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// extract(content) must never panic, regardless of input.
    #[test]
    fn extract_never_panics(s in ".*") {
        let _ = extract(&s);
    }

    /// The returned Vec contains each target at most once.
    #[test]
    fn extract_returns_no_duplicates((body, _targets) in body_with_wikilinks_strategy()) {
        let result = extract(&body);
        let mut seen = std::collections::HashSet::new();
        for t in &result {
            prop_assert!(seen.insert(t.clone()), "duplicate target in output: {t}");
        }
    }

    /// The output ordering is the FIRST-occurrence ordering of unique
    /// targets in the input.
    #[test]
    fn extract_preserves_document_order((body, mut targets) in body_with_wikilinks_strategy()) {
        let result = extract(&body);
        // Build the expected first-occurrence order by walking `targets`.
        let mut expected = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for t in &targets {
            if seen.insert(t.clone()) {
                expected.push(t.clone());
            }
        }
        // Mute the unused-mut warning if proptest's value isn't consumed.
        targets.clear();
        prop_assert_eq!(result, expected);
    }

    /// `[[X|alias]]` and `[[X#anchor]]` resolve to bare `X`.
    #[test]
    fn extract_strips_alias_and_anchor(slug in slug_strategy()) {
        // Plain
        let plain = extract(&format!("see [[{slug}]]"));
        prop_assert_eq!(&plain, &vec![slug.clone()]);
        // Alias
        let aliased = extract(&format!("see [[{slug}|the alias]]"));
        prop_assert_eq!(&aliased, &vec![slug.clone()]);
        // Anchor
        let anchored = extract(&format!("see [[{slug}#section-2]]"));
        prop_assert_eq!(&anchored, &vec![slug]);
    }

    /// Every returned target is non-empty and contains no newlines or
    /// characters the regex isn't supposed to admit.
    ///
    /// Note: literal `|` IS allowed in the target when the input contains
    /// the escaped form `\|` (Obsidian semantics, #27). The hash `#` is
    /// always stripped before this point — anchor-after-target syntax
    /// has no escape form.
    #[test]
    fn extract_returns_only_alphanumeric_safe_targets(s in ".*") {
        for t in extract(&s) {
            prop_assert!(!t.is_empty(), "empty target in output");
            prop_assert!(!t.contains('\n'), "newline in target: {t:?}");
            prop_assert!(!t.contains(']'), "closing bracket in target: {t:?}");
            // Hash should always be stripped before this point.
            prop_assert!(!t.contains('#'), "hash in target: {t:?}");
            // Backslash is rejected after escape-restoration, so it can
            // never appear in the output target.
            prop_assert!(!t.contains('\\'), "backslash in target: {t:?}");
        }
    }
}

#[test]
fn extract_skips_inside_code_fences() {
    let body = "Outside [[outside]]\n\
                ```\n\
                Inside the fence [[inside]]\n\
                ```\n\
                After [[after]]";
    let result = extract(body);
    assert_eq!(result, vec!["outside", "after"]);
    assert!(!result.contains(&"inside".to_string()));
}

#[test]
fn extract_skips_escaped_wikilinks() {
    let body = r"Real [[real]] but escaped \[[escaped]] is not.";
    let result = extract(body);
    assert_eq!(result, vec!["real"]);
    assert!(!result.contains(&"escaped".to_string()));
}

#[test]
fn extract_returns_empty_for_empty_input() {
    assert!(extract("").is_empty());
}

#[test]
fn extract_returns_empty_when_no_wikilinks() {
    assert!(extract("just plain markdown with [a link](https://x)").is_empty());
}
