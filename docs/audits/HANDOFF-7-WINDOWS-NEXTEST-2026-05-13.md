# Handoff #7 — Windows nextest enumeration (v0.36 prep state)

Date: 2026-05-13
Source: `cargo nextest run --workspace --no-fail-fast` on Windows host, post v0.36 prep commit `180881c`.
Total: 1895 passed / **27 failed** / 19 skipped (1922 run, 22.9s).

Audit Testing TEST-08 asked for concrete test names to drive the
"26 pre-existing Windows failures" triage. This document enumerates
all 27 failures (one likely new) by name and groups them by
root-cause category.

---

## Category A — Bash-script tests (10 failures)

Test bodies shell out to `bash` and consume stdout/exit-code. Under
Windows nextest these run via Git Bash but fail on path separator
semantics, `/usr/bin/awk` calls, or shell-built-in differences.

**`coral-cli::release_flow`** (all 9):
- `extract_changelog_section_returns_v0_21_4_block`
- `extract_changelog_section_missing_version_exits_1`
- `extract_changelog_section_skips_fenced_pseudo_headings`
- `release_gh_sh_dry_run_extracts_correct_section`
- `release_sh_preflight_caches_ci_locally_across_per_package_calls`
- `release_sh_preflight_fails_when_changelog_section_absent`
- `release_sh_preflight_fails_when_ci_locally_fails`
- `release_sh_bump_dry_run_no_changes`
- `release_sh_tag_rejects_wrong_head_subject`
- `release_sh_rewrites_footer_using_origin_owner_repo`

**Triage**: these exercise the actual `scripts/release.sh` + `release-gh.sh`
which we don't ship to Windows users; the bash scripts are Linux-only
CI helpers. Mark all 10 as `#[cfg(unix)]` to keep them out of the
Windows run, OR add a `cfg!(target_family = "unix")` early-return in
each test body.

**Effort**: 30 min mechanical. **Priority**: Low (CI runs them on Linux).

---

## Category B — Unix `/bin/echo` substitute (8 failures)

Test bodies set `RUNNER_BIN_OVERRIDE = "/bin/echo"` or invoke external
processes assuming Unix `echo` behavior. Windows lacks `/bin/echo`.

**`coral-runner::gemini`** (3):
- `gemini::tests::gemini_runner_non_zero_returns_error`
- `gemini::tests::gemini_runner_runs_against_echo_substitute_with_real_args`
- `gemini::tests::gemini_runner_non_zero_scrubs_bearer_token_from_error`

**`coral-runner::local`** (3):
- `local::tests::local_runner_non_zero_returns_error`
- `local::tests::local_runner_runs_against_echo_substitute_with_real_args`
- `local::tests::local_runner_non_zero_scrubs_bearer_token_from_error`

**`coral-runner::runner`** (2):
- `runner::tests::claude_runner_non_zero_returns_error`
- `runner::tests::claude_runner_uses_echo_substitute`

**Triage**: extract the echo-substitute path to a helper that picks
`cmd /C echo` on Windows. Or add `#[cfg(unix)]` gates. Linux CI
runs them so coverage is preserved.

**Effort**: 1-2 hours (helper extraction). **Priority**: Medium (covers
real runner error-paths; would be nice on Windows too).

---

## Category C — File-lock / path-separator artifacts (5 failures)

OS-specific behavior of either filesystem locking or path normalization.

- `coral-cli::commands::search::tests::run_embeddings_sqlite_backend_writes_db_file`
  → SQLite file-lock contention with tempdir Drop on Windows.
- `coral-cli::multi_repo_project::project_sync_clones_a_local_bare_repo_end_to_end`
  → `git clone` from a local bare repo; symlink or path-escape semantics.
- `coral-cli::template_validation::all_slash_commands_present`
- `coral-cli::template_validation::all_subagents_have_required_frontmatter`
- `coral-cli::template_validation::hermes_validator_subagent_present`
  → directory walking with backslash-vs-forward-slash path comparison.

**Triage**: each needs a separate ~30 min triage. The template_validation
trio is most likely a snapshot-style normalization issue and could be
batch-fixed with a `path.to_string_lossy().replace('\\', "/")`.

**Effort**: 2-3 hours. **Priority**: Medium-High (template_validation is
load-bearing for the plugin distribution; SQLite is a real race).

**`coral-env`**:
- `coral-env::compose_yaml::tests::watch_path_resolves_against_resolved_context`
  → relative-path resolution against canonicalized base. Likely UNC
  prefix `\\?\` leak (seen during v0.34.x register-marketplace work).

**Effort**: 30 min if `dunce::simplified` is applied. **Priority**: Low.

**`coral-session`**:
- `coral-session::claude_code::tests::find_latest_for_cwd_picks_most_recent_matching_jsonl`
  → JSONL file mtime ordering; Windows truncates filesystem mtime to
  100ns vs Unix nanos. Tests that compare two files written in the
  same test tick can flake.

**Effort**: 30 min (force `std::thread::sleep(Duration::from_millis(15))`
between writes in the test). **Priority**: Low.

**`coral-stats`**:
- `coral-stats::tests::stats_schema_matches_committed_file`
  → schemars-derived JSON Schema regen drift on Windows; likely
  serde line-ending difference (`\r\n` vs `\n`).

**Effort**: 30 min (commit normalized schema OR add CRLF→LF filter).
**Priority**: Low.

---

## Category D — POSSIBLY NEW post-v0.35 (1 failure)

- `coral-mcp::transport::http_sse::tests::session_table_reap_drops_expired_entries`
  → Session table reap timing. **Touched by v0.35 CP-3** (parking_lot
  migration + bearer auth port). Could be a real new flake from the
  Mutex → parking_lot semantic shift (different fairness, no
  poison), or could simply be a timing-sensitive test that needs a
  longer reap window on Windows runners.

**Triage**: investigate first. If it's a real bug, file an issue;
if it's a timing flake, bump the reap interval in the test or
relax the assertion.

**Effort**: 1 hour (reproduce + decide). **Priority**: **High** (only
candidate for a new regression introduced by v0.35).

---

## Recommended remediation order

1. **HIGH**: `session_table_reap_drops_expired_entries` (Cat D) — investigate
   whether parking_lot migration introduced a real race; could affect
   production MCP HTTP transport.
2. **MEDIUM-HIGH**: `coral-cli::template_validation` trio (Cat C, 3 tests) —
   batch-fix with path-separator normalization; this validates plugin
   manifest integrity.
3. **MEDIUM**: `coral-runner` 8 echo-substitute tests (Cat B) — extract
   helper; restores Windows coverage of runner error paths.
4. **LOW**: 10 release_flow bash tests (Cat A) — `#[cfg(unix)]` gate.
5. **LOW**: SQLite file-lock + git clone + compose_yaml + claude_code
   jsonl + stats schema (Cat C remainder, 5 tests) — each ~30 min,
   total ~2.5 hours.

Total Windows-fix effort estimate: **6-9 hours** to drop the 27→0.

---

## Cross-references

- v0.34.x audit `AUDIT-TESTING-2026-05-12.md` TEST-08 originally
  flagged the 26-test count; this enumeration replaces the proxy.
- v0.35 sprint changed http_sse.rs (CP-3 parking_lot) — likely
  origin of the only Cat D test.
- BACKLOG.md item #8 (test_script_lock cross-module) was previously
  deferred because the audit said the failures were not script-
  related; this enumeration confirms (only 8 are Unix-script tests,
  the other 19 are unrelated).
