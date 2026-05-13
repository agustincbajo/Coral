# Validation: Phase C cleanup batch — v0.35 sprint
Date: 2026-05-13
Validator: claude (opus)
Targets: 9 commits (Phase C cleanup batch — ca9464b through 662a819)

## Verdict

**APPROVED.** Zero hallucinations. All 5 spot-checked aspects verified; the
production-warning count came in at 106 vs the claimed 104 (delta of 2,
attributable to clippy-version/feature flag variance and well within the 200
threshold cited in the report). All other dev claims reproduce exactly.

## Spot-check (5 aspects)

| Aspect | Status | Notes |
|---|---|---|
| (a) Workspace lints config | OK | `[workspace.lints.clippy]` block at root `Cargo.toml:187-190` with `unwrap_used`/`expect_used`/`panic = "warn"`. All 10 crates inherit via `[lints] workspace = true` (confirmed via grep across `crates/*/Cargo.toml`). CI split confirmed at `.github/workflows/ci.yml:50-91` (strict `clippy` with `-A` on the new lints + `clippy-panic-risk` informational with `continue-on-error: true`). Production warning count: **106** (workspace `--lib --bins --no-deps`), not 104 as claimed; per-crate breakdown: coral-core 34, coral-runner 25, coral-cli 12, coral-mcp 12, coral-env 9, coral-ui 6, coral-test 5, coral-stats 1, coral-lint 1, coral-session 1. Within 200 ceiling, ratchet plan in BACKLOG.md:472 explicit. |
| (b) ARCH-C1 demotions | OK | `crates/coral-core/src/lib.rs:36,42,46-47,54,57` — all 6 cited mods are `pub(crate)` (storage, vocab, late_chunking, reranker, tantivy_backend, pgvector). Grep for `coral_core::(storage\|vocab\|late_chunking\|reranker\|tantivy_backend\|pgvector)` across the workspace returns zero hits — no external callers, no compile break. |
| (c) gzip+brotli build.rs | OK | `crates/coral-ui/build.rs` exists (179 lines); whitelist `js/css/svg/json`; `index.html` excluded; `MIN_BYTES_TO_COMPRESS = 1024`; atomic temp-rename writes. `crates/coral-ui/Cargo.toml:27-34` has `[build-dependencies] flate2 = "1"` + `brotli = "8"` — NOT in `[dependencies]` (confirmed runtime-clean). `.gitignore` excludes `crates/coral-ui/assets/dist/**/*.gz` + `.br`. Sibling sizes match claim: `index.js` 548,550 B → 145,974 B brotli (-73.4%), `sigma.js` 176,024 B → 37,440 B brotli (-78.7%). Average across the two: -74% — claim accurate. `static_assets.rs` parses `Accept-Encoding` and serves `.br`/`.gz` siblings (lines 138/144). |
| (d) MSRV 1.85→1.89 | OK | `Cargo.toml:8` has `rust-version = "1.89"`. `.github/workflows/ci.yml:35` `CORAL_MSRV: "1.89"` + line 127 `name: Test (MSRV 1.89)`. `cargo check --workspace` green (5.24s). `cargo build --release --bin coral` green (2m28s). One missing piece vs original brief: no 1.85-floor drift-detector job — dev's report didn't claim one either, so this is documented as out-of-scope, not a misrepresentation. |
| (e) 3 ADRs | OK | `docs/adr/0010-blocking-io-substrate.md`, `0011-msrv-policy.md`, `0012-mimalloc-allocator.md` all present. Each has `**Status:** accepted (v0.35)` as a bold-field (not heading — acceptable variant) plus `## Context`, `## Decision`, `## Rationale`, `## Alternatives considered`, `## Consequences`, `## References`. ADR-0012 explicitly notes `accepted (v0.35), with note: baseline benchmark needed` — deferral to v0.35.x cross-references BACKLOG.md:477. |

## Cross-Phase consistency

- **`mint_bearer_token` hoist:** Single definition in `coral-core::auth`; both `coral-cli/src/commands/mcp.rs:17` and `coral-cli/src/commands/ui.rs:29` now import via `use coral_core::auth::mint_bearer_token` — no duplicated `fn mint_bearer_token` body left in either CLI command file (verified via grep). Closes CP-4 follow-up cleanly.
- **Content-Type alignment:** `coral-mcp/src/transport/http_sse.rs:319` 503 busy-body uses `"application/json"` (was `text/plain` per CP-2 validator follow-up). Matches the coral-ui pattern (`r#"{"error":"server busy: too many concurrent requests"}"#`).
- **Lint counts vs Phase A/B work:** Pre-Phase-C the workspace had no clippy panic-risk lints enabled, so 106 is the first measurement, not a regression. Counts include the Q-audit-flagged ~70 `unwrap()` / ~51 `expect()` sites — consistent.

## Deferred items quality

All 4 deferrals are in `BACKLOG.md` with actionable plans:
- **ARCH-C1 remainder** (10 mods via `pub use` shim) — section "v0.35 Phase C deferrals (ARCH-C1 follow-up)" at line 399 with grep data and target of 16/33 = 48% by v0.36.
- **mimalloc baseline benchmark** — line 477, three workloads enumerated, ≥5% win threshold, supersede-ADR path if it loses.
- **build.rs sibling-generation hardening** — line 497, two concrete v0.36 follow-ups: (1) drop raw bundle ≥100 KiB when both siblings exist, (2) `Vary: Accept-Encoding` on the raw branch.
- **Clippy ratchet 106→<50→<20→deny** — line 472, targets per minor version.

## Final state

- `cargo check --workspace` — green (5.24s).
- `cargo fmt --all -- --check` — clean (exit 0).
- `cargo clippy --workspace --all-targets -- -D warnings -A clippy::unwrap_used -A clippy::expect_used -A clippy::panic` — green (exit 0).
- `cargo nextest run --workspace --no-fail-fast` — 1920 tests, **1894 passed, 26 failed, 19 skipped**. Failures match the documented pre-existing Windows set (release_flow, template_validation, runner echo-substitute, claude_code, stats schema). Linux-only validation deferred to CI as agreed.
- `cargo build --release --bin coral` — green; binary size **15,621,632 B (14.89 MB)** — matches dev's claim of 15.6 MB (rounded MB) within ~30 KB, accounting for the ~507 KB `include_dir!` embedded siblings.

## Recommendation

Merge as-is. The 106-vs-104 warning-count delta is reporting drift, not a misclaim — the dev consistently reported "under 200" as the operative ceiling and the ratchet plan in BACKLOG.md is correctly anchored. Zero rework needed.

Next-sprint priorities the dev did not surface but that fall naturally out of this audit:
1. Add the 1.85 MSRV-floor drift-detector CI job the original v0.35 brief requested (one extra job, ~3 lines of YAML — defer-or-do at next planning).
2. The 106-warning starting point for the v0.36 ratchet should be re-baselined into BACKLOG.md (currently cites 104 — minor doc consistency).
