//! Benchmark for the pure structural lint path.
//!
//! Runs the 7 pure structural checks (everything except `commit_in_git` and
//! `source_exists`, which shell out to git / hit the filesystem) against a
//! 100-page synthetic wiki. The fixture is shaped to give every check real
//! work to do: some pages have low confidence, some are orphans, some have
//! extras, some are archived-and-linked, etc.
//!
//! We bench through `run_structural` directly. Internally it dispatches the
//! 9 checks, and the 2 context-aware ones degrade gracefully when invoked
//! from a non-git tempdir-relative cwd — so their cost is effectively a
//! single failed `git rev-list` invocation per call. The dominant cost is
//! the 7 pure checks operating on 100 pages.

use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::page::Page;
use coral_lint::run_structural;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Builds a 100-page graph with realistic shapes:
///   - 80 pages link to one of 5 "hub" pages (so most non-hub pages are
///     orphans → OrphanPage fires)
///   - 10 pages have confidence < 0.6 (LowConfidence)
///   - 10 pages have confidence 0.7 with empty sources
///     (HighConfidenceWithoutSources)
///   - 5 pages are archived (Archived) and 5 non-archived pages link to them
///     (ArchivedPageLinked)
///   - 3 pages have an `audit` extra key (UnknownExtraField)
///   - 3 pages link to a non-existent slug "ghost" (BrokenWikilink)
///   - 5 pages have status: stale (StaleStatus)
fn build_pages() -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::with_capacity(100);
    for i in 0..100 {
        // Body: most pages link to "hub-N" where N is a small pool, and a
        // handful of pages link to "ghost" / "archived-N" to force lint hits.
        let body = if i < 3 {
            "see [[ghost]] and [[hub-0]]".to_string()
        } else if i < 8 {
            // 5 pages linking to archived pages.
            format!("relates to [[archived-{}]]", i % 5)
        } else if i < 88 {
            // 80 pages link to one of 5 hubs.
            format!("see [[hub-{}]]", i % 5)
        } else {
            // The hubs themselves and a few stragglers.
            "no outbound links here".to_string()
        };

        let (slug, status) = if i < 5 {
            (format!("archived-{i}"), Status::Archived)
        } else if i < 10 {
            (format!("hub-{}", i - 5), Status::Reviewed)
        } else if i >= 95 {
            // Last 5 pages: stale.
            (format!("stale-{}", i - 95), Status::Stale)
        } else {
            (format!("page-{i}"), Status::Reviewed)
        };

        let confidence = if (10..20).contains(&i) {
            // 10 low-confidence pages → LowConfidence warning.
            0.4
        } else if (20..30).contains(&i) {
            // 10 high-confidence-without-sources.
            0.7
        } else {
            0.8
        };

        let sources: Vec<String> = if (20..30).contains(&i) {
            vec![] // High-confidence without sources.
        } else {
            vec![format!("src/{slug}.rs")]
        };

        let mut extra = BTreeMap::new();
        if (30..33).contains(&i) {
            extra.insert(
                "audit".to_string(),
                serde_yaml_ng::Value::String("legal".to_string()),
            );
        }

        pages.push(Page {
            path: PathBuf::from(format!(".wiki/modules/{slug}.md")),
            frontmatter: Frontmatter {
                slug,
                page_type: PageType::Module,
                last_updated_commit: "abc".to_string(),
                confidence: Confidence::try_new(confidence).unwrap(),
                sources,
                backlinks: vec![],
                status,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                extra,
            },
            body,
        });
    }
    pages
}

fn bench_run_structural(c: &mut Criterion) {
    let pages = build_pages();
    c.bench_function("lint/run_structural_100_pages", |b| {
        b.iter(|| {
            let report = run_structural(black_box(&pages));
            black_box(report);
        });
    });
}

criterion_group!(benches, bench_run_structural);
criterion_main!(benches);
