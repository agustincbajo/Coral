//! v0.36 ADR-0012 baseline benchmark — mimalloc vs. system allocator.
//!
//! ADR-0012 froze `mimalloc` as the global allocator on `coral` in
//! v0.24.0 with a doc-comment claiming "10-20% throughput improvement
//! on hot paths." The claim was never measured; the v0.35 audit
//! flagged it as a deliverable, and this bench closes the loop.
//!
//! ## Toggle
//!
//! Default build wires `#[global_allocator]` to `mimalloc::MiMalloc`,
//! the same allocator the production binary uses. Pass
//! `--features system_alloc` to swap in the system allocator
//! (`std::alloc::System` — glibc / msvcrt / macOS libsystem_malloc).
//!
//! ```text
//! cargo bench --bench allocator                       # mimalloc
//! cargo bench --bench allocator --features system_alloc  # system
//! ```
//!
//! ## Workloads
//!
//! Three representative shapes drawn from the production hot paths
//! that mimalloc was supposed to help:
//!
//! - **TF-IDF scoring** (`coral search --algorithm tfidf`) — many
//!   small allocations for token vectors + the score sort. Per
//!   `docs/PERF.md` this is the canonical "hot allocator path."
//! - **Wiki page parse** (`coral lint`, `coral ingest`, every
//!   command that walks `.wiki/`) — frontmatter YAML deserialisation
//!   produces a wide allocation distribution from short strings to
//!   the body String. Most production runs see this 100s of times.
//! - **OpenAPI proptest generation** (`coral test --kind
//!   property-based`) — `serde_json::Value` tree construction is
//!   small-allocation dominated; the proptest strategies generate
//!   many ephemeral arenas.
//!
//! The bench keeps each workload deterministic (seeded inputs, fixed
//! corpus sizes) so result variance is allocator-attributable.

#[cfg(not(feature = "system_alloc"))]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "system_alloc")]
#[global_allocator]
static ALLOC: std::alloc::System = std::alloc::System;

use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
use coral_core::page::Page;
use coral_core::search::search;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Build a 100-page wiki fixture with 50-word bodies. Synthetic but
/// shaped like real ingested pages: varied vocabulary so TF-IDF has
/// scoring work, 10% of pages mention the query terms.
fn build_wiki_corpus() -> Vec<Page> {
    let lorem: &[&str] = &[
        "outbox",
        "dispatcher",
        "fans",
        "out",
        "every",
        "second",
        "pattern",
        "guarantees",
        "exactly",
        "once",
        "delivery",
        "consumer",
        "lorem",
        "ipsum",
        "dolor",
        "sit",
        "amet",
        "consectetur",
        "adipiscing",
        "elit",
        "sed",
        "do",
        "eiusmod",
        "tempor",
        "incididunt",
        "ut",
        "labore",
        "magna",
        "aliqua",
        "enim",
        "minim",
        "veniam",
        "quis",
        "nostrud",
        "exercitation",
        "ullamco",
        "laboris",
        "nisi",
        "aliquip",
        "ex",
        "ea",
        "commodo",
        "consequat",
        "duis",
        "aute",
        "irure",
        "reprehenderit",
        "voluptate",
        "velit",
        "esse",
    ];
    (0..100)
        .map(|i| {
            let body: String = (0..50)
                .map(|w| lorem[(i * 50 + w) % lorem.len()])
                .collect::<Vec<_>>()
                .join(" ");
            Page {
                path: PathBuf::from(format!(".wiki/modules/p{i}.md")),
                frontmatter: Frontmatter {
                    slug: format!("page-{i}"),
                    page_type: PageType::Module,
                    last_updated_commit: "abcdef0".to_string(),
                    confidence: Confidence::try_new(0.85).unwrap(),
                    sources: vec![],
                    backlinks: vec![],
                    status: Status::Reviewed,
                    generated_at: None,
                    valid_from: None,
                    valid_to: None,
                    superseded_by: None,
                    extra: BTreeMap::new(),
                },
                body,
            }
        })
        .collect()
}

// ─── Workload A: TF-IDF scoring ─────────────────────────────────────

fn workload_a_tfidf(c: &mut Criterion) {
    let pages = build_wiki_corpus();
    c.bench_function("allocator/A_tfidf_100p_2tok", |b| {
        b.iter(|| {
            // Realistic query shape: 2 tokens, both present in some
            // pages so the scorer's vector ops fire.
            let results = search(black_box(&pages), black_box("outbox dispatcher"), 10);
            black_box(results);
        });
    });
}

// ─── Workload B: Wiki page parsing ──────────────────────────────────

/// Build 50 stringly-formatted wiki page contents (frontmatter + body)
/// for the parse loop. Each one round-trips through serde_yaml_ng so
/// the parser's allocation distribution is exercised.
fn build_page_strings() -> Vec<String> {
    (0..50)
        .map(|i| {
            format!(
                r#"---
slug: page-{i}
type: module
last_updated_commit: abcdef0
confidence: 0.85
sources: []
backlinks: []
status: reviewed
---

# Page {i}

Body of page {i}. Some words: outbox dispatcher pattern guarantees
exactly-once delivery to a downstream consumer process. Lorem ipsum
dolor sit amet consectetur adipiscing elit sed do eiusmod tempor.
"#,
            )
        })
        .collect()
}

fn workload_b_page_parse(c: &mut Criterion) {
    let strings = build_page_strings();
    c.bench_function("allocator/B_page_parse_50_docs", |b| {
        b.iter(|| {
            for (i, s) in strings.iter().enumerate() {
                let path = PathBuf::from(format!("p{i}.md"));
                let p = Page::from_content(black_box(s), black_box(path)).unwrap();
                black_box(p);
            }
        });
    });
}

// ─── Workload C: serde_json::Value tree construction ────────────────

/// The OpenAPI property-test path drives `serde_json::Value`
/// generation via proptest strategies. We approximate the allocation
/// shape (lots of small Vec/String inserts into Map/Array) without
/// pulling proptest at bench time — the heap signature is the same.
fn workload_c_json_value(c: &mut Criterion) {
    let n_routes = 10;
    let n_props_per_route = 5;
    c.bench_function("allocator/C_json_value_10route_5prop", |b| {
        b.iter(|| {
            let mut root = serde_json::Map::new();
            for r in 0..n_routes {
                let mut route = serde_json::Map::new();
                for p in 0..n_props_per_route {
                    route.insert(
                        format!("prop_{p}"),
                        serde_json::json!({
                            "type": "object",
                            "required": ["id", "name"],
                            "properties": {
                                "id": {"type": "string", "format": "uuid"},
                                "name": {"type": "string", "maxLength": 64},
                                "tags": {"type": "array", "items": {"type": "string"}},
                                "nested": {"type": "object", "additionalProperties": false},
                            }
                        }),
                    );
                }
                root.insert(format!("/route_{r}"), serde_json::Value::Object(route));
            }
            let v = serde_json::Value::Object(root);
            black_box(v);
        });
    });
}

criterion_group!(
    benches,
    workload_a_tfidf,
    workload_b_page_parse,
    workload_c_json_value
);
criterion_main!(benches);
