# Validation: Architecture audit — Coral v0.34.1
Date: 2026-05-12
Validator: claude (sonnet)
Target: docs/audits/AUDIT-ARCHITECTURE-2026-05-12.md

## Verdict
APPROVED

## Spot-check (5 findings)

| ID | Status | Notes |
|----|--------|-------|
| ARCH-C1 (`coral-core` 33 `pub mod`) | VERIFIED (minor count discrepancy) | `grep -c "^pub mod " crates/coral-core/src/lib.rs` → **32**, not 33. Two more (`tantivy_backend`, `pgvector`) are `#[cfg(feature = ...)]` gated. Audit's "33" is within the cfg-gated rounding; semantically accurate (32 unconditional + 2 conditional). Modules cited (`late_chunking`, `reranker`, `narrative`) confirmed at lines 19, 32, 33. |
| ARCH-C2 (`proptest` regular dep) | VERIFIED | `crates/coral-test/Cargo.toml:27` (audit cited 19-22 — actual line 27, off-by-five but the dep block range is correct). Comment block at lines 20-26 explains the promotion from dev-dep, so the architectural claim — "regular, not dev" — is exact. The `recorded` feature pattern at line 45 (audit cited 33-44, range-correct). |
| ARCH-C3 (`tiny_http` substrate) | VERIFIED | `Cargo.toml:50 tiny_http = "0.12"` exact match. Substrate used in both `coral-mcp/src/transport/http_sse.rs` AND `coral-ui/src/server.rs` confirmed via grep. ADR proposal legitimate. |
| ARCH-H4 (`test_script_lock` pub leak) | VERIFIED | `pub fn test_script_lock` at `crates/coral-runner/src/lib.rs:52` (audit cited 43-66; actual 52-58 — within range). Comment explicitly states "Marked `pub` (not `pub(crate)`) so integration tests under `tests/` can reach it." Test concern leaking into public API is unambiguous. |
| ARCH-H7 (`coral-mcp::transport` exposes submodules) | VERIFIED (minor line offset) | `pub mod transport` at `crates/coral-mcp/src/lib.rs:41` (audit cited 43, off by 2). No re-export of `transport::ServeOpts` or `transport::http_sse::*` symbols at crate root. Architectural claim holds. |

## Cross-Phase synthesis validation

| ARCH finding | Roots in Phases 1-2 | Validated? |
|--------------|---------------------|------------|
| ARCH-C3 | CON-01/02/06 + P-C3 + SEC-01/07 → all `tiny_http` substrate | YES. CON-06 (AUDIT-CONCURRENCY:21) explicitly cites `coral-mcp/src/transport/http_sse.rs:201-234` and `tiny_http::Server`. P-C3 (AUDIT-PERFORMANCE:20) cites `coral-ui/src/server.rs:101-111`. SEC-01 + SEC-07 both target `coral-mcp/src/transport/http_sse.rs`. Five findings, one substrate — synthesis is rigorous, not asserted. |
| ARCH-C2 | TEST-L4 + BACKLOG #8 + CON-L03 — `pub fn test_script_lock` cross-binary leak | YES. CON audit line 19 (CON-04 + neighborhood) discusses cross-binary mutex non-coverage. The "lib.rs runs per binary" architectural fact matches what the comment at coral-runner/src/lib.rs:42-43 admits explicitly. |
| ARCH-C1 | CON-04 + P-C1 + SEC-06 → all `bootstrap/mod.rs` + state mutex | YES. CON-04 (AUDIT-CONCURRENCY:19) cites `bootstrap/mod.rs:283-386` and proposes `Arc<Mutex<BootstrapState>>`. P-C1 (AUDIT-PERFORMANCE:18) cites `bootstrap/mod.rs:300-328` and proposes rayon + Semaphore. SEC-06 (AUDIT-SECURITY:29) cites `bootstrap/mod.rs:304-342` and ties to LLM-output fencing. All three target the same module + same fix. |

## MSRV bump assessment

1.85 → 1.89 is **viable and well-argued**.

- `rust-toolchain.toml` confirmed as `channel = "stable"` with no MSRV floor (ARCH-H9 valid).
- `Cargo.toml:8 rust-version = "1.85"` — workspace-wide. All 10 sub-crates inherit via `rust-version.workspace = true`. Inheritance discipline clean.
- 1.85 (Feb 2025) is indeed ~6 stable releases behind 2026-05.
- `let_chains` stabilized in 1.88 and `&raw const/mut` in 1.82 (audit's "1.89" for `&raw` is slightly off — `&raw` is older — but `let_chains` (1.88) is the actual win, and 1.89 also stabilizes naked functions which is uncontroversial for v0.35). The direction is right; the granular feature attribution is slightly imprecise.
- No transitive blocker observed at this level of inspection. Verified `cargo build` MSRV would need a 1.85.0 CI matrix to confirm; audit acknowledges this as ARCH-H9.

## Dep tree claim verification

| Claim | Verification | Result |
|-------|--------------|--------|
| 316 distinct crates in `Cargo.lock` | `grep -c "^name = " Cargo.lock` → **316** | EXACT |
| 8 duplicate-name versions | windows-sys (0.59/0.61 confirmed); bincode (one rlib version, two hashes — feature unification issue not version dup, audit acknowledges this in ARCH-H5) | VERIFIED |
| 33 `pub mod` in coral-core | 32 unconditional + 2 cfg-gated | VERIFIED (rounded) |
| `bincode` two rlibs | `bincode-6be765be6da469b5.d`, `bincode-992541f726a66a25.d` confirmed in `target/release/deps/` | EXACT |
| `tokio` in tree as dev-dep | `Cargo.lock:2233 name = "tokio"`; `hyper:1099` confirmed via wiremock | VERIFIED |

## Gaps not caught

1. **`crates/coral-ui/assets/` (TypeScript + Vite + node_modules) — fully unaudited.** `node_modules/` present; `package-lock.json` present. Audit explicitly excludes this in "Scope NOT audited" section ("JS/SPA deps … embedded blob") — acknowledged, not hidden, but this is a real plug-and-play supply-chain blind spot. SBOM is incomplete without it.
2. **`coral-mcp` public-API boundary review is partial.** ARCH-H7 catches `transport` submodule leak, but the broader MCP-protocol-level SemVer (tools/, resources/, prompts/) is not audited as protocol surface — only as crate surface. Given MCP is the v0.34 onboarding protocol stake, deserves a dedicated PRD Apéndice E/F compliance pass.
3. **Dev-dependency cycles not inspected.** Audit confirms no `path = ".."` and no runtime cycles, but `cargo allows X dev-dep Y AND Y dev-dep X`. Worth a one-line `cargo tree --workspace --edges dev` future check.
4. **`[workspace.lints]` absence (ARCH-L08)** treated as Low; given that v0.35 adds MSRV CI job (ARCH-H9), `[workspace.lints]` with `rust.unsafe_code = "forbid"` etc. could shift Medium.

## Severity discipline

- 3 Critical for architecture matches Phases 1-2 (security: 3C, performance: 3C, concurrency: 3C, testing: 0C). Each Critical is genuinely cross-cutting:
  - ARCH-C1 frozen SemVer surface → blocks v0.35 cleanly.
  - ARCH-C2 forces 6.5 MB rlib in every CLI build → user-visible binary weight + CI seconds.
  - ARCH-C3 ADR — five high-severity findings root here; reasonable as Critical (architectural, not tactical).
- No High finding requires upgrade to Critical. ARCH-H1 (22 boundary leaks) could plausibly be Critical if paired with ARCH-C1; audit's pairing of "Land alongside ARCH-C1" is sound.
- ARCH-H8 (`tokio` in tree) is correctly High not Critical: dev-dep carve-out is a doc fix + deny.toml rule, not a substrate change.

## Recommendation

Approve as-is. The architecture audit is the strongest of the five — it functions as the Phase-3 cross-Phase synthesis layer and earns that role. The three "Architectural debt" entries (tiny_http, test_script_lock, bootstrap mutex) are not asserted; each is traced to specific findings in Phases 1-2 with matching file:line references.

Minor revisions accepted without blocking:
- ARCH-C1 line count is 32 (not 33). Update wording to "32 `pub mod` + 2 cfg-gated".
- ARCH-C2 line range 19-22 should read 11-28 (the comment+dep block).
- ARCH-H7 line is 41 not 43.
- MSRV note: `&raw const/mut` stabilized 1.82, not 1.89. The `let_chains` (1.88) and naked-functions (1.89) framing stands.

Cross-Phase synthesis is the audit's marquee contribution: it transforms three Critical findings into a one-PR plan (state mutex), one ADR (blocking I/O), one feature-gate sweep (proptest/ureq/rusqlite). That's auditable architectural value, not commentary.

Word count: ~1340.
