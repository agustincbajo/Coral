//! Structural lint checks — pure functions over `&[Page]`.
//!
//! Two of the checks ([`check_commit_in_git`] and [`check_source_exists`])
//! also touch the filesystem / git, so they take an extra `repo_root: &Path`
//! argument. Both degrade gracefully when git is missing or paths are
//! unreadable.

use crate::report::{LintCode, LintIssue, LintSeverity};
use coral_core::frontmatter::{PageType, Status};
use coral_core::page::Page;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

/// Reports a `BrokenWikilink` Critical for any outbound wikilink whose target
/// is not the slug of any page in the workspace.
///
/// `links` is a pre-computed slice parallel to `pages` — `links[i]` holds the
/// outbound wikilinks for `pages[i]`. This avoids redundant regex extraction
/// when multiple checks need the same link data.
pub fn check_broken_wikilinks(pages: &[Page], links: &[Vec<String>]) -> Vec<LintIssue> {
    let slugs: HashSet<&str> = pages.iter().map(|p| p.frontmatter.slug.as_str()).collect();

    let mut issues = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        for link in &links[i] {
            if !slugs.contains(link.as_str()) {
                issues.push(LintIssue {
                    code: LintCode::BrokenWikilink,
                    severity: LintSeverity::Critical,
                    page: Some(page.path.clone()),
                    message: format!("Wikilink target '{link}' has no matching page"),
                    context: Some(link.clone()),
                });
            }
        }
    }
    issues
}

/// Reports an `OrphanPage` Warning for any page that has zero incoming backlinks
/// AND zero references in any other page's body. Skips system pages
/// (PageType::Index, Log, Schema, Readme) — those are roots by design.
///
/// `links` is a pre-computed slice parallel to `pages` — `links[i]` holds the
/// outbound wikilinks for `pages[i]`.
pub fn check_orphan_pages(pages: &[Page], links: &[Vec<String>]) -> Vec<LintIssue> {
    let mut inbound: HashMap<String, usize> = HashMap::new();
    for page_links in links {
        for link in page_links {
            *inbound.entry(link.clone()).or_insert(0) += 1;
        }
    }

    let mut issues = Vec::new();
    for page in pages {
        if matches!(
            page.frontmatter.page_type,
            PageType::Index
                | PageType::Log
                | PageType::Schema
                | PageType::Readme
                | PageType::Interface
        ) {
            continue;
        }
        let count = inbound.get(&page.frontmatter.slug).copied().unwrap_or(0);
        if count == 0 {
            issues.push(LintIssue {
                code: LintCode::OrphanPage,
                severity: LintSeverity::Warning,
                page: Some(page.path.clone()),
                message: format!("Page '{}' has no inbound backlinks", page.frontmatter.slug),
                context: None,
            });
        }
    }
    issues
}

/// Reports `LowConfidence` for pages with confidence < 0.6.
/// Severity: Critical if < 0.3, Warning otherwise.
/// Skips pages with status == Reference (they're examples, exempt).
pub fn check_low_confidence(pages: &[Page]) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    for page in pages {
        if page.frontmatter.status == Status::Reference {
            continue;
        }
        let conf = page.frontmatter.confidence.as_f64();
        if conf < 0.3 {
            issues.push(LintIssue {
                code: LintCode::LowConfidence,
                severity: LintSeverity::Critical,
                page: Some(page.path.clone()),
                message: format!("Confidence {conf} below critical threshold 0.3"),
                context: None,
            });
        } else if conf < 0.6 {
            issues.push(LintIssue {
                code: LintCode::LowConfidence,
                severity: LintSeverity::Warning,
                page: Some(page.path.clone()),
                message: format!("Confidence {conf} below threshold 0.6"),
                context: None,
            });
        }
    }
    issues
}

/// Reports a `HighConfidenceWithoutSources` Warning for any page with
/// confidence >= 0.6 but `sources` field is empty.
pub fn check_high_confidence_without_sources(pages: &[Page]) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    for page in pages {
        if page.frontmatter.confidence.as_f64() >= 0.6 && page.frontmatter.sources.is_empty() {
            issues.push(LintIssue {
                code: LintCode::HighConfidenceWithoutSources,
                severity: LintSeverity::Warning,
                page: Some(page.path.clone()),
                message: format!(
                    "Page '{}' has confidence >= 0.6 but no sources listed",
                    page.frontmatter.slug
                ),
                context: None,
            });
        }
    }
    issues
}

/// Reports a `StaleStatus` Info for any page with status == Stale.
/// (This just surfaces explicit `stale` markings; staleness *detection* via
/// commit age is a future check.)
pub fn check_stale_status(pages: &[Page]) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    for page in pages {
        if page.frontmatter.status == Status::Stale {
            issues.push(LintIssue {
                code: LintCode::StaleStatus,
                severity: LintSeverity::Info,
                page: Some(page.path.clone()),
                message: "Page marked as stale".to_string(),
                context: None,
            });
        }
    }
    issues
}

/// Returns true for `last_updated_commit` values that are clearly not real
/// SHAs (test fixtures and historical placeholders). These pages are skipped
/// by [`check_commit_in_git`] to avoid drowning the report in false positives.
fn is_placeholder_commit(commit: &str) -> bool {
    let trimmed = commit.trim();
    trimmed.is_empty() || trimmed.len() < 7 || matches!(trimmed, "unknown" | "abc" | "zero")
}

/// Shells out **once** to `git rev-list --all --no-walk --objects` from
/// `repo_root` to collect every reachable commit SHA in the repository, and
/// reports `CommitNotInGit` Warning for each page whose
/// `frontmatter.last_updated_commit` is not in that set.
///
/// Skips pages whose commit is a known placeholder (empty, "unknown", "abc",
/// "zero", or shorter than 7 chars — see [`is_placeholder_commit`]) so test
/// fixtures and freshly-bootstrapped wikis do not light up the report.
///
/// If the `git rev-list` invocation fails for any reason (no git installed,
/// detached repo, permission denied, …) the check logs a warning via
/// `tracing::warn!` and returns an empty issue list — degrading gracefully
/// rather than failing the whole lint pass.
pub fn check_commit_in_git(pages: &[Page], repo_root: &Path) -> Vec<LintIssue> {
    let known: HashSet<String> = match collect_git_commits(repo_root) {
        Ok(set) => set,
        Err(err) => {
            tracing::warn!(
                error = %err,
                repo_root = %repo_root.display(),
                "check_commit_in_git: git rev-list failed; skipping check"
            );
            return Vec::new();
        }
    };

    let mut issues = Vec::new();
    for page in pages {
        let commit = &page.frontmatter.last_updated_commit;
        if is_placeholder_commit(commit) {
            continue;
        }
        if !known.contains(commit.as_str()) {
            // `git log` matches by prefix when commits are abbreviated, so do
            // the same: accept the commit if any known SHA starts with it.
            let prefix_match = known.iter().any(|sha| sha.starts_with(commit.as_str()));
            if prefix_match {
                continue;
            }
            issues.push(LintIssue {
                code: LintCode::CommitNotInGit,
                severity: LintSeverity::Warning,
                page: Some(page.path.clone()),
                message: format!(
                    "Commit '{}' for page '{}' not found in repo history",
                    commit, page.frontmatter.slug
                ),
                context: Some(commit.clone()),
            });
        }
    }
    issues
}

/// Runs `git rev-list --all` in `repo_root` and returns the set of commit
/// SHAs across the entire repo history (40-char hex strings, plus any short
/// SHAs that may appear). Returns an `Err(String)` describing the failure if
/// `git` is not available, the directory is not a repository, or the command
/// exits non-zero.
///
/// **Important**: do NOT add `--no-walk` here — without an explicit commit
/// list, `--no-walk` collapses to "tips of every ref" only (~10s of commits
/// in a real repo), missing every interior commit. We need the full history
/// so `check_commit_in_git` doesn't false-positive on legitimate ancestor
/// SHAs (caught this during v0.15.x dogfooding).
fn collect_git_commits(repo_root: &Path) -> std::result::Result<HashSet<String>, String> {
    let output = Command::new("git")
        .args(["rev-list", "--all"])
        .current_dir(repo_root)
        .output()
        .map_err(|e| format!("failed to invoke git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git rev-list exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Reports `SourceNotFound` Warning for each entry in `frontmatter.sources`
/// that does not resolve to a file or directory under `repo_root`.
///
/// Sources that look like URLs (`http://` / `https://` prefix) are skipped —
/// only filesystem paths are checked. Both files and directories are valid
/// targets (`Path::exists()` is sufficient).
/// v0.19.5 audit M6: minimal prompt-injection detector.
///
/// Scans page bodies for substrings that an attacker might use to
/// hijack an LLM that consumes the wiki:
///
/// - `<|system|>` / `</user>` / `<|assistant|>` — fake chat
///   delimiters used by some open models.
/// - `Authorization` / `Bearer ` / `x-api-key` — header lines that
///   look like the attacker is trying to leak a key.
/// - Base64-shaped runs > 100 chars (heuristic; real source paths
///   never look like this).
/// - Unicode bidi-override characters (U+202E and the tags block
///   U+E0000..U+E007F) — a common technique to hide content.
///
/// Reports a Warning so reviewers see it before `coral query` /
/// `coral mcp serve` flushes the page into an agent.
pub fn check_injection(pages: &[Page]) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    for page in pages {
        let body = &page.body;
        let mut hits: Vec<&str> = Vec::new();
        for marker in ["<|system|>", "</user>", "<|assistant|>", "x-api-key"] {
            if body.contains(marker) {
                hits.push(marker);
            }
        }
        // Case-insensitive checks for header-shaped substrings.
        let lower = body.to_lowercase();
        if lower.contains("authorization:") {
            hits.push("Authorization:");
        }
        if lower.contains("bearer ") {
            hits.push("Bearer");
        }
        // Long base64-ish run.
        if body
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '/' && c != '=')
            .any(|run| run.len() > 100)
        {
            hits.push("base64-like-run");
        }
        // Unicode bidi-override / tag chars.
        if body.chars().any(|c| {
            let cp = c as u32;
            cp == 0x202E || (0xE0000..=0xE007F).contains(&cp)
        }) {
            hits.push("unicode-bidi-or-tag");
        }
        if !hits.is_empty() {
            issues.push(LintIssue {
                code: LintCode::InjectionSuspected,
                severity: LintSeverity::Warning,
                page: Some(page.path.clone()),
                message: format!(
                    "page '{}' contains tokens that look like prompt-injection attempts: {}",
                    page.frontmatter.slug,
                    hits.join(", ")
                ),
                context: Some(hits.join(",")),
            });
        }
    }
    issues
}

pub fn check_source_exists(pages: &[Page], repo_root: &Path) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    for page in pages {
        for src in &page.frontmatter.sources {
            if src.starts_with("http://") || src.starts_with("https://") {
                continue;
            }
            // v0.19.5 audit M4: refuse to stat absolute paths or paths
            // containing `..` — both shapes either escape the repo
            // root or signal an attacker probing the filesystem.
            // Surface as a warning rather than `not found` so the
            // user sees the real reason.
            if src.starts_with('/') || src.contains("..") {
                issues.push(LintIssue {
                    code: LintCode::SourceNotFound,
                    severity: LintSeverity::Warning,
                    page: Some(page.path.clone()),
                    message: format!(
                        "Source path '{}' for page '{}' is absolute or contains `..`; \
                         refusing to probe outside the repo root",
                        src, page.frontmatter.slug
                    ),
                    context: Some(src.clone()),
                });
                continue;
            }
            let full = repo_root.join(src);
            if !full.exists() {
                issues.push(LintIssue {
                    code: LintCode::SourceNotFound,
                    severity: LintSeverity::Warning,
                    page: Some(page.path.clone()),
                    message: format!(
                        "Source path '{}' for page '{}' not found on disk",
                        src, page.frontmatter.slug
                    ),
                    context: Some(src.clone()),
                });
            }
        }
    }
    issues
}

/// Reports `ArchivedPageLinked` Warning once per (linker, archived target)
/// pair: any page whose body / backlinks reference an archived page's slug
/// gets flagged so the maintainer can either update the link or lift the
/// archive note. The issue's `page` field is the LINKER (the page with the
/// stale link), `context` is the archived target's slug.
///
/// `links` is a pre-computed slice parallel to `pages` — `links[i]` holds the
/// outbound wikilinks for `pages[i]`.
pub fn check_archived_linked_from_head(pages: &[Page], links: &[Vec<String>]) -> Vec<LintIssue> {
    let archived: HashSet<&str> = pages
        .iter()
        .filter(|p| p.frontmatter.status == Status::Archived)
        .map(|p| p.frontmatter.slug.as_str())
        .collect();
    if archived.is_empty() {
        return Vec::new();
    }

    let mut issues = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        // Skip self-links from archived pages — those are fine, and we don't
        // want archived → archived chatter.
        if page.frontmatter.status == Status::Archived {
            continue;
        }
        for link in &links[i] {
            if archived.contains(link.as_str()) {
                issues.push(LintIssue {
                    code: LintCode::ArchivedPageLinked,
                    severity: LintSeverity::Warning,
                    page: Some(page.path.clone()),
                    message: format!(
                        "Page '{}' links to archived page '{}'",
                        page.frontmatter.slug, link
                    ),
                    context: Some(link.clone()),
                });
            }
        }
    }
    issues
}

/// Reports `UnknownExtraField` Info — one issue per non-canonical key in
/// `frontmatter.extra`. Severity is **Info** on purpose: extra fields are
/// allowed (consumer wikis extend the SCHEMA) but worth surfacing for review.
///
/// v0.20.0: skips the `reviewed` and `source` keys because those carry
/// dedicated semantics for `coral session distill` output —
/// [`check_unreviewed_distilled`] handles `reviewed: false` explicitly,
/// so re-reporting it as `UnknownExtraField` would double-count and
/// mask the higher-severity issue under Info noise.
pub fn check_unknown_extra_field(pages: &[Page]) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    for page in pages {
        for key in page.frontmatter.extra.keys() {
            // v0.20.0: the dedicated check handles these.
            if key == "reviewed" || key == "source" {
                continue;
            }
            issues.push(LintIssue {
                code: LintCode::UnknownExtraField,
                severity: LintSeverity::Info,
                page: Some(page.path.clone()),
                message: format!(
                    "Page '{}' has non-canonical frontmatter field '{}'",
                    page.frontmatter.slug, key
                ),
                context: Some(key.clone()),
            });
        }
    }
    issues
}

/// Reports `UnreviewedDistilled` **Critical** for any page whose
/// frontmatter declares `reviewed: false` (a signal that
/// `coral session distill` or `coral test generate` produced the
/// page and a human has not yet reviewed + signed off).
///
/// **This is the load-bearing piece of v0.20's trust-by-curation
/// gate.** A pre-commit hook (set up by `coral init`'s template) runs
/// `coral lint` and rejects the commit if any Critical issue exists —
/// so a page with `reviewed: false` cannot accidentally land in
/// `.wiki/` without a human reviewer flipping the flag to `true`.
///
/// Skips pages without the `reviewed` key entirely (the canonical
/// SCHEMA doesn't include it; only LLM-generated artifacts do).
/// `reviewed: true` is also skipped — that's the success case.
///
/// Accepts both YAML boolean (`reviewed: false`) and string forms
/// (`reviewed: "false"`) defensively because the YAML serializer the
/// distill module uses round-trips a literal `false` but a human
/// editor might quote the value while reviewing.
///
/// **v0.20.1 cycle-4 audit H2 — qualification.** The check is
/// **only** triggered for pages that BOTH:
///   1. Carry `reviewed: false` (or string equivalent), AND
///   2. Carry a populated `source.runner` field naming an LLM
///      provider (`claude-sonnet-4-5`, `gemini-pro`, etc).
///
/// Hand-authored drafts (no `source` block, or a `source` block whose
/// `runner` is missing/empty) can use `reviewed: false` freely as a
/// workflow signal — the security boundary is "LLM-generated content
/// must be human-curated before commit", not "human drafts must be
/// final". Pre-fix the check fired on case 1/2 from the audit-prompt
/// matrix, surprising users who used `reviewed: false` to mark their
/// own drafts.
pub fn check_unreviewed_distilled(pages: &[Page]) -> Vec<LintIssue> {
    let mut issues = Vec::new();
    for page in pages {
        // v0.20.2 audit-followup #37: the qualifier moved to
        // `Page::is_unreviewed_distilled` so the MCP `render_page`
        // filter and this lint share one implementation. Both
        // call sites stay in sync via that helper.
        if !page.is_unreviewed_distilled() {
            continue;
        }
        issues.push(LintIssue {
            code: LintCode::UnreviewedDistilled,
            severity: LintSeverity::Critical,
            page: Some(page.path.clone()),
            message: format!(
                "Page '{}' has `reviewed: false` and `source.runner: {}` — flip `reviewed: true` after human review before committing.",
                page.frontmatter.slug,
                page.frontmatter
                    .extra
                    .get("source")
                    .and_then(|v| v.as_mapping())
                    .and_then(|m| m.get(serde_yaml_ng::Value::String("runner".into())))
                    .and_then(|v| match v {
                        serde_yaml_ng::Value::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or("?"),
            ),
            context: Some("reviewed".into()),
        });
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn mk_page(
        slug: &str,
        page_type: PageType,
        body: &str,
        confidence: f64,
        status: Status,
        sources: Vec<&str>,
    ) -> Page {
        mk_page_with(
            slug,
            page_type,
            body,
            confidence,
            status,
            sources,
            "abc",
            &[],
        )
    }

    /// Extended builder: lets a test override `last_updated_commit` and inject
    /// `extra` frontmatter keys (used by the new structural checks). The legacy
    /// `mk_page` delegates here with placeholder commit `"abc"` and no extras.
    #[allow(clippy::too_many_arguments)]
    fn mk_page_with(
        slug: &str,
        page_type: PageType,
        body: &str,
        confidence: f64,
        status: Status,
        sources: Vec<&str>,
        last_updated_commit: &str,
        extra_keys: &[&str],
    ) -> Page {
        let mut extra = BTreeMap::new();
        for k in extra_keys {
            extra.insert(
                (*k).to_string(),
                serde_yaml_ng::Value::String(format!("value-of-{k}")),
            );
        }
        Page {
            path: PathBuf::from(format!(".wiki/modules/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type,
                last_updated_commit: last_updated_commit.to_string(),
                confidence: Confidence::try_new(confidence).unwrap(),
                sources: sources.into_iter().map(String::from).collect(),
                backlinks: vec![],
                status,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra,
            },
            body: body.to_string(),
        }
    }

    // --- broken wikilinks -----------------------------------------------------

    #[test]
    fn broken_wikilink_critical() {
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[nonexistent]]",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "body",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
        ];
        let links: Vec<Vec<String>> = pages.iter().map(|p| p.outbound_links()).collect();
        let issues = check_broken_wikilinks(&pages, &links);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, LintCode::BrokenWikilink);
        assert_eq!(issues[0].severity, LintSeverity::Critical);
        assert_eq!(issues[0].context.as_deref(), Some("nonexistent"));
    }

    #[test]
    fn wikilink_to_existing_page_no_issue() {
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[b]]",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "body",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
        ];
        let links: Vec<Vec<String>> = pages.iter().map(|p| p.outbound_links()).collect();
        let issues = check_broken_wikilinks(&pages, &links);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn wikilink_with_anchor_resolves_to_slug() {
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[b#section]]",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "body",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
        ];
        let links: Vec<Vec<String>> = pages.iter().map(|p| p.outbound_links()).collect();
        let issues = check_broken_wikilinks(&pages, &links);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    // --- orphans --------------------------------------------------------------

    #[test]
    fn orphan_page_emits_warning() {
        // Graph: B → A. C is isolated.
        // Expected orphans: B and C (nobody links to them). A is NOT orphan (B links to A).
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "alone",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "see [[a]]",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
            mk_page(
                "c",
                PageType::Module,
                "lonely",
                0.8,
                Status::Draft,
                vec!["src/c.rs"],
            ),
        ];
        let links: Vec<Vec<String>> = pages.iter().map(|p| p.outbound_links()).collect();
        let issues = check_orphan_pages(&pages, &links);
        let orphan_slugs: Vec<&str> = issues
            .iter()
            .filter_map(|i| {
                i.page
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .map(|s| s.to_str().unwrap())
            })
            .collect();
        assert!(
            orphan_slugs.contains(&"b"),
            "b should be orphan: {issues:?}"
        );
        assert!(
            orphan_slugs.contains(&"c"),
            "c should be orphan: {issues:?}"
        );
        assert!(
            !orphan_slugs.contains(&"a"),
            "a is referenced by b, must NOT be orphan: {issues:?}"
        );
        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn orphan_skips_system_pages() {
        let pages = vec![mk_page(
            "index",
            PageType::Index,
            "nothing",
            0.8,
            Status::Draft,
            vec!["src/i.rs"],
        )];
        let links: Vec<Vec<String>> = pages.iter().map(|p| p.outbound_links()).collect();
        let issues = check_orphan_pages(&pages, &links);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn orphan_skips_log_schema_readme() {
        let pages = vec![
            mk_page(
                "log",
                PageType::Log,
                "",
                0.8,
                Status::Draft,
                vec!["src/l.rs"],
            ),
            mk_page(
                "schema",
                PageType::Schema,
                "",
                0.8,
                Status::Draft,
                vec!["src/s.rs"],
            ),
            mk_page(
                "readme",
                PageType::Readme,
                "",
                0.8,
                Status::Draft,
                vec!["src/r.rs"],
            ),
        ];
        let links: Vec<Vec<String>> = pages.iter().map(|p| p.outbound_links()).collect();
        let issues = check_orphan_pages(&pages, &links);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    // --- low confidence -------------------------------------------------------

    #[test]
    fn low_confidence_critical_below_03() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.2,
            Status::Draft,
            vec![],
        )];
        let issues = check_low_confidence(&pages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, LintSeverity::Critical);
        assert_eq!(issues[0].code, LintCode::LowConfidence);
    }

    #[test]
    fn low_confidence_warning_below_06() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.5,
            Status::Draft,
            vec![],
        )];
        let issues = check_low_confidence(&pages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, LintSeverity::Warning);
    }

    #[test]
    fn low_confidence_no_issue_at_or_above_06() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.6,
            Status::Draft,
            vec![],
        )];
        let issues = check_low_confidence(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn low_confidence_skips_reference_status() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.1,
            Status::Reference,
            vec![],
        )];
        let issues = check_low_confidence(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    // --- high confidence without sources --------------------------------------

    #[test]
    fn high_confidence_without_sources_warns() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.7,
            Status::Draft,
            vec![],
        )];
        let issues = check_high_confidence_without_sources(&pages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, LintSeverity::Warning);
        assert_eq!(issues[0].code, LintCode::HighConfidenceWithoutSources);
    }

    #[test]
    fn high_confidence_with_sources_no_issue() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.7,
            Status::Draft,
            vec!["src"],
        )];
        let issues = check_high_confidence_without_sources(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    #[test]
    fn low_confidence_without_sources_no_issue() {
        // This SPECIFIC check (high_conf_without_sources) should not emit when conf < 0.6.
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.4,
            Status::Draft,
            vec![],
        )];
        let issues = check_high_confidence_without_sources(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    // --- stale status ---------------------------------------------------------

    #[test]
    fn stale_status_emits_info() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Stale,
            vec!["src"],
        )];
        let issues = check_stale_status(&pages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, LintSeverity::Info);
        assert_eq!(issues[0].code, LintCode::StaleStatus);
    }

    #[test]
    fn stale_status_other_no_issue() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Reviewed,
            vec!["src"],
        )];
        let issues = check_stale_status(&pages);
        assert!(issues.is_empty(), "got: {issues:?}");
    }

    // --- run_structural aggregator -------------------------------------------

    #[test]
    fn run_structural_aggregates_all_checks() {
        // Setup:
        // - 1 broken wikilink: page "a" links to "nonexistent"
        // - 1 orphan: page "c" (nobody links to c)
        // - 1 low confidence: page "d" with confidence 0.2 (and conf >= 0.6 isn't here, so
        //   no high-conf-without-sources for d). a is conf 0.8 with sources, so no issue.
        // - "a" is also orphan, but a is referenced if b → a... let's design carefully.
        //
        // Layout:
        //   a (conf 0.8, sources=[src/a.rs], links to "nonexistent") → BrokenWikilink
        //   b (conf 0.8, sources=[src/b.rs], links to "a")           → no issue (b is orphan though)
        //   c (conf 0.8, sources=[src/c.rs])                          → OrphanPage
        //   d (conf 0.2, sources=[src/d.rs], links to "a")            → LowConfidence (also orphan)
        //
        // Expected issues (counting): 1 broken + 3 orphans (a is referenced by b+d so NOT orphan;
        // b, c, d are all orphans) + 1 low conf = 5.
        // Actually a: referenced by b ([[a]]) and d ([[a]]) → not orphan. b: nobody. c: nobody.
        // d: nobody. So orphans = {b, c, d} = 3.
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[nonexistent]]",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "see [[a]]",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
            mk_page(
                "c",
                PageType::Module,
                "",
                0.8,
                Status::Draft,
                vec!["src/c.rs"],
            ),
            mk_page(
                "d",
                PageType::Module,
                "see [[a]]",
                0.2,
                Status::Draft,
                vec!["src/d.rs"],
            ),
        ];
        let report = crate::run_structural(&pages);
        let broken = report
            .issues
            .iter()
            .filter(|i| i.code == LintCode::BrokenWikilink)
            .count();
        let orphans = report
            .issues
            .iter()
            .filter(|i| i.code == LintCode::OrphanPage)
            .count();
        let low_conf = report
            .issues
            .iter()
            .filter(|i| i.code == LintCode::LowConfidence)
            .count();
        assert_eq!(broken, 1, "expected 1 broken wikilink: {report:?}");
        assert_eq!(orphans, 3, "expected 3 orphans (b, c, d): {report:?}");
        assert_eq!(low_conf, 1, "expected 1 low confidence: {report:?}");
    }

    #[test]
    fn run_structural_sorts_by_severity() {
        // 1 critical (broken wikilink), 1 warning (orphan), 1 info (stale).
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[ghost]]",
                0.8,
                Status::Stale,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "see [[a]]",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
        ];
        // Pages: a → broken wikilink (Critical) + stale (Info). b → orphan (Warning).
        let report = crate::run_structural(&pages);
        // The first issue must be critical, the last must be info.
        assert!(!report.issues.is_empty());
        assert_eq!(report.issues[0].severity, LintSeverity::Critical);
        let last = report.issues.last().expect("at least one");
        assert_eq!(last.severity, LintSeverity::Info);
        // And in the middle there's at least one warning.
        let has_warning = report
            .issues
            .iter()
            .any(|i| i.severity == LintSeverity::Warning);
        assert!(has_warning, "expected a warning in: {report:?}");
    }

    // --- helpers for the context-aware checks --------------------------------

    /// Spin up a real on-disk git repo inside a TempDir so `check_commit_in_git`
    /// can shell out to `git rev-list` against something concrete. Returns the
    /// guard plus the SHA of the single commit we made.
    fn init_git_repo_with_one_commit() -> (tempfile::TempDir, String) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();

        // `git init` (use -q so we don't pollute test output).
        let out = std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(root)
            .output()
            .expect("git init");
        assert!(out.status.success(), "git init failed: {out:?}");

        // Local config so the commit can be created without inheriting the
        // host's identity (which CI may not have).
        for kv in [
            ("user.email", "test@example.com"),
            ("user.name", "Test"),
            ("commit.gpgsign", "false"),
        ] {
            let out = std::process::Command::new("git")
                .args(["config", kv.0, kv.1])
                .current_dir(root)
                .output()
                .expect("git config");
            assert!(out.status.success(), "git config {} failed: {out:?}", kv.0);
        }

        std::fs::write(root.join("README.md"), "hello\n").expect("write readme");
        let out = std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(root)
            .output()
            .expect("git add");
        assert!(out.status.success(), "git add failed: {out:?}");
        let out = std::process::Command::new("git")
            .args(["commit", "-q", "-m", "initial"])
            .current_dir(root)
            .output()
            .expect("git commit");
        assert!(
            out.status.success(),
            "git commit failed: {} / {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .expect("git rev-parse");
        assert!(out.status.success(), "git rev-parse failed: {out:?}");
        let sha = String::from_utf8(out.stdout).unwrap().trim().to_string();
        assert_eq!(sha.len(), 40, "expected 40-char SHA, got {sha:?}");
        (tmp, sha)
    }

    // --- check_commit_in_git --------------------------------------------------

    #[test]
    fn commit_in_git_skips_placeholder_commits() {
        let (tmp, _sha) = init_git_repo_with_one_commit();
        let pages = vec![
            mk_page_with(
                "p1",
                PageType::Module,
                "",
                0.8,
                Status::Draft,
                vec![],
                "abc",
                &[],
            ),
            mk_page_with(
                "p2",
                PageType::Module,
                "",
                0.8,
                Status::Draft,
                vec![],
                "",
                &[],
            ),
            mk_page_with(
                "p3",
                PageType::Module,
                "",
                0.8,
                Status::Draft,
                vec![],
                "zero",
                &[],
            ),
            mk_page_with(
                "p4",
                PageType::Module,
                "",
                0.8,
                Status::Draft,
                vec![],
                "unknown",
                &[],
            ),
        ];
        let issues = check_commit_in_git(&pages, tmp.path());
        assert!(
            issues.is_empty(),
            "placeholder commits must be skipped, got {issues:?}"
        );
    }

    #[test]
    fn commit_in_git_real_shape_unknown_sha_emits_warning() {
        let (tmp, _sha) = init_git_repo_with_one_commit();
        // 40-char hex value that won't collide with any real SHA in the test repo.
        let bogus = "f".repeat(40);
        let pages = vec![mk_page_with(
            "p1",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec![],
            &bogus,
            &[],
        )];
        let issues = check_commit_in_git(&pages, tmp.path());
        assert_eq!(issues.len(), 1, "expected 1 issue, got {issues:?}");
        assert_eq!(issues[0].code, LintCode::CommitNotInGit);
        assert_eq!(issues[0].severity, LintSeverity::Warning);
        assert_eq!(issues[0].context.as_deref(), Some(bogus.as_str()));
    }

    #[test]
    fn commit_in_git_known_sha_no_issue() {
        let (tmp, sha) = init_git_repo_with_one_commit();
        let pages = vec![mk_page_with(
            "p1",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec![],
            &sha,
            &[],
        )];
        let issues = check_commit_in_git(&pages, tmp.path());
        assert!(
            issues.is_empty(),
            "known SHA should be accepted, got {issues:?}"
        );
    }

    #[test]
    fn commit_in_git_no_repo_returns_empty() {
        // tempdir without `git init` → git rev-list fails → graceful empty.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let pages = vec![mk_page_with(
            "p1",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec![],
            &"f".repeat(40),
            &[],
        )];
        let issues = check_commit_in_git(&pages, tmp.path());
        assert!(
            issues.is_empty(),
            "no .git/ should degrade gracefully, got {issues:?}"
        );
    }

    // --- check_source_exists --------------------------------------------------

    #[test]
    fn source_exists_no_issue() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("a.rs"), "fn main() {}\n").unwrap();
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec!["src/a.rs"],
        )];
        let issues = check_source_exists(&pages, tmp.path());
        assert!(
            issues.is_empty(),
            "existing source should not emit, got {issues:?}"
        );
    }

    #[test]
    fn source_exists_missing_emits_warning() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec!["src/missing.rs"],
        )];
        let issues = check_source_exists(&pages, tmp.path());
        assert_eq!(issues.len(), 1, "expected 1 issue, got {issues:?}");
        assert_eq!(issues[0].code, LintCode::SourceNotFound);
        assert_eq!(issues[0].severity, LintSeverity::Warning);
        assert_eq!(issues[0].context.as_deref(), Some("src/missing.rs"));
    }

    #[test]
    fn source_exists_skips_https_urls() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec!["https://example.com/spec.pdf", "http://example.com/other"],
        )];
        let issues = check_source_exists(&pages, tmp.path());
        assert!(
            issues.is_empty(),
            "URL sources must be skipped, got {issues:?}"
        );
    }

    #[test]
    fn source_exists_multiple_missing_emits_one_per_path() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec!["src/missing-1.rs", "src/missing-2.rs", "src/missing-3.rs"],
        )];
        let issues = check_source_exists(&pages, tmp.path());
        assert_eq!(issues.len(), 3, "expected 3 issues, got {issues:?}");
        let contexts: Vec<&str> = issues.iter().filter_map(|i| i.context.as_deref()).collect();
        assert!(contexts.contains(&"src/missing-1.rs"));
        assert!(contexts.contains(&"src/missing-2.rs"));
        assert!(contexts.contains(&"src/missing-3.rs"));
    }

    // --- check_archived_linked_from_head -------------------------------------

    #[test]
    fn archived_linked_one_linker_emits_one_issue() {
        let pages = vec![
            mk_page(
                "old",
                PageType::Module,
                "",
                0.8,
                Status::Archived,
                vec!["src/old.rs"],
            ),
            mk_page(
                "live",
                PageType::Module,
                "see [[old]]",
                0.8,
                Status::Draft,
                vec!["src/live.rs"],
            ),
        ];
        let links: Vec<Vec<String>> = pages.iter().map(|p| p.outbound_links()).collect();
        let issues = check_archived_linked_from_head(&pages, &links);
        assert_eq!(issues.len(), 1, "expected 1 issue, got {issues:?}");
        assert_eq!(issues[0].code, LintCode::ArchivedPageLinked);
        assert_eq!(issues[0].severity, LintSeverity::Warning);
        // The page field is the LINKER (live), context is the archived target (old).
        assert_eq!(
            issues[0].page.as_deref().unwrap().to_str().unwrap(),
            ".wiki/modules/live.md"
        );
        assert_eq!(issues[0].context.as_deref(), Some("old"));
    }

    #[test]
    fn archived_linked_two_linkers_emit_two_issues() {
        let pages = vec![
            mk_page(
                "old",
                PageType::Module,
                "",
                0.8,
                Status::Archived,
                vec!["src/old.rs"],
            ),
            mk_page(
                "live1",
                PageType::Module,
                "see [[old]]",
                0.8,
                Status::Draft,
                vec!["src/live1.rs"],
            ),
            mk_page(
                "live2",
                PageType::Module,
                "see [[old]]",
                0.8,
                Status::Draft,
                vec!["src/live2.rs"],
            ),
        ];
        let links: Vec<Vec<String>> = pages.iter().map(|p| p.outbound_links()).collect();
        let issues = check_archived_linked_from_head(&pages, &links);
        assert_eq!(issues.len(), 2, "expected 2 issues, got {issues:?}");
        let pages_with_issue: Vec<String> = issues
            .iter()
            .filter_map(|i| i.page.as_ref().map(|p| p.display().to_string()))
            .collect();
        assert!(pages_with_issue.contains(&".wiki/modules/live1.md".to_string()));
        assert!(pages_with_issue.contains(&".wiki/modules/live2.md".to_string()));
    }

    #[test]
    fn archived_linked_only_from_archived_no_issue() {
        // Self-noise filter: archived → archived links must not fire.
        let pages = vec![
            mk_page(
                "old1",
                PageType::Module,
                "see [[old2]]",
                0.8,
                Status::Archived,
                vec!["src/old1.rs"],
            ),
            mk_page(
                "old2",
                PageType::Module,
                "",
                0.8,
                Status::Archived,
                vec!["src/old2.rs"],
            ),
        ];
        let links: Vec<Vec<String>> = pages.iter().map(|p| p.outbound_links()).collect();
        let issues = check_archived_linked_from_head(&pages, &links);
        assert!(
            issues.is_empty(),
            "archived → archived must be silenced, got {issues:?}"
        );
    }

    #[test]
    fn archived_linked_no_linkers_no_issue() {
        let pages = vec![
            mk_page(
                "old",
                PageType::Module,
                "",
                0.8,
                Status::Archived,
                vec!["src/old.rs"],
            ),
            mk_page(
                "live",
                PageType::Module,
                "no link here",
                0.8,
                Status::Draft,
                vec!["src/live.rs"],
            ),
        ];
        let links: Vec<Vec<String>> = pages.iter().map(|p| p.outbound_links()).collect();
        let issues = check_archived_linked_from_head(&pages, &links);
        assert!(
            issues.is_empty(),
            "no linkers means no issue, got {issues:?}"
        );
    }

    // --- check_unknown_extra_field -------------------------------------------

    #[test]
    fn unknown_extra_field_empty_no_issue() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec!["src/a.rs"],
        )];
        let issues = check_unknown_extra_field(&pages);
        assert!(
            issues.is_empty(),
            "no extras means no issue, got {issues:?}"
        );
    }

    #[test]
    fn unknown_extra_field_three_keys_emit_three_issues() {
        let pages = vec![mk_page_with(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec!["src/a.rs"],
            "abc",
            &["audit", "priority", "owner"],
        )];
        let issues = check_unknown_extra_field(&pages);
        assert_eq!(issues.len(), 3, "expected 3 issues, got {issues:?}");
        let contexts: Vec<&str> = issues.iter().filter_map(|i| i.context.as_deref()).collect();
        assert!(contexts.contains(&"audit"));
        assert!(contexts.contains(&"priority"));
        assert!(contexts.contains(&"owner"));
    }

    #[test]
    fn unknown_extra_field_severity_is_info() {
        let pages = vec![mk_page_with(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec!["src/a.rs"],
            "abc",
            &["audit"],
        )];
        let issues = check_unknown_extra_field(&pages);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, LintCode::UnknownExtraField);
        assert_eq!(issues[0].severity, LintSeverity::Info);
    }

    /// v0.20.0: the extra-field check now skips `reviewed` and `source`
    /// because the dedicated `check_unreviewed_distilled` and the
    /// per-page LLM-source nesting handle them. Pin so a future
    /// regression doesn't double-count these keys.
    #[test]
    fn unknown_extra_field_skips_reviewed_and_source_keys() {
        let mut extra = BTreeMap::new();
        extra.insert("reviewed".into(), serde_yaml_ng::Value::Bool(false));
        extra.insert(
            "source".into(),
            serde_yaml_ng::Value::String("placeholder".into()),
        );
        let page = Page {
            path: PathBuf::from(".wiki/synthesis/foo.md"),
            frontmatter: Frontmatter {
                slug: "foo".into(),
                page_type: PageType::Synthesis,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(0.4).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Draft,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra,
            },
            body: String::new(),
        };
        let issues = check_unknown_extra_field(&[page]);
        assert!(
            issues.is_empty(),
            "reviewed/source must be skipped here, got {issues:?}"
        );
    }

    // --- check_unreviewed_distilled (v0.20.0; v0.20.1 H2 qualifier) -----

    /// Helper: build a Synthesis page with the given `reviewed` value
    /// and (optional) `source.runner` populated. The runner field
    /// distinguishes distilled output from a hand-authored draft —
    /// see the v0.20.1 H2 qualifier.
    fn mk_distilled_page(slug: &str, reviewed: serde_yaml_ng::Value, runner: Option<&str>) -> Page {
        let mut extra = BTreeMap::new();
        extra.insert("reviewed".into(), reviewed);
        if let Some(r) = runner {
            let mut src = serde_yaml_ng::Mapping::new();
            src.insert(
                serde_yaml_ng::Value::String("runner".into()),
                serde_yaml_ng::Value::String(r.into()),
            );
            src.insert(
                serde_yaml_ng::Value::String("prompt_version".into()),
                serde_yaml_ng::Value::Number(1.into()),
            );
            extra.insert("source".into(), serde_yaml_ng::Value::Mapping(src));
        }
        Page {
            path: PathBuf::from(format!(".wiki/synthesis/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.into(),
                page_type: PageType::Synthesis,
                last_updated_commit: "abc".into(),
                confidence: Confidence::try_new(0.4).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Draft,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra,
            },
            body: String::new(),
        }
    }

    /// `reviewed: false` AND `source.runner` populated: classic
    /// distill output that hasn't been human-curated yet → Critical.
    #[test]
    fn unreviewed_distilled_bool_false_with_runner_is_critical() {
        let page = mk_distilled_page(
            "foo",
            serde_yaml_ng::Value::Bool(false),
            Some("claude-sonnet-4-5"),
        );
        let issues = check_unreviewed_distilled(&[page]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, LintCode::UnreviewedDistilled);
        assert_eq!(issues[0].severity, LintSeverity::Critical);
        assert_eq!(issues[0].context.as_deref(), Some("reviewed"));
        assert!(
            issues[0].message.contains("claude-sonnet-4-5"),
            "message should mention the runner: {}",
            issues[0].message
        );
    }

    /// `reviewed: true` (the success case) emits no issue, even with
    /// runner populated.
    #[test]
    fn unreviewed_distilled_bool_true_no_issue() {
        let page = mk_distilled_page("foo", serde_yaml_ng::Value::Bool(true), Some("claude"));
        let issues = check_unreviewed_distilled(&[page]);
        assert!(issues.is_empty());
    }

    /// String form `reviewed: "false"` with populated runner also
    /// fires (defensive against hand-edits that quote the value).
    #[test]
    fn unreviewed_distilled_string_false_with_runner_critical() {
        let page = mk_distilled_page(
            "foo",
            serde_yaml_ng::Value::String("false".into()),
            Some("gemini-pro"),
        );
        let issues = check_unreviewed_distilled(&[page]);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, LintSeverity::Critical);
    }

    /// Page without a `reviewed` key produces no issue (the canonical
    /// SCHEMA doesn't require it).
    #[test]
    fn unreviewed_distilled_missing_key_no_issue() {
        let pages = vec![mk_page(
            "a",
            PageType::Module,
            "",
            0.8,
            Status::Draft,
            vec!["src/a.rs"],
        )];
        let issues = check_unreviewed_distilled(&pages);
        assert!(issues.is_empty());
    }

    // --- v0.20.1 cycle-4 audit H2 qualifier matrix ---------------------
    //
    // The audit matrix:
    //   1. reviewed: false, source: {runner: "claude-..."}  → fires Critical.
    //   2. reviewed: false, source: {runner: ""}            → does NOT fire.
    //   3. reviewed: false, no `source` field               → does NOT fire.
    //   4. reviewed: true,  source: {runner: "..."}         → does NOT fire.

    /// Matrix case 1: distilled-shaped page → fires Critical.
    #[test]
    fn h2_matrix_case_1_distilled_unreviewed_fires() {
        let page = mk_distilled_page(
            "case1",
            serde_yaml_ng::Value::Bool(false),
            Some("claude-sonnet-4-5"),
        );
        let issues = check_unreviewed_distilled(&[page]);
        assert_eq!(issues.len(), 1, "case 1 must fire");
    }

    /// Matrix case 2: empty-string runner → does NOT fire.
    /// A page that says `source.runner: ""` is malformed but it's
    /// not LLM output — the lint stays out of the way.
    #[test]
    fn h2_matrix_case_2_empty_runner_does_not_fire() {
        let page = mk_distilled_page("case2", serde_yaml_ng::Value::Bool(false), Some(""));
        let issues = check_unreviewed_distilled(&[page]);
        assert!(
            issues.is_empty(),
            "case 2 (empty runner) must NOT fire; got: {issues:?}"
        );
    }

    /// Matrix case 3: no `source` field at all → does NOT fire. This
    /// is the hand-authored-draft path: a user marks `reviewed: false`
    /// in their own page as a workflow signal. We don't gate that.
    #[test]
    fn h2_matrix_case_3_no_source_does_not_fire() {
        let page = mk_distilled_page("case3", serde_yaml_ng::Value::Bool(false), None);
        let issues = check_unreviewed_distilled(&[page]);
        assert!(
            issues.is_empty(),
            "case 3 (no source field) must NOT fire; got: {issues:?}"
        );
    }

    /// Matrix case 4: `reviewed: true` with runner → does NOT fire.
    #[test]
    fn h2_matrix_case_4_reviewed_true_does_not_fire() {
        let page = mk_distilled_page(
            "case4",
            serde_yaml_ng::Value::Bool(true),
            Some("claude-sonnet-4-5"),
        );
        let issues = check_unreviewed_distilled(&[page]);
        assert!(issues.is_empty(), "case 4 must NOT fire");
    }

    // --- run_structural_with_root aggregator --------------------------------

    #[test]
    fn run_structural_with_root_clean_workspace_zero_issues() {
        // A pristine, hand-crafted workspace: every page links to every other,
        // confidence high, sources point at real files, no extras, placeholder
        // commit (skipped). Must produce 0 issues end-to-end.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(src.join("b.rs"), "fn b() {}\n").unwrap();
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[b]]",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "see [[a]]",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
        ];
        let report = crate::run_structural_with_root(&pages, tmp.path());
        assert!(
            report.issues.is_empty(),
            "clean workspace produced unexpected issues: {report:?}"
        );
    }

    #[test]
    fn run_structural_with_root_triggers_each_new_check() {
        // Build a deliberately-bad workspace so each of the 4 new checks fires
        // at least once — and assert that ONLY the expected codes appear from
        // the new family.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        // git init so commit-in-git can run, but the page references a SHA
        // that doesn't exist → 1 CommitNotInGit.
        let _ = std::process::Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(tmp.path())
            .output()
            .expect("git init");
        for kv in [
            ("user.email", "test@example.com"),
            ("user.name", "Test"),
            ("commit.gpgsign", "false"),
        ] {
            let _ = std::process::Command::new("git")
                .args(["config", kv.0, kv.1])
                .current_dir(tmp.path())
                .output()
                .expect("git config");
        }
        std::fs::write(tmp.path().join("README.md"), "hi\n").unwrap();
        let _ = std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(tmp.path())
            .output();
        let _ = std::process::Command::new("git")
            .args(["commit", "-q", "-m", "initial"])
            .current_dir(tmp.path())
            .output();

        // Build pages:
        //   archived (Archived)                 → target for ArchivedPageLinked
        //   live (links to archived,            → +ArchivedPageLinked
        //         missing source,               → +SourceNotFound
        //         bogus 40-char SHA,            → +CommitNotInGit
        //         extra key)                    → +UnknownExtraField
        let pages = vec![
            mk_page(
                "archived",
                PageType::Module,
                "",
                0.8,
                Status::Archived,
                vec![],
            ),
            mk_page_with(
                "live",
                PageType::Module,
                "see [[archived]]",
                0.8,
                Status::Draft,
                vec!["src/missing.rs"],
                &"f".repeat(40),
                &["audit"],
            ),
        ];
        let report = crate::run_structural_with_root(&pages, tmp.path());

        let codes: std::collections::HashSet<LintCode> =
            report.issues.iter().map(|i| i.code).collect();
        assert!(
            codes.contains(&LintCode::ArchivedPageLinked),
            "missing ArchivedPageLinked: {report:?}"
        );
        assert!(
            codes.contains(&LintCode::SourceNotFound),
            "missing SourceNotFound: {report:?}"
        );
        assert!(
            codes.contains(&LintCode::CommitNotInGit),
            "missing CommitNotInGit: {report:?}"
        );
        assert!(
            codes.contains(&LintCode::UnknownExtraField),
            "missing UnknownExtraField: {report:?}"
        );
    }

    #[test]
    fn run_structural_backward_compat_delegates() {
        // The legacy `run_structural(&pages)` must still work and produce the
        // same set of structural issues for a graph-only fixture (where the
        // context-aware checks happen to find nothing because the cwd `.` has
        // no .git access for the bogus paths the pages reference).
        let pages = vec![
            mk_page(
                "a",
                PageType::Module,
                "see [[ghost]]",
                0.8,
                Status::Draft,
                vec!["src/a.rs"],
            ),
            mk_page(
                "b",
                PageType::Module,
                "see [[a]]",
                0.8,
                Status::Draft,
                vec!["src/b.rs"],
            ),
        ];
        let report_legacy = crate::run_structural(&pages);
        let report_explicit = crate::run_structural_with_root(&pages, std::path::Path::new("."));
        // Both must surface BrokenWikilink for [[ghost]].
        let broken_legacy = report_legacy
            .issues
            .iter()
            .filter(|i| i.code == LintCode::BrokenWikilink)
            .count();
        let broken_explicit = report_explicit
            .issues
            .iter()
            .filter(|i| i.code == LintCode::BrokenWikilink)
            .count();
        assert_eq!(broken_legacy, 1);
        assert_eq!(broken_explicit, 1);
        // And the two should agree on the issue count: legacy is just a
        // delegating wrapper.
        assert_eq!(
            report_legacy.issues.len(),
            report_explicit.issues.len(),
            "legacy and explicit must agree:\n  legacy={report_legacy:?}\n  explicit={report_explicit:?}"
        );
    }
}
