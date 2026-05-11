//! Coral stats: wiki health dashboard.

use ahash::AHashSet;
use coral_core::frontmatter::{PageType, Status};
use coral_core::page::Page;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Aggregated wiki health metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatsReport {
    pub total_pages: usize,
    pub by_type: BTreeMap<String, usize>,
    pub by_status: BTreeMap<String, usize>,
    pub confidence_avg: f64,
    pub confidence_min: f64,
    pub confidence_max: f64,
    pub low_confidence_count: usize,
    pub critical_low_confidence_count: usize,
    pub stale_count: usize,
    pub archived_count: usize,
    pub total_outbound_links: usize,
    pub orphan_candidates: Vec<String>,
    /// Count of pages where `frontmatter.sources` is empty. A high value
    /// means many pages lack verifiable evidence — pair with the lint's
    /// `HighConfidenceWithoutSources` check to find the worst offenders.
    pub pages_without_sources_count: usize,
    /// Slugs of the 5 pages with the oldest `last_updated_commit`,
    /// sorted ascending by commit string then slug. Useful for spotting
    /// long-untouched pages that may have drifted from HEAD. (Future
    /// improvement: real timestamp comparison via `git log`.)
    pub oldest_commit_age_pages: Vec<String>,
    /// Confidence distribution into 4 buckets. Keys: `"0.0-0.3"`,
    /// `"0.3-0.6"`, `"0.6-0.8"`, `"0.8-1.0"`. Empty buckets are still
    /// listed (with value 0) so the JSON shape is stable.
    pub pages_by_confidence_bucket: BTreeMap<String, usize>,
}

impl StatsReport {
    /// Computes the report from a slice of pages.
    pub fn new(pages: &[Page]) -> Self {
        let total_pages = pages.len();

        let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
        let mut by_status: BTreeMap<String, usize> = BTreeMap::new();
        let mut confidence_sum = 0.0_f64;
        let mut confidence_min = f64::INFINITY;
        let mut confidence_max = f64::NEG_INFINITY;
        let mut low_confidence_count = 0usize;
        let mut critical_low_confidence_count = 0usize;
        let mut stale_count = 0usize;
        let mut archived_count = 0usize;
        let mut total_outbound_links = 0usize;
        let mut pages_without_sources_count = 0usize;

        // Initialize confidence buckets with all 4 keys mapped to 0 so empty
        // buckets are still represented in the JSON output (stable shape).
        let mut pages_by_confidence_bucket: BTreeMap<String, usize> = BTreeMap::new();
        pages_by_confidence_bucket.insert("0.0-0.3".to_string(), 0);
        pages_by_confidence_bucket.insert("0.3-0.6".to_string(), 0);
        pages_by_confidence_bucket.insert("0.6-0.8".to_string(), 0);
        pages_by_confidence_bucket.insert("0.8-1.0".to_string(), 0);

        for p in pages {
            *by_type
                .entry(page_type_name(p.frontmatter.page_type).to_string())
                .or_insert(0) += 1;
            *by_status
                .entry(status_name(p.frontmatter.status).to_string())
                .or_insert(0) += 1;

            let c = p.frontmatter.confidence.as_f64();
            confidence_sum += c;
            if c < confidence_min {
                confidence_min = c;
            }
            if c > confidence_max {
                confidence_max = c;
            }
            if c < 0.6 {
                low_confidence_count += 1;
            }
            if c < 0.3 {
                critical_low_confidence_count += 1;
            }

            match p.frontmatter.status {
                Status::Stale => stale_count += 1,
                Status::Archived => archived_count += 1,
                _ => {}
            }

            total_outbound_links += p.outbound_links().len();

            if p.frontmatter.sources.is_empty() {
                pages_without_sources_count += 1;
            }

            // Bucket the confidence value. The top bucket is closed at 1.0
            // so a perfect-confidence page lands in "0.8-1.0".
            let bucket_key = if c < 0.3 {
                "0.0-0.3"
            } else if c < 0.6 {
                "0.3-0.6"
            } else if c < 0.8 {
                "0.6-0.8"
            } else {
                "0.8-1.0"
            };
            if let Some(slot) = pages_by_confidence_bucket.get_mut(bucket_key) {
                *slot += 1;
            }
        }

        // Collect (commit, slug) pairs and pick the 5 oldest by lexicographic
        // commit string then slug. Lexicographic ordering on commit hashes
        // isn't a real timestamp, but it's deterministic and cheap; a future
        // improvement can swap in `git log` for true age.
        let mut commit_slug_pairs: Vec<(String, String)> = pages
            .iter()
            .map(|p| {
                (
                    p.frontmatter.last_updated_commit.clone(),
                    p.frontmatter.slug.clone(),
                )
            })
            .collect();
        commit_slug_pairs.sort();
        let oldest_commit_age_pages: Vec<String> = commit_slug_pairs
            .into_iter()
            .take(5)
            .map(|(_, slug)| slug)
            .collect();

        let confidence_avg = if total_pages == 0 {
            0.0
        } else {
            confidence_sum / total_pages as f64
        };
        let (confidence_min, confidence_max) = if total_pages == 0 {
            (0.0, 0.0)
        } else {
            (confidence_min, confidence_max)
        };

        // Build inbound set: any slug referenced from another page (outbound link or
        // backlinks field) is recorded as having inbound traffic. Pages whose slug
        // never appears in any inbound get flagged as orphan candidates.
        let known_slugs: AHashSet<&str> =
            pages.iter().map(|p| p.frontmatter.slug.as_str()).collect();
        let mut inbound: AHashSet<String> = AHashSet::new();
        for p in pages {
            for link in p.outbound_links() {
                if known_slugs.contains(link.as_str()) {
                    inbound.insert(link);
                }
            }
        }

        // System page types are structural — never count them as orphans.
        let mut orphan_candidates: Vec<String> = pages
            .iter()
            .filter(|p| !is_system_type(p.frontmatter.page_type))
            .filter(|p| !inbound.contains(&p.frontmatter.slug))
            .map(|p| p.frontmatter.slug.clone())
            .collect();
        orphan_candidates.sort();
        orphan_candidates.dedup();

        Self {
            total_pages,
            by_type,
            by_status,
            confidence_avg,
            confidence_min,
            confidence_max,
            low_confidence_count,
            critical_low_confidence_count,
            stale_count,
            archived_count,
            total_outbound_links,
            orphan_candidates,
            pages_without_sources_count,
            oldest_commit_age_pages,
            pages_by_confidence_bucket,
        }
    }

    /// Renders a Markdown dashboard for human consumption.
    pub fn as_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# Wiki stats\n\n");
        out.push_str(&format!("- Total pages: {}\n", self.total_pages));

        out.push_str("- By type:\n");
        if self.by_type.is_empty() {
            out.push_str("  - (none)\n");
        } else {
            for (k, v) in &self.by_type {
                out.push_str(&format!("  - {k}: {v}\n"));
            }
        }

        out.push_str("- By status:\n");
        if self.by_status.is_empty() {
            out.push_str("  - (none)\n");
        } else {
            for (k, v) in &self.by_status {
                out.push_str(&format!("  - {k}: {v}\n"));
            }
        }

        out.push_str(&format!(
            "- Confidence: avg {:.2} (min {:.2}, max {:.2})\n",
            self.confidence_avg, self.confidence_min, self.confidence_max
        ));
        out.push_str(&format!(
            "- Low confidence (<0.6): {}\n",
            self.low_confidence_count
        ));
        out.push_str(&format!(
            "- Critical low confidence (<0.3): {}\n",
            self.critical_low_confidence_count
        ));
        out.push_str(&format!("- Stale pages: {}\n", self.stale_count));
        out.push_str(&format!("- Archived pages: {}\n", self.archived_count));
        out.push_str(&format!(
            "- Total outbound links: {}\n",
            self.total_outbound_links
        ));

        out.push_str(&format!(
            "- Pages without sources: {}\n",
            self.pages_without_sources_count
        ));

        if self.oldest_commit_age_pages.is_empty() {
            out.push_str("- Oldest commits (top 5): (none)\n");
        } else {
            out.push_str("- Oldest commits (top 5):\n");
            for slug in &self.oldest_commit_age_pages {
                out.push_str(&format!("  - `{slug}`\n"));
            }
        }

        out.push_str("- Confidence distribution:\n");
        for (k, v) in &self.pages_by_confidence_bucket {
            out.push_str(&format!("  - {k}: {v}\n"));
        }

        if self.orphan_candidates.is_empty() {
            out.push_str("- Orphan candidates: 0\n");
        } else {
            out.push_str(&format!(
                "- Orphan candidates: {} ({})\n",
                self.orphan_candidates.len(),
                self.orphan_candidates.join(", ")
            ));
        }

        out
    }

    /// Renders the report as pretty JSON.
    pub fn as_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Returns the JSON schema for `StatsReport` as a pretty-printed string.
    /// Use this to validate downstream tooling (jq pipelines, dashboards, etc.)
    /// against the contract Coral emits.
    pub fn json_schema() -> String {
        let schema = schemars::schema_for!(StatsReport);
        serde_json::to_string_pretty(&schema).expect("StatsReport schema is always serializable")
    }
}

fn page_type_name(t: PageType) -> &'static str {
    match t {
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

fn status_name(s: Status) -> &'static str {
    match s {
        Status::Draft => "draft",
        Status::Reviewed => "reviewed",
        Status::Verified => "verified",
        Status::Stale => "stale",
        Status::Archived => "archived",
        Status::Reference => "reference",
    }
}

fn is_system_type(t: PageType) -> bool {
    matches!(
        t,
        PageType::Index | PageType::Log | PageType::Schema | PageType::Readme | PageType::Reference
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn make_page(
        slug: &str,
        page_type: PageType,
        status: Status,
        confidence: f64,
        body: &str,
        backlinks: Vec<&str>,
    ) -> Page {
        Page {
            path: PathBuf::from(format!("test/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type,
                last_updated_commit: "abc".to_string(),
                confidence: Confidence::try_new(confidence).unwrap(),
                sources: vec![],
                backlinks: backlinks.into_iter().map(|s| s.to_string()).collect(),
                status,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra: BTreeMap::new(),
            },
            body: body.to_string(),
        }
    }

    #[test]
    fn stats_empty_pages() {
        let report = StatsReport::new(&[]);
        assert_eq!(report.total_pages, 0);
        assert_eq!(report.confidence_avg, 0.0);
        assert_eq!(report.confidence_min, 0.0);
        assert_eq!(report.confidence_max, 0.0);
        assert!(report.by_type.is_empty());
        assert!(report.by_status.is_empty());
        assert!(report.orphan_candidates.is_empty());
        assert_eq!(report.total_outbound_links, 0);
    }

    #[test]
    fn stats_counts_by_type_and_status() {
        let pages = vec![
            make_page("a", PageType::Module, Status::Draft, 0.5, "", vec![]),
            make_page("b", PageType::Module, Status::Reviewed, 0.6, "", vec![]),
            make_page("c", PageType::Module, Status::Verified, 0.7, "", vec![]),
            make_page("d", PageType::Concept, Status::Draft, 0.8, "", vec![]),
            make_page("e", PageType::Concept, Status::Reviewed, 0.9, "", vec![]),
        ];
        let r = StatsReport::new(&pages);
        assert_eq!(r.total_pages, 5);
        assert_eq!(r.by_type.get("module").copied(), Some(3));
        assert_eq!(r.by_type.get("concept").copied(), Some(2));
        assert_eq!(r.by_status.get("draft").copied(), Some(2));
        assert_eq!(r.by_status.get("reviewed").copied(), Some(2));
        assert_eq!(r.by_status.get("verified").copied(), Some(1));
    }

    #[test]
    fn stats_low_confidence_thresholds() {
        let pages = vec![
            make_page("a", PageType::Module, Status::Draft, 0.2, "", vec![]),
            make_page("b", PageType::Module, Status::Draft, 0.5, "", vec![]),
            make_page("c", PageType::Module, Status::Draft, 0.7, "", vec![]),
        ];
        let r = StatsReport::new(&pages);
        assert_eq!(r.low_confidence_count, 2);
        assert_eq!(r.critical_low_confidence_count, 1);
        assert!((r.confidence_avg - (0.2 + 0.5 + 0.7) / 3.0).abs() < 1e-9);
        assert_eq!(r.confidence_min, 0.2);
        assert_eq!(r.confidence_max, 0.7);
    }

    #[test]
    fn stats_orphan_candidates() {
        // A links to B in the body; B does not link back. A has no inbound → orphan.
        let pages = vec![
            make_page(
                "a",
                PageType::Module,
                Status::Draft,
                0.8,
                "see [[b]]\n",
                vec![],
            ),
            make_page("b", PageType::Module, Status::Draft, 0.8, "", vec![]),
        ];
        let r = StatsReport::new(&pages);
        assert!(
            r.orphan_candidates.contains(&"a".to_string()),
            "expected 'a' as orphan, got {:?}",
            r.orphan_candidates
        );
        assert!(
            !r.orphan_candidates.contains(&"b".to_string()),
            "'b' is linked from 'a', should not be orphan"
        );
    }

    #[test]
    fn stats_excludes_system_types_from_orphans() {
        let pages = vec![
            make_page(
                "master-index",
                PageType::Index,
                Status::Reference,
                1.0,
                "",
                vec![],
            ),
            make_page(
                "schema",
                PageType::Schema,
                Status::Reference,
                1.0,
                "",
                vec![],
            ),
            make_page(
                "readme",
                PageType::Readme,
                Status::Reference,
                1.0,
                "",
                vec![],
            ),
            make_page(
                "activity-log",
                PageType::Log,
                Status::Reference,
                1.0,
                "",
                vec![],
            ),
            make_page(
                "ref-page",
                PageType::Reference,
                Status::Reference,
                1.0,
                "",
                vec![],
            ),
            make_page("regular", PageType::Module, Status::Draft, 0.5, "", vec![]),
        ];
        let r = StatsReport::new(&pages);
        assert!(
            !r.orphan_candidates.contains(&"master-index".to_string()),
            "Index should not be orphan"
        );
        assert!(
            !r.orphan_candidates.contains(&"schema".to_string()),
            "Schema should not be orphan"
        );
        assert!(
            !r.orphan_candidates.contains(&"readme".to_string()),
            "Readme should not be orphan"
        );
        assert!(
            !r.orphan_candidates.contains(&"activity-log".to_string()),
            "Log should not be orphan"
        );
        assert!(
            !r.orphan_candidates.contains(&"ref-page".to_string()),
            "Reference should not be orphan"
        );
        assert!(
            r.orphan_candidates.contains(&"regular".to_string()),
            "Regular page with no inbound should be orphan"
        );
    }

    #[test]
    fn stats_markdown_renders_all_sections() {
        let pages = vec![
            make_page(
                "a",
                PageType::Module,
                Status::Draft,
                0.5,
                "see [[b]]\n",
                vec![],
            ),
            make_page("b", PageType::Concept, Status::Reviewed, 0.8, "", vec![]),
        ];
        let md = StatsReport::new(&pages).as_markdown();
        assert!(md.contains("Total pages"), "missing 'Total pages': {md}");
        assert!(md.contains("By type"), "missing 'By type': {md}");
        assert!(md.contains("Confidence"), "missing 'Confidence': {md}");
        assert!(
            md.contains("Total outbound links"),
            "missing 'Total outbound': {md}"
        );
        assert!(md.contains("Orphan candidates"), "missing 'Orphan': {md}");
    }

    #[test]
    fn stats_json_roundtrip_is_valid() {
        let pages = vec![make_page(
            "a",
            PageType::Module,
            Status::Draft,
            0.5,
            "",
            vec![],
        )];
        let json = StatsReport::new(&pages).as_json().expect("json ok");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(value["total_pages"], 1);
        assert!(value["by_type"].is_object());
        assert!(value["orphan_candidates"].is_array());
    }

    #[test]
    fn stats_total_outbound_counts_all_pages() {
        let pages = vec![
            make_page(
                "a",
                PageType::Module,
                Status::Draft,
                0.5,
                "[[b]] [[c]]\n",
                vec![],
            ),
            make_page("b", PageType::Module, Status::Draft, 0.5, "[[c]]\n", vec![]),
            make_page("c", PageType::Module, Status::Draft, 0.5, "", vec![]),
        ];
        let r = StatsReport::new(&pages);
        // a: 2 links, b: 1 link, c: 0 → 3 total.
        assert_eq!(r.total_outbound_links, 3);
    }

    #[test]
    fn stats_handles_page_with_self_link() {
        // Page A has [[a]] in its body — self-link.
        // Decision: self-loops count as inbound, so the page is NOT an orphan.
        let pages = vec![make_page(
            "a",
            PageType::Module,
            Status::Draft,
            0.5,
            "see [[a]]\n",
            vec![],
        )];
        let r = StatsReport::new(&pages);
        assert_eq!(r.total_outbound_links, 1, "self-link counts as outbound");
        assert!(
            !r.orphan_candidates.contains(&"a".to_string()),
            "self-link should mark inbound, so 'a' is not orphan: {:?}",
            r.orphan_candidates
        );
    }

    #[test]
    fn stats_handles_page_with_no_outbound_links() {
        let pages = vec![
            make_page("a", PageType::Module, Status::Draft, 0.5, "", vec![]),
            make_page("b", PageType::Module, Status::Draft, 0.5, "", vec![]),
        ];
        let r = StatsReport::new(&pages);
        assert_eq!(
            r.total_outbound_links, 0,
            "no body links and no backlinks → 0 outbound"
        );
    }

    #[test]
    fn stats_perf_500_pages_under_50ms() {
        // Build 500 synthetic pages, each linking the next two slugs.
        let pages: Vec<Page> = (0..500)
            .map(|i| {
                let body = format!(
                    "link to [[p{}]] and [[p{}]]\n",
                    (i + 1) % 500,
                    (i + 2) % 500
                );
                make_page(
                    &format!("p{i}"),
                    PageType::Module,
                    Status::Draft,
                    0.5,
                    &body,
                    vec![],
                )
            })
            .collect();

        let start = std::time::Instant::now();
        let report = StatsReport::new(&pages);
        let elapsed = start.elapsed();

        assert_eq!(report.total_pages, 500);
        // 500 pages × 2 links = 1000 outbound.
        assert_eq!(report.total_outbound_links, 1000);
        assert!(
            elapsed.as_millis() < 50,
            "stats over 500 pages took {:?} (>50ms)",
            elapsed
        );
    }

    #[test]
    fn stats_json_schema_is_valid_json() {
        let schema = StatsReport::json_schema();
        let value: serde_json::Value =
            serde_json::from_str(&schema).expect("schema must be valid JSON");
        // Top-level schema object should expose properties + describe StatsReport.
        assert!(
            value.get("properties").is_some(),
            "schema is missing 'properties' key: {schema}"
        );
        let props = value.get("properties").and_then(|v| v.as_object());
        assert!(
            props
                .map(|m| m.contains_key("total_pages"))
                .unwrap_or(false),
            "schema properties missing 'total_pages': {schema}"
        );
    }

    #[test]
    fn stats_json_output_validates_against_schema() {
        // Light-touch validation: confirm the JSON output round-trips back to a
        // StatsReport. That proves the schema's contract holds at the field
        // level without pulling jsonschema into the dep graph.
        let pages = vec![
            make_page("a", PageType::Module, Status::Draft, 0.5, "[[b]]\n", vec![]),
            make_page("b", PageType::Module, Status::Reviewed, 0.8, "", vec![]),
        ];
        let report = StatsReport::new(&pages);
        let json = report.as_json().expect("json ok");
        let roundtripped: StatsReport = serde_json::from_str(&json).expect("output must roundtrip");
        assert_eq!(roundtripped, report, "json output must roundtrip exactly");
    }

    #[test]
    fn stats_counts_stale_and_archived() {
        let pages = vec![
            make_page("a", PageType::Module, Status::Stale, 0.5, "", vec![]),
            make_page("b", PageType::Module, Status::Stale, 0.5, "", vec![]),
            make_page("c", PageType::Module, Status::Archived, 0.5, "", vec![]),
            make_page("d", PageType::Module, Status::Draft, 0.5, "", vec![]),
        ];
        let r = StatsReport::new(&pages);
        assert_eq!(r.stale_count, 2);
        assert_eq!(r.archived_count, 1);
    }

    /// Helper that mirrors `make_page` but lets the caller override
    /// `last_updated_commit` and `sources`. Used by the new metric tests.
    fn make_page_full(slug: &str, confidence: f64, commit: &str, sources: Vec<&str>) -> Page {
        Page {
            path: PathBuf::from(format!("test/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type: PageType::Module,
                last_updated_commit: commit.to_string(),
                confidence: Confidence::try_new(confidence).unwrap(),
                sources: sources.into_iter().map(|s| s.to_string()).collect(),
                backlinks: vec![],
                status: Status::Draft,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                superseded_by: None,
                extra: BTreeMap::new(),
            },
            body: String::new(),
        }
    }

    // ---------- pages_without_sources_count ----------

    #[test]
    fn stats_pages_without_sources_mixed() {
        // 5 pages: 2 with sources, 3 without → 3.
        let pages = vec![
            make_page_full("a", 0.5, "c1", vec!["src1"]),
            make_page_full("b", 0.5, "c2", vec![]),
            make_page_full("c", 0.5, "c3", vec!["src1", "src2"]),
            make_page_full("d", 0.5, "c4", vec![]),
            make_page_full("e", 0.5, "c5", vec![]),
        ];
        let r = StatsReport::new(&pages);
        assert_eq!(r.pages_without_sources_count, 3);
    }

    #[test]
    fn stats_pages_without_sources_zero_pages() {
        let r = StatsReport::new(&[]);
        assert_eq!(r.pages_without_sources_count, 0);
    }

    #[test]
    fn stats_pages_without_sources_all_have_sources() {
        let pages = vec![
            make_page_full("a", 0.5, "c1", vec!["src1"]),
            make_page_full("b", 0.5, "c2", vec!["src1"]),
        ];
        let r = StatsReport::new(&pages);
        assert_eq!(r.pages_without_sources_count, 0);
    }

    // ---------- oldest_commit_age_pages ----------

    #[test]
    fn stats_oldest_commits_caps_at_five() {
        // 7 pages with commits ordered descending alphabetically; expect the
        // 5 oldest (lex-ascending) to appear in ascending order.
        let pages = vec![
            make_page_full("a", 0.5, "g", vec![]),
            make_page_full("b", 0.5, "f", vec![]),
            make_page_full("c", 0.5, "e", vec![]),
            make_page_full("d", 0.5, "d", vec![]),
            make_page_full("e", 0.5, "c", vec![]),
            make_page_full("f", 0.5, "b", vec![]),
            make_page_full("g", 0.5, "a", vec![]),
        ];
        let r = StatsReport::new(&pages);
        // Sorted by (commit, slug) asc, take 5: a→g, b→f, c→e, d→d, e→c.
        assert_eq!(
            r.oldest_commit_age_pages,
            vec![
                "g".to_string(),
                "f".to_string(),
                "e".to_string(),
                "d".to_string(),
                "c".to_string()
            ]
        );
    }

    #[test]
    fn stats_oldest_commits_no_padding_under_five() {
        let pages = vec![
            make_page_full("a", 0.5, "c1", vec![]),
            make_page_full("b", 0.5, "c2", vec![]),
            make_page_full("c", 0.5, "c3", vec![]),
        ];
        let r = StatsReport::new(&pages);
        // 3 pages → all 3 (sorted by commit asc).
        assert_eq!(
            r.oldest_commit_age_pages,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn stats_oldest_commits_empty_pages() {
        let r = StatsReport::new(&[]);
        assert!(r.oldest_commit_age_pages.is_empty());
    }

    #[test]
    fn stats_oldest_commits_tie_break_by_slug() {
        // Two pages with identical commit strings; expect ascending slug order.
        let pages = vec![
            make_page_full("zeta", 0.5, "samecommit", vec![]),
            make_page_full("alpha", 0.5, "samecommit", vec![]),
        ];
        let r = StatsReport::new(&pages);
        assert_eq!(
            r.oldest_commit_age_pages,
            vec!["alpha".to_string(), "zeta".to_string()],
            "tie should break by slug ascending"
        );
    }

    // ---------- pages_by_confidence_bucket ----------

    #[test]
    fn stats_buckets_distribution() {
        // 1 at 0.1, 2 at 0.4, 3 at 0.7, 4 at 0.95 → {1, 2, 3, 4}.
        let mut pages = vec![make_page_full("low", 0.1, "c", vec![])];
        for i in 0..2 {
            pages.push(make_page_full(&format!("mid{i}"), 0.4, "c", vec![]));
        }
        for i in 0..3 {
            pages.push(make_page_full(&format!("high{i}"), 0.7, "c", vec![]));
        }
        for i in 0..4 {
            pages.push(make_page_full(&format!("top{i}"), 0.95, "c", vec![]));
        }
        let r = StatsReport::new(&pages);
        assert_eq!(
            r.pages_by_confidence_bucket.get("0.0-0.3").copied(),
            Some(1)
        );
        assert_eq!(
            r.pages_by_confidence_bucket.get("0.3-0.6").copied(),
            Some(2)
        );
        assert_eq!(
            r.pages_by_confidence_bucket.get("0.6-0.8").copied(),
            Some(3)
        );
        assert_eq!(
            r.pages_by_confidence_bucket.get("0.8-1.0").copied(),
            Some(4)
        );
    }

    #[test]
    fn stats_buckets_boundary_lower_inclusive() {
        // 0.3 falls in [0.3, 0.6); 0.6 falls in [0.6, 0.8); 0.8 falls in [0.8, 1.0].
        let pages = vec![
            make_page_full("at_03", 0.3, "c", vec![]),
            make_page_full("at_06", 0.6, "c", vec![]),
            make_page_full("at_08", 0.8, "c", vec![]),
        ];
        let r = StatsReport::new(&pages);
        assert_eq!(
            r.pages_by_confidence_bucket.get("0.0-0.3").copied(),
            Some(0)
        );
        assert_eq!(
            r.pages_by_confidence_bucket.get("0.3-0.6").copied(),
            Some(1)
        );
        assert_eq!(
            r.pages_by_confidence_bucket.get("0.6-0.8").copied(),
            Some(1)
        );
        assert_eq!(
            r.pages_by_confidence_bucket.get("0.8-1.0").copied(),
            Some(1)
        );
    }

    #[test]
    fn stats_buckets_boundary_top_closed_at_1() {
        // 1.0 is included in the top bucket (closed interval).
        let pages = vec![make_page_full("perfect", 1.0, "c", vec![])];
        let r = StatsReport::new(&pages);
        assert_eq!(
            r.pages_by_confidence_bucket.get("0.8-1.0").copied(),
            Some(1)
        );
        assert_eq!(
            r.pages_by_confidence_bucket.get("0.6-0.8").copied(),
            Some(0)
        );
    }

    #[test]
    fn stats_buckets_empty_pages_have_all_keys_at_zero() {
        let r = StatsReport::new(&[]);
        assert_eq!(r.pages_by_confidence_bucket.len(), 4);
        for k in ["0.0-0.3", "0.3-0.6", "0.6-0.8", "0.8-1.0"] {
            assert_eq!(
                r.pages_by_confidence_bucket.get(k).copied(),
                Some(0),
                "bucket {k} should be present at 0"
            );
        }
    }

    #[test]
    fn stats_buckets_iteration_is_lex_sorted() {
        // BTreeMap iter order matches lex sort, which happens to match
        // numeric bucket order here.
        let r = StatsReport::new(&[]);
        let keys: Vec<&str> = r
            .pages_by_confidence_bucket
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(keys, vec!["0.0-0.3", "0.3-0.6", "0.6-0.8", "0.8-1.0"]);
    }

    // ---------- markdown rendering ----------

    #[test]
    fn stats_markdown_oldest_commits_none_when_empty() {
        let md = StatsReport::new(&[]).as_markdown();
        assert!(
            md.contains("- Oldest commits (top 5): (none)"),
            "expected '(none)' line for empty oldest_commit_age_pages: {md}"
        );
    }

    #[test]
    fn stats_markdown_buckets_section_prints_when_all_zero() {
        // Empty pages → all 4 buckets are 0, but the section still prints
        // each bucket line.
        let md = StatsReport::new(&[]).as_markdown();
        assert!(
            md.contains("- Confidence distribution:"),
            "missing header: {md}"
        );
        assert!(md.contains("- 0.0-0.3: 0"), "missing 0.0-0.3 line: {md}");
        assert!(md.contains("- 0.3-0.6: 0"), "missing 0.3-0.6 line: {md}");
        assert!(md.contains("- 0.6-0.8: 0"), "missing 0.6-0.8 line: {md}");
        assert!(md.contains("- 0.8-1.0: 0"), "missing 0.8-1.0 line: {md}");
    }

    // ---------- schema currency ----------

    #[test]
    fn stats_schema_matches_committed_file() {
        // Schema in lib must equal the checked-in docs/schemas/stats.schema.json,
        // byte-for-byte (modulo the trailing newline `println!` adds).
        let live = StatsReport::json_schema();
        let on_disk = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/schemas/stats.schema.json"),
        )
        .expect("schema file readable");
        // The committed file has a trailing newline (from println!); strip it.
        let on_disk_trimmed = on_disk.trim_end_matches('\n');
        assert_eq!(
            live.trim_end_matches('\n'),
            on_disk_trimmed,
            "stats.schema.json is out of sync with StatsReport. Regenerate via:\n  cargo run -p coral-stats --example print_schema > docs/schemas/stats.schema.json"
        );
    }
}
