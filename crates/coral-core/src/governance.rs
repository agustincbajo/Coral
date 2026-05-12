//! Governance rules engine for wiki quality gates.
//!
//! Validates pages against configurable policies. Used by `coral lint`
//! and CI gates to enforce wiki standards.

use crate::page::Page;
use serde::{Deserialize, Serialize};

/// A governance rule violation.
#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub slug: String,
    pub rule: String,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Configurable governance policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernancePolicy {
    /// Minimum confidence for pages to pass governance (default: 0.5).
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f64,
    /// Maximum age (days) before a page is flagged as stale.
    #[serde(default = "default_max_stale_days")]
    pub max_stale_days: u64,
    /// Required fields that every page must have a non-empty value for.
    #[serde(default)]
    pub required_extra_fields: Vec<String>,
    /// Page types that require at least one source.
    #[serde(default = "default_require_sources_for")]
    pub require_sources_for: Vec<String>,
    /// Maximum body length in characters (0 = no limit).
    #[serde(default)]
    pub max_body_chars: usize,
    /// Minimum body length in characters.
    #[serde(default = "default_min_body_chars")]
    pub min_body_chars: usize,
}

impl Default for GovernancePolicy {
    fn default() -> Self {
        Self {
            min_confidence: default_min_confidence(),
            max_stale_days: default_max_stale_days(),
            required_extra_fields: Vec::new(),
            require_sources_for: default_require_sources_for(),
            max_body_chars: 0,
            min_body_chars: default_min_body_chars(),
        }
    }
}

fn default_min_confidence() -> f64 {
    0.5
}
fn default_max_stale_days() -> u64 {
    90
}
fn default_require_sources_for() -> Vec<String> {
    vec!["module".into(), "flow".into()]
}
fn default_min_body_chars() -> usize {
    50
}

/// Run all governance rules against the given pages and return violations.
pub fn check(pages: &[Page], policy: &GovernancePolicy) -> Vec<Violation> {
    let mut violations = Vec::new();

    // Build a set of archived slugs for backlink check.
    let archived_slugs: std::collections::HashSet<&str> = pages
        .iter()
        .filter(|p| p.frontmatter.status == crate::frontmatter::Status::Archived)
        .map(|p| p.frontmatter.slug.as_str())
        .collect();

    for page in pages {
        let slug = &page.frontmatter.slug;
        let page_type = format!("{:?}", page.frontmatter.page_type).to_lowercase();

        // Rule 1: Low confidence
        if page.frontmatter.confidence.as_f64() < policy.min_confidence {
            violations.push(Violation {
                slug: slug.clone(),
                rule: "low_confidence".into(),
                severity: Severity::Warning,
                message: format!(
                    "confidence {:.2} is below threshold {:.2}",
                    page.frontmatter.confidence.as_f64(),
                    policy.min_confidence
                ),
            });
        }

        // Rule 2: Missing sources for required page types
        if policy.require_sources_for.contains(&page_type) && page.frontmatter.sources.is_empty() {
            violations.push(Violation {
                slug: slug.clone(),
                rule: "missing_sources".into(),
                severity: Severity::Error,
                message: format!("page type '{page_type}' requires at least one source"),
            });
        }

        // Rule 3: Body too short
        if page.body.len() < policy.min_body_chars {
            violations.push(Violation {
                slug: slug.clone(),
                rule: "body_too_short".into(),
                severity: Severity::Warning,
                message: format!(
                    "body length {} is below minimum {}",
                    page.body.len(),
                    policy.min_body_chars
                ),
            });
        }

        // Rule 4: Body too long
        if policy.max_body_chars > 0 && page.body.len() > policy.max_body_chars {
            violations.push(Violation {
                slug: slug.clone(),
                rule: "body_too_long".into(),
                severity: Severity::Info,
                message: format!(
                    "body length {} exceeds maximum {}",
                    page.body.len(),
                    policy.max_body_chars
                ),
            });
        }

        // Rule 5: Missing required extra fields
        for field in &policy.required_extra_fields {
            let has_value = page
                .frontmatter
                .extra
                .get(field)
                .map(|v| match v {
                    serde_yaml_ng::Value::String(s) => !s.trim().is_empty(),
                    serde_yaml_ng::Value::Null => false,
                    _ => true,
                })
                .unwrap_or(false);
            if !has_value {
                violations.push(Violation {
                    slug: slug.clone(),
                    rule: "missing_required_field".into(),
                    severity: Severity::Error,
                    message: format!("required extra field '{field}' is missing or empty"),
                });
            }
        }

        // Rule 6: Archived pages with backlinks pointing to them
        // (Check outbound links from non-archived pages pointing to archived pages)
        if page.frontmatter.status != crate::frontmatter::Status::Archived {
            for link in page.outbound_links() {
                if archived_slugs.contains(link.as_str()) {
                    violations.push(Violation {
                        slug: slug.clone(),
                        rule: "link_to_archived".into(),
                        severity: Severity::Warning,
                        message: format!("links to archived page '{link}'"),
                    });
                }
            }
        }
    }

    violations
}

/// Render violations as a Markdown report.
pub fn render_markdown(violations: &[Violation]) -> String {
    if violations.is_empty() {
        return "## Governance: all checks passed\n".to_string();
    }

    let mut out = String::from("## Governance violations\n\n");
    out.push_str(&format!("Found {} violation(s):\n\n", violations.len()));

    for v in violations {
        let icon = match v.severity {
            Severity::Error => "ERROR",
            Severity::Warning => "WARN",
            Severity::Info => "INFO",
        };
        out.push_str(&format!(
            "- **[{icon}]** `{}` — {} (rule: {})\n",
            v.slug, v.message, v.rule
        ));
    }
    out
}

/// Render violations as a JSON value.
pub fn render_json(violations: &[Violation]) -> serde_json::Value {
    serde_json::json!({
        "governance": {
            "total": violations.len(),
            "errors": violations.iter().filter(|v| v.severity == Severity::Error).count(),
            "warnings": violations.iter().filter(|v| v.severity == Severity::Warning).count(),
            "info": violations.iter().filter(|v| v.severity == Severity::Info).count(),
            "violations": violations,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use crate::page::Page;
    use std::path::PathBuf;

    fn make_page(slug: &str, page_type: PageType, confidence: f64, body: &str) -> Page {
        Page {
            path: PathBuf::from(format!("{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type,
                last_updated_commit: "abc123".into(),
                confidence: Confidence::try_new(confidence).unwrap(),
                sources: Vec::new(),
                backlinks: Vec::new(),
                status: Status::Draft,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra: std::collections::BTreeMap::new(),
            },
            body: body.to_string(),
        }
    }

    #[test]
    fn rule_low_confidence() {
        let pages = vec![make_page(
            "low",
            PageType::Concept,
            0.3,
            "Some body text that is long enough to pass the minimum length check",
        )];
        let policy = GovernancePolicy::default();
        let violations = check(&pages, &policy);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule, "low_confidence");
        assert_eq!(violations[0].severity, Severity::Warning);
    }

    #[test]
    fn rule_missing_sources() {
        let pages = vec![make_page(
            "mod1",
            PageType::Module,
            0.8,
            "This is a module page with enough text to pass minimum body length check",
        )];
        let policy = GovernancePolicy::default();
        let violations = check(&pages, &policy);
        let source_violations: Vec<_> = violations
            .iter()
            .filter(|v| v.rule == "missing_sources")
            .collect();
        assert_eq!(source_violations.len(), 1);
        assert_eq!(source_violations[0].severity, Severity::Error);
    }

    #[test]
    fn rule_body_too_short() {
        let pages = vec![make_page("short", PageType::Concept, 0.8, "hi")];
        let policy = GovernancePolicy::default();
        let violations = check(&pages, &policy);
        let short_violations: Vec<_> = violations
            .iter()
            .filter(|v| v.rule == "body_too_short")
            .collect();
        assert_eq!(short_violations.len(), 1);
        assert_eq!(short_violations[0].severity, Severity::Warning);
    }

    #[test]
    fn rule_body_too_long() {
        let long_body = "x".repeat(200);
        let pages = vec![make_page("long", PageType::Concept, 0.8, &long_body)];
        let policy = GovernancePolicy {
            max_body_chars: 100,
            ..Default::default()
        };
        let violations = check(&pages, &policy);
        let long_violations: Vec<_> = violations
            .iter()
            .filter(|v| v.rule == "body_too_long")
            .collect();
        assert_eq!(long_violations.len(), 1);
        assert_eq!(long_violations[0].severity, Severity::Info);
    }

    #[test]
    fn rule_missing_required_field() {
        let pages = vec![make_page(
            "missing",
            PageType::Concept,
            0.8,
            "Enough body text to pass the minimum length check for governance rules",
        )];
        let policy = GovernancePolicy {
            required_extra_fields: vec!["owner".into()],
            ..Default::default()
        };
        let violations = check(&pages, &policy);
        let field_violations: Vec<_> = violations
            .iter()
            .filter(|v| v.rule == "missing_required_field")
            .collect();
        assert_eq!(field_violations.len(), 1);
        assert_eq!(field_violations[0].severity, Severity::Error);
    }

    #[test]
    fn rule_link_to_archived() {
        let mut archived = make_page(
            "old",
            PageType::Concept,
            0.8,
            "Enough body text to pass the minimum length check for governance rules",
        );
        archived.frontmatter.status = Status::Archived;

        // A page that links to the archived page via body wikilink.
        let content = "---\nslug: linker\ntype: concept\nlast_updated_commit: abc\nconfidence: 0.8\nstatus: draft\n---\n\nSee [[old]] for details on the archived content here.";
        let linker = Page::from_content(content, "linker.md").unwrap();

        let pages = vec![archived, linker];
        let policy = GovernancePolicy::default();
        let violations = check(&pages, &policy);
        let archived_violations: Vec<_> = violations
            .iter()
            .filter(|v| v.rule == "link_to_archived")
            .collect();
        assert_eq!(archived_violations.len(), 1);
        assert_eq!(archived_violations[0].slug, "linker");
    }

    #[test]
    fn no_violations_for_healthy_page() {
        let mut page = make_page(
            "healthy",
            PageType::Concept,
            0.9,
            "This page has enough content to pass all governance checks easily.",
        );
        page.frontmatter.sources = vec!["src/lib.rs".into()];
        let pages = vec![page];
        let policy = GovernancePolicy::default();
        let violations = check(&pages, &policy);
        assert!(
            violations.is_empty(),
            "expected no violations, got: {violations:?}"
        );
    }

    #[test]
    fn render_markdown_empty() {
        let md = render_markdown(&[]);
        assert!(md.contains("all checks passed"));
    }

    #[test]
    fn render_json_structure() {
        let violations = vec![Violation {
            slug: "test".into(),
            rule: "low_confidence".into(),
            severity: Severity::Warning,
            message: "too low".into(),
        }];
        let json = render_json(&violations);
        assert_eq!(json["governance"]["total"], 1);
        assert_eq!(json["governance"]["warnings"], 1);
    }
}
