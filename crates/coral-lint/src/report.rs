//! Lint report — issues, severities, and Markdown rendering.

use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LintCode {
    BrokenWikilink,
    OrphanPage,
    LowConfidence,
    HighConfidenceWithoutSources,
    StaleStatus,
    /// Reserved for semantic lint (Phase D).
    Contradiction,
    /// Reserved for semantic lint.
    ObsoleteClaim,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LintIssue {
    pub code: LintCode,
    pub severity: LintSeverity,
    pub page: Option<PathBuf>,
    pub message: String,
    /// Optional anchor for the issue, e.g., the wikilink target that's broken.
    pub context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
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
