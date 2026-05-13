# Validation: Security audit — Coral v0.34.1
Date: 2026-05-12
Validator: claude (sonnet)
Target: `docs/audits/AUDIT-SECURITY-2026-05-12.md`

## Verdict

**APPROVED_WITH_REVISIONS**

Zero hallucinations in the spot-checked findings, but one significant
coverage gap (LLM-generated patch application via `git apply`) and a small
editorial inconsistency in the top-3 actions warrant revisions, not a
re-audit.

## Spot-check results (5 findings)

| ID | Status | Notes |
|----|--------|-------|
| SEC-01 | VERIFIED | `http_sse.rs:309-360` checks Origin only; `handle_post` / `handle_get_sse` / `handle_delete` dispatch at L338/L350/L352 with no bearer check. Off-loopback exposure is real. Proposed fix (constant-time bearer compare mirroring `coral-ui/src/auth.rs`) is implementable today. |
| SEC-02 | VERIFIED | `ui.rs:60-71` confirms the `--token`/`CORAL_UI_TOKEN` plumb with no entropy floor and no auto-generation. Off-loopback is correctly gated, but loopback default is "no auth". Fix is one PR. |
| SEC-03 | VERIFIED | `config.rs:438-445` confirms `#[cfg(unix)]` only; `set_perm_600_unix` at L486-493 has no Windows sibling. The in-code comment ("Windows: ACLs don't have a tidy equivalent — we rely on the user's profile permissions") matches the audit's framing. |
| SEC-05 | VERIFIED | `self_register_marketplace.rs:205-212` writes the backup BEFORE the flock (the audit also flags this overlap in SEC-M03); `backup_path_for` at L332-343 produces no rotation. Findings are consistent. |
| SEC-07 | VERIFIED | `http_sse.rs:820-840`: `new_session_id` uses `chrono::Utc::now().timestamp_nanos_opt()` + atomic counter, explicitly documented as "NOT cryptographically random". `getrandom` is available transitively via `rand`. |

Counts: VERIFIED 5 / OVERSTATED 0 / UNDERSTATED 0 / HALLUCINATED 0.

## Gaps not caught by the audit

1. **`coral session distill --as-patch` shells out to `git apply` on
   LLM-generated patches** — `crates/coral-session/src/distill_patch.rs:545-576`.
   The auditor's "Scope NOT audited" list omits `coral-session` entirely,
   yet this surface invokes `git apply --directory=.wiki` on patch files
   the LLM authored, with `--apply` writing to the working tree. Defense
   is in place (slug allowlist, header validation, no `--unsafe-paths`,
   module-doc references "audit-003"), but the surface deserves at least
   a Low-severity acknowledgement so v0.35 changes don't regress the
   defenses silently. Recommend: SEC-L11 (Low, "git apply on LLM
   patches — defense documented, regression-test coverage advised").

2. **`coral test` user-defined runner runs arbitrary `exec[0]` from spec
   files** — `crates/coral-test/src/user_defined_runner.rs:428`. The audit
   excludes `coral-test` as "testing scaffold, not exposed to
   network/secrets". True — but the input is a user-supplied YAML spec
   that can run `Command::new(arbitrary_string)` against the user's
   `$PATH`. If a contributor ever wires `coral test` into CI that
   consumes third-party specs (e.g. a marketplace skill ships a spec),
   the threat model flips. Recommend: SEC-L12 (Low, "user_defined_runner
   `exec` is arbitrary command execution by design — document trust
   boundary explicitly").

3. **`coral.toml` / `.coral/config.toml` parsing has no size cap before
   `toml::from_str`** — `crates/coral-core/src/config.rs:346-351`. The
   file is user-supplied (single-user trust model holds), but a wizard
   user can be socialed into pasting a multi-MB TOML; `toml` is not
   reentrant-safe under adversarial input historically (DoS, not RCE).
   Recommend: SEC-L13 (Low/Info, "add 1 MiB cap on
   `.coral/config.toml` before parse").

The other gaps I checked are clean:
`coral skill build` is write-only (no zip-slip surface); `release.yml`
provenance (L367-403) is properly wired via
`actions/attest-build-provenance@v2`; `Cargo.lock` shows only canonical
`hyper`/`ring`/`rustls`/`windows-strings` transitives (no `openssl-sys`,
no `libgit2`/`git2`).

## Severity discipline observations

- **SEC-01 should arguably be CRITICAL**, not High. The audit's own
  language — "unauthenticated tool-execution endpoint reachable from the
  LAN" — describes RCE-equivalent exposure once `--bind 0.0.0.0` is
  used, and `coral mcp serve` is the documented integration path. The
  classifier landed on High because the dangerous bind is opt-in. That
  is defensible (Critical = exploitable in default config), but the
  executive summary's framing of this as the #1 risk and the High-only
  classification creates a small mismatch. Recommend: keep at High,
  add a sentence to the executive summary noting "Critical IFF the
  user follows the documented `--bind 0.0.0.0` warning".

- **SEC-04 (downgrade) is probably MEDIUM, not High.** The audit lists
  "social engineering" as the attack vector and SHA-256 verification as
  the integrity backstop. The only loss is replay of an older signed
  binary; data exposure does not follow. This is squarely a freshness
  concern. Recommend: down-classify SEC-04 to Medium.

- **SEC-M05 (install.sh provenance check) may belong in Highs.** The
  audit notes "origin trust = GitHub releases CDN". For a "plug-and-play
  distribution goal" project, an unsigned download path is the riskiest
  end-user surface. Recommend: leave at Medium for now, but note in the
  executive summary that v0.35 should re-evaluate after `cosign verify`
  lands.

No over-classification observed in the Low tier.

## Editorial assessment

- The executive summary's top-3 risks correctly mirror SEC-01, SEC-02,
  and SEC-03; this matches the High table.
- The "Top-3 next actions" block, however, **bundles SEC-07 into action
  #1 ("MCP HTTP auth")**. That is operationally sensible (one PR can
  fix both `http_sse.rs` files) but it visually demotes SEC-07 — a
  separate High — to a footnote. Recommend either listing SEC-07 as a
  distinct sub-bullet under action #1 or splitting it into action #4.
- All three proposed fixes are implementable (not aspirational):
  `getrandom` is already transitively available, `SetNamedSecurityInfoW`
  is in `windows-sys` per `Cargo.lock`, and the WebUI auto-mint pattern
  is a literal copy of `coral-ui/src/auth.rs:137`.
- `Mcp-Session-Id` fix is correctly described as already-available
  (`getrandom` is transitive via `rand`).

## Recommendation

1. **Revise SEC-04 to Medium** (downgrade-via-replay is a freshness
   concern, not High-severity).
2. **Add SEC-L11/L12/L13** for the three uncovered surfaces:
   `distill_patch.rs`'s `git apply`, `user_defined_runner.rs`'s
   arbitrary `exec`, and `config.rs`'s missing parse-size cap.
3. **Restructure "Top-3 next actions"** so SEC-07 is visible (either
   sub-bullet under #1 or a standalone #4).
4. **Add a sentence to the executive summary** clarifying that SEC-01
   is High-in-default-config / Critical-on-`--bind 0.0.0.0`.
5. **No follow-up audit pass required** — the spot-check found zero
   hallucinations and the file:line citations were exact in all five
   cases checked. The gaps above are addressable as edits to the
   existing document.
