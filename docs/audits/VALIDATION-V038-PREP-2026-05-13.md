# Validation: v0.38 prep batch (5 autonomous items + 1 fmt cleanup)

Date: 2026-05-13
Validator: claude (sonnet)
Targets: 6 commits `26aa0b8..a39af90`

## Verdict

**APPROVED_WITH_REVISIONS** — All hard gates green, all 5 functional
deliverables verified, sanitization invariant holds. Two minor doc
drifts found (README body still references MSRV 1.85 in 4 spots
outside the bumped badge; CHANGELOG Unreleased blurb mentions
`arboard` clipboard crate but the actual implementation correctly
ships a pipe-hint and no `arboard` dep). Neither blocks v0.38.0
tagging — both are doc-only and can be swept in a follow-up
`docs(readme):` commit.

## Spot-check (5 areas)

| Aspect | Status | Notes |
|---|---|---|
| **a. `coral wiki serve` removal** | OK | `wiki serve --help` errors with `unrecognized subcommand 'serve'`. `wiki --help` lists only `at`. `crates/coral-cli/src/commands/serve.rs` confirmed deleted (Glob no-match). `coral-cli/Cargo.toml`: `webui` feature retired, no direct `tiny_http` dep. coral-ui + coral-mcp still build (workspace dep retained). All `coral wiki serve` references in README/docs/UI.md/SKILL.md/lib.rs/wiki.rs are in historical/migration/removal-notice contexts (no live usage examples). |
| **b. CHANGELOG Keep-a-Changelog** | OK | File at repo root. Header declares Keep a Changelog 1.1.0 + SemVer 2.0.0. `Unreleased` section present with `Added` / `Changed` / `Removed (breaking — pre-1.0)` groupings. v0.34.0 → v0.37.0 all present (5 release sections backfilled, including v0.34.1 patch). Each release dated. Cross-refs to BACKLOG #9 + ADR-0012 + audit docs present. |
| **c. INSTALL.md v0.37 surface** | OK | `--version v0.37.0` referenced in installer one-liners. 5 skills listed (bootstrap, query, onboard, ui, doctor). 2 slash commands (`/coral:coral-bootstrap`, `/coral:coral-doctor`). SessionStart hook section present. `--token` 128-bit entropy floor + auto-mint paragraph present. SLSA verification subsection with `gh attestation verify` one-liner present. MSRV 1.85 → 1.89 callout at line 20. |
| **d. `coral feedback submit`** | OK | Help text shows `--copy` flag. Live run in this repo emits valid JSON (`coral_version: 0.37.0` — workspace pre-bump, expected). JSON output sanitization holds: zero `/`, `\\`, `sk-ant-`, `github.com`, `.git`, `api_key`, `secret`, or `.bootstrap-state.json` values. 11 unit tests in `crates/coral-cli/src/commands/feedback.rs` present + nextest passes them all. `--copy` flag emits `\| clip` on Windows (correct platform branch). Stderr warning when no bootstrap state intentionally omits absolute path (sanitization invariant covers stderr per comment). |
| **e. README MSRV badge** | OK (with drift) | Badge at line 8 now reads `rust-1.89%2B` (correct). All 6 badges render (CI, Release, License, Rust, MCP, OpenSSF Scorecard). **Minor drift**: lines 128, 263, 1754, 1997, 2043 still say "MSRV 1.85" in body text — dev only bumped the badge, not body. Not a blocker but noted for sweep. |

## SemVer call

v0.38.0 minor bump justified:
- Coral is pre-1.0; SemVer §4 permits breaking changes in minor releases.
- 3-version deprecation honoured (announced v0.34.1, original target v0.36.0, actual removal v0.38.0 — 2 trains late but the stderr banner has been printing since v0.34.1, well past the "3 versions of warning" customary floor).
- `CHANGELOG.md` Unreleased flags removal under `Removed (breaking — pre-1.0)` with explicit migration line pointing at `coral ui serve` (same default port `3838`, same `--bind` flag).
- Commit message uses `refactor!:` (Conventional Commits breaking marker) — correct.

**Confirmed:** v0.38.0 is the right tag for this batch.

## Workspace state

- `cargo check --workspace`: green (finished 2.26s, no warnings).
- `cargo fmt --all -- --check`: green (silent — no diffs).
- `cargo clippy --workspace --all-targets -- -D warnings -A clippy::unwrap_used -A clippy::expect_used -A clippy::panic`: green (strict gate, finished 8.21s).
- `cargo clippy --workspace --lib --bins --no-deps -- -D warnings`: green (panic-risk hard gate, finished 6.54s).
- `cargo nextest run --workspace --no-fail-fast`: **1909 passed, 19 skipped** (= 1928 total). Matches dev claim 1909/1928 exactly.

## Autonomous decisions review

1. **`--copy` pipe-hint vs `arboard` dep** — Sound trade-off. `arboard`
   would add ~6 native deps (X11/Wayland/Cocoa/Win32 bindings) and 4
   cfg-gated code paths for a feature 95% of users will invoke twice
   in their life. Pipe-hint costs zero bytes and teaches the user the
   underlying tool, which is friendlier for the AF-1 manual-paste
   workflow. Module doc-comment explicitly defers `clipboard` as a
   future optional cargo feature — clean exit ramp. **Approved.**
   *(Note: CHANGELOG Unreleased blurb still says "via `arboard`
   best-effort"; the actual code uses the pipe hint. CHANGELOG copy
   needs a one-line fix.)*

2. **crates.io + marketplace badges deliberately omitted** — Correct
   call. Coral is not currently published to crates.io (install path
   is install.sh / install.ps1 / GitHub releases tarballs). Adding a
   crates.io badge that 404s would mislead. Same for a marketplace
   badge that has no live listing. **Approved.**

3. **CHANGELOG commit prefix `docs:`** — Industry-conventional.
   Conventional Commits explicitly lists `docs:` for "documentation
   only changes"; `CHANGELOG.md` is documentation. Using `feat:` for
   a backfilled CHANGELOG would be incorrect (no feature shipped).
   **Approved.**

## Minor revisions (non-blocking)

1. **README body MSRV references** — 4 lines still say "MSRV 1.85" /
   "Rust 1.85". Suggest a follow-up `docs(readme): sweep MSRV body
   refs to 1.89` commit. Trivial sed-style fix.
2. **CHANGELOG Unreleased `arboard` mention** — The Unreleased
   `coral feedback submit` bullet says `--copy` "attempts clipboard
   via `arboard` (best-effort; gracefully degrades to manual paste)";
   actual code uses platform-appropriate pipe hints (pbcopy / clip /
   xclip). Suggest editing the Unreleased blurb to match the shipped
   behaviour before tag.

## Recommendation

**APPROVED_WITH_REVISIONS — ship v0.38.0 after the two minor doc fixes
above land.** No correctness issues. No sanitization leak. Workspace
gates green. SemVer call sound. Autonomous decisions defensible.
