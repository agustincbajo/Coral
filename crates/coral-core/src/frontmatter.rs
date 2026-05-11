//! YAML frontmatter parsing and serialization for Coral wiki pages.

use crate::error::{CoralError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Page type — corresponds to a subdirectory under `.wiki/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageType {
    Module,
    Concept,
    Entity,
    Flow,
    Decision,
    Synthesis,
    Operation,
    Source,
    Gap,
    Index,
    Log,
    Schema,
    Readme,
    Reference,
    /// v0.24: Interface pages describe API contracts (OpenAPI, protobuf,
    /// GraphQL schemas). They're linked to `.coral/contracts/` and
    /// monitored for semantic drift by `coral contract check`.
    Interface,
}

/// Status of a page in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Draft,
    Reviewed,
    Verified,
    Stale,
    Archived,
    Reference,
}

/// Newtype for confidence values, validated to be in [0.0, 1.0].
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Confidence(f64);

impl Confidence {
    pub fn try_new(v: f64) -> Result<Self> {
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err(CoralError::InvalidConfidence(v));
        }
        Ok(Self(v))
    }

    pub fn as_f64(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Confidence {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let v = f64::deserialize(d)?;
        Confidence::try_new(v).map_err(serde::de::Error::custom)
    }
}

/// Frontmatter — YAML metadata block at the top of every wiki page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frontmatter {
    pub slug: String,
    #[serde(rename = "type")]
    pub page_type: PageType,
    pub last_updated_commit: String,
    pub confidence: Confidence,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub backlinks: Vec<String>,
    pub status: Status,
    /// Optional generated_at timestamp (ISO 8601).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    /// ISO-8601 timestamp for when this page's content became valid.
    /// Enables bi-temporal queries: "what did the wiki say about X at time T?"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    /// ISO-8601 timestamp for when this page's content ceased to be valid.
    /// If None, the page is currently valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    /// Catch-all for additional fields the consumer's SCHEMA may add.
    /// Uses `BTreeMap` (deterministic ordering, derives Serialize/Deserialize natively)
    /// instead of `AHashMap` which would require extra serde feature work.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
}

impl Frontmatter {
    /// Returns true if this page is currently valid (no valid_to set).
    pub fn is_current(&self) -> bool {
        self.valid_to.is_none()
    }

    /// Returns true if this page was valid at the given timestamp.
    /// `at` must be ISO-8601 formatted. Uses string comparison
    /// which works for ISO-8601 dates.
    pub fn is_valid_at(&self, at: &str) -> bool {
        let after_from = match &self.valid_from {
            Some(from) => at >= from.as_str(),
            None => true,
        };
        let before_to = match &self.valid_to {
            Some(to) => at < to.as_str(),
            None => true,
        };
        after_from && before_to
    }
}

/// Parses a Markdown document with YAML frontmatter.
/// Returns (Frontmatter, body) where body is the text AFTER the closing `---`.
/// `path` is used for error reporting only.
pub fn parse(content: &str, path: impl Into<PathBuf>) -> Result<(Frontmatter, String)> {
    let path = path.into();

    // Iterate lines preserving line endings is awkward; we walk char-by-char with `split_inclusive`
    // so the body re-assembly preserves \n exactly.
    let mut lines = content.split_inclusive('\n');

    // First line must be `---` (with optional trailing whitespace, including \n).
    let first = match lines.next() {
        Some(l) => l,
        None => return Err(CoralError::MissingFrontmatter { path }),
    };
    if first.trim_end() != "---" {
        return Err(CoralError::MissingFrontmatter { path });
    }

    // Collect YAML lines until we hit a closing `---`.
    let mut yaml_buf = String::new();
    let mut found_close = false;
    let mut consumed_close = 0usize; // bytes of `first` + closing line + everything in between

    consumed_close += first.len();
    for line in lines.by_ref() {
        consumed_close += line.len();
        if line.trim_end() == "---" {
            found_close = true;
            break;
        }
        yaml_buf.push_str(line);
    }

    if !found_close {
        return Err(CoralError::UnterminatedFrontmatter { path });
    }

    // Body = everything after the closing `---\n`.
    let body_start = consumed_close;
    let mut body = if body_start >= content.len() {
        String::new()
    } else {
        content[body_start..].to_string()
    };

    // Drop UNA línea vacía inmediatamente después del cierre (separación canónica).
    // Si hay más líneas vacías, solo descartamos la primera.
    if body.starts_with('\n') {
        body = body[1..].to_string();
    } else if body.starts_with("\r\n") {
        body = body[2..].to_string();
    }

    let fm: Frontmatter = serde_yaml_ng::from_str(&yaml_buf)?;
    Ok((fm, body))
}

/// Like `parse()`, but skips YAML deserialization and just returns the body.
/// Used by the walk cache fast-path when the frontmatter was already parsed
/// in a previous invocation. Returns the empty string if the frontmatter
/// block is unterminated; returns the original content if there's no
/// frontmatter at all.
///
/// v0.19.6 audit N3: also recognizes CRLF (`\r\n`) line endings. Pages
/// authored on Windows or pasted from Office tools commonly arrive
/// with CRLF, and the previous LF-only fast path silently treated
/// them as "no frontmatter" — the body field then ended up containing
/// the YAML, causing the walk cache to disagree with the slow
/// `parse()` path's output.
pub fn body_after_frontmatter(content: &str) -> String {
    // Identify the opener (`---\n` for LF, `---\r\n` for CRLF) and the
    // matching closer. Pick whichever line terminator the file uses
    // and stick with it — mixed line endings inside a single
    // frontmatter block aren't a real-world shape we need to handle.
    let (open_len, close_seq, close_seq_len) = if content.starts_with("---\r\n") {
        (5usize, "\r\n---\r\n", 7usize)
    } else if content.starts_with("---\n") {
        (4usize, "\n---\n", 5usize)
    } else {
        return content.to_string();
    };
    let after_open = &content[open_len..];
    if let Some(close_pos) = after_open.find(close_seq) {
        let body_start = open_len + close_pos + close_seq_len;
        let mut body = &content[body_start..];
        // Drop ONE blank line after the closing `---` (canonical
        // separator). Handle both LF and CRLF.
        if let Some(rest) = body.strip_prefix("\r\n") {
            body = rest;
        } else if let Some(rest) = body.strip_prefix('\n') {
            body = rest;
        }
        return body.to_string();
    }
    String::new()
}

/// Serializes a Frontmatter + body back into a complete Markdown document.
/// Output format: `---\n{yaml}---\n\n{body}` (single blank line between FM and body).
/// The body is appended verbatim — caller controls trailing newline.
pub fn serialize(fm: &Frontmatter, body: &str) -> Result<String> {
    let yaml = serde_yaml_ng::to_string(fm)?;
    // serde_yaml_ng::to_string already terminates with `\n`.
    let mut out = String::with_capacity(yaml.len() + body.len() + 16);
    out.push_str("---\n");
    out.push_str(&yaml);
    out.push_str("---\n\n");
    out.push_str(body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_fm_yaml() -> &'static str {
        "\
---
slug: order
type: module
last_updated_commit: abc123
confidence: 0.8
status: draft
---

# Order

This is the order module.
"
    }

    #[test]
    fn parse_valid_minimal() {
        let (fm, body) = parse(minimal_fm_yaml(), "test.md").expect("parse ok");
        assert_eq!(fm.slug, "order");
        assert_eq!(fm.page_type, PageType::Module);
        assert_eq!(fm.last_updated_commit, "abc123");
        assert_eq!(fm.confidence.as_f64(), 0.8);
        assert_eq!(fm.status, Status::Draft);
        assert!(fm.sources.is_empty());
        assert!(fm.backlinks.is_empty());
        assert!(fm.generated_at.is_none());
        assert!(fm.extra.is_empty());
        assert!(body.starts_with("# Order"));
    }

    #[test]
    fn parse_with_optional_fields() {
        let content = "\
---
slug: order
type: module
last_updated_commit: abc123
confidence: 0.95
status: verified
sources:
  - src/order.rs
  - src/order_state.rs
backlinks:
  - flows/checkout
generated_at: 2026-04-30T10:00:00Z
---

body content
";
        let (fm, body) = parse(content, "test.md").expect("parse ok");
        assert_eq!(fm.sources, vec!["src/order.rs", "src/order_state.rs"]);
        assert_eq!(fm.backlinks, vec!["flows/checkout"]);
        assert_eq!(fm.generated_at.as_deref(), Some("2026-04-30T10:00:00Z"));
        assert_eq!(fm.status, Status::Verified);
        assert!(body.starts_with("body content"));
    }

    #[test]
    fn parse_with_extra_field() {
        let content = "\
---
slug: gdpr
type: concept
last_updated_commit: abc
confidence: 0.7
status: draft
audit: legal
priority: high
---

body
";
        let (fm, _body) = parse(content, "test.md").expect("parse ok");
        assert_eq!(
            fm.extra.get("audit").and_then(|v| v.as_str()),
            Some("legal")
        );
        assert_eq!(
            fm.extra.get("priority").and_then(|v| v.as_str()),
            Some("high")
        );
    }

    #[test]
    fn parse_missing_frontmatter() {
        let content = "# No frontmatter here\n\nJust content.\n";
        let err = parse(content, "test.md").expect_err("should fail");
        assert!(matches!(err, CoralError::MissingFrontmatter { .. }));
    }

    #[test]
    fn parse_unterminated_frontmatter() {
        let content = "\
---
slug: order
type: module
last_updated_commit: abc
confidence: 0.5
status: draft

never closed
";
        let err = parse(content, "test.md").expect_err("should fail");
        assert!(matches!(err, CoralError::UnterminatedFrontmatter { .. }));
    }

    #[test]
    fn parse_empty_body() {
        let content = "\
---
slug: order
type: module
last_updated_commit: abc
confidence: 0.5
status: draft
---
";
        let (_fm, body) = parse(content, "test.md").expect("parse ok");
        assert_eq!(body, "");
    }

    #[test]
    fn parse_body_preserves_internal_blank_lines() {
        let content = "\
---
slug: order
type: module
last_updated_commit: abc
confidence: 0.5
status: draft
---

first paragraph


second paragraph after two blanks
";
        let (_fm, body) = parse(content, "test.md").expect("parse ok");
        // After dropping ONE blank line right after `---`, we should still have the 2 internal blanks.
        assert!(
            body.contains("first paragraph\n\n\nsecond paragraph"),
            "body lost internal blanks: {body:?}"
        );
    }

    #[test]
    fn parse_invalid_confidence() {
        let content = "\
---
slug: order
type: module
last_updated_commit: abc
confidence: 1.5
status: draft
---

body
";
        let err = parse(content, "test.md").expect_err("should fail");
        // Confidence validation happens during YAML parse → wrapped as Yaml error.
        assert!(matches!(err, CoralError::Yaml(_)));
    }

    #[test]
    fn parse_invalid_page_type() {
        let content = "\
---
slug: order
type: martian
last_updated_commit: abc
confidence: 0.5
status: draft
---

body
";
        let err = parse(content, "test.md").expect_err("should fail");
        assert!(matches!(err, CoralError::Yaml(_)));
    }

    #[test]
    fn serialize_roundtrip() {
        let original = "\
---
slug: order
type: module
last_updated_commit: abc123
confidence: 0.85
status: verified
sources:
  - src/order.rs
backlinks: []
---

# Body

content here
";
        let (fm1, body1) = parse(original, "test.md").expect("parse 1");
        let serialized = serialize(&fm1, &body1).expect("serialize");
        let (fm2, body2) = parse(&serialized, "test.md").expect("parse 2");
        assert_eq!(fm1, fm2, "frontmatter should roundtrip");
        assert_eq!(body1, body2, "body should roundtrip");
    }

    #[test]
    fn confidence_try_new_rejects_nan() {
        let err = Confidence::try_new(f64::NAN).expect_err("should fail");
        assert!(matches!(err, CoralError::InvalidConfidence(_)));
    }

    #[test]
    fn confidence_try_new_rejects_infinity() {
        let err = Confidence::try_new(f64::INFINITY).expect_err("should fail");
        assert!(matches!(err, CoralError::InvalidConfidence(_)));
    }

    #[test]
    fn confidence_try_new_rejects_negative() {
        let err = Confidence::try_new(-0.1).expect_err("should fail");
        assert!(matches!(err, CoralError::InvalidConfidence(_)));
    }

    #[test]
    fn confidence_try_new_accepts_zero_and_one() {
        assert_eq!(Confidence::try_new(0.0).expect("zero ok").as_f64(), 0.0);
        assert_eq!(Confidence::try_new(1.0).expect("one ok").as_f64(), 1.0);
    }

    #[test]
    fn body_after_frontmatter_strips_canonical_separator() {
        let content = "\
---
slug: x
type: module
---

body line 1
body line 2
";
        let body = body_after_frontmatter(content);
        assert_eq!(body, "body line 1\nbody line 2\n");
    }

    #[test]
    fn body_after_frontmatter_returns_full_content_when_no_frontmatter() {
        let content = "# Just a doc\n\nNo YAML at top.\n";
        let body = body_after_frontmatter(content);
        assert_eq!(body, content);
    }

    #[test]
    fn body_after_frontmatter_returns_empty_when_unterminated() {
        let content = "---\nslug: x\nbody started\nno closing fence ever\n";
        let body = body_after_frontmatter(content);
        assert_eq!(body, "");
    }

    /// v0.19.6 audit N3: CRLF-line-ended pages must be parsed
    /// correctly. Pre-fix the fast path's `starts_with("---\n")` check
    /// rejected `\r\n`-terminated openers, the function silently
    /// returned the entire CRLF document as "body", and the walk
    /// cache's body diverged from the slow `parse()` path's body.
    #[test]
    fn body_after_frontmatter_handles_crlf_line_endings() {
        let content = "---\r\nslug: x\r\ntype: module\r\n---\r\n\r\nbody line 1\r\nbody line 2\r\n";
        let body = body_after_frontmatter(content);
        assert_eq!(body, "body line 1\r\nbody line 2\r\n");
    }

    /// v0.19.6 audit N3: also handle CRLF documents with no blank
    /// line after the close fence.
    #[test]
    fn body_after_frontmatter_handles_crlf_without_blank_separator() {
        let content = "---\r\nslug: x\r\n---\r\nbody starts immediately\r\n";
        let body = body_after_frontmatter(content);
        assert_eq!(body, "body starts immediately\r\n");
    }

    #[test]
    fn serialize_omits_empty_extra() {
        let fm = Frontmatter {
            slug: "order".to_string(),
            page_type: PageType::Module,
            last_updated_commit: "abc".to_string(),
            confidence: Confidence::try_new(0.5).unwrap(),
            sources: vec![],
            backlinks: vec![],
            status: Status::Draft,
            generated_at: None,
            valid_from: None,
            valid_to: None,
            extra: BTreeMap::new(),
        };
        let out = serialize(&fm, "body\n").expect("serialize");
        // No extra keys should appear in the YAML output.
        assert!(!out.contains("audit:"), "should not contain extra: {out}");
        assert!(
            !out.contains("generated_at:"),
            "should not contain optional none: {out}"
        );
        assert!(
            !out.contains("valid_from:"),
            "should not contain optional none: {out}"
        );
        assert!(
            !out.contains("valid_to:"),
            "should not contain optional none: {out}"
        );
    }

    #[test]
    fn interface_page_type_round_trips() {
        let yaml = "interface";
        let parsed: PageType = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(parsed, PageType::Interface);
        let back = serde_yaml_ng::to_string(&parsed).unwrap();
        assert!(back.trim() == "interface");
    }

    // ── bi-temporal frontmatter (M2.16) ──────────────────────────────

    #[test]
    fn parse_with_valid_from_and_valid_to() {
        let content = "\
---
slug: old-arch
type: decision
last_updated_commit: abc
confidence: 0.9
status: archived
valid_from: 2024-01-01T00:00:00Z
valid_to: 2024-08-01T00:00:00Z
---

Old architecture decision.
";
        let (fm, body) = parse(content, "test.md").expect("parse ok");
        assert_eq!(fm.slug, "old-arch");
        assert_eq!(fm.valid_from.as_deref(), Some("2024-01-01T00:00:00Z"));
        assert_eq!(fm.valid_to.as_deref(), Some("2024-08-01T00:00:00Z"));
        assert!(body.starts_with("Old architecture decision."));
    }

    #[test]
    fn is_current_true_when_valid_to_is_none() {
        let (fm, _) = parse(minimal_fm_yaml(), "test.md").expect("parse ok");
        assert!(fm.is_current(), "page without valid_to should be current");
    }

    #[test]
    fn is_current_false_when_valid_to_is_set() {
        let content = "\
---
slug: old-arch
type: decision
last_updated_commit: abc
confidence: 0.9
status: archived
valid_to: 2024-08-01T00:00:00Z
---

body
";
        let (fm, _) = parse(content, "test.md").expect("parse ok");
        assert!(!fm.is_current(), "page with valid_to should not be current");
    }

    #[test]
    fn is_valid_at_within_range() {
        let content = "\
---
slug: arch-v2
type: decision
last_updated_commit: abc
confidence: 0.9
status: archived
valid_from: 2024-01-01T00:00:00Z
valid_to: 2024-08-01T00:00:00Z
---

body
";
        let (fm, _) = parse(content, "test.md").expect("parse ok");
        // Inside the range
        assert!(fm.is_valid_at("2024-06-15T00:00:00Z"));
        // Exactly at valid_from (inclusive)
        assert!(fm.is_valid_at("2024-01-01T00:00:00Z"));
        // Before the range
        assert!(!fm.is_valid_at("2023-12-31T23:59:59Z"));
        // Exactly at valid_to (exclusive)
        assert!(!fm.is_valid_at("2024-08-01T00:00:00Z"));
        // After the range
        assert!(!fm.is_valid_at("2025-01-01T00:00:00Z"));
    }

    #[test]
    fn is_valid_at_open_ended() {
        // No valid_from, no valid_to -> always valid
        let (fm, _) = parse(minimal_fm_yaml(), "test.md").expect("parse ok");
        assert!(fm.is_valid_at("2020-01-01"));
        assert!(fm.is_valid_at("2099-12-31"));
    }

    #[test]
    fn is_valid_at_only_valid_from() {
        let content = "\
---
slug: new-arch
type: decision
last_updated_commit: abc
confidence: 0.9
status: reviewed
valid_from: 2024-08-01T00:00:00Z
---

body
";
        let (fm, _) = parse(content, "test.md").expect("parse ok");
        assert!(!fm.is_valid_at("2024-07-31T23:59:59Z"));
        assert!(fm.is_valid_at("2024-08-01T00:00:00Z"));
        assert!(fm.is_valid_at("2099-12-31T23:59:59Z"));
    }

    #[test]
    fn backward_compat_pages_without_temporal_fields() {
        // Existing pages that lack valid_from/valid_to must still parse
        // with both fields as None.
        let (fm, _) = parse(minimal_fm_yaml(), "test.md").expect("parse ok");
        assert!(fm.valid_from.is_none());
        assert!(fm.valid_to.is_none());
        assert!(fm.is_current());
    }
}
