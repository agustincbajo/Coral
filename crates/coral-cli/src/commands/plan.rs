//! Shared types for parsing LLM-generated bootstrap/ingest plans.

use coral_core::error::{CoralError, Result as CoralResult};
use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::page::Page;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Create,
    Update,
    Retire,
}

/// v0.34.0 (FR-ONB-30): `Serialize` was added so the full plan can
/// be persisted inside `.wiki/.bootstrap-state.json` (the `--resume`
/// checkpoint). The existing `Deserialize` path used to parse the
/// LLM's YAML output is unchanged.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlanEntry {
    pub slug: String,
    /// Optional in YAML: when missing (e.g. bootstrap output) we default to Create.
    #[serde(default = "default_action")]
    pub action: Action,
    pub r#type: Option<PageType>,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub rationale: String,
    pub body: Option<String>,
}

fn default_action() -> Action {
    Action::Create
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Plan {
    pub plan: Vec<PlanEntry>,
}

impl Plan {
    /// Tries to parse a YAML response from the runner. Tolerates leading
    /// fences (` ```yaml `) and trailing prose.
    pub fn parse(stdout: &str) -> CoralResult<Self> {
        let trimmed = strip_yaml_fence(stdout);
        serde_yaml_ng::from_str(trimmed).map_err(CoralError::Yaml)
    }
}

/// Strip a Markdown code fence from the runner's stdout, tolerating
/// arbitrary prose before AND after the fence. Examples that all
/// resolve to the inner YAML:
///
/// - Bare YAML (no fence): returned as-is.
/// - "```yaml\n...\n```": classic Markdown fenced block.
/// - "I have enough context. Here's the plan:\n\n```yaml\n...\n```":
///   conversational prelude before the fence (LLMs vary on whether
///   they include this; we used to fail when they did).
/// - Closing fence followed by trailing commentary: ignored.
///
/// If no fence is present, returns the trimmed input verbatim — assume
/// the caller produced bare YAML.
pub(crate) fn strip_yaml_fence(s: &str) -> &str {
    let s = s.trim();
    // Find the START of a YAML fence anywhere in the input.
    let (start_idx, fence_len) = match s.find("```yaml\n") {
        Some(i) => (i, "```yaml\n".len()),
        None => match s.find("```\n") {
            Some(i) => (i, "```\n".len()),
            // Also accept a fence that immediately starts the buffer
            // with no trailing newline (rare but possible).
            None => match s.find("```yaml") {
                Some(i) => (i, "```yaml".len()),
                None => return s,
            },
        },
    };
    let after_fence = &s[start_idx + fence_len..];
    // Find the closing fence (first occurrence after the opener).
    if let Some(end) = after_fence.find("```") {
        return after_fence[..end].trim_end();
    }
    after_fence
}

/// Builds a Page in memory from a `create` PlanEntry. Caller writes to disk.
///
/// v0.19.5 audit C4: validate the slug against
/// [`coral_core::slug::is_safe_filename_slug`] before any path
/// interpolation. The slug arrives from the LLM and was previously
/// joined into `wiki_root` directly — `slug: ../etc/passwd` would
/// have escaped the wiki.
pub fn build_page(entry: &PlanEntry, head_sha: &str, wiki_root: &Path) -> CoralResult<Page> {
    if !coral_core::slug::is_safe_filename_slug(&entry.slug) {
        return Err(CoralError::Git(format!(
            "create entry slug `{}` is not a safe filename slug; refusing to build page",
            entry.slug
        )));
    }
    let page_type = entry.r#type.ok_or_else(|| {
        CoralError::Git(format!(
            "create entry for `{}` missing `type` field",
            entry.slug
        ))
    })?;
    let confidence = Confidence::try_new(entry.confidence.unwrap_or(0.5))?;
    let body = entry.body.clone().unwrap_or_else(|| {
        format!(
            "# {}\n\n_Stub created by coral. Fill in body._\n\n_Rationale: {}_\n",
            entry.slug, entry.rationale
        )
    });

    let subdir = page_type_subdir(page_type);
    let path: PathBuf = wiki_root.join(subdir).join(format!("{}.md", entry.slug));

    let frontmatter = Frontmatter {
        slug: entry.slug.clone(),
        page_type,
        last_updated_commit: head_sha.to_string(),
        confidence,
        sources: vec![],
        backlinks: vec![],
        status: Status::Draft,
        generated_at: Some(chrono::Utc::now().to_rfc3339()),
        valid_from: None,
        valid_to: None,
        superseded_by: None,
        extra: BTreeMap::new(),
    };

    Ok(Page {
        path,
        frontmatter,
        body,
    })
}

/// Returns the subdirectory (relative to `.wiki/`) that hosts pages of this type.
/// Top-level pages (`index`, `log`, `schema`, `readme`) live at the root; we
/// return `"."` for those and the caller resolves to `wiki_root` directly.
pub fn page_type_subdir(t: PageType) -> &'static str {
    use PageType::*;
    match t {
        Module => "modules",
        Concept => "concepts",
        Entity => "entities",
        Flow => "flows",
        Decision => "decisions",
        Synthesis => "synthesis",
        Operation => "operations",
        Source => "sources",
        Gap => "gaps",
        Index => ".",
        Log => ".",
        Schema => ".",
        Readme => ".",
        Reference => "examples",
        Interface => "interfaces",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_plan() {
        let yaml = "\
plan:
  - slug: order
    action: create
    type: module
    confidence: 0.7
    rationale: top-level entity
    body: |
      # Order

      Body text.
  - slug: outbox
    action: update
    rationale: handler signature changed
";
        let plan = Plan::parse(yaml).expect("parse ok");
        assert_eq!(plan.plan.len(), 2);
        assert_eq!(plan.plan[0].slug, "order");
        assert_eq!(plan.plan[0].action, Action::Create);
        assert_eq!(plan.plan[0].r#type, Some(PageType::Module));
        assert!((plan.plan[0].confidence.unwrap() - 0.7).abs() < 1e-9);
        assert!(plan.plan[0].body.as_deref().unwrap().contains("# Order"));
        assert_eq!(plan.plan[1].action, Action::Update);
    }

    #[test]
    fn parse_with_yaml_fence() {
        let yaml = "```yaml\nplan:\n  - slug: order\n    type: module\n    confidence: 0.6\n    rationale: anchor\n    body: |\n      # Order\n```";
        let plan = Plan::parse(yaml).expect("parse ok");
        assert_eq!(plan.plan.len(), 1);
        assert_eq!(plan.plan[0].slug, "order");
        // Default action when omitted is Create — the bootstrap shape skips the field.
        assert_eq!(plan.plan[0].action, Action::Create);
    }

    #[test]
    fn parse_invalid_yaml() {
        let err = Plan::parse("").expect_err("empty must error");
        assert!(matches!(err, CoralError::Yaml(_)));
    }

    /// Real LLMs (Claude, Gemini) sometimes emit conversational prose
    /// before the YAML code fence, e.g. "I have enough context. Here's
    /// the plan:\n\n```yaml\n...\n```". The previous strip_yaml_fence
    /// only handled fence-at-start; this case caused dogfooding apply
    /// to fail with "found character that cannot start any token at
    /// line 3 column 1". Pin the fix.
    #[test]
    fn parse_tolerates_prose_before_fence() {
        let with_prelude = "I have enough context to write a precise plan. Here it is:\n\n\
                            ```yaml\nplan:\n  - slug: order\n    type: module\n    confidence: 0.7\n    \
                            rationale: anchor\n    body: |\n      # Order\n```";
        let plan = Plan::parse(with_prelude).expect("must tolerate prelude prose");
        assert_eq!(plan.plan.len(), 1);
        assert_eq!(plan.plan[0].slug, "order");
    }

    /// And trailing prose after the closing fence too.
    #[test]
    fn parse_tolerates_prose_after_fence() {
        let with_postscript = "```yaml\nplan:\n  - slug: order\n    type: module\n    \
                               confidence: 0.7\n    rationale: anchor\n    body: |\n      # Order\n```\n\
                               \nLet me know if you'd like adjustments.\n";
        let plan = Plan::parse(with_postscript).expect("must tolerate postscript");
        assert_eq!(plan.plan.len(), 1);
        assert_eq!(plan.plan[0].slug, "order");
    }

    /// Both at once — the realistic LLM output shape.
    #[test]
    fn parse_tolerates_prose_around_fence() {
        let wrapped = "Here's the plan:\n\n```yaml\nplan:\n  - slug: order\n    type: module\n    \
                       confidence: 0.7\n    rationale: anchor\n    body: |\n      # Order\n```\n\n\
                       Happy to refine if needed.\n";
        let plan = Plan::parse(wrapped).expect("must tolerate prose around fence");
        assert_eq!(plan.plan.len(), 1);
        assert_eq!(plan.plan[0].slug, "order");
    }

    #[test]
    fn parse_invalid_action() {
        let yaml = "plan:\n  - slug: x\n    action: nuke\n    rationale: r\n";
        let err = Plan::parse(yaml).expect_err("must error");
        assert!(matches!(err, CoralError::Yaml(_)));
    }

    #[test]
    fn build_page_create_module() {
        let entry = PlanEntry {
            slug: "order".to_string(),
            action: Action::Create,
            r#type: Some(PageType::Module),
            confidence: Some(0.7),
            rationale: "top-level".to_string(),
            body: Some("# Order\n\nBody.\n".to_string()),
        };
        let page = build_page(&entry, "deadbeef", Path::new(".wiki")).expect("build");
        assert_eq!(page.frontmatter.slug, "order");
        assert_eq!(page.frontmatter.page_type, PageType::Module);
        assert_eq!(page.frontmatter.last_updated_commit, "deadbeef");
        assert!((page.frontmatter.confidence.as_f64() - 0.7).abs() < 1e-9);
        assert_eq!(page.frontmatter.status, Status::Draft);
        assert_eq!(
            page.path,
            Path::new(".wiki").join("modules").join("order.md")
        );
        assert!(page.body.contains("# Order"));
    }

    #[test]
    fn build_page_create_missing_type_errors() {
        let entry = PlanEntry {
            slug: "rogue".to_string(),
            action: Action::Create,
            r#type: None,
            confidence: Some(0.5),
            rationale: "n/a".to_string(),
            body: Some("body".to_string()),
        };
        let err = build_page(&entry, "abc", Path::new(".wiki")).expect_err("must error");
        match err {
            CoralError::Git(msg) => assert!(msg.contains("rogue")),
            other => panic!("expected Git error, got {other:?}"),
        }
    }

    /// v0.19.5 audit C4: a malicious LLM-emitted slug must NOT escape
    /// the wiki root via path traversal.
    #[test]
    fn build_page_rejects_path_traversal_slug() {
        let entry = PlanEntry {
            slug: "../etc/passwd".to_string(),
            action: Action::Create,
            r#type: Some(PageType::Module),
            confidence: Some(0.5),
            rationale: "evil".to_string(),
            body: Some("body".to_string()),
        };
        let err = build_page(&entry, "abc", Path::new(".wiki")).expect_err("must error");
        match err {
            CoralError::Git(msg) => assert!(
                msg.contains("not a safe filename slug"),
                "unexpected error: {msg}"
            ),
            other => panic!("expected Git error, got {other:?}"),
        }
    }

    #[test]
    fn build_page_defaults_confidence_to_half_when_missing() {
        let entry = PlanEntry {
            slug: "x".to_string(),
            action: Action::Create,
            r#type: Some(PageType::Concept),
            confidence: None,
            rationale: "n/a".to_string(),
            body: Some("body".to_string()),
        };
        let page = build_page(&entry, "abc", Path::new(".wiki")).expect("build");
        assert!((page.frontmatter.confidence.as_f64() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn page_type_subdir_covers_all_variants() {
        // Sanity: every concrete dir maps to a non-empty string.
        for t in [
            PageType::Module,
            PageType::Concept,
            PageType::Entity,
            PageType::Flow,
            PageType::Decision,
            PageType::Synthesis,
            PageType::Operation,
            PageType::Source,
            PageType::Gap,
            PageType::Reference,
        ] {
            let s = page_type_subdir(t);
            assert!(!s.is_empty());
            assert_ne!(s, ".");
        }
        // Index / Log / Schema / Readme live at the wiki root.
        for t in [
            PageType::Index,
            PageType::Log,
            PageType::Schema,
            PageType::Readme,
        ] {
            assert_eq!(page_type_subdir(t), ".");
        }
    }
}
