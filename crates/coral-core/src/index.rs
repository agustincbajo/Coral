//! `WikiIndex` — catalog of all wiki pages + last-commit anchor for incremental ingest.

use crate::error::Result;
use crate::frontmatter::{Confidence, PageType, Status};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Single entry in the wiki index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub slug: String,
    pub page_type: PageType,
    pub path: String,
    pub confidence: Confidence,
    pub status: Status,
    pub last_updated_commit: String,
}

/// Wiki index — catalog of all pages + last_commit anchor for incremental ingest.
#[derive(Debug, Clone, PartialEq)]
pub struct WikiIndex {
    pub last_commit: String,
    pub generated_at: DateTime<Utc>,
    pub entries: Vec<IndexEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct IndexFrontmatter {
    last_commit: String,
    generated_at: DateTime<Utc>,
}

impl WikiIndex {
    pub fn new(last_commit: impl Into<String>) -> Self {
        Self {
            last_commit: last_commit.into(),
            generated_at: Utc::now(),
            entries: Vec::new(),
        }
    }

    /// Parse from the contents of `.wiki/index.md`.
    /// Format: frontmatter with `last_commit` + `generated_at`, body is a Markdown table.
    pub fn parse(content: &str) -> Result<Self> {
        // Manually peel the `---\n...\n---\n` block — the index frontmatter shape
        // is different from `Frontmatter`, so we deserialize into `IndexFrontmatter` directly.
        let mut lines = content.split_inclusive('\n');
        let first = lines.next().unwrap_or("");
        if first.trim_end() != "---" {
            return Err(crate::error::CoralError::MissingFrontmatter {
                path: std::path::PathBuf::from("<index>"),
            });
        }
        let mut yaml_buf = String::new();
        let mut consumed = first.len();
        let mut found_close = false;
        for line in lines.by_ref() {
            consumed += line.len();
            if line.trim_end() == "---" {
                found_close = true;
                break;
            }
            yaml_buf.push_str(line);
        }
        if !found_close {
            return Err(crate::error::CoralError::UnterminatedFrontmatter {
                path: std::path::PathBuf::from("<index>"),
            });
        }
        let fm: IndexFrontmatter = serde_yaml_ng::from_str(&yaml_buf)?;

        // Body = everything after the closing `---\n`.
        let body = if consumed >= content.len() {
            ""
        } else {
            &content[consumed..]
        };

        let entries = parse_table(body)?;

        Ok(Self {
            last_commit: fm.last_commit,
            generated_at: fm.generated_at,
            entries,
        })
    }

    /// Serialize to a Markdown document suitable for `.wiki/index.md`.
    pub fn to_string(&self) -> Result<String> {
        let mut out = String::new();
        out.push_str("---\n");
        out.push_str(&format!("last_commit: {}\n", self.last_commit));
        out.push_str(&format!(
            "generated_at: {}\n",
            self.generated_at.to_rfc3339()
        ));
        out.push_str("---\n\n");
        out.push_str("# Wiki index\n\n");
        out.push_str("| Type | Slug | Path | Confidence | Status | Last commit |\n");
        out.push_str("|------|------|------|------------|--------|-------------|\n");

        let mut sorted = self.entries.clone();
        sorted.sort_by(|a, b| {
            page_type_key(&a.page_type)
                .cmp(page_type_key(&b.page_type))
                .then_with(|| a.slug.cmp(&b.slug))
        });
        for e in &sorted {
            out.push_str(&format!(
                "| {} | {} | {} | {:.2} | {} | {} |\n",
                page_type_key(&e.page_type),
                e.slug,
                e.path,
                e.confidence.as_f64(),
                status_key(&e.status),
                e.last_updated_commit,
            ));
        }
        Ok(out)
    }

    /// Insert or update an entry by slug. Idempotent.
    pub fn upsert(&mut self, entry: IndexEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.slug == entry.slug) {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    /// Find an entry by slug.
    pub fn find(&self, slug: &str) -> Option<&IndexEntry> {
        self.entries.iter().find(|e| e.slug == slug)
    }

    /// Set the last_commit and bump generated_at to `Utc::now()`.
    pub fn bump_last_commit(&mut self, commit: impl Into<String>) {
        self.last_commit = commit.into();
        self.generated_at = Utc::now();
    }
}

fn parse_table(body: &str) -> Result<Vec<IndexEntry>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in body.lines() {
        let l = line.trim();
        if !l.starts_with('|') {
            continue;
        }
        // Split on `|`. After split, `|a|b|` → ["", "a", "b", ""].
        let cells: Vec<String> = l.split('|').map(|c| c.trim().to_string()).collect();
        // Drop the head/tail empty cells produced by the leading/trailing `|`.
        let mut trimmed: Vec<String> = cells;
        if trimmed.first().map(|s| s.is_empty()).unwrap_or(false) {
            trimmed.remove(0);
        }
        if trimmed.last().map(|s| s.is_empty()).unwrap_or(false) {
            trimmed.pop();
        }
        if trimmed.is_empty() {
            continue;
        }
        rows.push(trimmed);
    }

    if rows.len() < 2 {
        return Ok(Vec::new());
    }

    // Skip header row + separator row (---- pattern).
    let mut entries = Vec::with_capacity(rows.len().saturating_sub(2));
    for row in rows.into_iter().skip(2) {
        if row.len() < 6 {
            continue;
        }
        // Skip rows that look like separators (all dashes/colons).
        if row.iter().all(|c| {
            c.chars()
                .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
        }) {
            continue;
        }
        let page_type = parse_page_type(&row[0])?;
        let slug = row[1].clone();
        let path = row[2].clone();
        let conf_f64: f64 = row[3].parse().map_err(|e: std::num::ParseFloatError| {
            crate::error::CoralError::Yaml(serde_yaml_ng::Error::custom_with_kind(
                "parse confidence",
                e.to_string(),
            ))
        })?;
        let confidence = Confidence::try_new(conf_f64)?;
        let status = parse_status(&row[4])?;
        let last_updated_commit = row[5].clone();
        entries.push(IndexEntry {
            slug,
            page_type,
            path,
            confidence,
            status,
            last_updated_commit,
        });
    }

    Ok(entries)
}

fn parse_page_type(s: &str) -> Result<PageType> {
    let v = serde_yaml_ng::Value::String(s.to_string());
    serde_yaml_ng::from_value(v).map_err(crate::error::CoralError::from)
}

fn parse_status(s: &str) -> Result<Status> {
    let v = serde_yaml_ng::Value::String(s.to_string());
    serde_yaml_ng::from_value(v).map_err(crate::error::CoralError::from)
}

fn page_type_key(pt: &PageType) -> &'static str {
    match pt {
        PageType::Module => "module",
        PageType::Concept => "concept",
        PageType::Entity => "entity",
        PageType::Flow => "flow",
        PageType::Decision => "decision",
        PageType::Synthesis => "synthesis",
        PageType::Operation => "operation",
        PageType::Source => "source",
        PageType::Gap => "gap",
        PageType::Index => "index",
        PageType::Log => "log",
        PageType::Schema => "schema",
        PageType::Readme => "readme",
        PageType::Reference => "reference",
        PageType::Interface => "interface",
    }
}

fn status_key(s: &Status) -> &'static str {
    match s {
        Status::Draft => "draft",
        Status::Reviewed => "reviewed",
        Status::Verified => "verified",
        Status::Stale => "stale",
        Status::Archived => "archived",
        Status::Reference => "reference",
    }
}

// Bridge from `serde_yaml_ng::Error` for the table cell parsing branch.
trait YamlErrorBridge {
    fn custom_with_kind(_kind: &'static str, msg: String) -> serde_yaml_ng::Error;
}

impl YamlErrorBridge for serde_yaml_ng::Error {
    fn custom_with_kind(_kind: &'static str, msg: String) -> serde_yaml_ng::Error {
        <serde_yaml_ng::Error as serde::de::Error>::custom(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(slug: &str, pt: PageType, path: &str) -> IndexEntry {
        IndexEntry {
            slug: slug.to_string(),
            page_type: pt,
            path: path.to_string(),
            confidence: Confidence::try_new(0.85).unwrap(),
            status: Status::Reviewed,
            last_updated_commit: "abc123".to_string(),
        }
    }

    #[test]
    fn index_new_starts_empty() {
        let idx = WikiIndex::new("zero");
        assert!(idx.entries.is_empty());
        assert_eq!(idx.last_commit, "zero");
    }

    #[test]
    fn index_upsert_idempotent() {
        let mut idx = WikiIndex::new("zero");
        idx.upsert(entry("foo", PageType::Module, "modules/foo.md"));
        idx.upsert(entry("foo", PageType::Module, "modules/foo-renamed.md"));
        assert_eq!(idx.entries.len(), 1);
        assert_eq!(idx.entries[0].path, "modules/foo-renamed.md");
    }

    #[test]
    fn index_find() {
        let mut idx = WikiIndex::new("zero");
        idx.upsert(entry("a", PageType::Module, "modules/a.md"));
        idx.upsert(entry("b", PageType::Concept, "concepts/b.md"));
        assert!(idx.find("a").is_some());
        assert_eq!(idx.find("a").unwrap().slug, "a");
        assert!(idx.find("c").is_none());
    }

    #[test]
    fn index_bump_last_commit() {
        let mut idx = WikiIndex::new("zero");
        let before = idx.generated_at;
        // Sleep a tick so the timestamp advances; chrono uses monotonic clock under the hood for now()
        // but a tiny sleep is enough on any non-broken system.
        std::thread::sleep(std::time::Duration::from_millis(2));
        idx.bump_last_commit("abc123");
        assert_eq!(idx.last_commit, "abc123");
        assert!(
            idx.generated_at >= before,
            "generated_at should be >= before"
        );
    }

    #[test]
    fn index_serialize_roundtrip() {
        let mut idx = WikiIndex::new("zero");
        idx.upsert(entry("a", PageType::Module, "modules/a.md"));
        idx.upsert(entry("b", PageType::Concept, "concepts/b.md"));
        idx.upsert(entry("c", PageType::Entity, "entities/c.md"));
        let serialized = idx.to_string().expect("serialize");
        let reparsed = WikiIndex::parse(&serialized).expect("parse");
        assert_eq!(reparsed.last_commit, idx.last_commit);
        // Entries get sorted on serialize; compare sets, not order.
        assert_eq!(reparsed.entries.len(), idx.entries.len());
        for e in &idx.entries {
            assert!(
                reparsed
                    .entries
                    .iter()
                    .any(|x| x.slug == e.slug && x.page_type == e.page_type && x.path == e.path),
                "entry {} missing in reparsed",
                e.slug
            );
        }
        // generated_at compared as rfc3339 strings (the serialize path uses rfc3339).
        assert_eq!(
            idx.generated_at.to_rfc3339(),
            reparsed.generated_at.to_rfc3339()
        );
    }

    #[test]
    fn index_parse_empty_body() {
        let content =
            "---\nlast_commit: abc\ngenerated_at: 2026-04-30T18:00:00Z\n---\n\n# Wiki index\n\n";
        let idx = WikiIndex::parse(content).expect("parse");
        assert_eq!(idx.last_commit, "abc");
        assert!(idx.entries.is_empty());
    }

    #[test]
    fn index_to_string_sorts_entries() {
        let mut idx = WikiIndex::new("xyz");
        idx.upsert(entry("c-concept", PageType::Concept, "concepts/c.md"));
        idx.upsert(entry("a-module", PageType::Module, "modules/a.md"));
        idx.upsert(entry("b-entity", PageType::Entity, "entities/b.md"));
        let s = idx.to_string().expect("serialize");

        // Find the row positions in the output.
        let pos_concept = s.find("c-concept").expect("c-concept in output");
        let pos_module = s.find("a-module").expect("a-module in output");
        let pos_entity = s.find("b-entity").expect("b-entity in output");

        // Alphabetical order of page_type keys: concept < entity < module
        assert!(pos_concept < pos_entity, "concept before entity in {s}");
        assert!(pos_entity < pos_module, "entity before module in {s}");
    }
}
