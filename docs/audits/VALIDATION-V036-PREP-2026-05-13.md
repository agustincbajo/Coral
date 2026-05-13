# Validation: v0.36 prep batch — 4 items

Date: 2026-05-13
Validator: claude (sonnet)
Targets: 5 commits `4229f2f..180881c`

## Verdict

**APPROVED_WITH_REVISIONS** — all four claimed deliverables verified on
the working tree; one minor counting drift in the ARCH-C1 ratio
(off-by-one) and the workspace-wide nextest summary differs from the
dev-reported scope. No fabricated artifacts, no breaking-change
miscount, no production-logic regression.

## Spot-check (5 areas)

| Aspect              | Status         | Notes                                                                                                                                                                                                                                                            |
| ------------------- | -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| ARCH-C1 ratio       | minor drift    | `crates/coral-core/src/lib.rs` head: **15 `pub mod` + 18 `pub(crate) mod` = 33 total**. Demoted = 18/33 = **54.5%** (dev claims 17/33 = 51%). Off-by-one. Lib.rs doc comment itself says "33 → ~17 mods (~48%)" which is closer to ground truth than the report. |
| ARCH-C1 re-exports  | verified       | 12 curated `pub use` lines at crate root cover `WikiIndex`, `IndexEntry`, `WalkCache`, `EmbeddingsIndex`, `SqliteEmbeddingsIndex`, `Symbol`, `SymbolKind`, `extract_from_dir`, `PageDiff`, narrative + eval + gc + wikilinks helpers, `SyncOutcome/sync_repo`.    |
| ARCH-C1 callsites   | verified       | `crates/coral-cli/src/commands/{search,ingest,bootstrap,diff,project/sync,consolidate}.rs` all consume via `use coral_core::{WalkCache, …}` crate-root path. Zero remaining hits for `coral_core::cache::WalkCache` or peer internal paths.                       |
| build.rs ≥100 KiB   | verified       | `crates/coral-ui/build.rs:48` — `const DROP_RAW_THRESHOLD_BYTES: u64 = 100 * 1024;`. Comment block 43-47 documents the three offenders (index/sigma/markdown).                                                                                                    |
| dist/ raw drop      | verified       | `crates/coral-ui/assets/dist/assets/` lists only `index.js.{br,gz}`, `sigma.js.{br,gz}`, `markdown.js.{br,gz}` plus `index.css{,.br,.gz}`. No raw `.js`. Matches the 875 KiB drop claim.                                                                          |
| Vary always-emit    | verified       | `crates/coral-ui/src/static_assets.rs:65` exposes `vary_accept_encoding: bool`; lines 113 + 158 set it on both compressed and identity-fallback branches. `server.rs:476-484` enforces emission per RFC 9110 §15.5.4.                                             |
| decompress fallback | verified       | `static_assets.rs:219 fn decompress_any_sibling` exists; consumed at line 141. Brotli-then-gzip order matches the report.                                                                                                                                        |
| mimalloc bench file | verified       | `crates/coral-core/benches/allocator.rs` — 3 `criterion_group!` entries: `workload_a_tfidf`, `workload_b_page_parse`, `workload_c_json_value`. Page-parse workload uses real `Page::from_content`; JSON workload mirrors OpenAPI proptest shape.                  |
| mimalloc results    | verified       | `docs/bench/MIMALLOC-BASELINE-2026-05-13.md` records **+29.7% / +42.4% / +42.7%** medians with `[low, high]` Criterion intervals. Numbers match commit message exactly.                                                                                           |
| ADR-0012 status     | verified       | `docs/adr/0012-mimalloc-allocator.md` — `**Status:** **accepted, baseline measured**` with cross-link to the bench file. Doc body retains the original "freeze the claim" note + the 2026-05-13 measurement appendix.                                            |
| Clippy ratchet      | verified       | `cargo clippy --workspace --lib --bins 2>&1 \| grep "^warning:" \| grep -vE "generated [0-9]+ warning" \| wc -l` → **45**, exactly the dev-claimed target.                                                                                                       |
| Strict clippy gate  | verified       | `cargo clippy --workspace --all-targets -- -D warnings -A clippy::unwrap_used -A clippy::expect_used -A clippy::panic` → EXIT=0.                                                                                                                                 |
| Workspace check     | verified       | `cargo check --workspace --all-targets` → EXIT=0.                                                                                                                                                                                                                |
| fmt                 | verified       | `cargo fmt --all -- --check` → EXIT=0.                                                                                                                                                                                                                           |
| Nextest             | scope mismatch | Workspace-wide: **1895 passed / 27 failed / 19 skipped**. The 27 failures are the documented Windows-host baseline (see commit `b1a9c16 docs(handoff): enumerate 27 Windows nextest failures by name`). Dev's "942/942 touched-crate" framing is plausible.      |

Five `#[allow]` annotations spot-checked in `coral-core` test fixtures
and `regex caps.get(N).unwrap()` sites carry inline justification
comments (form: `// safe: regex group N is mandatory in pattern`).
No "wrap-unwrap-in-helper" laundering observed.

## Breaking changes assessment

The 10 `pub mod → pub(crate) mod` demotions are **breaking under strict
Rust SemVer** because any downstream `use coral_core::cache::…` path
ceases to compile. Coral is a single-product workspace today — no
external `Cargo.toml` declares `coral-core` as a dependency — so the
real-world blast radius is zero. The curated `pub use` shim covers all
internal callsites; spot-grep in `crates/coral-cli` shows zero
remaining internal-path imports for any of the 10 demoted modules.

A `cargo public-api` diff is the canonical check; it is not in the
workspace toolchain, so the validation falls back to the
grep-based survey above. The dev's stated rationale ("path moves =
breaking") is the conservative call and the right one for a project
that publishes a CLI tag.

## Workspace state

- `cargo check --workspace --all-targets`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings -A clippy::unwrap_used -A clippy::expect_used -A clippy::panic`: clean.
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --lib --bins` warnings: **45** (target hit).
- `cargo nextest run --workspace --no-fail-fast`: 1895 passed, 27
  pre-existing Windows failures (documented baseline), 19 skipped.
- Workspace version still `0.35.0` across the eleven crates — version
  bump to `0.36.0` is a separate commit that has not landed yet.

## Tag recommendation review

- Path-move demotions trigger a minor bump under Coral's stricter
  pre-1.0 SemVer interpretation (the workspace honors the public-API
  contract beyond what Cargo itself requires for `0.x`). **Minor bump
  required.**
- mimalloc bench: pure measurement, no functional delta. Doesn't move
  the version.
- Clippy ratchet (104 → 45): allow-annotations + safe `.unwrap()`
  removals. Doesn't move the version.
- build.rs hardening: build-time asset filtering + an always-on `Vary`
  header. User-visible effect is a smaller binary and correct cache
  behavior — non-breaking improvement. Doesn't move the version on its
  own.

Combined, the ARCH-C1 demotions are the sole driver. **v0.36.0 (minor
bump) is the correct SemVer call.** A v0.35.1 patch tag would
misrepresent the public-surface contraction.

The 30-43% mimalloc gains are plausible on Windows MSVC where the
system allocator is `HeapAlloc` — mimalloc's small-object segregation
and thread-local heaps routinely beat it by 25-50% on the kind of
many-small-Vec/String workloads represented in the three benches.
Criterion's `p=0.00` significance flag on every workload is consistent
with the magnitude. The Linux-rerun follow-up is correctly flagged in
the bench doc; on glibc the spread is typically narrower.

## Recommendation

Accept the batch as the substrate for the v0.36.0 tag. Before tagging:

1. Reconcile the **18-demoted-of-33** ground truth into the BACKLOG
   line and the v0.36 release notes (commit message says "10 demoted in
   this batch + Phase C 6" = 16, doc comment in lib.rs says "~17",
   actual tree shows 18). One-line text fix; no code change.
2. Land the workspace-wide version bump `0.35.0 → 0.36.0` as its own
   `chore(release)` commit on top of `180881c`.
3. Consider gating the v0.36.1 patch on either the Linux mimalloc
   rerun or a `cargo public-api` baseline snapshot, both flagged as
   follow-ups in the bench doc and the ADR.

No revisions required to the four delivered artifacts themselves —
they verify clean against the working tree.
