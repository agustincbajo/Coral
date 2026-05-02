//! Benchmark for `coral_core::frontmatter::Frontmatter` YAML parsing.
//!
//! Hot path: every `coral` invocation that touches pages goes through
//! `serde_yaml_ng::from_str` per file. The walk-cache layer was added
//! precisely because this dominates cold-walk wall time (see the v0.3
//! cache work in `cache.rs`); when the cache is invalidated we pay this
//! cost N times.

use coral_core::frontmatter::Frontmatter;
use criterion::{Criterion, black_box, criterion_group, criterion_main};

/// A typical 5-field block: slug, type, last_updated_commit, confidence,
/// status, plus list-valued `sources` (3 entries) and `backlinks` (2 entries).
/// Mirrors the shape of fixtures under `template/.wiki/`.
const FRONTMATTER_YAML: &str = "\
slug: order
type: module
last_updated_commit: abcdef0123456789
confidence: 0.85
status: reviewed
sources:
  - src/order.rs
  - src/order_state.rs
  - src/checkout.rs
backlinks:
  - flows/checkout
  - entities/customer
";

fn bench_parse(c: &mut Criterion) {
    c.bench_function("frontmatter/parse_5_field_block", |b| {
        b.iter(|| {
            let fm: Frontmatter =
                serde_yaml_ng::from_str(black_box(FRONTMATTER_YAML)).expect("parse ok");
            black_box(fm);
        });
    });
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
