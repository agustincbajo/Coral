# Validation: v0.37 prep batch (3 devs + Cat D + fmt)

Date: 2026-05-13
Validator: claude (opus 4.7, 1M context)
Targets: 16 commits `26f93f8..c594f93` (Cat D fix + Dev A 7 commits + Dev B 7 commits + fmt cleanup)

## Verdict

**APPROVED.** Every claim in the handoff reproduces against the live tree on Windows. Zero hallucinations, zero laundering. Three of Dev A's "test-only" Cat C fixes are confirmed real cross-platform production bugs (not test artifacts); Dev B's clippy ratchet hit 0 prod warnings (claim was "<20", actual 0); the bench-allocator workflow, ADR-0012 promotion, and CI hard-gate are all wired exactly as described.

## Spot-check

| Aspect                                         | Status | Notes                                                                 |
| ---------------------------------------------- | ------ | --------------------------------------------------------------------- |
| `cargo fmt --all -- --check`                   | PASS   | Silent exit 0.                                                        |
| `cargo clippy --workspace --lib --bins --no-deps -- -D warnings` | PASS | Finished in 7.99 s, 0 warnings across 10 crates.                      |
| `cargo nextest run --workspace --no-fail-fast` | PASS   | 1904 passed / 19 skipped / **0 failed** in 19.66 s.                   |
| Cat D `session_table_reap` flake fix           | PASS   | Process-relative timing, no `checked_sub`; helper `reap_expired_sessions_with_ttl` accepts injected TTL while production path still calls `SESSION_TTL = 1h`. |

The "known flake under load" carve-out the handoff allowed (`atomic_write_concurrent_readers_never_see_torn_writes`) did NOT trigger — the run was clean. Documenting it here regardless: the test was watched in the run summary and passed.

## Production bugs surfaced (3 from Dev A Cat C)

| Bug                                                                                              | Confirmed real?                                                                                                                                                                                                                                                                | Severity                                                                  |
| ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------- |
| `crates/coral-cli/src/commands/search.rs` SQLite connection drop before `remove_file` (f5871be)  | YES. Windows refuses unlink with an open handle (os error 32). POSIX silently allows. Without the explicit `std::mem::drop(index)` Windows `coral search` would fail on every stale-schema rebuild. Tests caught it because the fixture path is `tempfile`-backed.             | High on Windows (silent data-corruption on Linux/macOS — kernel-buffered handle keeps writing into an unlinked inode). |
| `crates/coral-core/src/project/manifest.rs` `render_toml` escape (1906d18)                       | YES. `coral project add --url C:\Users\…` produced literal `\` in the TOML, which the reload step parsed as an invalid `\U`-prefixed unicode escape. `toml_escape` helper applied to 17 quoted-field sites in `render_toml` + `render_tier`; coverage looks complete by grep.   | High — manifest round-trip broken on any Windows-authored path.            |
| `crates/coral-env/src/compose_yaml.rs` watch path separator normalization (bundled in 4791a66)   | YES. `PathBuf::join` produced `/work/repos/api\./src` on Windows hosts; Docker compose accepts forward slashes universally, so `.replace('\\', "/")` on the resolved string is the correct fix. The commit message focuses on coral-cli unwrap retire but the diff is real.    | Medium — Compose-mode users on Windows would have hit mixed-separator artefacts. |

None of the three is a test-only artefact. Dev A's call to escalate them from Cat B (test fixtures) to Cat C (real bugs) is correct.

## Clippy ratchet hygiene (5 `#[allow]` spot-checks)

Random sample from the post-batch tree:

1. `coral-ui/src/server.rs:502` — `expect_used`, `reason = "static ASCII Content-Type header"`. Justification real (`Header::from_bytes` only fails on non-ASCII bytes in name/value; both are byte-string literals). No semantic change.
2. `coral-ui/src/error.rs:120` — same pattern, same justification. Production logic preserved.
3. `coral-cli/src/commands/serve.rs:117` — `expect_used`, `reason = "static ASCII header name + value"`. Adjacent comment explains the reasoning chain. Clean.
4. `coral-core` Dev B commit `436cc72` — every new allow grep'd carries `reason = "..."` (`"static regex; compile validity guarded by unit tests"`, `"CARGO_MANIFEST_DIR is guaranteed by cargo build-script contract"`, `"schemars::Schema is pure Serialize, no failure modes"`, `"PATTERNS entries are static literals compiled by tests"`). Each justification is checkable.
5. `coral-cli/src/commands/sync.rs` — was an `.expect("--version required")` on a `clap` arg; replaced with `ok_or_else(|| anyhow!("--version required"))`. **Real error-path improvement**, not an allow-laundering.

Pre-existing bare `#[allow(clippy::expect_used)]` blocks (no `reason =`) in `coral-core/src/log.rs` and `coral-core/src/symbols.rs` were authored in `1693372` (the original `104 → 45` ratchet, predating this batch). Dev B chose not to retrofit them in v0.37 prep. Worth a future polish pass but not a regression — the workspace gate (`--lib --bins -D warnings`) still passes, so they have functional justification.

No production logic was downgraded into `#[allow]` to bypass the gate. The two semantic refactors (`sync.rs` and `bootstrap/mod.rs::cwd`) replaced `.expect` with proper `anyhow::Context`, which is a strict improvement.

## CI gate verification

`.github/workflows/ci.yml:75-104`:

- No `continue-on-error` on `clippy-panic-risk`.
- Step command: `cargo clippy --workspace --lib --bins --no-deps -- -D warnings`. Hard deny.
- Scope is production-only (`--lib --bins`), so tests can still legitimately use `unwrap()`/`expect()` on known-good inputs.
- Workspace `Cargo.toml` clippy lint table still has `unwrap_used = "warn"`, `expect_used = "warn"`, `panic = "warn"` (no downgrade). The strict workspace clippy job at line 68 keeps `--all-targets` allowance for tests.

The strict / panic-risk split documented in the inline comment (lines 76-94) is accurate and matches the actual job invocations.

## bench-allocator workflow shape

`.github/workflows/bench-allocator.yml`:

- Triggers: `workflow_dispatch` + `schedule: cron: '30 4 * * 1'` (weekly, Monday 04:30 UTC). Matches claim.
- Matrix: `os: [ubuntu-latest, macos-latest]`. Windows excluded (per ADR; already measured).
- `timeout-minutes: 45` per matrix row.
- Steps: cargo build mimalloc → cargo build `--features system_alloc` → bench `--save-baseline mimalloc` → bench `--baseline mimalloc` (comparison) → upload artefacts (criterion HTML + plain-text summaries).

Shape matches the handoff claim exactly.

## ADR-0012 update

`docs/adr/0012-mimalloc-allocator.md`:

- Status line: **"accepted, baselines measured cross-platform"** with reference to `docs/bench/MIMALLOC-BASELINE-2026-05-13.md`.
- Threshold rule present: line 109 cites `≥ 10%` for "keep mimalloc cross-platform"; line 139 cites `< 5%` as the Windows-only-fallback trigger.
- Linux baseline section (line 122) and macOS baseline section (line 126) both carry `_pending first workflow_dispatch run_` placeholders.

Promotion is correctly hedged — the ADR doesn't claim measurements that don't exist yet; it claims a measurement *pipeline*.

## Cat D fix sanity

`crates/coral-mcp/src/transport/http_sse.rs`:

- `reap_expired_sessions_with_ttl(sessions, ttl)` at line 1035 — new helper with injected TTL.
- `reap_expired_sessions(sessions)` at line 1024 — production wrapper, still calls `reap_expired_sessions_with_ttl(sessions, SESSION_TTL)` where `SESSION_TTL = Duration::from_secs(60 * 60)` (line 59).
- Test (line 1119-) uses 50 ms `test_ttl` + 80 ms `thread::sleep` between two `Instant::now()` inserts. No `checked_sub` anywhere. The narrative comment explicitly calls out the Cat D root cause.

Process-relative timing is correct. Production behaviour preserved.

## Recommendation

**APPROVED — ship v0.37.0.** The "Windows nextest 26 → 0" claim is now verifiable on demand and CI hard-gates a regression on the panic-risk lints. The three real bugs Dev A surfaced from Cat C alone justify a minor bump (cross-platform manifest emit, Windows search rebuild, compose watch on Windows hosts).

This is **not** a v0.36.1 patch: Dev B's clippy ratchet is a developer-visible quality improvement, the bench-allocator workflow is new infrastructure, and ADR-0012 was promoted out of "follow-up" status. Minor bump is appropriate.

Next:

1. Bump workspace version from `0.36.0` (current) → `0.37.0`. (The most recent release commit message reads "0.35.0" but the live `Cargo.toml` carries `0.36.0`; the v0.37 bump should be a clean step from 0.36.0.)
2. Tag `v0.37.0`.
3. Trigger the first manual `workflow_dispatch` of `bench-allocator.yml` to fill in the `_pending_` sections of ADR-0012.

Word count: ~970.
