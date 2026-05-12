//! `coral export-skill` — skill manifest export with autodetection (M3.11).
//!
//! When `--autodetect` is passed, scans the wiki for pages that match
//! skill patterns (page_type containing "skill" in the slug, or pages
//! with trigger/capability frontmatter patterns) and generates a skill
//! manifest at `.coral/skills-manifest.json`.

use anyhow::{Context, Result};
use clap::Args;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Arguments for `coral export-skill`.
#[derive(Args, Debug, Clone)]
pub struct ExportSkillArgs {
    /// Autodetect skill pages from the wiki by scanning for pages whose
    /// slug contains "skill" or whose frontmatter/body matches skill
    /// patterns (triggers, capabilities, descriptions).
    #[arg(long)]
    pub autodetect: bool,

    /// Wiki root path. Defaults to `.wiki/` in the current directory.
    #[arg(long, default_value = ".wiki")]
    pub wiki_root: PathBuf,

    /// Output path for the skills manifest.
    #[arg(long, default_value = ".coral/skills-manifest.json")]
    pub output: PathBuf,
}

/// A detected skill entry in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    /// Source wiki page slug this skill was derived from.
    pub source_page: String,
}

/// The full skills manifest written to `.coral/skills-manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillsManifest {
    pub version: String,
    pub skills: Vec<SkillEntry>,
    pub autodetected: bool,
}

/// Patterns in page slugs/content that indicate a skill page.
const SKILL_SLUG_PATTERNS: &[&str] = &["skill", "capability", "trigger"];

/// Patterns in page body that indicate skill-relevant content.
const SKILL_BODY_PATTERNS: &[&str] = &[
    "## triggers",
    "## capabilities",
    "## skill",
    "trigger:",
    "capability:",
];

pub fn run(args: ExportSkillArgs) -> Result<ExitCode> {
    if !args.autodetect {
        eprintln!(
            "coral export-skill currently requires --autodetect.\n\
             Pass --autodetect to scan the wiki for skill pages."
        );
        return Ok(ExitCode::from(2));
    }

    let skills = autodetect_skills(&args.wiki_root)?;

    let manifest = SkillsManifest {
        version: env!("CARGO_PKG_VERSION").to_string(),
        skills,
        autodetected: true,
    };

    // Write manifest.
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating output dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&manifest).context("serializing skills manifest")?;
    std::fs::write(&args.output, &json)
        .with_context(|| format!("writing manifest to {}", args.output.display()))?;

    eprintln!(
        "Discovered {} skill(s), manifest written to {}",
        manifest.skills.len(),
        args.output.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// Scan the wiki root for pages that look like skill definitions.
///
/// Detection heuristic (ordered by specificity):
/// 1. Slug contains "skill", "capability", or "trigger"
/// 2. Body contains headings/keys matching skill patterns
///
/// Returns the discovered skill entries sorted by name.
pub fn autodetect_skills(wiki_root: &Path) -> Result<Vec<SkillEntry>> {
    let mut skills = Vec::new();

    if !wiki_root.is_dir() {
        return Ok(skills);
    }

    let pages = discover_wiki_pages(wiki_root)?;

    for (slug, body) in &pages {
        if is_skill_page(slug, body) {
            let entry = extract_skill_entry(slug, body);
            skills.push(entry);
        }
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// Walk the wiki directory and collect (slug, body) pairs for all .md files.
fn discover_wiki_pages(wiki_root: &Path) -> Result<Vec<(String, String)>> {
    let mut pages = Vec::new();

    fn walk_dir(dir: &Path, pages: &mut Vec<(String, String)>) -> Result<()> {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !name.starts_with('.') && name != "_archive" {
                    walk_dir(&path, pages)?;
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                let slug = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let body = std::fs::read_to_string(&path).unwrap_or_default();
                pages.push((slug, body));
            }
        }
        Ok(())
    }

    walk_dir(wiki_root, &mut pages)?;
    Ok(pages)
}

/// Determine if a page matches skill patterns.
pub fn is_skill_page(slug: &str, body: &str) -> bool {
    let slug_lower = slug.to_lowercase();
    // Check slug patterns.
    for pattern in SKILL_SLUG_PATTERNS {
        if slug_lower.contains(pattern) {
            return true;
        }
    }
    // Check body patterns.
    let body_lower = body.to_lowercase();
    for pattern in SKILL_BODY_PATTERNS {
        if body_lower.contains(pattern) {
            return true;
        }
    }
    false
}

/// Extract a skill entry from a page's slug and body.
///
/// Attempts to parse frontmatter for description; falls back to the
/// first non-heading line of the body.
pub fn extract_skill_entry(slug: &str, body: &str) -> SkillEntry {
    let name = slug_to_name(slug);
    let description = extract_description(body);
    let triggers = extract_triggers(body);

    SkillEntry {
        name,
        description,
        triggers,
        source_page: slug.to_string(),
    }
}

/// Convert a slug like "my-skill-page" to a human name "my skill page".
fn slug_to_name(slug: &str) -> String {
    slug.replace(['-', '_'], " ")
}

/// Extract the description from a page body.
///
/// Priority: YAML frontmatter `description:` field, then first
/// non-heading, non-empty line.
fn extract_description(body: &str) -> String {
    let mut in_frontmatter = false;
    let mut past_frontmatter = false;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            if !in_frontmatter && !past_frontmatter {
                in_frontmatter = true;
                continue;
            } else if in_frontmatter {
                in_frontmatter = false;
                past_frontmatter = true;
                continue;
            }
        }
        if in_frontmatter {
            if let Some(rest) = trimmed.strip_prefix("description:") {
                let desc = rest.trim().trim_matches('"').trim_matches('\'');
                if !desc.is_empty() {
                    return desc.to_string();
                }
            }
        }
        if (past_frontmatter || !in_frontmatter)
            && !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("---") {
                return trimmed.to_string();
            }
    }
    String::new()
}

/// Extract triggers from a page body (lines under a "## Triggers" heading).
fn extract_triggers(body: &str) -> Vec<String> {
    let mut triggers = Vec::new();
    let mut in_triggers_section = false;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.to_lowercase().starts_with("## trigger") {
            in_triggers_section = true;
            continue;
        }
        if in_triggers_section {
            if trimmed.starts_with("## ") || trimmed.starts_with("# ") {
                break;
            }
            if let Some(rest) = trimmed.strip_prefix("- ") {
                if !rest.is_empty() {
                    triggers.push(rest.to_string());
                }
            }
        }
    }
    triggers
}

// ---------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn autodetect_finds_skill_pages_by_slug() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path();

        // Create a page with "skill" in the slug.
        std::fs::write(
            wiki.join("deploy-skill.md"),
            "---\ndescription: Deploy automation\n---\n\n# Deploy Skill\n\n## Triggers\n- on push to main\n- manual dispatch\n",
        )
        .unwrap();

        // Create a non-skill page.
        std::fs::write(
            wiki.join("auth-module.md"),
            "---\ndescription: Auth module\n---\n\n# Auth\n\nHandles login.\n",
        )
        .unwrap();

        let skills = autodetect_skills(wiki).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source_page, "deploy-skill");
        assert_eq!(skills[0].description, "Deploy automation");
        assert_eq!(
            skills[0].triggers,
            vec!["on push to main", "manual dispatch"]
        );
    }

    #[test]
    fn autodetect_finds_skill_pages_by_body_pattern() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path();

        // Page without "skill" in slug but with trigger section.
        std::fs::write(
            wiki.join("ci-pipeline.md"),
            "# CI Pipeline\n\nRuns tests.\n\n## Triggers\n- on pull request\n- on schedule\n",
        )
        .unwrap();

        let skills = autodetect_skills(wiki).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source_page, "ci-pipeline");
        assert_eq!(skills[0].triggers, vec!["on pull request", "on schedule"]);
    }

    #[test]
    fn autodetect_produces_valid_manifest() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path();

        std::fs::write(
            wiki.join("lint-skill.md"),
            "---\ndescription: Lints all the things\n---\n\n# Lint Skill\n\n## Triggers\n- on save\n",
        )
        .unwrap();
        std::fs::write(
            wiki.join("format-capability.md"),
            "---\ndescription: Auto-formatter\n---\n\n# Format\n\nFormats code.\n",
        )
        .unwrap();

        let skills = autodetect_skills(wiki).unwrap();
        let manifest = SkillsManifest {
            version: env!("CARGO_PKG_VERSION").to_string(),
            skills,
            autodetected: true,
        };

        // Validate it serializes to valid JSON.
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: SkillsManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.skills.len(), 2);
        assert!(parsed.autodetected);
        assert!(!parsed.version.is_empty());

        // Skills sorted alphabetically.
        assert_eq!(parsed.skills[0].name, "format capability");
        assert_eq!(parsed.skills[1].name, "lint skill");
    }

    #[test]
    fn autodetect_empty_wiki_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let skills = autodetect_skills(tmp.path()).unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn is_skill_page_detects_patterns() {
        assert!(is_skill_page("my-skill", "# content"));
        assert!(is_skill_page("deploy-capability", "plain body"));
        assert!(is_skill_page("normal-page", "## Triggers\n- on push"));
        assert!(!is_skill_page("auth-module", "# Auth\n\nnothing special"));
    }

    #[test]
    fn extract_description_from_frontmatter() {
        let body = "---\nslug: test\ndescription: My skill does stuff\n---\n\n# Body";
        assert_eq!(extract_description(body), "My skill does stuff");
    }

    #[test]
    fn extract_triggers_from_body() {
        let body = "# Skill\n\n## Triggers\n- event A\n- event B\n\n## Other\n\ntext";
        let triggers = extract_triggers(body);
        assert_eq!(triggers, vec!["event A", "event B"]);
    }
}
