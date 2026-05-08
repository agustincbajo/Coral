//! Lint report — issues, severities, and Markdown rendering.

use schemars::JsonSchema;
use serde::Serialize;
use std::path::PathBuf;

/// Severity tier for a lint issue. Drives sort order in the report and the
/// CLI exit code policy (any `Critical` flips lint to a non-zero exit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LintSeverity {
    Critical,
    Warning,
    Info,
}

impl From<LintSeverity> for u8 {
    fn from(s: LintSeverity) -> u8 {
        match s {
            LintSeverity::Critical => 0,
            LintSeverity::Warning => 1,
            LintSeverity::Info => 2,
        }
    }
}

/// Stable identifier for each lint rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LintCode {
    BrokenWikilink,
    OrphanPage,
    LowConfidence,
    HighConfidenceWithoutSources,
    StaleStatus,
    /// `frontmatter.last_updated_commit` does not exist in the repo's git history.
    /// Indicates either a typo, a force-pushed branch that rewrote history, or a
    /// commit that lives only in a workdir that was never pushed.
    CommitNotInGit,
    /// One of the entries in `frontmatter.sources` does not resolve to a file or
    /// directory under the repo root. Surfaces stale paths after refactors.
    SourceNotFound,
    /// A page with `status: archived` is still being linked to from a non-archived
    /// page. Either the linker should be updated to point elsewhere or the archive
    /// note should be lifted.
    ArchivedPageLinked,
    /// The page's frontmatter contains an extra (non-canonical) key. Not necessarily
    /// wrong — consumer wikis are allowed to extend the SCHEMA — but worth a look.
    UnknownExtraField,
    /// Reserved for semantic lint (Phase D).
    Contradiction,
    /// Reserved for semantic lint.
    ObsoleteClaim,
    /// v0.19.5 audit M6: a page body contains tokens that look like
    /// prompt-injection attempts — fake system prompts, encoded auth
    /// headers, very long base64 chunks, or unicode bidi-override
    /// characters that hide content from human reviewers. Surfaces a
    /// Warning so the maintainer reviews before the page reaches an
    /// LLM context window.
    InjectionSuspected,
    /// v0.20.0: the page's frontmatter declares `reviewed: false`,
    /// indicating LLM-generated content (`coral session distill`,
    /// `coral test generate`, etc.) that has not yet been
    /// human-reviewed. **Critical** so the pre-commit hook blocks
    /// the commit until the reviewer flips the flag to `true`. The
    /// check is the load-bearing piece of the trust-by-curation
    /// gate — without it, LLM output could land in `.wiki/` without
    /// a human in the loop.
    UnreviewedDistilled,
}

/// A single lint finding emitted by a check. `page` is `None` for global
/// issues (no specific source file), and `context` carries an optional anchor
/// such as the wikilink target that failed to resolve.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct LintIssue {
    pub code: LintCode,
    pub severity: LintSeverity,
    pub page: Option<PathBuf>,
    pub message: String,
    /// Optional anchor for the issue, e.g., the wikilink target that's broken.
    pub context: Option<String>,
}

/// Top-level lint output: the sorted list of [`LintIssue`]s produced by a run.
/// This is the type emitted by `coral lint --format json`; its JSON schema is
/// published at `docs/schemas/lint.schema.json` for downstream tooling.
#[derive(Debug, Clone, PartialEq, Default, Serialize, JsonSchema)]
pub struct LintReport {
    pub issues: Vec<LintIssue>,
}

impl LintReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_issues(mut issues: Vec<LintIssue>) -> Self {
        // Stable sort: critical first, then warning, then info; within severity by page path,
        // then by message for fully deterministic output.
        issues.sort_by(|a, b| {
            u8::from(a.severity)
                .cmp(&u8::from(b.severity))
                .then_with(|| a.page.cmp(&b.page))
                .then_with(|| a.message.cmp(&b.message))
        });
        Self { issues }
    }

    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn count(&self, sev: LintSeverity) -> usize {
        self.issues.iter().filter(|i| i.severity == sev).count()
    }

    pub fn critical_count(&self) -> usize {
        self.count(LintSeverity::Critical)
    }

    pub fn warning_count(&self) -> usize {
        self.count(LintSeverity::Warning)
    }

    pub fn info_count(&self) -> usize {
        self.count(LintSeverity::Info)
    }

    /// Returns the JSON schema for `LintReport` as a pretty-printed string.
    /// Use this to validate downstream tooling (jq pipelines, dashboards,
    /// CI gates) against the contract Coral emits from
    /// `coral lint --format json`. The committed schema lives at
    /// `docs/schemas/lint.schema.json` and is regenerated from this method.
    pub fn json_schema() -> String {
        let schema = schemars::schema_for!(LintReport);
        serde_json::to_string_pretty(&schema).expect("LintReport schema serializes to JSON")
    }

    /// Renders the report as a Markdown document for human consumption.
    pub fn as_markdown(&self) -> String {
        if self.is_empty() {
            return "# Lint report\n\n✅ No issues found.\n".to_string();
        }

        let mut out = String::new();
        out.push_str("# Lint report\n\n");
        out.push_str(&format!("- 🚨 Critical: {}\n", self.critical_count()));
        out.push_str(&format!("- ⚠️ Warning: {}\n", self.warning_count()));
        out.push_str(&format!("- ℹ️ Info: {}\n\n", self.info_count()));

        for (sev, header) in [
            (LintSeverity::Critical, "## 🚨 Critical\n\n"),
            (LintSeverity::Warning, "## ⚠️ Warning\n\n"),
            (LintSeverity::Info, "## ℹ️ Info\n\n"),
        ] {
            let issues_for_sev: Vec<&LintIssue> =
                self.issues.iter().filter(|i| i.severity == sev).collect();
            if issues_for_sev.is_empty() {
                continue;
            }
            out.push_str(header);
            for issue in issues_for_sev {
                let page_str = match &issue.page {
                    Some(p) => p.display().to_string(),
                    None => "<global>".to_string(),
                };
                out.push_str(&format!(
                    "- **{:?}** in `{}`: {}\n",
                    issue.code, page_str, issue.message
                ));
                if let Some(ctx) = &issue.context {
                    out.push_str(&format!("  Context: {ctx}\n"));
                }
            }
            out.push('\n');
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_issue(code: LintCode, sev: LintSeverity, page: Option<&str>, msg: &str) -> LintIssue {
        LintIssue {
            code,
            severity: sev,
            page: page.map(PathBuf::from),
            message: msg.to_string(),
            context: None,
        }
    }

    #[test]
    fn report_empty_says_no_issues() {
        let report = LintReport::new();
        let md = report.as_markdown();
        assert!(md.contains("No issues"), "expected 'No issues' in: {md}");
    }

    #[test]
    fn report_counts() {
        let issues = vec![
            mk_issue(
                LintCode::BrokenWikilink,
                LintSeverity::Critical,
                Some("a.md"),
                "broken 1",
            ),
            mk_issue(
                LintCode::LowConfidence,
                LintSeverity::Critical,
                Some("b.md"),
                "low 1",
            ),
            mk_issue(
                LintCode::OrphanPage,
                LintSeverity::Warning,
                Some("c.md"),
                "orphan",
            ),
        ];
        let report = LintReport::from_issues(issues);
        assert_eq!(report.critical_count(), 2);
        assert_eq!(report.warning_count(), 1);
        assert_eq!(report.info_count(), 0);
    }

    #[test]
    fn report_from_issues_sorts_by_severity() {
        let issues = vec![
            mk_issue(
                LintCode::StaleStatus,
                LintSeverity::Info,
                Some("z.md"),
                "info",
            ),
            mk_issue(
                LintCode::BrokenWikilink,
                LintSeverity::Critical,
                Some("a.md"),
                "critical",
            ),
            mk_issue(
                LintCode::OrphanPage,
                LintSeverity::Warning,
                Some("m.md"),
                "warning",
            ),
        ];
        let report = LintReport::from_issues(issues);
        assert_eq!(report.issues[0].severity, LintSeverity::Critical);
        assert_eq!(report.issues[1].severity, LintSeverity::Warning);
        assert_eq!(report.issues[2].severity, LintSeverity::Info);
    }

    #[test]
    fn lint_report_json_schema_is_valid_json() {
        let schema = LintReport::json_schema();
        let value: serde_json::Value =
            serde_json::from_str(&schema).expect("schema must be valid JSON");
        assert!(
            value.is_object(),
            "schema root must be a JSON object: {schema}"
        );
        assert_eq!(
            value.get("title").and_then(|v| v.as_str()),
            Some("LintReport"),
            "schema root must declare title=LintReport: {schema}"
        );
    }

    #[test]
    fn lint_report_json_schema_has_expected_definitions() {
        let schema = LintReport::json_schema();
        let value: serde_json::Value =
            serde_json::from_str(&schema).expect("schema must be valid JSON");
        // schemars 0.8 emits `definitions`; v1.x emits `$defs`. Accept either.
        let defs = value
            .get("definitions")
            .or_else(|| value.get("$defs"))
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("schema missing definitions/$defs: {schema}"));
        for expected in ["LintCode", "LintIssue", "LintSeverity"] {
            assert!(
                defs.contains_key(expected),
                "schema definitions missing `{expected}`: {schema}"
            );
        }
    }

    #[test]
    fn lint_report_serialized_matches_schema_keys() {
        // Round-trip: serialize a real LintReport with one issue, then assert the
        // top-level JSON keys match what the schema declares as required.
        let issue = mk_issue(
            LintCode::BrokenWikilink,
            LintSeverity::Critical,
            Some("a.md"),
            "broken",
        );
        let report = LintReport::from_issues(vec![issue]);
        let json = serde_json::to_string(&report).expect("report must serialize");
        let value: serde_json::Value = serde_json::from_str(&json).expect("report json must parse");
        let obj = value.as_object().expect("report must be a JSON object");

        let schema = LintReport::json_schema();
        let schema_value: serde_json::Value =
            serde_json::from_str(&schema).expect("schema must parse");
        let required: Vec<String> = schema_value
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        assert!(
            !required.is_empty(),
            "schema should declare at least one required field: {schema}"
        );
        for key in &required {
            assert!(
                obj.contains_key(key),
                "serialized report missing schema-required key `{key}`: {json}"
            );
        }
        // Spot-check the contract everyone depends on.
        assert!(required.iter().any(|k| k == "issues"));
    }

    #[test]
    fn lint_code_enum_in_schema_lists_every_variant() {
        let schema = LintReport::json_schema();
        let value: serde_json::Value = serde_json::from_str(&schema).expect("schema must parse");
        let defs = value
            .get("definitions")
            .or_else(|| value.get("$defs"))
            .and_then(|v| v.as_object())
            .expect("schema has definitions");
        let lint_code = defs.get("LintCode").expect("LintCode definition exists");

        // schemars represents docstring-decorated enum variants as a `oneOf`
        // of single-value `enum` arrays; bare variants are collapsed into one
        // `enum` array. Walk both shapes and collect every string value.
        let mut found: std::collections::BTreeSet<String> = Default::default();
        let mut collect = |v: &serde_json::Value| {
            if let Some(arr) = v.get("enum").and_then(|e| e.as_array()) {
                for entry in arr {
                    if let Some(s) = entry.as_str() {
                        found.insert(s.to_string());
                    }
                }
            }
        };
        collect(lint_code);
        if let Some(one_of) = lint_code.get("oneOf").and_then(|v| v.as_array()) {
            for branch in one_of {
                collect(branch);
            }
        }

        // Mirrors the snake_case rename of every LintCode variant.
        let expected: [&str; 13] = [
            "broken_wikilink",
            "orphan_page",
            "low_confidence",
            "high_confidence_without_sources",
            "stale_status",
            "commit_not_in_git",
            "source_not_found",
            "archived_page_linked",
            "unknown_extra_field",
            "contradiction",
            "obsolete_claim",
            "injection_suspected",
            "unreviewed_distilled",
        ];
        for variant in expected {
            assert!(
                found.contains(variant),
                "LintCode schema missing variant `{variant}`; found: {found:?}"
            );
        }
        assert_eq!(
            found.len(),
            expected.len(),
            "LintCode schema variant count drift; found {found:?}, expected {expected:?}"
        );
    }

    #[test]
    fn report_markdown_includes_all_sections() {
        let issues = vec![
            mk_issue(
                LintCode::BrokenWikilink,
                LintSeverity::Critical,
                Some("a.md"),
                "critical msg",
            ),
            mk_issue(
                LintCode::OrphanPage,
                LintSeverity::Warning,
                Some("b.md"),
                "warning msg",
            ),
            mk_issue(
                LintCode::StaleStatus,
                LintSeverity::Info,
                Some("c.md"),
                "info msg",
            ),
        ];
        let report = LintReport::from_issues(issues);
        let md = report.as_markdown();
        assert!(md.contains("## 🚨 Critical"), "missing critical: {md}");
        assert!(md.contains("## ⚠️ Warning"), "missing warning: {md}");
        assert!(md.contains("## ℹ️ Info"), "missing info: {md}");
        assert!(md.contains("critical msg"), "missing critical msg: {md}");
        assert!(md.contains("warning msg"), "missing warning msg: {md}");
        assert!(md.contains("info msg"), "missing info msg: {md}");
    }
}
