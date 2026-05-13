# Audit: Security — Coral v0.34.1
Date: 2026-05-12
Auditor: claude (sonnet)
Scope: workspace crates (`coral-cli`, `coral-core`, `coral-mcp`, `coral-runner`, `coral-ui`, `coral-session`, `coral-test`, `coral-env`, `coral-lint`, `coral-stats`); `scripts/install.sh`, `scripts/install.ps1`; `.claude-plugin/scripts/on-session-start.*`; `deny.toml` / `Cargo.lock`; `.github/workflows/ci.yml` security jobs.

## Executive summary

Coral's threat model is **a single-user, local-development tool that handles third-party API keys, drives subprocess LLMs, and exposes optional HTTP surfaces (WebUI, MCP)**. The trust boundary is the user's account: Coral assumes the local user is benign and the local machine is not multi-tenant by default. Off-loopback exposure is gated but uneven.

Top risks at a glance:

1. **MCP HTTP transport has no authentication.** Only origin allowlist + 127.0.0.1 default. A user who follows the documented `--bind 0.0.0.0` warning still ends up with an unauthenticated tool-execution endpoint reachable from the LAN.
2. **WebUI bearer token is operator-supplied with zero entropy enforcement.** No minimum length, no auto-generation, no rotation. Users may pick weak or reused tokens.
3. **`coral self-upgrade` permits cross-version downgrade within the same major.** A v0.34.1 user can be socialed into running `--version v0.34.0` to roll back to a less-patched build; SHA-256 verification covers integrity but not freshness.
4. **`.coral/config.toml` permission hardening is Unix-only**; on Windows the API key file inherits user-profile ACLs (acknowledged in code, undocumented to end-users).
5. **Backup files (`.claude/settings.json.coral-backup-*`) accumulate** with no cleanup policy, and persist any pre-existing token/credential the user had in settings.json.

Most urgent actions: add a `--token`/`CORAL_MCP_TOKEN` to MCP HTTP, mint WebUI tokens server-side when omitted, and document the Windows API-key threat in `coral doctor` output.

## Findings (Critical + High only)

| ID | Severity | Title | File:Line | Proposed fix |
|----|----------|-------|-----------|--------------|
| SEC-01 | High | MCP HTTP transport accepts unauthenticated POST /mcp (tool execution) | `crates/coral-mcp/src/transport/http_sse.rs:309-360` | Require a bearer token (mint or accept via `--token` / `CORAL_MCP_TOKEN`) for non-loopback binds; constant-time compare like `coral-ui/src/auth.rs:137`. |
| SEC-02 | High | WebUI accepts user-supplied tokens with no entropy floor or auto-generation | `crates/coral-cli/src/commands/ui.rs:60-92`; `crates/coral-ui/src/server.rs:33-41` | If `--token` absent on loopback, mint a 32-byte URL-safe token via `rand` + print once on stderr; reject `--token` strings < 16 bytes. |
| SEC-03 | High | API-key file `.coral/config.toml` not permission-hardened on Windows | `crates/coral-core/src/config.rs:438-445, 481-493` | Add `#[cfg(windows)]` branch that calls `SetNamedSecurityInfoW` to strip group/everyone ACEs (or surface a one-time warning the wizard prints when running on Windows). |
| SEC-04 | High | `coral self-upgrade --version vX.Y.Z` permits downgrade with no recency check | `crates/coral-cli/src/commands/self_upgrade.rs:64-97, 139-151` | Refuse target < current unless `--allow-downgrade` is passed; print a clear "downgrading from N to M" prompt under `dialoguer::Confirm`. |
| SEC-05 | High | `.claude/settings.json.coral-backup-*` files accumulate indefinitely, may hold pre-existing secrets | `crates/coral-cli/src/commands/self_register_marketplace.rs:205-212, 329-343` | After atomic write, rotate backups (keep most recent 3) and chmod 600 on Unix for the freshly-created backup; on Windows inherit ACLs. |
| SEC-06 | High | Bootstrap LLM output is not fenced/validated; LLM-generated markdown is parsed by frontmatter without injection check | `crates/coral-cli/src/commands/bootstrap/mod.rs:304-342` (vs `crates/coral-cli/src/commands/common/untrusted_fence.rs:65-78`) | Run `coral_lint::structural::check_injection` over each LLM-emitted body before `build_page` accepts it, and reject pages with `[suspicious-content-detected]` markers. |
| SEC-07 | High | MCP HTTP `Mcp-Session-Id` is not cryptographically random; predictable via timestamp + counter | `crates/coral-mcp/src/transport/http_sse.rs:820-840` | Switch to `getrandom::fill(&mut bytes)` (`getrandom` is already in the tree via `rand`). Doc even flags this. |

## Methodology

Static review only — the audit was read-only per scope. Tools:

- Repo navigation via `Glob` + `Grep` (ripgrep).
- Cross-crate symbol search for `Command::new`, `process::Command`, `tracing!`, `eprintln!`, `chmod`, `set_permissions`, `api_key`, `token`, `bearer`, `authorization`, `canonicalize`, `flock`, `backup`, `coral-backup`, `is_loopback`, `is_origin_allowed`.
- `cargo audit` / `cargo deny` were NOT run live — instead, the audit cross-references `.github/workflows/ci.yml:262-291` (the CI security jobs) and the `deny.toml` advisory ignore list. The CI surface is the source of truth.
- Dependency vulnerability triage: read `Cargo.lock` for versions of `ring`, `rustls`, `ureq`, `tiny_http`, `bincode`, `chrono`. All are current as of 2026-05-12. The single advisory in scope is **RUSTSEC-2025-0141** (bincode 1.x/2.x unmaintained), tracked in `deny.toml:22-33` with detailed rationale; this is informational, not exploitable.
- Threat modelling: STRIDE walk over six external surfaces (CLI subprocess, MCP stdio, MCP HTTP, WebUI HTTP, install.sh, install.ps1, SessionStart hook). Findings tagged below.

## Scope NOT audited

- `cargo audit` and `cargo deny` were not executed live. CI runs them per push (`ci.yml:261-291`); the audit takes their results as authoritative. If the orchestrator wants ground-truth, request a CI run snapshot.
- **`coral test`** runners (`crates/coral-test/`) — testing scaffold, not exposed to network/secrets.
- **`coral wiki serve`** legacy server — superseded by `coral ui serve` and slated for removal; intentionally skipped per BACKLOG.
- **`coral.toml` exclusions for adversarial files** — feature is **not implemented** in v0.34.1 (no `exclude` field in bootstrap; confirmed via grep of `crates/coral-cli/src/commands/bootstrap/`). Cannot audit a feature that doesn't exist.
- **Embedded JS in `crates/coral-ui/assets/dist/`** — minified Vite bundle, treated as upstream artifact. Source-side review would need the SPA source tree (not in the workspace).
- **Plugin marketplace JSON schemas** — schema validity is a correctness concern, not a security one for this audit; the patch path is covered in SEC-05.
- **macOS xattr removal and Gatekeeper interaction** (`install.sh:275-277`) — defers to OS-level trust; not Coral's contract.
- **Signal-handling races** in the long-lived servers — operational concern, not in a security-finding bucket.
- **Cryptographic primitives** in `coral-core` (BM25 SHA-256 cache key, embeddings) — these are not used for security decisions.

## Top-3 next actions

1. **MCP HTTP auth (SEC-01, SEC-07).** Add a `--token` flag to `coral mcp serve` mirroring `coral ui serve`'s shape, persist into `AppState`, enforce in `handle_post` and `handle_delete` at `crates/coral-mcp/src/transport/http_sse.rs:338, 352`. Generate `Mcp-Session-Id` via `getrandom` (it's already a transitive dep). This is one PR.
2. **WebUI token entropy + auto-mint (SEC-02).** When the user runs `coral ui serve` on loopback with no `--token`, mint a `[A-Za-z0-9_-]{43}` token (32 bytes base64url), print it on stderr once, and store it in `state.token`. Off-loopback already errors out; the loopback default should still default-secure as the WebUI grows. Edit `crates/coral-cli/src/commands/ui.rs:60-72`.
3. **Windows API-key ACLs (SEC-03).** Either (a) wire `windows-sys`'s `SetNamedSecurityInfoW` into `set_perm_600_unix`'s sibling `set_perm_owner_only_windows`, or (b) at minimum print a warning from `coral doctor --wizard` on first key write on Windows describing the `icacls` invocation users should run. The first is correct; the second is the fast mitigation.

## Appendix: Medium + Low findings

| ID | Severity | Title | File:Line | Proposed fix |
|----|----------|-------|-----------|--------------|
| SEC-M01 | Medium | Gemini wizard ping puts `api_key` in URL query string | `crates/coral-cli/src/commands/doctor.rs:340-343` | Documented as Google API requirement; mitigate by ensuring no `tracing!` ever logs the request URL. Verify `ureq` 2.12.1's default tracing config doesn't log full URLs. |
| SEC-M02 | Medium | `scrub_secrets` regex misses Gemini-style `?key=AIza...` URL parameters | `crates/coral-runner/src/runner.rs:154-170` | Extend regex: `(?:\?|&)key=[A-Za-z0-9_-]+`. Same scrubber, additional pattern. |
| SEC-M03 | Medium | `self_register_marketplace` backup file created BEFORE the flock, between `canonicalize` and `copy` | `crates/coral-cli/src/commands/self_register_marketplace.rs:140-212` | Move backup INSIDE the `with_exclusive_lock` block, after re-canonicalizing under the lock. Closes a small TOCTOU window. |
| SEC-M04 | Medium | Windows binary is not code-signed; SmartScreen warning shipped to end-users | `scripts/install.ps1:208-211` | Tracked for v0.35 per script comment. No fix in scope for this audit. |
| SEC-M05 | Medium | `install.sh` resolves "latest" tag via unauthenticated GitHub API (no SLSA provenance check at download time) | `scripts/install.sh:112-122` | SHA-256 in sidecar is verified, but origin trust = GitHub releases CDN. Consider `cosign verify-blob` against `docs/SLSA-VERIFICATION.md`'s flow; or print provenance URL in the success message. |
| SEC-M06 | Medium | `coral self-upgrade` uses GitHub API unauthenticated → 60 req/h rate limit shared across CI/IP | `crates/coral-cli/src/commands/self_upgrade.rs:43, 57-59` | Already documented in `--check-only` help; surface `GITHUB_TOKEN` honour in the doc string for `self-upgrade` itself, not just `--check-only`. |
| SEC-L01 | Low | `looks_like_jsonc` is char-iterator scan with no string-literal-aware single-line comment detection beyond `"` | `crates/coral-cli/src/commands/self_register_marketplace.rs:285-316` | Working as intended (fail-closed). False positives produce a clear error. No change needed; document the false-positive rate. |
| SEC-L02 | Low | `is_loopback` admits the literal string `"localhost"` without DNS resolution check | `crates/coral-ui/src/auth.rs:14-16` | Standard pattern; DNS-rebinding is mitigated by `Host` header validation downstream (`auth.rs:24-50`). No change. |
| SEC-L03 | Low | `Default::default()` body in error paths leaks `path.display()` for `.coral/config.toml` (operational info disclosure) | `crates/coral-core/src/config.rs:346-358` | Cosmetic; the path is non-secret. Skip. |
| SEC-L04 | Low | SessionStart hook honours `CLAUDE_PROJECT_DIR` env var (set by Claude Code) without validation | `.claude-plugin/scripts/on-session-start.sh:31` | Claude Code is the trust authority for the var; out of Coral's contract. Worth a brief CLAUDE.md note. |
| SEC-L05 | Low | `mimalloc` global allocator used for cli binary — supply-chain surface adds a build-script C dep | `Cargo.toml:81-86` | Justified per PRD; documented. No fix. |
| SEC-L06 | Low | MCP HTTP server has no per-IP rate limit; only `MAX_CONCURRENT_HANDLERS = 32` | `crates/coral-mcp/src/transport/http_sse.rs:53` | Acceptable on loopback; relevant only if SEC-01 is fixed and the server is exposed off-loopback intentionally. |
| SEC-L07 | Low | `tar` invoked via `Command::new("tar")` during self-upgrade — relies on system `tar` being non-malicious | `crates/coral-cli/src/commands/self_upgrade.rs:337-345` | If `$PATH` has a hostile `tar` shim, the attacker can already win via simpler vectors (`coral` itself). Trust boundary acknowledged. |
| SEC-L08 | Low | `body_tempfile` uses `mode 0600` on Unix; on Windows relies on tempdir ACL inheritance | `crates/coral-runner/src/body_tempfile.rs:65-67, 113-125` | Tempdir is `%TEMP%` (per-user). Acceptable. |
| SEC-L09 | Low | `coral ui serve` does not emit a `Content-Security-Policy` header on the embedded SPA | `crates/coral-ui/src/server.rs:281-291` | Defense-in-depth for XSS; not exploitable today because the SPA is the only client and is built from a frozen Vite bundle. |
| SEC-L10 | Low | `RegisterMarketplaceArgs::scope=user` resolution does not validate `$HOME` is a directory | `crates/coral-cli/src/commands/self_register_marketplace.rs:93-118` | Edge case: `HOME=/etc/passwd` would error at `create_dir_all`; benign. |
