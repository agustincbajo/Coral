//! Benchmark for `coral_core::walk::read_pages`.
//!
//! End-to-end micro: disk I/O + YAML parse + body extraction over a
//! 100-page tempdir-backed wiki. This is the realistic shape of every
//! `coral lint` / `coral stats` cold invocation.

use coral_core::walk::read_pages;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Builds a 100-page wiki tempdir, distributed across 4 subdirs that mirror
/// the canonical Coral layout (`modules/`, `concepts/`, `entities/`, `flows/`).
/// 25 pages per subdir → 100 pages total.
fn build_fixture() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    let subdirs = ["modules", "concepts", "entities", "flows"];
    for sub in &subdirs {
        let sd = root.join(sub);
        fs::create_dir_all(&sd).expect("mkdir");
        for i in 0..25 {
            let slug = format!("{sub}-{i}");
            write_page(&sd.join(format!("{slug}.md")), &slug);
        }
    }
    dir
}

fn write_page(path: &Path, slug: &str) {
    let content = format!(
        "---\n\
slug: {slug}\n\
type: module\n\
last_updated_commit: abcdef0\n\
confidence: 0.7\n\
status: reviewed\n\
sources:\n  - src/{slug}.rs\n\
backlinks: []\n\
---\n\n\
# {slug}\n\n\
Body for {slug} mentioning [[outbox]] and [[customer]].\n"
    );
    fs::write(path, content).expect("write");
}

fn bench_read_pages(c: &mut Criterion) {
    c.bench_function("walk/read_pages_100_pages_4_subdirs", |b| {
        b.iter_with_setup(build_fixture, |dir| {
            let pages = read_pages(black_box(dir.path())).expect("read_pages");
            black_box(pages);
            // `dir` drops here; tempdir cleanup is part of the iter cost but
            // happens after the inner work.
        });
    });
}

criterion_group!(benches, bench_read_pages);
criterion_main!(benches);
