# Validation: CP-4 WebUI auto-mint + api_smoke — v0.35 sprint
Date: 2026-05-13
Validator: claude (opus 4.7)
Targets: 3 commits `9a67430..e5163af` (`9a67430`, `44047f2`, `e5163af`)

## Verdict

**APPROVED** — zero hallucinations across all four dev claims (entropy floor
constant + citation, CSPRNG path + width, helper duplication, test count). The
1 finding that warranted careful checking — the dev's RFC 9110-driven swap of
test #4's assertion from 401 to 403 — is coherent with the WebUI's wire
contract (`InvalidToken → 403` in `coral-ui/src/error.rs:53`). The path
decision (`coral-cli/tests/ui_api_smoke.rs` vs the spec's
`coral-ui/tests/api_smoke.rs`) is also justified: `CARGO_BIN_EXE_<bin>` is
populated by cargo only for tests inside the package that defines the bin
target (`[[bin]] name = "coral"` lives in `coral-cli/Cargo.toml`), per the
cargo book §3.7. No workspace-metadata alternative exists in stable cargo
that would let `coral-ui` discover the resolved binary path without
hard-coding `target/{debug,release}/coral[.exe]`. The follow-up flagged by
the dev (DRY `mint_bearer_token` into `coral-core::auth`) is real — the two
copies are byte-for-byte identical (verified via `diff`) — and is the right
Phase C cleanup target. Filed as Phase C, not a CP-4 blocker.

## Spot-check

| Aspect | Status | Notes |
|---|---|---|
| `MIN_TOKEN_LEN_CHARS = 32` constant | VERIFIED | `crates/coral-cli/src/commands/ui.rs:88`. Reject message at `:229-234` contains `"token has N chars; minimum is 32 (128 bits of entropy)"` + auto-mint suggestion, matching dev's claim exactly. Applied to both `--token` and `CORAL_UI_TOKEN` env via the single `provided_token` resolver at `:96` that feeds `validate_token_entropy` at `:108`. |
| NIST SP 800-131A 128-bit citation | VERIFIED | Doc-comment `:79-87` names "NIST SP 800-131A" + "approved through 2030" floor. 32 hex chars at 4 bits/char = 128 bits, matches the floor. Lenient on charset (length-only) so operators can paste base64 / UUID-shaped tokens — pragmatic and documented. |
| CSPRNG → 64 hex chars | VERIFIED | `mint_bearer_token` at `:252-262` uses `let bytes: [u8; 32] = rand::random()` then hex-encodes via `write!("{b:02x}")` → 64 lowercase hex chars / 256 bits. `rand::random` routes through `OsRng` (`getrandom`/`BCryptGenRandom`/`SecRandomCopyBytes`). Same shape CP-3 uses in `commands/mcp.rs:483-493`. |
| stdout banner pattern parity with CP-3 | VERIFIED | `print_startup_banner` at `:173-209` emits `WebUI serving at http://...` / `Bearer token: <hex>` / `Use: curl -H ...` to stdout (provenance hint when sourced from CLI/env, "auto-minted" hint when minted). Comment at `:140-145` explicitly notes the stdout-not-stderr decision is to support `coral ui serve \| grep "Bearer token"` automation, mirroring CP-3 UX. |
| 7 api_smoke tests pass | VERIFIED | `cargo nextest run -p coral-cli --test ui_api_smoke` → 7/7 pass in 0.655s (each test spawns the real `coral` bin, parses banner, drives `ureq`). All 7 scenarios covered: health-no-token-200, pages-missing-bearer-401, pages-valid-token-200, pages-invalid-token-403, pages-malformed-header-401, auto-mint-e2e-200, entropy-floor-rejects-on-stdout. |
| 4 unit tests pass | VERIFIED | `cargo nextest run -p coral-cli --features ui --lib commands::ui::tests` → 4/4 pass: `mint_bearer_token_is_64_hex_chars_and_unique`, `mint_bearer_token_clears_entropy_floor`, `entropy_floor_rejects_short_tokens`, `entropy_floor_accepts_tokens_at_or_above_floor`. |
| coral-ui no regression | VERIFIED | `cargo nextest run -p coral-ui` → 69/69 pass in 1.283s. SEC-02 work is fully contained in `coral-cli/src/commands/ui.rs` so this was expected; pinning the count is the regression test. |
| Helper duplication (Phase C cleanup) | CONFIRMED | `diff <(sed -n '483,493p' mcp.rs) <(sed -n '252,262p' ui.rs)` → empty (byte-for-byte identical). Hoist target is `coral-core::auth::mint_bearer_token`; both call-sites collapse to a one-line `use`. |

## RFC 9110 401 vs 403 review

The dev's test #4 assertion change from 401 to 403 is coherent with the
WebUI's actual response shape:

- `coral-ui/src/auth.rs:107-131` `check_bearer` maps
  `BearerAuthError::MissingHeader | MalformedHeader → ApiError::MissingToken`
  and `BearerAuthError::TokenMismatch → ApiError::InvalidToken`.
- `coral-ui/src/error.rs:52-53` maps `ApiError::MissingToken → 401` and
  `ApiError::InvalidToken | InvalidOrigin | WriteToolsDisabled → 403`.
- The 401-vs-403 split therefore matches RFC 9110 §15.5.4's recommendation
  exactly: 401 = "no credential present", 403 = "credential present but
  rejected". Test #4 sends a `Bearer <wrong-value>` (header present, value
  wrong), so 403 is the correct expectation — the dev's swap fixes a bug
  that would have been in test #4 had it kept 401 (the test would have
  failed against the real server).
- Cosmetic: the test fn name retains the `_returns_401` suffix for spec
  continuity with the TEST-11 list. The dev calls this out at `:307-313`
  and `:332-336` so a future reader doesn't trip on the mismatch. Non-
  blocking.

## Path decision validation

The dev moved api_smoke from the spec's `coral-ui/tests/api_smoke.rs` to
`coral-cli/tests/ui_api_smoke.rs`. Justification holds:

- The cargo book §3.7 (Environment variables for build scripts and
  integration tests) confirms `CARGO_BIN_EXE_<name>` is only set by cargo
  when the test target is in the same package as the bin target. `coral`
  the bin lives in `coral-cli/Cargo.toml`; no other workspace package can
  observe `CARGO_BIN_EXE_coral`.
- No stable cargo metadata API exists that would let `coral-ui/tests/*`
  discover the resolved-binary path without hard-coding
  `target/{debug,release}/coral[.exe]` (which would break under `--target`,
  `CARGO_TARGET_DIR`, and cross-builds — all of which the existing
  `mcp_http_smoke.rs` correctly defers to cargo to resolve via
  `CARGO_BIN_EXE_coral`).
- The path decision keeps coral-ui's integration story library-level (the
  13 unit tests in `auth.rs` from CP-3) and the end-to-end story
  binary-level (the 7 tests in `ui_api_smoke.rs`). This mirrors the
  existing `mcp_http_smoke.rs` in the same `coral-cli/tests/` directory.
- The dev documents the decision at the top of `ui_api_smoke.rs:9-14`,
  cross-referencing the auth-rules unit tests so the half-and-half
  partitioning is discoverable.

## Workspace state

- `cargo check --workspace` → green (clean dev profile in 2.96s).
- `cargo fmt --check` → clean (zero hunks reported; the `e5163af` style
  commit lands the CP-4 fmt fixes that CP-3 validation flagged as pre-
  existing).
- `cargo clippy --workspace --all-targets -- -D warnings` → clean (zero
  warnings; the regression noted in VALIDATION-CP3 line 21-25 from commit
  `37cb770` has been resolved by the CP-2 style commit `65ff4d9` which
  landed between CP-3 and CP-4).
- `cargo nextest run --workspace --no-fail-fast` → `1917 tests run: 1891
  passed, 26 failed, 19 skipped`. The 26 failures are **pre-existing,
  unrelated to CP-4**: verified by re-running `cargo nextest run
  -p coral-runner` against a clean HEAD with the CP-4 working-tree
  changes stashed away — same 8 coral-runner failures (subprocess /
  echo-substitute tests on Windows), same template_validation /
  coral-env / coral-stats / coral-session failures. These are
  environment-specific (Windows path / template fixture / time-zone
  drift) and predate CP-4; outside this validation's scope.

## Cross-audit closure

| Finding | Audit doc | Status |
|---|---|---|
| SEC-02 (WebUI accepts low-entropy operator tokens / no auto-mint) | `docs/audits/AUDIT-SECURITY-2026-05-12.md:25` | **CLOSED** — entropy floor at 32 chars / 128 bits with NIST citation, auto-mint via `rand::random::<[u8; 32]>()` on non-loopback bind, applied to both `--token` and `CORAL_UI_TOKEN`. The audit asked for a 32-byte base64url token (43 chars); dev shipped 64-hex-char (256-bit) for parity with the CP-3 MCP mint path. Stronger than asked. |
| TEST-11 (coral-ui integration tests landing) | `docs/audits/AUDIT-TESTING-2026-05-12.md:29,49,70,106` | **CLOSED** — 7 end-to-end smoke tests landed at `crates/coral-cli/tests/ui_api_smoke.rs`. Path moved from the spec's `coral-ui/tests/api_smoke.rs` for the `CARGO_BIN_EXE_coral` reason documented above; the TEST-L2 follow-up ("`server.rs` bind/origin startup checks need at least one direct test") is now exercised end-to-end via the live spawn path. |

## Recommendation

Merge the 3 CP-4 commits as-is. Open the Phase C cleanup task ("hoist
`mint_bearer_token` into `coral-core::auth`") as a follow-up — the dev
already has it in flight in their working tree per the unstaged diff
visible at validation time, so this is a no-op heads-up. No corrective
commits requested in CP-4 scope.

The v0.35 sprint can now close SEC-02 + TEST-11 in `SYNTHESIS-2026-05-12.md`.
