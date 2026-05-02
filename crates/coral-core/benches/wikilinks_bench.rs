//! Benchmark for `coral_core::wikilinks::extract`.
//!
//! Hot path: per-page regex extraction. Runs N times per `lint` / `stats`
//! invocation (once per page), so even sub-millisecond regressions
//! compound across a 200-page wiki.

use coral_core::wikilinks::extract;
use criterion::{Criterion, black_box, criterion_group, criterion_main};

/// Builds a body of ~200 chars of prose interleaved with 50 unique wikilinks
/// distributed evenly. Each wikilink is `[[link-N]]` (10 bytes for N=0..9,
/// 11 bytes for N=10..50), so the body grows past 200 chars once links are
/// included — by design, since we want the regex engine to actually have to
/// scan content, not just match in tight bursts.
fn build_body() -> String {
    let prose_chunk = "lorem ipsum dolor sit amet consectetur adipiscing ";
    let mut body = String::with_capacity(2_000);
    for i in 0..50 {
        body.push_str(prose_chunk);
        body.push_str(&format!("[[link-{i}]] "));
    }
    body
}

fn bench_extract(c: &mut Criterion) {
    let body = build_body();
    c.bench_function("wikilinks/extract_50_links", |b| {
        b.iter(|| {
            let links = extract(black_box(&body));
            black_box(links);
        });
    });
}

criterion_group!(benches, bench_extract);
criterion_main!(benches);
