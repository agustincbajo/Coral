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
    /// Catch-all for additional fields the consumer's SCHEMA may add.
    /// Uses `BTreeMap` (deterministic ordering, derives Serialize/Deserialize natively)
    /// instead of `AHashMap` which would require extra serde feature work.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
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
            extra: BTreeMap::new(),
        };
        let out = serialize(&fm, "body\n").expect("serialize");
        // No extra keys should appear in the YAML output.
        assert!(!out.contains("audit:"), "should not contain extra: {out}");
        assert!(
            !out.contains("generated_at:"),
            "should not contain optional none: {out}"
        );
    }
}
