//! Semantic lint — uses the LLM runner to detect contradictions, obsolete
//! claims, and inconsistencies between pages and code.

use crate::report::{LintCode, LintIssue, LintSeverity};
use coral_core::page::Page;
use coral_runner::{Prompt, Runner};

/// Runs a semantic lint pass via the runner. The runner is asked to
/// surface contradictions (e.g., a page claims X but another page or
/// the SCHEMA contradicts it). For v0.1 the result is deliberately
/// minimal: any non-empty stdout from the runner is parsed line-by-line
/// as `<severity>:<page>:<message>` and emitted as issues.
///
/// On runner error, returns a single Critical issue noting the failure.
pub fn check_semantic(pages: &[Page], runner: &dyn Runner) -> Vec<LintIssue> {
    let context = build_context(pages);
    let prompt = Prompt {
        system: Some(SEMANTIC_SYSTEM_PROMPT.to_string()),
        user: format!(
            "{context}\n\nReport contradictions in this format, one per line:\nseverity:slug:message\nWhere severity is one of: critical, warning, info.\nIf no issues, output exactly: NONE"
        ),
        model: None,
        cwd: None,
        timeout: None,
    };
    match runner.run(&prompt) {
        Ok(out) => parse_response(&out.stdout),
        Err(e) => vec![LintIssue {
            code: LintCode::ObsoleteClaim,
            severity: LintSeverity::Critical,
            page: None,
            message: format!("semantic lint failed: {e}"),
            context: None,
        }],
    }
}

const SEMANTIC_SYSTEM_PROMPT: &str = "You are the Coral wiki linter. Read the page summaries and surface contradictions between pages, claims that the SCHEMA invalidates, and obvious obsolescence. Be terse.";

fn build_context(pages: &[Page]) -> String {
    let mut s = String::from("Wiki snapshot:\n\n");
    for p in pages.iter().take(50) {
        s.push_str(&format!(
            "## {} ({})\nconfidence: {}\nbody (first 500 chars): {}\n\n",
            p.frontmatter.slug,
            slug_type_name(&p.frontmatter),
            p.frontmatter.confidence.as_f64(),
            p.body.chars().take(500).collect::<String>()
        ));
    }
    s
}

fn slug_type_name(fm: &coral_core::frontmatter::Frontmatter) -> String {
    serde_json::to_value(fm.page_type)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "unknown".into())
}

fn parse_response(stdout: &str) -> Vec<LintIssue> {
    if stdout.trim() == "NONE" {
        return vec![];
    }
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut parts = line.splitn(3, ':');
            let sev = parts.next()?.trim().to_lowercase();
            let slug = parts.next()?.trim();
            let msg = parts.next()?.trim();
            let severity = match sev.as_str() {
                "critical" => LintSeverity::Critical,
                "warning" => LintSeverity::Warning,
                "info" => LintSeverity::Info,
                _ => return None,
            };
            Some(LintIssue {
                code: LintCode::Contradiction,
                severity,
                page: None,
                message: msg.to_string(),
                context: Some(slug.to_string()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_runner::MockRunner;

    fn empty_runner() -> MockRunner {
        let r = MockRunner::new();
        r.push_ok("NONE");
        r
    }

    #[test]
    fn semantic_returns_no_issues_when_runner_says_none() {
        let r = empty_runner();
        assert!(check_semantic(&[], &r).is_empty());
    }

    #[test]
    fn semantic_parses_pipe_response() {
        let r = MockRunner::new();
        r.push_ok("critical:order:status machine contradicts SCHEMA invariants\nwarning:idempotency:claim is unverified\n");
        let issues = check_semantic(&[], &r);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].severity, LintSeverity::Critical);
        assert_eq!(issues[1].severity, LintSeverity::Warning);
    }

    #[test]
    fn semantic_skips_malformed_lines() {
        let r = MockRunner::new();
        r.push_ok("not formatted\ncritical:slug:valid msg\nbadsev:slug:msg");
        let issues = check_semantic(&[], &r);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn semantic_emits_critical_on_runner_error() {
        let r = MockRunner::new();
        r.push_err(coral_runner::RunnerError::NotFound);
        let issues = check_semantic(&[], &r);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, LintSeverity::Critical);
        assert!(issues[0].message.contains("not found") || issues[0].message.contains("semantic"));
    }
}
