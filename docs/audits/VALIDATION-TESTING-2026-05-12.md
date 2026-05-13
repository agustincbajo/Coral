# Validation: Testing audit — Coral v0.34.1
Date: 2026-05-12
Validator: claude (sonnet)
Target: docs/audits/AUDIT-TESTING-2026-05-12.md

## Verdict

**APPROVED_WITH_REVISIONS** — 0 hallucinations on spot-check, all 5 sampled findings VERIFIED, but the audit fails to cross-reference the parallel security audit. TEST-01 understates severity by not mentioning it covers the exact same code surface as SEC-01 / SEC-02 (the auth guards that would defend the un-authenticated MCP HTTP transport and the WebUI bearer logic).

## Spot-check results (5 findings)

| ID | Status | Notes |
|----|--------|-------|
| TEST-01 (Critical) | **VERIFIED** | `crates/coral-ui/src/auth.rs` confirmed: `validate_host:24`, `require_bearer:62`, `validate_origin:96` have **no direct tests**. Only `is_loopback` and `constant_time_eq` are covered in `#[cfg(test)] mod tests` at lines 148–167. `crates/coral-ui/tests/` directory does **not exist**. Severity is *correctly* Critical for v0.34.1; arguably should be uplifted further (see cross-audit matrix). |
| TEST-02 (Critical) | **VERIFIED** | `crates/coral-test/src/orchestrator.rs` is 293 lines exactly as cited; `grep '#\[test\]\|#\[cfg\(test\)\]'` returns **zero matches**. Shared CLI/monitor pipeline confirmed in lines 1–14 module doc. Severity Critical is appropriate — this is the silent backbone of `coral test` and `coral monitor up`. |
| TEST-03 (Critical) | **VERIFIED** | No `**/fuzz/**` or `cargo-fuzz` directories found anywhere in the workspace. Parsers cited (`frontmatter::parse`, `wikilinks::extract`, `config::Config::from_str`) confirmed as user-input surfaces. *However*, severity discipline is questionable: "no fuzzing" is a missing **future** capability, not a present regression. Compare to TEST-01/02 where untested code already exists. Recommend reclassification to **High** (see below). |
| TEST-05 (High) | **VERIFIED** | `playwright-ci.yml.disabled` exists; 5 spec files confirmed (`graph/manifest/nav/pages/query.spec.ts`). Workflow has no `workflow_dispatch` trigger uncommented (line 14). The seed-fixture proposal is accurate. |
| TEST-09 (High) | **VERIFIED** | `proptest_index.rs:59` has `slug_strategy() -> "[a-z][a-z0-9-]{2,15}"` — happy-path-biased by construction, exactly as claimed. `confidence_strategy()` at line 69 uses `n/100` so the 2-decimal round-trip is exact by construction (no negative space exercised). |

**Bonus verifications (sanity sample):** TEST-04 (4 of 9 commands sampled — `ui.rs`, `up.rs`, `verify.rs`, `wiki.rs` — all 0 `#[test]`), TEST-07 (`stdio.rs` 0 tests confirmed), TEST-08 (7 `#[cfg(unix)]` guards in `cross_runner_contract.rs` confirmed).

**Hallucinations: 0.** All cited file paths, line numbers, and LoC counts that I checked match reality.

## Cross-audit reinforcement matrix

`grep -c "SEC-0"` against `AUDIT-TESTING-2026-05-12.md` returns **0** — the testing audit does **not** cross-reference any security finding. This is the primary gap.

| Testing finding | Security finding | Combined risk uplift |
|-----------------|------------------|--------------------------|
| TEST-01 (auth.rs guards) | SEC-02 (WebUI token entropy floor) | **YES** — auth.rs is the file SEC-02 proposes to harden. Testing the guards without a regression net guarantees the SEC-02 fix will silently break under a future refactor. Combined severity confirms Critical. |
| TEST-01 (auth.rs guards) | SEC-01 (MCP HTTP no bearer) | **YES (indirect)** — TEST-01 covers `coral-ui`'s guards; SEC-01 says the same pattern is missing from `coral-mcp/transport/http_sse.rs`. Once SEC-01 is fixed by porting the constant-time-eq pattern from `auth.rs:137`, the *target* of that copy must itself be test-covered (TEST-01). This is a hard dependency the testing auditor missed. |
| TEST-07 (stdio MCP not exercised E2E) | SEC-01 (MCP HTTP unauth) | **NO direct overlap** but adjacent — stdio is the secure-by-default transport; the audit should note that the *insecure* transport (HTTP, SEC-01) is what needs urgent E2E coverage, not stdio. TEST-07 prioritisation is therefore slightly off. |
| TEST-11 (coral-ui no integration tests) | SEC-02 (token mint) | **YES** — adding `tests/api_smoke.rs` per TEST-11 is the natural place to land the auto-minted-token assertion from SEC-02. |
| TEST-06 (no mutation testing in CI) | SEC-01/02 broadly | Weak — mutation testing would have caught a missing-token regression in `auth.rs`, but only if the unit tests existed first (TEST-01). Sequencing matters; mutation is a *follow-on* lever. |

**Net cross-audit verdict:** TEST-01 should be reclassified as a **Critical+ blocker** for the v0.34.x line because it gates the SEC-02 fix from being safely landed. The auditor did not call this out.

## Gaps not caught by the audit

1. **No cross-reference to the security audit at all.** The two audits were produced concurrently; the testing audit references `audit/raw/test-quality.md` (an older multi-agent pass) but never `AUDIT-SECURITY-2026-05-12.md`. A consolidated risk register would have escalated TEST-01.
2. **`coral-mcp/transport/http_sse.rs` is the un-tested file SEC-01 calls out** — but the testing audit only flags `transport/stdio.rs` (TEST-07). The HTTP transport's `handle_post`/`handle_delete` (`http_sse.rs:309-360` per SEC-01) have no dedicated integration test for the auth path. This is a missed finding (TEST-HTTPAUTH should have been a High).
3. **`coral self-upgrade` downgrade path (SEC-04) has zero coverage check.** The testing audit doesn't audit `crates/coral-cli/src/commands/self_upgrade.rs:64-97` for test density. Per the audit appendix this whole user path is invisible.
4. **Snapshot stability under rustc upgrades** is flagged in TEST-M5 but not investigated. The auditor notes the concern but never runs `cargo test -- --include-ignored` to look for actual flakes. The appendix admits this is out of scope, which is fair, but no follow-up issue is filed.
5. **proptest dep is in `Cargo.toml` but only 8 files use it across `coral-core`.** `coral-lint`, `coral-runner`, `coral-mcp`, `coral-ui` have zero property tests despite having complex input parsers (URL builders, manifest validators, JSON-RPC envelopes). The audit notes "lint engine has a single property test" but doesn't lift this to a High finding for the other crates.

## Severity discipline observations

1. **TEST-03 (fuzz infra) → recommend downgrade to High.** Three Critical findings is a high count. TEST-01 and TEST-02 are "untested code that ships *today*." TEST-03 is "missing capability for tomorrow." It's a sustained-investment item, not a defect-of-record. Recommended Critical count: **2** (TEST-01, TEST-02).
2. **TEST-11 (coral-ui no integration tests) → consider uplift to Critical.** Cross-referenced with SEC-02 (token mint at startup), the absence of any integration test against the actual `serve()` binary means the SEC-02 fix lands blind. This is structurally Critical when combined.
3. **TEST-04 (9 zero-test CLI commands) — severity High is correct**, but `ui.rs` specifically should be split out: it's the user-facing entrypoint for everything in TEST-11 and SEC-02. A standalone TEST-04a (Critical) is defensible.

## Methodology assessment

The **tests-per-100-LoC proxy** is a reasonable triage tool given the constraint (no `cargo llvm-cov` locally; CI run is ~30 min). The audit is honest about this limitation in §Methodology item 4. However:

- The proxy can mislead in two directions: (a) one mega-test that exercises 80% of a file looks like 0.1 density but covers everything; (b) 50 tiny tests on a getter look like 5.0 density but cover one branch. The audit's qualitative read of `auth.rs` and `orchestrator.rs` (where it inspects code, not just counts) is the correct mitigation.
- The TEST-10 proposal — land `COVERAGE-BASELINE-v0.34.1.md` *before* bumping the floor — is actionable, accountable, and gates the BACKLOG #5 ratchet on real numbers. Approved.
- Per-crate "v0.37 risk" tags in the coverage map are subjective but consistent with the findings narrative.

The 3 lowest-density crates (coral-cli 1.70, coral-session 1.90, coral-ui 2.00) are credible user-path candidates. coral-ui is correctly flagged as highest risk — the REST surface plus auth guards plus zero integration tests is the worst combination in the workspace.

## Recommendation

**Land the audit as APPROVED_WITH_REVISIONS.** Required revisions before merging into the risk register:

1. **Add a "Cross-audit links" subsection** to TEST-01 and TEST-11 referencing SEC-02 (and SEC-01 indirectly). One-line each.
2. **Reclassify TEST-03 to High** unless an exploitable parser panic is demonstrated. Reserve Critical for present-state defects.
3. **Open TEST-HTTPAUTH (High)** as a missed finding: `coral-mcp/transport/http_sse.rs` has no integration test for the auth path that SEC-01 will install. The fix-and-test must land in the same PR.
4. **File a follow-up issue** for proptest expansion into `coral-lint`/`coral-runner`/`coral-mcp`/`coral-ui` (currently zero coverage despite the dep being declared).

No findings need to be withdrawn. Density methodology is acceptable given local constraints; the TEST-10 baseline lands first per the audit's own top-3 priority.
