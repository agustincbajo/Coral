//! Benchmark for `coral_core::search::search`.
//!
//! Hot path: TF-IDF over the full page corpus on every `coral search` call.
//! Per `docs/PERF.md`, the cold target is <10 ms for the 200-page baseline.
//! This bench scales the input down to 100 pages with 5-token bodies and
//! issues a 2-token query, which is the realistic shape of a CLI invocation.

use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::page::Page;
use coral_core::search::search;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Builds a 100-page corpus with 5-token bodies. A handful of pages mention
/// the query terms so TF-IDF actually has scoring work to do; the rest are
/// filler that contributes to the IDF denominator.
fn build_corpus() -> Vec<Page> {
    let bodies: [&str; 10] = [
        "outbox dispatcher polls every second",
        "the outbox pattern guarantees delivery",
        "lorem ipsum dolor sit amet",
        "consectetur adipiscing elit sed do",
        "eiusmod tempor incididunt ut labore",
        "magna aliqua enim ad minim",
        "veniam quis nostrud exercitation ullamco",
        "laboris nisi ut aliquip ex",
        "outbox handler dispatcher fans out",
        "duis aute irure dolor reprehenderit",
    ];
    (0..100)
        .map(|i| Page {
            path: PathBuf::from(format!(".wiki/modules/p{i}.md")),
            frontmatter: Frontmatter {
                slug: format!("p{i}"),
                page_type: PageType::Module,
                last_updated_commit: "abcdef0".to_string(),
                confidence: Confidence::try_new(0.8).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Reviewed,
                generated_at: None,
                extra: BTreeMap::new(),
            },
            body: bodies[i % bodies.len()].to_string(),
        })
        .collect()
}

fn bench_search(c: &mut Criterion) {
    let pages = build_corpus();
    c.bench_function("search/100_pages_2_token_query", |b| {
        b.iter(|| {
            let results = search(black_box(&pages), black_box("outbox dispatcher"), 10);
            black_box(results);
        });
    });
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
