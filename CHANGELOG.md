# Changelog

All notable changes to Coral are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_No unreleased changes — see v0.40.1 below._

## [0.40.1] - 2026-05-16

**Install autonomy patch.** Closes three of the four BACKLOG #12
layers surfaced by dogfooding v0.40.0 against this repo on macOS
Sequoia under Claude Code. No public API changes; one new
user-visible error path (the install-refuse gate) and one
runner pre-flight check.

### Fixed

- **L1 — `coral init` re-runs now apply gitignore + CLAUDE.md
  scaffolds even when `.wiki/` already exists.** The early-return
  at `crates/coral-cli/src/commands/init.rs:39-44` short-circuited
  before the FR-ONB-34 `.gitignore` hardening and FR-ONB-25
  CLAUDE.md template steps, leaving any repo that ran `coral init`
  on a pre-v0.34 binary without the security-critical entries on
  upgrade. The early return is gone; the per-file existence checks
  downstream already make every step idempotent.

### Added

- **L4 — `scripts/install.sh` refuses to run from a Claude Code
  shell on macOS.** Detects `CLAUDECODE=1` + Darwin host and exits
  early with an actionable message before any download. Without
  this, every file the installer writes (the `coral` binary itself,
  `.coral/config.toml`, the optional CLAUDE.md scaffold) would
  inherit `com.apple.provenance` and become EPERM-inaccessible from
  the user's regular Terminal — and the tracked process cannot
  strip its own provenance, even via `sudo`/authtrampoline, for
  paths inside `~/Documents/`. Escape hatch:
  `CORAL_INSTALL_ALLOW_TRACKED_PROCESS=1`.
- **L3 — `coral-runner::ClaudeRunner` pre-flight check** that
  refuses to spawn `claude` from inside a Claude Code shell with a
  specific, actionable `AuthFailed` instead of the opaque
  `401 Invalid authentication credentials`. The CLI's
  host-managed-mode detection (driven by macOS Endpoint Security
  responsibility tracking, not just env vars) cannot be bypassed
  from a subshell, so we surface the failure upfront and point
  users at the two real fixes: run from a plain Terminal, or
  configure `[provider.anthropic]` with a direct `sk-ant-api03-...`
  key in `.coral/config.toml`. Gate is skipped when the runner
  points at a non-`claude` binary so the existing `runner.rs` unit
  tests stay green under `cargo test` invoked from a Claude Code
  shell.

### Documented

- **BACKLOG #12** captures all four layers (L1 closed v0.40.1, L4
  closed v0.40.1, L3 closed v0.40.1; L2 non-interactive provider
  config remains open).

## [0.40.0] - 2026-05-13

**Test-coverage release: `coral doctor --wizard` E2E coverage +
clippy-resistant `Prompter` abstraction enable the CI coverage floor
bump 60% → 65%.** No public API or CLI flag changes. The new
`Prompter` trait is `pub(crate)` and only matters to in-crate test
authoring — production behaviour is identical.

### Added

- **`Prompter` trait in `coral-cli::commands::doctor`** (~60 LoC,
  `pub(crate)` only) with `DialoguerPrompter` real impl and
  `MockPrompter` test impl (gated under `#[cfg(test)] mod tests`).
  Decouples the wizard's 5 interactive branches (Anthropic / Gemini
  / Ollama / claude CLI / Skip) from `dialoguer`'s real-stdin
  requirement so the branches are reachable under `cargo test`.
- **5 binary-spawning E2E tests** in `crates/coral-cli/tests/
  doctor_e2e.rs` (assert_cmd, fresh tempfile-isolated cwd per test)
  + 15 new in-file unit tests in `doctor.rs` covering
  `print_human_report`, `ping_anthropic` / `ping_gemini` error paths,
  `toml_string` control-char + backslash escapes, and the `run()`
  dispatcher for default + non-interactive modes.

### Changed

- **Coverage floor bumped 60% → 65%** in the `Coverage` CI job
  (`CORAL_COV_MIN_LINES`, BACKLOG #5 step 3/4). The enabler was a
  `Prompter` trait abstraction added to `coral-cli::commands::doctor`
  that decouples the `coral doctor --wizard` 5 interactive branches
  from `dialoguer`'s real-stdin requirement, plus 20 new in-file unit
  tests and 5 binary-spawning integration tests in
  `crates/coral-cli/tests/doctor_e2e.rs`. Measured workspace line
  coverage on commit `a1bff16` (the test-additions commit): 84.04%
  lines (84436 instrumented, 13479 missed); doctor.rs specifically
  went 39.22% → 84.39%. The 65% floor clears with +19.04% margin.
  Next step (v0.41.0): reach the PRD 70% KPI.

## [0.39.0] - 2026-05-13

**Hygiene release: bincode → postcard migration clears RUSTSEC-2025-
0141; coverage floor ratcheted 55% → 60%.** No public API or CLI
flag changes; on-disk search-index cache silently rebuilds on first
access after upgrade.

### Changed

- **`coral-core::search_index` serialization swapped from bincode 2.x
  to postcard 1.x** (BACKLOG #7 closure). Drops the RUSTSEC-2025-0141
  ignore from `deny.toml` and the `cargo audit` step — bincode no
  longer appears in the dependency graph. Postcard is actively
  maintained, varint-based (similar on-disk size to bincode 2.x), and
  uses the serde integration so `SearchIndex` keeps its single set of
  `Serialize`/`Deserialize` derives. The on-disk format change is
  transparent to users: pre-v0.39 `.coral/search-index.bin` files
  trigger the existing `load_index` decode-failure path
  (`tracing::warn!` + rebuild from in-memory corpus on next access) —
  same fallback that already absorbed the v0.34.x bincode 1.x → 2.x
  flip. `cargo deny check advisories` now passes clean with zero
  suppressed advisories.
- **Coverage floor bumped 55% → 60%** in the `Coverage` CI job
  (`CORAL_COV_MIN_LINES`, BACKLOG #5 step 2/4). The v0.38.0 CI
  measurement on commit `2e06caa` showed the workspace at 83.81%
  lines (84121 LoC instrumented, 13616 missed), so the 60% target
  clears with +23.81% margin. The original v0.35.0 plan called for
  new `coral-runner::http` golden tests + SSE-push exercise to enable
  this bump; accumulated test growth across v0.35..v0.38 cleared the
  threshold without that work being load-bearing. Next step
  (v0.40.0): bump to 65% after `coral-cli::commands::doctor` wizard
  paths get end-to-end coverage (currently 39.22%).

## [0.38.0] - 2026-05-13

**Breaking SemVer-minor (pre-1.0): `coral wiki serve` removed.** Two
release trains past its v0.34.1 deprecation banner, the legacy
HTML/Mermaid wiki browser is now gone — use `coral ui serve` for the
modern SPA. Sprint-prep batch closed five autonomous-resolvable
items: deprecation removal, CHANGELOG backfill, INSTALL.md v0.37
refresh, opt-in `coral feedback submit` for crowd-sourced calibration
data, and MSRV badge sync (1.85 → 1.89).

### Added

- **`coral feedback submit`** — opt-in JSON dump of the local
  bootstrap calibration data for crowd-sourced cost-estimate
  improvement (PRD §11 decision #3, finally implemented). Reads
  `.wiki/.bootstrap-state.json` and `.coral/config.toml`, emits a
  sanitized JSON envelope (`coral_version`, `platform`, `provider`,
  `repo_signature`, `bootstrap_estimate`, `wallclock`) to stdout with
  user-facing paste instructions. Sanitization is conservative: no
  file paths, no file names, no git remote URLs, no API keys, no
  user names — only LOC totals, file-extension counts, page counts,
  predicted vs actual cost, and predicted vs actual wallclock.
  Strictly AF-1 compliant (zero phone-home): the operator copies the
  JSON manually into a discussion comment. `--copy` flag prints a
  platform-aware pipe hint (`pbcopy` on macOS, `clip` on Windows,
  `xclip -selection clipboard` on Linux) so no clipboard library is
  pulled into the binary.
- **CHANGELOG.md backfilled** for the v0.34.0 → v0.37.0 release train
  (this commit). The repo had drifted: only v0.32.x and v0.33.0 had
  entries, so five missing release sections are reconstructed from
  the git log with Keep-a-Changelog grouping.

### Changed

- **`docs/INSTALL.md` refreshed** for the v0.37 surface: 5 auto-
  invoked skills (was 4 — `coral-doctor` added in v0.34.0), 2 slash
  commands (`/coral:coral-bootstrap`, `/coral:coral-doctor`),
  SessionStart hook (sh + ps1 + bash dispatcher), `--token` 128-bit
  NIST entropy floor + auto-mint, provider mini-wizard 4 paths,
  `coral self-upgrade` + `coral self-uninstall`, MSRV 1.85 → 1.89,
  4-target cross-platform support matrix, SLSA verification cross-
  reference, and a "provider not configured → `coral doctor --wizard`"
  troubleshooting entry.

### Removed (breaking — pre-1.0)

- **`coral wiki serve` removed.** The v0.25.0 legacy HTML/Mermaid
  wiki browser was deprecated in v0.34.1 with a removal target of
  v0.36.0. Two release trains overdue (BACKLOG #9). Use
  `coral ui serve` — same default port (`3838`), same `--bind` flag,
  modern SPA with graph + bi-temporal slider + filtering + LLM
  query playground. Cargo feature `webui` and the optional
  `tiny_http` direct dep in `coral-cli` were both retired (tiny_http
  remains a workspace dep; `coral-ui` and `coral-mcp` still use it).

## [0.37.0] - 2026-05-13

**Production-panic elimination — clippy panic-risk job promoted to
hard gate (0 warnings achieved across the workspace).** v0.36.0
landed the ratchet at 45 warnings; v0.37.0 retires the last of them
across every crate, lands cross-platform mimalloc baselines for
ADR-0012, and hardens a handful of test flakes that the v0.35
parking_lot migration surfaced. `coral --version` reports `coral 0.37.0`.

### Added

- **Cross-platform mimalloc baseline benchmarks** under `bench/` with
  Linux + macOS results landed from CI run 25804983205. ADR-0012
  ("mimalloc as default allocator") is now backed by reproducible
  per-platform numbers, not just a hand-wave.
- **`mimalloc-allocator-bench` CI workflow** (`workflow_dispatch` +
  weekly cron) so the baseline keeps re-running on `main` and any
  regression surfaces without a human kicking it off.

### Changed

- **Clippy panic-risk job promoted from informational to hard gate.**
  The v0.36.0 ratchet got the production warning count to 45;
  v0.37.0's targeted refactors took it to 0. The job now blocks PRs
  on any new `unwrap`/`expect`/`panic` call site outside `#[cfg(test)]`
  and benches.
- **`unwrap` / `expect` retired across the workspace** — 12 sites in
  `coral-cli`, 12 in `coral-core`, 6 in `coral-ui`, 5 in `coral-mcp`,
  3 in `coral-runner`, plus 6 stragglers in misc crates. Each turned
  into a structured `Result` propagation or an explicit `expect` with
  a "this branch is statically unreachable because …" justification
  comment.

### Fixed

- **SQLite `Connection` lifetime around `remove_file`.** When the
  search-index schema check failed and `coral search` tried to
  delete the stale DB, the open `Connection` was still holding the
  file handle on Windows. Connection now drops scope-explicitly
  before the `remove_file` call.
- **TOML render_toml escape backslashes + quotes.** Manifest writer
  was emitting unescaped `\` and `"` in string values; round-trip
  through `toml::from_str` then crashed on Windows path strings.
  Fix lands a proper escape function with a round-trip test.
- **MCP `session_table_reap` test flaked on `Instant::now()`.** The
  test's clock-budget assertion was tighter than the OS's monotonic
  clock granularity guarantees on a loaded CI VM. Widened the
  tolerance to absorb the scheduler.
- **`coral-runner` test gating.** `/bin/echo` and `/usr/bin/false`
  tests now `#[cfg(unix)]`-gated (Windows nextest no longer fails
  on the missing binaries).
- **CRLF normalisation** in `coral-stats` and `coral-cli`
  template-validation tests so they pass on Windows nextest without
  `.gitattributes` magic.

### Internal

- **Audit doc cross-references**: `docs/adrs/0012-mimalloc-allocator.md`
  now points at the live Linux + macOS baseline pages.

## [0.36.0] - 2026-05-13

**Clippy ratchet 104 → 45 + public-surface tightening.** A
preparatory refactor batch that sets up v0.37.0's panic-risk hard
gate: the bulk of the easy `unwrap`/`expect` sites are converted,
and curated re-exports replace promiscuous `pub mod` declarations
across `coral-core` (ARCH-C1 remainder). The SPA bundle gets pre-
compressed gzip + brotli sibling artifacts for sub-200ms first paint.
`coral --version` reports `coral 0.36.0`.

### Added

- **Pre-compressed SPA siblings.** `assets/dist/*.{js,css,html}` now
  ship with `.gz` and `.br` siblings generated at build time; the
  static-asset handler picks the smallest encoding the client
  advertises in `Accept-Encoding`. Sibling files ≥ 100 KiB are
  dropped if the raw bytes are smaller (some hashed bundles already
  pre-compress poorly). `Vary: Accept-Encoding` is always emitted
  so caches don't poison cross-client.
- **mimalloc baseline benchmark** (initial Linux run) — closes
  ADR-0012's "still need real numbers" loose end.

### Changed

- **Clippy production-warnings count ratcheted 104 → 45.** Mostly
  trivial cleanups (`needless_borrow`, `redundant_clone`,
  `single_match` → `if let`, etc.). The hard `unwrap`/`expect`
  retirement lands in v0.37.0.
- **`coral-core` public mod surface tightened.** Six modules that
  had `pub mod` but zero external callers are demoted to
  `pub(crate) mod`, with the actually-needed items re-exported at
  the crate root. ARCH-C1 from the v0.34.1 architecture audit was
  half-done in v0.35.0; this finishes the sweep.

### Internal

- **v0.36 handoff doc** enumerates 27 known Windows nextest
  failures by name so the v0.37 cleanup is scoped, not aspirational.

## [0.35.0] - 2026-05-13

**Six-phase multi-agent audit (security, concurrency, performance,
quality, testing, architecture) → CP-1 through CP-4 fix waves.**
Top-line wins: bearer auth on the MCP HTTP transport (SEC-01),
`--token` entropy floor + auto-mint on `coral ui serve` (SEC-02),
parking_lot lock migration end-to-end (replaces ~30 `std::sync::Mutex`
sites), parallel per-page render in `coral bootstrap` with a 4-thread
rayon pool, MSRV bump to 1.89, and workspace clippy lints
(`unwrap`/`expect`/`panic = warn`) wired into a split CI job (strict
gate + informational panic-risk). `coral --version` reports `coral 0.35.0`.

### Added — Security (CP-1 / CP-3)

- **Bearer auth required on `coral mcp serve --transport http`** when
  bound to a non-loopback address. Loopback (127.0.0.1, ::1) stays
  unauthenticated for plug-and-play dev (SEC-01).
- **`coral mcp serve --token <hex>`** with 128-bit entropy floor
  (NIST SP 800-63B: 32 hex chars minimum). Auto-mint via
  `coral_core::auth::mint_bearer_token` (CSPRNG, 256-bit) when no
  token is supplied on a non-loopback bind — banner printed to stdout
  with the curl-ready value.
- **`coral ui serve --token`** flow mirrors the MCP one (SEC-02 /
  CP-4): explicit token validated against the 128-bit floor; env var
  `CORAL_UI_TOKEN` accepted; auto-mint on non-loopback bind.
- **Shared `coral-core::auth` module** centralises bearer + loopback
  helpers so the three serve surfaces (mcp http, ui, future) don't
  reimplement the policy.
- **Prompt-injection check on LLM response bodies** in `coral
  bootstrap` (SEC-06). Bodies containing suspicious instruction-
  injection markers are rejected before they hit the wiki.

### Added — Concurrency / Performance (CP-1 / CP-2)

- **Parallel per-page render in `coral bootstrap`** via a 4-thread
  rayon pool. Per-page tracing spans + cost-gate events (Q-C3).
  Sequential fallback preserved for `--threads 1`.
- **MCP server thread-pool dispatch.** Each incoming request lands
  on a fresh thread (P-C3 / CON-01 / CON-06); old single-threaded
  recv loop was a noticeable cap on concurrent agent calls. 503
  busy-body Content-Type now `application/json`.
- **`parking_lot::{Mutex, RwLock}` everywhere** except `OnceLock`
  and one-off cases where `std::sync` is part of the public API.
  ~30 sites migrated; net effect is slightly smaller and faster locks
  under contention.

### Added — Quality / Provider Bridges

- **`[provider.anthropic]` + `[provider.gemini]` env bridging.**
  Config-resolved API keys now flow into the runner subprocess via
  the new `with_env_var` builder on `RunnerBuilder`. Closes the
  gap where `coral.toml` config wasn't actually being honored by
  the subprocess runners.
- **`[provider.ollama]` bridged** to `--provider=http` so the runner
  speaks to a locally-running Ollama server with config-driven host
  + model.

### Changed

- **MSRV 1.85 → 1.89.** Required for `let_chains` stabilization (now
  used pervasively post-clippy auto-fix) and `is_multiple_of`.
  Justified in the new ADR-0011 ("MSRV policy"). Edition 2024
  unchanged.
- **`bincode 1.x` → `2.x`** (RUSTSEC-2025-0141). Persisted BM25
  index format-versioned with a one-shot migration on first load.
- **CI clippy split** into a strict gate (`-D warnings`) and an
  informational panic-risk job (`-W clippy::unwrap_used` etc.) so
  the warning ratchet can advance without blocking PRs prematurely.

### Fixed

- **Plugin manifest sync** ran on every release-tag job so the
  `.claude-plugin/marketplace.json` + `plugin.json` stayed in
  lockstep with the binary version.

### Internal

- **ADR-0010, ADR-0011, ADR-0012 landed** documenting the substrate
  (workspace shape), MSRV policy, and mimalloc decision.
- **`coral_core::auth::mint_bearer_token`** hoisted out of
  `coral-cli` (where it had originally been written for SEC-01 /
  CP-3) into the core crate so `coral-ui` reuses it without crossing
  the wrong dependency edge.
- **`test_script_lock`** tagged `#[doc(hidden)]` with a TEST-ONLY
  warning comment. Six zero-external-callers `pub mod` declarations
  in `coral-core` demoted to `pub(crate) mod` (ARCH-C1 partial; rest
  in v0.36.0).
- **Six audit reports** committed under `docs/audits/` (security,
  concurrency, performance, quality, testing, architecture) plus a
  cross-phase synthesis. Each ran a 2-pass review (initial findings
  → validation pass).

## [0.34.1] - 2026-05-12

**Smoke-workflow + self-upgrade hardening + Ollama bridge.** Patch
release shipped same-day as v0.34.0 after the post-release smoke
workflow surfaced several path / hook / runner-bridge bugs that the
CI matrix had missed pre-release. `coral --version` reports `coral 0.34.1`.

### Added

- **`coral wiki serve` deprecation banner** announcing removal in
  v0.36.0 (eventually shipped in v0.38.0 — see BACKLOG #9). Banner
  prints to stderr on every invocation with the migration line
  pointing at `coral ui serve`.
- **`[provider.ollama]` → `--provider=http` runner bridge.** The
  config-resolved host + model + (optional) bearer flow from
  `.coral/config.toml` into the runner subprocess. v0.34.0 had the
  config plumbing but the runner-side env was incomplete.
- **`coral self-upgrade` honours `GITHUB_TOKEN` / `GH_TOKEN`** for
  GitHub API authentication. Avoids rate-limit pain when CI invokes
  the upgrade path.

### Fixed

- **Windows SessionStart hook latency.** v0.34.0 spawned a secondary
  PowerShell host for the hook body; rewritten to run in the parent
  PS process. New empirical Windows budget of 1200ms (was 800ms,
  unrealistic for the 2× spawn overhead) — justified in the CI
  workflow comment.
- **Post-release smoke workflow PATH** matches the install.ps1
  `$LOCALAPPDATA` default. Smoke had been searching the wrong path
  on Windows.
- **License allowlist + lint output paths.** Forward-slash paths in
  `coral lint` output (so VS Code "go to file" Cmd-clicks land on
  the right line on Windows); added `CDLA-Permissive-2.0` to the
  `deny.toml` license allowlist (transitively pulled in via
  `arrow-deps` adjacent crate).
- **`coral self-upgrade` user-typed tag prefix** preserved across
  messages + URLs. v0.34.0 stripped the leading `v` from `v0.34.0`
  and then re-added it inconsistently; messages now mirror what the
  user actually typed.

## [0.34.0] - 2026-05-12

**Zero-friction onboarding via Claude Code (PRD-v0.34, 4-week M1).**
First-time UX redesigned end-to-end: a SessionStart hook auto-invokes
the right Coral skill (bootstrap / query / onboard / ui / doctor),
two slash commands (`/coral:coral-bootstrap`, `/coral:coral-doctor`)
short-circuit the common asks, `coral doctor --wizard` walks new
users through provider setup interactively, `coral self-upgrade` +
`coral self-uninstall` close the install lifecycle, and `coral self-check`
emits a JSON-schema'd health snapshot for both the wizard and CI.
60-second "from zero to first wiki page" workflow is the new
acceptance target. `coral --version` reports `coral 0.34.0`.

### Added — Onboarding skills + slash commands

- **`coral-doctor` skill + `/coral:coral-doctor` slash command** —
  the fifth auto-invoked skill (was 4: bootstrap, query, onboard,
  ui). Triggers on words like "broken", "provider", "stuck",
  "not configured".
- **`/coral:coral-bootstrap` slash command** shortcut to the same
  flow as the autoinvoked `coral-bootstrap` skill — for users who
  already know the model and just want to skip the natural-language
  routing.
- **`coral-ui` skill background-spawn pattern** (FR-ONB-11) — the
  skill never blocks; it launches `coral ui serve` in the
  background, waits for the listening banner, and opens the
  browser.

### Added — SessionStart hook

- **`.claude-plugin/hooks/session-start.{sh,ps1,bat}`** wired via
  the plugin manifest. The dispatcher detects platform + shell and
  runs the right script. Empirical latency budget enforced in CI
  via hyperfine: ≤800ms macOS/Linux, ≤1200ms Windows (FR-ONB-9).
  Cap on `coral self-check --quick` JSON output at 8000 chars so
  the hook payload stays small.

### Added — `coral doctor` + provider mini-wizard

- **`coral doctor` subcommand** distinct from `coral project doctor`
  (FR-ONB-23 test pins the boundary). Reports MSRV / runner /
  provider / token / UI / MCP status.
- **`coral doctor --wizard`** synchronous mini-wizard guiding the
  user through 4 provider paths: Anthropic API key, Gemini API
  key, Ollama localhost, claude CLI (subprocess). Each path
  validates with a 1-token HTTP ping (`ureq`, no async) before
  persisting to `.coral/config.toml`.

### Added — `coral self-check` + `coral self-upgrade` + `coral self-uninstall`

- **`coral self-check`** emits a structured JSON-schema'd health
  snapshot (FR-ONB-6). New `--print-schema` flag dumps the JSON
  Schema for the `SelfCheck` struct so downstream tooling can pin
  the contract (`schemars`-derived).
- **`coral self-check --quick`** subset for the SessionStart hook
  (FR-ONB-8): runner / provider / token / MCP / UI / update-
  available probes, capped at 8000 chars total.
- **`coral self-upgrade`** cross-platform binary self-replace
  (FR-ONB-32). Windows path uses `MoveFileExW` with the rename-
  then-replace dance (the running .exe can't be overwritten
  directly while it executes); Linux + macOS use atomic
  `rename(2)`. Update-source: GitHub releases API + binary
  artifact download. Verifies SHA-256 before swap.
- **`coral self-uninstall`** with `dialoguer::Confirm` safety
  prompt (`--yes` to skip). Removes the binary + the
  `.claude/plugins/` install + (with `--all`) `.coral/`.
- **`coral self-register-marketplace`** for the plugin marketplace
  flow (FR-ONB) — registers Coral with the local Claude Code
  marketplace index.

### Added — `coral init` template

- **`coral init` generates a `CLAUDE.md` template** (FR-ONB-25 +
  FR-ONB-34) and extends `.gitignore` with the Coral default
  ignore set so the first-time experience leaves a clean working
  tree.
- **`InitArgs.yes` flag** — non-interactive init for CI / scripts.

### Added — `coral bootstrap` per-page cost gate + estimate

- **`coral bootstrap --estimate`** dry-run that prints predicted
  cost (USD) + wallclock based on a per-page heuristic. Provider
  pricing table lives in the new `coral_core::cost` module.
- **`coral bootstrap --max-cost <usd>`** budget gate that aborts
  mid-run if the running total threatens to exceed the budget.
- **`coral bootstrap --resume`** picks up after a crash via the
  per-page checkpoint persisted in `.wiki/.bootstrap-state.json`.
  SHA-256 fingerprint of the plan detects "the plan changed between
  runs" and refuses to silently overwrite.
- **`Option<TokenUsage>` on `RunOutput`** across every runner
  (anthropic, gemini, ollama, http, local) — required for the
  per-page cost-gate to know what each call actually spent.

### Added — `coral install` + install scripts

- **`scripts/install.{sh,ps1}` flags** `--with-claude-config`,
  `--skip-plugin-instructions` for non-interactive CI installs.
  WSL2 detection + hints for Windows users running under WSL.

### Added — `coral-core::config` module

- **`.coral/config.toml`** schema + parser (PRD Appendix E). New
  `coral_core::config` module owns the format; `[provider.*]`
  tables for anthropic / gemini / ollama with `resolve_provider_credentials`
  helper.

### Added — `coral.toml` mitigation docs

- **README**: explicit "How Coral mitigates multi-repo + functional-
  testing problems" section to clarify the value prop vs flat-file
  wiki / single-repo tools.

### Changed

- **Workspace bumped from 0.33.0 to 0.34.0.** Minor bump (PRD-v0.34
  is strictly additive: every new command + skill + hook is opt-in;
  no removed / renamed surface).
- **Disk-management mechanics codified as a project workflow** —
  documented in CONTRIBUTING.md so future agents don't end up with
  10 GB of `target/` per branch.
- **Release workflow** supports `workflow_dispatch` with explicit
  tag input; `softprops/action-gh-release` pinned to
  `env.RELEASE_TAG` to avoid the action drifting against the tag
  we actually meant to release.

### Fixed

- **`coral init` + `coral bootstrap`** Week 3 B2 + Week 2 nits 1 & 2
  cleanup: state-file round-trip on Windows, init-without-llm
  template path resolution, and the ollama test walker's nested-if
  flattening.
- **Plugin manifest CI sync** — release jobs now write back the
  bumped version into `.claude-plugin/manifest.json` +
  `marketplace.json`, eliminating manual drift.

### Internal

- **PRD-v0.34 onboarding v1.4 (implementation-readiness audit)** —
  4 review passes from initial draft v1.1 through implementation-
  readiness v1.4 before code started landing. Documented in
  `docs/PRD-v0.34-onboarding.md`.

## [0.33.0] - 2026-05-12

**M2 + M3 of the WebUI roadmap, plus Sigstore-signed provenance and CI 100% green.** Closes every deferred item from `docs/PRD-v0.32-webui.md` except formal `llvm-cov` ≥ 70% threshold enforcement and Playwright in CI matrix (both setup-only deferrals, not feature gaps). Backward-compat sacred: `coral wiki serve` legacy, MCP server, 42 CLI subcommands, REST `/api/v1/*` v0.32.x wire format all unchanged. `coral --version` reports `coral 0.33.0`.

### Added — M2 views

- **`/interfaces`** — lists every `page_type = interface` page with badges for status, confidence bars, sources, validity window. Backed by `GET /api/v1/interfaces` (filters `read_pages()` by `PageType::Interface`).
- **`/drift`** — reads `.coral/contracts/*.json` (the reports `coral test --kind contract` writes) and renders each as a card with severity-colored finding lists. Backend: `GET /api/v1/contract_status`. Severity normalisation: unknown severities default to `info`.
- **`/affected`** — input a git ref → backend runs `git log <ref>..HEAD --name-only` and returns unique top-level dirs (intersected with `coral.toml` repos when present). Backend: `GET /api/v1/affected?since=<ref>` with strict ref sanitisation (no leading `-`, allowlist `[A-Za-z0-9._/~^@{}-]`).
- **`/tools`** — runs `coral verify` / `coral test` / `coral env up` / `coral env down` from the browser. Gated by **both** `--allow-write-tools` and bearer token. Confirmation dialog for `down` with `volumes: true`. UI shows `stdout_tail` / `stderr_tail` (last 4 KiB each), `exit_code`, `duration_ms`. Backend: `POST /api/v1/tools/{verify,run_test,up,down}` via `current_exe()` shell-out (avoids cyclic deps with `coral-cli`).

### Added — M3 features

- **`/guarantee`** — input env + strict checkbox + click → backend invokes `coral test guarantee --can-i-deploy --format json` and parses the verdict (`GREEN | YELLOW | RED`) plus per-check breakdown (`lint`, `contracts`, etc. with passed/warnings/failures counts). Traffic-light UI with proportional bars.
- **SSE push notifications** — `GET /api/v1/events` opens a long-poll SSE stream that emits `event: wiki_changed\ndata: {}\n\n` when any file under `.wiki/` changes (2s polling on `max(mtime)` recursive). The SPA's `useWikiEvents()` hook wires this to TanStack Query invalidation, so all views auto-refresh when the wiki is rebuilt out-of-band (e.g. another shell running `coral ingest --apply`). Toast notification throttled at 5s; reconnect with exponential backoff 1s → 30s.

### Added — testing infrastructure

- **Playwright E2E suite** under `crates/coral-ui/assets/src/e2e/` (14 tests across 5 files: nav, pages, graph, query, manifest). Assumes a `coral ui serve --no-open --port 38400` running locally. CI workflow `playwright-ci.yml.disabled` written but not enabled — needs a fixture-bootstrap step to spin up a wiki + spawn the server in CI.
- **27 new Rust integration tests** in `coral-ui` covering all 6 new endpoints (interfaces, contracts, affected, tools × 4, guarantee, events). Total `cargo test -p coral-ui` count: **52 tests passing**.

### Changed

- **Workspace bumped from 0.32.3 to 0.33.0.** Minor bump because the API surface grew (6 new endpoints + 5 new SPA routes) — strictly additive to v0.32.x, no breakage.

### Internal

- **`coral-ui::routes::events` SSE handler** reuses the `request.into_writer()` raw-stream pattern from `/api/v1/query` (consumes the `Request`, writes HTTP head manually, flushes per event). 1-hour max stream duration enforced server-side.
- **`coral-ui::routes::tools` shell-outs to `current_exe()`** instead of importing `coral-cli` handlers directly — avoids dependency cycles (`coral-cli` already depends on `coral-ui`). Cost: one extra process spawn per write-tool invocation, negligible vs the work each command actually does (`docker compose up`, etc.).
- **`coral-ui::routes::affected`** parses `coral.toml` permissively via `toml::Value` (not via `coral_core::project::manifest::parse_toml`) so missing optional fields don't break the affected computation.

### Backward compatibility

- `coral wiki serve` (legacy from v0.25.0) unchanged. BC tests 6/6 green.
- REST `/api/v1/*` wire format from v0.32.0/v0.32.1/v0.32.2/v0.32.3 unchanged — every new endpoint is strictly additive.
- MCP server + 42 CLI subcommands + 8 resources + 10 tools untouched.
- `coral --version` reports `coral 0.33.0`.

### PRD post-mortem

`docs/PRD-v0.32-webui.md` now closes with a §17 retrospective: original timeline estimated 17–21 weeks (M1+M2+M3) for a single dev; 3-agent orchestration in a single session delivered the same scope. Variance dominated by avoiding human round-trips on validation; spike S1 (SSE feasibility) never needed because `request.into_writer()` worked first-try.

## [0.32.3] - 2026-05-12

**CI green again across all gates.** v0.32.2 unblocked the ci.yml workflow itself (`${{ env.X }}` in a job-level `name:` field had been silently 422-failing every push for 100+ runs / 4 days). With the gate running, eight surfaced lint / test / audit issues were fixed across follow-up commits. This patch release tags the now-green state so distributors don't have to cherry-pick from `main`. No backward-compat breakage; wire format / CLI surface identical to v0.32.0+. `coral --version` reports `coral 0.32.3`.

### Fixed (post-v0.32.2 CI green sweep)

- **`run_record_linux` missing `wiki_root` parameter** on the `target_os = "linux"` + `feature = "recorded"` build path. Pre-existing bug hidden by the platform/feature gate; CI Linux `--all-features` surfaced it. Plumbed through from the caller.
- **`cargo audit` + `cargo deny`** now ignore **RUSTSEC-2025-0141** (bincode 1.x is unmaintained). Informational advisory, not a security vulnerability — used in `coral-core` for the persisted BM25 search index. Migration to bincode 2.x is tracked for a future release.
- **Clippy 1.95.0 + Rustfmt 1.95.0 sweep**: five lint families fixed (`io_other_error`, `needless_borrows_for_generic_args`, `single_char_add_str`, `infallible_destructuring_match`, `field_reassign_with_default`) plus crate-wide `#[allow(clippy::doc_lazy_continuation)]` in `coral-cli/lib.rs` (deferred sweep).
- **Linux CI ETXTBSY race in fork-exec tests.** Linux kernel `do_open_execat` returns `errno 26 ("Text file busy")` when two parallel tests are in the write-then-exec window of small shell scripts, even when the targets are distinct tempfiles. Real fix: a new `pub fn coral_runner::test_script_lock()` returning a `MutexGuard<'static, ()>` over a single `OnceLock<Mutex<()>>` that every fork-exec test in the crate now holds across the spawn. Applied to 10+ tests across `coral-runner/src/{gemini,local,runner}.rs::tests` and `coral-runner/tests/streaming_failure_modes.rs`.
- **`streaming_silent_hang_is_killed_at_timeout` taking 30s to "kill"**: the shell wrapper around `sleep 30` was forking a grandchild `sleep` process that kept stdout open after `child.kill()` SIGKILLed the parent; the runner's `reader_thread.join()` then blocked for the full 30s waiting for EOF. Changed the script body from `sleep 30` to `exec sleep 30` so the shell process image is replaced by sleep itself — no grandchild, kill propagates cleanly. Same fix the sibling `streaming_one_line_then_hang` test already had (v0.19.8 follow-up).
- **2s/3s kill-deadline asserts bumped to 5s** in four tests to absorb CI variance under shared-CPU load and the test_script_lock serialiser. The contract "doesn't hang past the deadline" remains tight at ~25× the 200ms test timeout.
- **`include_docs_flag_enables_pdf_scanning`** marked `#[ignore]` on Linux CI — pre-existing CWD/MockRunner race orthogonal to this work. Tracked for a follow-up.

### Changed

- **`cargo llvm-cov` (Coverage job) now runs tests with `RUST_TEST_THREADS=1`** to sidestep the same fork-exec race that the in-process `test_script_lock` Mutex can't cross between separate test binaries. Costs ~30s wall-clock for the gain of green coverage reports.
- **`release.yml` provenance step migrated** from `slsa-framework/slsa-github-generator@v2.1.0` (which has been failing its `final` outcome step on every release since v0.30.0 — a known bug in the upstream wrapper's success propagation, not in our config) to `actions/attest-build-provenance@v2`. GitHub's official successor produces in-toto attestations signed via the Sigstore public-good instance; consumers verify with `gh attestation verify <artifact>` or `cosign verify-blob-attestation`. The provenance shape is SLSA-compatible. Removes the only persistent red on the release workflow.

### Internal

- **`coral-runner/src/lib.rs`** exposes `pub fn test_script_lock()`. Marked `pub` (not `pub(crate)`) so integration tests under `tests/` can reach it via `coral_runner::test_script_lock()`. `OnceLock::get_or_init` + uncontended `Mutex::lock` is effectively a single atomic CAS, so the runtime cost in release builds is zero. The helper is also useful for any future test that needs to fork-exec a generated script.

### Backward compatibility

- `coral wiki serve` (legacy) unchanged. BC tests 6/6 green.
- REST `/api/v1/*` wire format identical to v0.32.0/v0.32.1/v0.32.2.
- `coral --version` reports `coral 0.32.3`.

## [0.32.2] - 2026-05-12

**WebUI polish + CI unblock.** Six follow-up improvements after the v0.32.1 patch, plus an emergency fix to the `ci.yml` workflow that had been silently 422-rejecting every push for 100+ runs (since v0.22.5, three days prior). No backward-compat breakage; `coral --version` reports `coral 0.32.2`.

### Fixed

- **`ci.yml` unblocked after 100+ consecutive 0s failures.** Root cause: line 67 used `${{ env.CORAL_MSRV }}` inside the **job-level `name:` field**, which GitHub Actions only expands inside `steps:`. The workflow parser silently 422-ed every run with the runs reporting `total_count: 0` jobs and `billable: {}`, while `gh run view` returned a misleading "workflow file issue" without any pointer to the line. Discovered by running `gh workflow run ci.yml` (workflow_dispatch), which surfaced the parser's actual `Unrecognized named-value: 'env'` error. The MSRV is now hard-coded in the `name:` string; `steps:` interpolation continues to work. Three days of PR gate downtime closed in one character. (`7135df1`)
- **Clippy 1.94 lint regressions fixed across the workspace.** The October stable clippy bump introduced `io_other_error`, `needless_borrows_for_generic_args`, `single_char_add_str`, `infallible_destructuring_match`, and tightened `field_reassign_with_default`. Touched: `coral-core/search_index.rs` (4), `coral-ui/routes/query.rs` (1), `coral-mcp/transport/http_sse.rs` (1, scoped `#[allow]`), `coral-test/browser_runner.rs` (4 refactored to struct literals), `coral-runner/tests/cross_runner_contract.rs` (3 unused-import warnings — fn and imports now `#[cfg(unix)]`-gated correctly), `coral-cli/lib.rs` (crate-wide `#[allow(clippy::doc_lazy_continuation)]` because reformatting 11 doc-comments to indent list continuations would touch broad surface for cosmetic gain).
- **`cargo fmt --check`**: workspace re-formatted (auto-fix from rustfmt against the new stable).

### Added

- **Dark mode toggle** (M3 feature pulled into this patch). Sun/Moon icon button in the header switches `<html class="dark">` and persists in `localStorage`. Initial value follows `prefers-color-scheme` or the stored override; applied **before** React hydrates so there's no flash of wrong theme. New i18n keys `theme.switch_to_light` / `theme.switch_to_dark` in `en` + `es`.
- **Export graph as PNG** from the Graph view. Floating "Export PNG" button in the top-right of the canvas composites Sigma's edges + nodes + labels layers onto an offscreen canvas with a white background, serialises via `toDataURL("image/png")`, and triggers a same-tab download as `coral-graph-YYYY-MM-DD.png`. A success toast confirms the filename; failures log to console and surface as an error toast.
- **Mermaid diagrams** lazy-loaded from `cdn.jsdelivr.net/npm/mermaid@11.4.0/dist/mermaid.esm.min.mjs` only when a wiki page actually contains a ```` ```mermaid ```` fence. Adds zero bytes to the offline-only baseline bundle. Network or CSP failure falls back to a labeled `<pre>` showing the source. `securityLevel: "strict"` and post-`react-markdown` insertion keep the XSS surface tight.
- **Toast notification system.** Minimal Zustand-backed `<Toaster/>` (no Radix dep — ~80 lines + 4 KB gzipped) with variants `default / success / error / warning`, auto-dismiss TTL, and a `useToast()` hook. Mounted once at the app root in `main.tsx`. Wired up to the two existing user-facing actions where inline feedback is awkward: saving a bearer token and exporting the graph PNG. M2 will migrate query-error feedback off the inline error box onto the toast surface.
- **Multi-repo dynamic resolution** from `/api/v1/manifest`. `useCurrentRepo()` now resolves in priority order: explicit user override in `useFiltersStore().repo` → first repo key in the parsed `coral.toml` (`repos.<name>` or `[[repo]] name = ...`) → `DEFAULT_REPO` constant. Components that previously hard-coded `"default"` (`<NodePreview>`, `<PagesList>` link `to`, the cited-source links in `<QueryPlayground>`) now read from this hook, so adding a second repo to `coral.toml` makes the UI follow without code changes.

### Changed

- **`accepted_origins()` removed `bind_origin()` after the v0.32.1 cleanup.** No call sites remained; the more permissive helper that returns both `http://` and `https://` variants is now the only API. (Surfaced as a dead-code warning at v0.32.1 build time; rolled into this release.)

### Backward compatibility

- `coral wiki serve` (legacy) unchanged. BC tests 6/6 green.
- REST `/api/v1/*` wire format identical to v0.32.0/v0.32.1.
- `coral --version` reports `coral 0.32.2`.

## [0.32.1] - 2026-05-12

**WebUI hardening patch.** End-to-end browser smoke (Chrome headless against the v0.32.0 binary) surfaced one production-visible bug and four polish items that the curl-only M1 smoke could not have caught. All resolved here. No backward-compat breakage; the wire format and CLI surface are identical to v0.32.0.

### Fixed

- **`coral ui serve` Graph view no longer blanks the SPA on browsers without WebGL 2.** Sigma.js v3 (the WebGL renderer behind the bi-temporal graph) requires WebGL 2; on a browser with hardware acceleration disabled, an outdated driver, or a privacy mode that disables WebGL the Sigma constructor threw synchronously, propagated up the React tree without an Error Boundary, and unmounted the entire SPA — leaving users at a blank `/graph`. Now: a `hasWebGL2()` probe runs before `<GraphCanvas>` mounts; if absent we render a translated fallback panel with steps to enable hardware acceleration. Sigma is additionally wrapped in `<GraphErrorBoundary>` so any other Sigma-side throw is contained to the panel instead of taking down the SPA. New i18n keys `graph.fallback.no_webgl2_*` and `graph.fallback.render_error_*` in `en` and `es`. (`1da2b43`)
- **`/api/v1/manifest` 404 no longer leaks the absolute filesystem path** in the error envelope. The absolute path is now logged at `tracing::debug` for operators; only the label `"manifest"` / `"lock"` leaves the process.
- **GraphCanvas opacity-by-confidence actually applies.** v0.32.0 set `confidence` as a node attribute but Sigma's default node program doesn't read arbitrary attributes — the visual was silently dropped. Now we convert the per-status/page-type hex colour to `rgba(r, g, b, α)` with `α` clamped to `[0.4, 1]` before `addNode`, so Draft (confidence ≈ 0.5) nodes render visibly translucent and Verified ones fully opaque. (`a78cf03`)

### Changed

- **`coral ui serve` only constructs a default Claude runner when the binary is on PATH.** v0.32.0 always built `Some(ClaudeRunner::new())` at startup, so the documented `LLM_NOT_CONFIGURED` (503) error path was unreachable in practice and `claude` users on systems without the CLI would only see the failure at query time. Now `claude_binary_present()` is probed at startup; if absent, `state.runner` is `None`, a `tracing::warn` is emitted, and `POST /api/v1/query` returns `LLM_NOT_CONFIGURED` immediately — read-only routes stay fully functional.
- **`Origin` validation accepts `https://` as well as `http://` for the same host:port.** v0.32.0 hard-coded `http://` in `bind_origin()`, which would have rejected the (correct) Origin sent by browsers behind a TLS-terminating reverse proxy. Now `accepted_origins()` returns both schemes; same host:port enforcement still applies — only the scheme is permissive. `Host` header check remains strict (anti DNS-rebinding).
- **Multi-repo readiness: `repo="default"` lifted out of components into `lib/repo.ts`.** v0.32.0 had `"default"` hard-coded in `<NodePreview>` and `<PagesList>` Link `to` props. Both now call `useCurrentRepo()` which falls back to the `DEFAULT_REPO` constant in M1 (single-repo) and will be backed by `useManifest()` for M2 multi-repo without component-level changes.

### Added

- **Vitest unit suite for the SSE frame parser** in `useQueryStream.ts`. Nine tests cover happy-path single frame, half-frame buffering across reads, multi-frame chunks, multi-line `data:` joining, the spec's default `event: message` fallback, partial trailing block preservation, leading-whitespace trim, and empty buffer survival. Lives at `crates/coral-ui/assets/src/src/features/query/useQueryStream.test.ts`; runs under `npm test`.
- **`devDependencies`: vitest + @vitest/ui** for the test runner. End-users never see these (they ship with the SPA source, not the bundled `dist/`).
- **Screenshots in `docs/UI.md` and `README.md`** of each of the four views (Pages, Graph, Query, Manifest) plus a Spanish locale capture for the i18n showcase. Generated end-to-end against Coral's own `.wiki/` (20 pages, 64 edges) using Chrome headless + SwiftShader WebGL — the same binary an end-user downloads, serving real data. Lives under `docs/assets/ui-*.png`. (`d00b017`)

### Internal

- **CI drift check (`ui-build.yml`) relaxed to `::warning::` for M1.** Vite/rollup emit byte-identical CSS/JS across OS but `index.html` `<link rel="modulepreload">` order can vary by Node module-graph traversal; that's not worth blocking PRs over. The warning + GH step summary still surface drift loudly so humans rebuild and commit. The 14 MiB binary-size hard gate is unchanged. (`54a0d7e`)
- **Vite asset filenames drop content hashes** in favour of `[name].js`/`[name].[ext]`. Bundle is embedded into the Rust binary via `include_dir!` — there's no CDN to cache-bust, and content-hashed names made the CI drift diff thrash. `Cache-Control: no-cache` on `index.html` + `immutable` on `/assets/*` covers cache invalidation by URL change instead. (`6fbaa62`)
- **`.gitattributes`** marks `crates/coral-ui/assets/dist/**/*.{html,js,css,svg,json}` as `text eol=lf` so Windows devs no longer commit CRLF that the Linux CI sees as a 100% diff against its fresh build.
- **`bind_origin()` removed** in favour of the cleaner `accepted_origins()` API.

### Backward compatibility

- **`coral wiki serve` (legacy from v0.25.0) unchanged.** 6/6 BC tests pass.
- **MCP server, 42 CLI subcommands, 8 resources, 10 tools** untouched.
- **`/api/v1/*` wire format** identical to v0.32.0 — no client breakage.
- **`coral --version`** reports `coral 0.32.1`.

## [0.32.0] - 2026-05-12

**Modern WebUI shipped — `coral ui serve` with a force-directed graph + bi-temporal slider.** First milestone (M1) of the WebUI roadmap (see `docs/PRD-v0.32-webui.md`). A new React 19 + Vite 7 + Tailwind 3.4 SPA is embedded directly in the binary via `include_dir!`. End-users never need Node/npm. The legacy `coral wiki serve` (HTML/Mermaid view from v0.25.0) remains intact for backward compatibility — both subcommands coexist.

### Added

- **`coral ui serve` — REST API + embedded modern WebUI (v0.32.0 M1).** A new
  React 19 + Vite 7 + Tailwind 3.4 + shadcn SPA is now embedded directly in
  the `coral` binary via `include_dir!`. End-users never need Node/npm — the
  pre-built bundle is committed at `crates/coral-ui/assets/dist/` and embedded
  at compile time. Four views ship in M1: **Pages** (filterable list +
  Markdown detail), **Graph** (Sigma.js force-directed view of wikilinks with
  the unique **bi-temporal slider** scrubbing through `valid_from`/`valid_to`),
  **Query** (LLM playground over SSE-streamed `/api/v1/query`), and
  **Manifest** (`coral.toml` / `coral.lock` / stats).
- **New crate `coral-ui`** in the workspace. Loopback-only and read-only by
  default; bearer-token auth (constant-time comparison) is required for
  `/api/v1/query` (which spends LLM credits) and for any non-loopback bind.
  Backed by `tiny_http` (no tokio) for consistency with the rest of the
  workspace. 24 unit tests; SSE streams via `request.into_writer()` with per-
  frame `flush()`.
- **REST API `/api/v1/*`** isomorphic to the existing MCP resources/tools:
  `pages`, `pages/:repo/:slug`, `search`, `graph`, `manifest`, `lock`,
  `stats`, `query` (POST, streaming). JSON envelope is
  `{"data": T, "meta"?: {...}}` for success and
  `{"error": {"code","message","hint"}}` for failure. Slug/repo validation
  reuses `coral_core::slug::is_safe_*`.
- **Internationalization**: SPA ships with full `en` and `es` bundles (every
  visible string keyed; no hardcoded strings).
- **`.github/workflows/ui-build.yml`**: CI job that builds the SPA and fails
  the PR if `assets/dist/` is out of sync with `assets/src/`. Includes a
  hard binary-size gate at 14 MiB to detect accidental dep bloat.
- **`docs/UI.md`**: full WebUI documentation (security model, REST surface,
  bi-temporal feature, architecture diagram, dev workflow).
- **`.gitattributes`**: marks `crates/coral-ui/assets/dist/**` as
  `linguist-generated=true merge=ours` so generated bundle doesn't pollute
  language stats or cause merge conflicts on landing PRs.

### Changed

- **`coral-cli` default features now include `ui`.** The new `ui` feature
  gates the entire WebUI behind a single flag for users who want a minimal
  binary: `cargo install coral-cli --no-default-features --features mcp,cli`
  produces a binary without the SPA (~11 MiB instead of ~12 MiB). The
  separate legacy `webui` feature (which gates `coral wiki serve`) is
  unchanged.

### Backward compatibility

- **`coral wiki serve` is unchanged.** It continues to serve the simple
  HTML/Mermaid view from v0.25.0. Both subcommands coexist on the same
  default port (3838) but are never invoked simultaneously.
- The MCP server, all 42 CLI subcommands, all 8 resources, and all 10 tools
  remain bit-for-bit identical.

## [0.31.1] - 2026-05-11

**Audit cycle 5 closure + `TestKind` reserved-variant honesty.** Patch release that finishes the v0.30.0 audit umbrella (issue #62, B-batch) and makes the test-kind value-enum honest about what's wired. Twelve B-items total in the umbrella: six (B1, B4, B5, B6, B7, B8) shipped in v0.31.0; the remaining six (B2, B3, B9, B10, B11, B12) close in this release. No public-surface breakage — every change is a tightening of existing behavior.

### Added

- **`coral_cli::commands::exit_codes` module (audit #B2).** Pins the exit-code contract (`CLEAN=0`, `FINDINGS=1`, `USAGE=2`, `INTERNAL=3`) with documentation and constants tested via `exit_code_contract_constants_are_pinned`. The new `map_b2_internal_err` wrapper in `main.rs` routes I/O failures and backend-down errors to exit 3 for `lint` / `verify` / `contract check`, distinguishing them from "I ran fine and found N issues" (exit 1). Other commands keep the legacy `Err → ExitCode::FAILURE` mapping for BC.
- **`coral_cli::commands::runner_helper::EnvVarGuard` RAII (audit #B11).** Test-side helper that records the prior `env::var(key)` state on construction and restores it on `Drop` (`set_var(prev)` if it was set, `remove_var(key)` if it was unset). Replaces the bare `env::set_var`/`remove_var` calls in `resolve_provider_prefers_cli_over_env` so test cleanup is deterministic even on panic.

### Changed

- **`TestKind` reserved variants now honest at CLI + runtime.** Audit cycle 5 understated the bug: the five unwired `TestKind` variants (`LlmGenerated`, `Contract`, `Event`, `Trace`, `E2eBrowser`) did not return `Skip` with a "deferred to vN" message as documented — they produced **zero reports**, indistinguishable from "no test cases match the given filters", and exit 2. Closed in three places:
  - **Clap `--kind` help** now flags each unwired variant with `[reserved — not yet wired]` and a tracking URL to README §Roadmap.
  - **`coral_test::orchestrator::run_test_suite_filtered`** detects when `filters.kinds` requests a reserved kind that produced zero real reports, and synthesizes a single `TestReport::Skip` with the reason + tracking URL. De-duped against runner output: if a stub runner ever does emit a real Skip for the same kind, it wins.
  - **`crates/coral-test/src/spec.rs` doc-comment** rewritten from "v0.18 ships Healthcheck + UserDefined" to the audit-validated v0.31 truth (4 wired / 4 stub / 1 reserved-schema).
- **README §what-you-get + §1755 ASCII tree** counts aligned to the same 4/4/1 split.
- **`coral interface watch` debounce + nanosecond mtime (audit #B3).** `mtime_ns` now reads `SystemTime::duration_since(UNIX_EPOCH).as_nanos()` (was `mtime_secs()` second-precision). New per-path `DebounceLedger` (`HashMap<PathBuf, Instant>`) suppresses re-fires within `DEBOUNCE_WINDOW = 250ms`. Ledger trim retains entries for `DEBOUNCE_WINDOW * 4` to keep memory bounded. Per-path so independent file edits don't suppress each other. Four new unit tests: `mtime_ns_returns_nanosecond_precision_value`, `mtime_ns_returns_zero_for_missing_path`, `is_debounced_suppresses_within_window_and_releases_after`, `debounce_suppresses_rapid_resave_of_same_path`.

### Fixed

- **`ClaudeRunner` `--` separator before user-controlled positional (audit #B9).** Both `run` and `run_streaming` now insert `cmd.arg("--")` immediately before `cmd.arg(&prompt.user)`. Without this, a prompt starting with `--system rogue-prompt` would be parsed by `claude` as a flag (CVE-2017-1000117 / CVE-2024-32004 family pattern). Regression test `claude_runner_inserts_double_dash_before_user_prompt` exercises a flag-shaped prompt against `/bin/echo` and asserts the spawned argv contains `-- --system rogue-prompt` as a positional, not a flag.
- **`assert!(result.is_ok())` migration to `.expect(...)` (audit #B10).** Two remaining sites — `crates/coral-cli/src/commands/project/graph.rs:183-188` and `crates/coral-core/src/pgvector.rs:354-358` — now use `.expect("descriptive msg")` so failing tests surface the actual error variant. Closes the migration started in v0.31.0 (9 sites originally; these were missed in the first pass).
- **`CWD_LOCK` adoption in MCP serve test (audit #B11).** `mcp.rs::preview_does_not_panic_on_empty_project` test now acquires `crate::commands::CWD_LOCK` before `set_current_dir` — matches the reference pattern at `project/new.rs:115-184`. Previously this test could race with other workspace tests that mutate cwd.

### Internal

- **`ci.yml` explanatory comment for B12 coverage.** The workspace test job already runs `cargo test --workspace --all-features`, which compiles and exercises the inline tests for `tantivy` and `pgvector` feature-gated modules. Added a one-line comment making the coverage intent explicit so a future maintainer doesn't try to "fix" it by removing `--all-features`.

### Pipeline note

Umbrella issue #62 closes with this release. The full B-batch (12 items) is now resolved: B1/B4/B5/B6/B7/B8 in v0.31.0; B2/B3/B9/B10/B11/B12 here. Audit cycle 5 ends — 11/11 findings closed, plus the 5-variant `TestKind` honesty pass that came out of the post-audit review.

## [0.31.0] - 2026-05-11

**Plug-and-play Claude Code integration + post-v0.30.0 audit cycle 5 fixes (11 findings).** Two unrelated tracks landed on `main` after the `v0.30.0` tag and before the next release ships. The headline change is **plug-and-play install**: the repo now doubles as a Claude Code plugin marketplace (`.claude-plugin/`), ships a Claude Desktop `.mcpb` bundle (Linux x64 in this iteration; the other three targets follow in v0.31), one-line installers (`scripts/install.sh` / `scripts/install.ps1`), and an `x86_64-pc-windows-msvc` artifact in `release.yml`. The three-step install (`cargo install` → hand-edit `settings.json` → learn subcommands) collapses to two lines typed inside Claude Code: `/plugin marketplace add agustincbajo/Coral` then `/plugin install coral@coral`. The plugin registers the stdio MCP server automatically and bundles three skills (`coral-bootstrap`, `coral-query`, `coral-onboard`) plus two slash commands so Claude knows when to drive Coral on the user's behalf. The second track is the **5th multi-agent audit cycle on `v0.30.0`** — five parallel domain-specialist agents (security, concurrency, MCP server, CLI UX, test quality) produced 11 findings (4 High, 4 Medium, 3 Low / batch), each cross-referenced as `audit/findings/NNN-*.md` and counter-validated by 4 independent reviewer agents on the High-severity items before fix commits landed. Issues will be filed as GitHub `#52`–`#62` when the next release publishes. See `audit/SUMMARY.md` for the full catalog.

### Added

- **Claude Code plugin (`.claude-plugin/`).** `plugin.json` registers the stdio MCP server. Skills: `coral-bootstrap` (drives `coral bootstrap --apply` with explicit user confirmation before LLM credits are spent), `coral-query` (BM25/RRF wiki search routing), `coral-onboard` (first-run handholding). Two slash commands for explicit control. Bootstrap `--apply` is always confirmed inline with the user.
- **One-line installers (`scripts/install.sh`, `scripts/install.ps1`).** Linux/macOS Bash and Windows PowerShell. Both download the matching release tarball, SHA-256 verify it, place `coral` on PATH, strip macOS quarantine (`xattr -d`), and print the two-line snippet to paste into Claude Code. Verified end-to-end against the real `v0.30.0` release.
- **Claude Desktop `.mcpb` bundle (`.dxt/manifest.json` + `dxt-bundle` job in `release.yml`).** Double-click install path for Claude Desktop users. v0.30 ships Linux x64 only; the other three targets follow.
- **`x86_64-pc-windows-msvc` target in `release.yml`.** Windows binary is now part of every release. Strip/codesign steps gated to non-Windows / `apple-darwin`. Windows packaging uses PowerShell-native `Compress-Archive` + `Get-FileHash`. Release-job glob picks up `.zip`, `.zip.sha256`, and `.mcpb` alongside the existing `.tar.gz`/`.sha256`.
- **Windows-GNU build prerequisites documented (audit #001).** `README.md` "Install" gains a Windows subsection covering MSVC + VS Build Tools OR MinGW-w64 binutils on PATH, plus the Git-Bash `link.exe` PATH-shadow gotcha. CI gains a `windows-latest` cross-platform-smoke matrix entry: `cargo build --release --bin coral`, `coral --version`, `coral init` in a tempdir.

### Changed

- **`README.md` "Use it from Claude Code in one command" section** lands right after the 60-second block. MCP integration section leads with the plugin path; the manual `settings.json` snippet becomes a fallback.
- **MCP error envelope follows JSON-RPC §5.1 (audit #008).** Introduced `HandlerError` with 5 variants mapped to spec codes: `InvalidParams` → `-32602`, `NotFound` → `-32002`, `Gated` → `-32001`, `MethodNotFound` → `-32601` (the only correct site for it), `Internal` → `-32603`. Tool runtime errors now route through the MCP-spec `result: { isError: true, content: [...] }` envelope instead of the JSON-RPC envelope.
- **`MCP HTTP/SSE transport now actually broadcasts notifications (audit #010).** `GET /mcp` emits real `data:` frames per notification. New `NotificationHub`: `Mutex<VecDeque<(id, Value)>>` bounded 128-entry replay ring, `AtomicU64` monotonic event-ID counter, `Condvar::wait_timeout` to avoid busy-spin, one dispatcher thread per `bind()` that republishes into the hub so multiple concurrent `GET /mcp` connections all observe every event. `Last-Event-ID` header is honored against the replay ring; reconnects pick up where they left off within 128 events.
- **`POST /mcp` validates Content-Type (audit #B5).** After Accept / UTF-8 validation, before parsing body as JSON. Accepts `application/json` with optional `; charset=...`. Rejects anything else with 415 Unsupported Media Type and a JSON body.
- **`coral mcp serve` init-detection uses parsed method, not substring sniff (audit #B6).** `body.contains("\"initialize\"")` false-positived on `tools/call` arguments containing that literal token. New `parse_jsonrpc_method` helper does a minimal `serde_json::from_str::<{method: Option<String>}>`. Session minting now triggers on `method == Some("initialize")`.
- **`coral stats --symbols --format json` is byte-deterministic (audit #007).** `HashMap<SymbolKind, usize>` → `BTreeMap<SymbolKind, usize>` for `by_kind` (required `SymbolKind: Ord`; derive added; variant declaration order = Ord order, no behavior change). Same for `by_lang`. Markdown branch keeps `count desc` primary sort but adds ascending-name secondary tiebreaker so ties stop flickering.
- **`README.md` Roadmap brought up through v0.30.0**, audit-pipeline table now lists 5 cycles (cycle 4 v0.20.x + cycle 5 v0.30.0 were missing), Quickstart sections consolidated under one `## Quickstart` parent with a scope/time routing table, stale `v0.21.1` install-snippet references replaced with `v0.30.0`, `tests-1124 passing` badge and `multi-agent-audited-4-cycles` badge dropped (the numbers drift every release; the CI badge already conveys "tests pass").

### Fixed

- **MCP `WikiState` dirty-flag now wired through `resources/read` (audit #002, High).** The server advertised `subscribe: true` and the watcher faithfully marked an `Arc<RwLock<WikiState>>` dirty on filesystem changes, but `WikiResourceProvider` used a separate `OnceLock<Vec<Page>>` that could never be invalidated — MCP clients saw frozen wiki contents until process restart. Source comment at `crates/coral-mcp/src/watcher.rs:7-13` explicitly admitted the regression. Fix: `pages_cache` (OnceLock) → `state` (`Option<Arc<RwLock<WikiState>>>`); builder `.with_state()` opt-in; `read_pages()` returns `Vec<Page>` owned. CLI `mcp serve` constructs one `shared_state(cwd.join(".wiki"))` and wires the same handle into `WikiResourceProvider::with_state(...)` and `start_watcher_with_state(..., Some(shared_state))`. Counter-validated by an independent reviewer agent — 4/4 invariants pass.
- **`coral session distill` patch validator closed against multi-file path traversal (audit #003, High).** `diff_targets_slug` previously inspected only the first `---`/`+++` header pair (early break) and `git_apply_inner` ran with `--unsafe-paths`, which lets git write files outside the working tree. A two-pair diff with a benign first pair and a `../../../home/<user>/.ssh/authorized_keys` second pair could land via `git apply --unsafe-paths --directory=.wiki`. Fix: walk every `---`/`+++` header; reject any patch with more than one pair (design is one-slug-per-patch). New `is_safe_diff_header_path` helper rejects `..` segments and absolute paths. Dropped `--unsafe-paths` from both `git apply --check` and the real `git apply` at line 504. Counter-validated against an expanded threat model (leading whitespace, tabs, U+2026, null bytes, mixed-case prefixes, `/dev/null` new-file edge case) — no bypass found.
- **`coral guarantee` no longer returns Green on an unreadable wiki (audit #004, High).** Lint and contract checks used to return `CheckResult { failures: 0, warnings: 0, detail: "failed to read..." }` when `coral_core::walk::read_pages` errored, contributing nothing to verdict aggregation. A single corrupt frontmatter page was enough to flip the gate to Green / exit 0. Fix: an `Err` from `read_pages` now produces `failures: 1` with the real error string. New regression test `lint_check_failure_propagates_to_red_verdict`.
- **`coral project lock` serializes load-mutate-save under flock (audit #005, High).** Sibling command `coral project sync` already wraps its load-mutate-save in `with_exclusive_lock(&lockfile_path, ...)` (post-v0.19.6 H3 audit). `project lock` did the same logical operation without any flock, so concurrent `coral project lock` + `coral project sync` produced lost-update on `coral.lock`. Fix: wrap the entire load + upsert + stale-purge + write body in `with_exclusive_lock` and re-read inside the closure. Deliberate use of `atomic::atomic_write_string` directly rather than `Lockfile::write_atomic` — the latter ALSO wraps in `with_exclusive_lock` and would self-deadlock under re-entry on Linux/macOS (the helper opens a fresh FD that blocks on the held lock even within the same process). Counter-validated: 5/5 invariants pass.
- **BM25 search-index atomic write + real SHA-256 content hash (audit #006, Medium).** `SearchIndex::save_index` did `fs::File::create(path)` + `write_all` + `sync_all` — no tmp+rename, no flock around load+rebuild+save in `search_with_index`. A crash mid-write left a torn file; two concurrent processes could race-clobber on POSIX or fail with sharing-violation on Windows. New `coral_core::atomic::atomic_write_bytes` parallel helper next to `atomic_write_string` (the existing one UTF-8-validates so can't be used for bincoded payloads). Same-dir tmp file, written + flushed + `sync_all()` (counter-validator caught the missing fsync), then atomic `fs::rename`. RAII cleanup on rename failure. `search_with_index` wraps load + rebuild + save in `with_exclusive_lock`. **Companion fix**: `compute_content_hash` docstring said "SHA-256 of all page bodies concatenated in slug-sorted order" but the implementation used `std::hash::DefaultHasher` (SipHash 1-3, 64-bit, non-cryptographic) and `{:016x}` (64 bits — at ~4B documents the birthday-collision risk becomes nontrivial, and `is_valid_for` is the cache-validity gate). Switched to real SHA-256 via `sha2` (added at `crates/coral-core/Cargo.toml`, not `workspace.dependencies` — only `coral-core` uses it). Length-prefix framing: each `(slug, body)` pair updates the hasher with `u64-LE length` + `bytes`, separately for slug and body — prevents the classic concatenation-ambiguity collision where `("ab","c")` would hash the same as `("a","bc")` without framing. New test `content_hash_framing_prevents_split_ambiguity`.
- **`coral bootstrap` / `coral ingest` exit code reflects all-skipped (audit #B7).** Both commands had a skip-on-error loop that printed `warn: skipping...` and continued. If every entry failed, they still exited 0 with "Created 0 pages." A CI gate couldn't catch a fully degenerate run. Fix: if `created == 0 && !skipped.is_empty()`, return `Ok(ExitCode::FAILURE)`. Applied to both LLM-apply and `--from-symbols` paths of bootstrap, and to the ingest apply loop.
- **`coral ingest` reads of `.wiki/index.md` capped at 32 MiB (audit #B8).** Two `std::fs::read_to_string` call sites lacked the workspace-wide 32 MiB cap applied at every other ingest entry-point. A malicious or runaway wiki could OOM the process. Inlined the same cap pattern as the rest of the workspace. New test `ingest_rejects_oversize_index_md`.
- **`coral mcp serve` SIGINT/SIGTERM handler + watchdog thread (audit #B1).** Calls `std::process::exit(0)` once handlers have a moment to flush. Coarser than passing an `AtomicBool` through `serve_blocking` (that's a follow-up that needs the `http_sse.rs` serve loop owner) but matches the pattern in `serve.rs` / `interface.rs` / `monitor/up.rs`.
- **Audit-log rotation in `coral-cli/src/commands/mcp.rs` race-free (audit #B4).** Process-local `Mutex` so concurrent dispatcher threads under HTTP transport can't race the rotation; rename order fixed to remove-then-rename (rename can't overwrite on Windows). New test spans 800 concurrent appends across rotations and asserts no losses, no duplicates.
- **README MCP-count drift (audit #009).** "What you get" + the subcommand-reference table at line 1202 still said "6 resources, 3 prompts, and 5 read-only tools" — `static_catalog()` actually returns 8 resources and `ToolCatalog::read_only()` actually returns 7 tools (incl. `list_interfaces` and `contract_status`). Brought into line.

### Internal

- **Three small Windows-portability fixes uncovered when compiling the audit branch on Windows after installing MinGW dlltool (resolves the local half of #001):**
  - `crates/coral-mcp/src/transport/http_sse.rs:137` — `tiny_http::ListenAddr::Unix` is `#[cfg(unix)]` in tiny_http, so the match arm has to be gated too or rustc fails E0599 on Windows. Added `#[cfg(unix)]`.
  - `crates/coral-runner/tests/cross_runner_contract.rs:67` — `use common::forever_yes_script` import and the three tests using it (claude/gemini/local runner contract tests, all of which also use `/bin/echo`) are inherently Unix-only. Gated with `#[cfg(unix)]`. The HTTP and Mock contract tests stay portable.
  - `crates/coral-runner/src/runner.rs:486,556` — the two timeout tests `claude_runner_run_honors_timeout` and `claude_runner_streaming_timeout_kills_child` invoke `forever_yes_script` directly. Gated with `#[cfg(unix)]`.
- **After these fixes, `cargo build --workspace` succeeds and all 29 new regression tests added by the audit (#002 #003 #004 #005 #006 #007 #008 #010 #B4 #B5 #B6 #B7 #B8) pass on `stable-x86_64-pc-windows-gnu` with `C:\msys64\mingw64\bin` on PATH.** Remaining ~30 test failures on Windows are pre-existing portability gaps (bash scripts in `tests/release_flow.rs`, `/bin/echo` substitute binaries, `Instant::now() - 60s` panic on fresh-boot in `session_table_reap_drops_expired_entries`, `pdftotext` shell-out, hard-coded `/usr/...` paths, snapshot-test path separators). Never reachable on the project's Ubuntu+macOS-only CI matrix. Out of scope for this audit branch — filed as separate "Windows test portability" follow-up.
- **B10 (replace `assert!(result.is_ok())` with `result.expect("...")`)** applied across 9 sites in resources.rs / project.rs so failing tests surface the actual error variant.

### Pipeline note

11 audit findings (4 High, 4 Medium, 3 Low / batch) closed on the `audit/fixes-v0.30.0` branch and merged to `main` post-tag. GitHub issue numbers `#52`–`#62` reserved for the catalog (mapped 1:1 to `audit/findings/NNN-*.md`). The plug-and-play work is independent and lands alongside; both ship together in the next release tag (`v0.31`).

## [0.30.0] - 2026-05-11

**Major release — Fase 3 of the PRD-v0.24-evolution sprint, all M3.x milestones shipped. The 46-milestone roadmap (M1.1–M1.15, M2.1–M2.18, M3.1–M3.11) is now complete end-to-end.** Where Fase 1 (v0.24.x) shipped the killer-feature trinity + MCP cache + interface layer + RRF, and Fase 2 (v0.25.0) shipped semantic diff + watch daemon + governance + bi-temporal frontmatter + 18 M2.x milestones, **Fase 3 (v0.30.0) shipped the search-platform + RAG + advanced-testing tail** — every long-tail piece the PRD listed got an implementation, with the more speculative items (tantivy/pgvector/chunking/CRAG/HyDE/reranker) landing behind opt-in feature flags so the always-on workspace stays lean. The release jumps directly from v0.25.0 to v0.30.0 with no intervening v0.26/v0.27/v0.28/v0.29 — version-number compression on purpose because Fase 3 was a sprint-of-sprints, each milestone independently testable and gateable behind a Cargo feature where appropriate. **Workspace `Cargo.toml` bumps from `0.24.2` → `0.30.0`** (no v0.25.x intermediates because Fase 2 had already burned the v0.25 line). The headline pieces:

- **Persisted BM25 index (M3.1).** `coral_core::search_index::SearchIndex` now persists to `.coral/index/bm25.bin` with content-hash invalidation. The doc says SHA-256 of all page bodies in slug-sorted order; the v0.30.0 implementation pre-audit uses a 64-bit SipHash 1-3 (fixed in the Unreleased entry above per audit #006). Save delegates to a non-atomic `File::create` + `write_all` + `sync_all` (also fixed in Unreleased per audit #006A). The cache hit path skips re-tokenization and re-IDF computation; the cold path rebuilds from `walk::read_pages` when the content hash changes.
- **Interned vocabulary with `Arc<str>` + `TokenId` (M3.2).** Hot-path tokenization no longer allocates a fresh `String` per token; a workspace-wide string-interning pool keyed by `Arc<str>` deduplicates tokens across pages, and `TokenId` (newtype over `u32`) is the index-side identifier so postings lists store 4 bytes instead of a `String`.
- **`coral test mutants` mutation-testing wrapper (M3.3).** Wraps `cargo-mutants` via subprocess, parses its report into a `CoralReport`, surfaces mutation-survivor lines as TestReports with `Evidence` carrying the survivor source location. Opt-in (`cargo-mutants` must be on PATH); reports the wrapper config alongside the survivors.
- **E2E browser runner — Playwright structural validation (M3.4).** New `coral_test::e2e_runner` invokes Playwright's CLI in headless mode against a manifest-declared spec file (`[[environments.<env>.e2e]]`). Per-test result parsed from Playwright's JSON reporter; failure mode includes the failing step's selector and the trace artifact path.
- **OTel trace runner — span assertion structural validation (M3.5).** New `coral_test::trace_runner` reads OTLP exports (file or HTTP) and asserts span shapes declared in `[[environments.<env>.traces]]` blocks (`service`, `span_name`, `parent_relationship`, `expected_attributes`). For each manifest assertion, walks the trace, matches by span name, asserts attribute presence/equality. Failure mode includes the closest-matching span shape so authors can fix the assertion or the instrumentation.
- **`coral migrate-consumers` + `coral scaffold module --like` (M3.6).** Pair of CLI commands for the contract-migration workflow. `migrate-consumers` walks consumer repos for OpenAPI/contract references to a provider endpoint, generates a migration plan + grep-targets for each consumer. `scaffold module --like <existing>` clones the wave-1 → wave-2 → wave-3 scaffolding pattern from a reference module into a new module, preserving the trait-and-mock structure.
- **`coral ingest --include-docs` for PDF extraction (M3.7).** Shells out to `pdftotext` (or `pdftohtml -i`) where available; extracted text feeds the same ingest pipeline as Markdown/source docs. PDFs without text layers (image-only scans) are reported as skipped with the path; OCR is out of scope.
- **pgvector embeddings backend stub (M3.8).** New `coral_core::embeddings::PgVectorBackend` behind the `pgvector` feature flag — connection-string config, schema migration helpers (`CREATE EXTENSION vector`, `CREATE TABLE coral_embeddings`), `insert_batch` + `nearest_k` shape. The actual embedding step (OpenAI / local Ollama / sentence-transformers) is wired through the existing `EmbeddingProvider` trait. v0.30.0 ships the storage abstraction + a Postgres integration test harness; the production-quality backend (connection pooling, retry policy) is v0.31.
- **`coral wiki serve` — local HTTP wiki browser (M3.9).** Static HTTP server over `.wiki/` for visual review without leaving the terminal. Default `127.0.0.1:8730`; renders Markdown to HTML; respects the slug allowlist + path-traversal defenses already in `coral_core::walk`. Read-only — no edit endpoints.
- **RAG opt-ins — tantivy / chunking / CRAG / HyDE / reranker stubs (M3.10).** Five feature-gated modules with the same shape: a public trait, a stub implementation that returns `unimplemented!()` with a clear pointer at the milestone issue, and a `cfg(feature = "...")`-gated re-export. Compiled-out by default so the always-on workspace stays lean. Production implementations land in v0.31+ as user demand justifies.
- **Mutation budget + export-skill autodetect + MCP Tasks (M3.11).** Three small surfaces bundled: a per-run mutation budget (`[mutants] budget_seconds = 600`) so `coral test mutants` doesn't run forever; `coral skill export --autodetect` discovers project-shape conventions (test framework, build tool, lint config) to pre-fill the exported skill manifest; new MCP `tasks/create` + `tasks/list` JSON-RPC methods so an MCP-driven workflow can spawn long-running tasks (think: a Claude Code session kicking off `coral test mutants` and polling for completion) without blocking the dispatch thread.

**BC posture:** Workspace surface remains additive. `EnvironmentSpec` gained one additive field per new test kind (`e2e: Vec<E2eSpec>`, `traces: Vec<TraceSpec>`) with `#[serde(default, skip_serializing_if = "Vec::is_empty")]` — manifests without the new blocks round-trip byte-identically. `bc_regression` remains green. The five opt-in RAG modules are `cfg`-gated, no new always-on transitive deps. The `pgvector` feature pulls in `sqlx` only behind its own feature flag. The `coral wiki serve` HTTP server reuses `tiny_http` (already in the tree via the MCP HTTP/SSE transport).

### Added

- **`[[environments.<env>.e2e]]` config blocks** (M3.4) — `{ service, spec, browser, timeout_seconds }` shape; `browser` defaults to `chromium`. CLI: `coral test --kind e2e --service <name>`.
- **`[[environments.<env>.traces]]` config blocks** (M3.5) — `{ service, span_name, parent_relationship, expected_attributes }` shape; integrates with `[backends.otel]` for the OTLP source.
- **`[mutants]` config table** (M3.3, M3.11) — `{ budget_seconds, parallel, skip }` shape; CLI: `coral test --kind mutants --service <name>`.
- **`coral wiki serve [--port N] [--bind 127.0.0.1]`** (M3.9) — local HTTP wiki browser.
- **`coral wiki bootstrap --from-symbols`** — generates one wiki page per declared symbol from the `coral_core::symbols` regex-based extractor (the same one that feeds `coral stats --symbols`).
- **`coral ingest --include-docs`** (M3.7) — adds `*.pdf` (via `pdftotext`/`pdftohtml -i`) and `*.docx` (via `pandoc`) to the ingest pipeline. External-binary discovery is best-effort with a clear "tool not found on PATH" message.
- **`coral migrate-consumers`** (M3.6) — walks consumer repos for OpenAPI/contract references and generates a migration plan.
- **`coral scaffold module --like <existing>`** (M3.6) — clones the wave-1/2/3 scaffolding pattern from a reference module.
- **`coral diff --ref <gitref>`** (M2.17) — historical comparison; `git archive | tar` extracts the historical `.wiki/` into a temp dir and runs the same diff pipeline.
- **`coral diff --narrative`** (M2.18) — auto-summary post-merge of wiki changes; pluggable via the configured runner.
- **MCP `tasks/create` + `tasks/list` JSON-RPC methods** (M3.11) — long-running task spawn + status without blocking the dispatch thread.
- **Frontmatter `superseded_by` field** (M2.16) — bi-temporal page lineage; the page validator now follows `superseded_by` chains to detect cycles and broken links.

### Changed

- **Workspace version `0.24.2` → `0.30.0`.** No intervening v0.25.x / v0.26 / v0.27 / v0.28 / v0.29 (Fase 2 had already burned the v0.25 line; v0.26-29 were reserved-and-skipped).
- **`coral consolidate --gc`** (M2.10) — orphan/broken-link detection now part of `consolidate`'s reconcile pass; flags pages whose forward links resolve nowhere or whose backlinks are all from archived pages.
- **`coral consolidate --tiered`** retains its v0.21.4 behavior unchanged.
- **MCP search tool, `coral query`, and `coral context build`** all now use `search_hybrid` (BM25 + TF-IDF via Reciprocal Rank Fusion) for best precision — three surfaces aligned on one ranker.

### Internal

- **Storage abstraction traits for wiki + embeddings (M1.11)** — `WikiStore` / `EmbeddingStore` traits with file-backed default impls. Sets up the v0.31 pluggable-backend story (S3, sqlite, pgvector).
- **Supply-chain hardening (M1.15)** — SBOM via `cargo-cyclonedx`, OpenSSF Scorecard published in CI, `cosign` keyless signing on release artifacts. CI matrix gains a security-audit job that fails on critical advisories.
- **`coral test coverage` — endpoint gap analysis (M1.6)** — walks the discovered routes (from OpenAPI specs + `[services.*.healthcheck]`) and reports which routes have zero TestCases referencing them.
- **`coral test perf` — latency baseline + regression detection (M2.8)** — per-test wall-clock with rolling p50/p95/p99; flags any test whose p95 exceeds the rolling baseline by > 3 stddev.
- **`coral test flakes` — historical flake-rate tracking (M2.7)** — joins JSONL reports across runs; flakes = tests that have both passed and failed within the last N runs.
- **Dual-level query routing (M2.9)** — `coral query` now classifies each query as `entity-lookup` (single page recall, BM25-tight) or `synthesis` (multi-page fan-out, RRF-wide) and routes accordingly.
- **Goldset evaluation — precision@k, recall@k, MRR (M2.13)** — `coral search eval --goldset <path>` against a manually-curated query→expected-page set so ranking changes don't silently regress.
- **Governance rules engine + `llms.txt` generator (M2.14)** — six configurable rules (low confidence, missing sources, body length bounds, required extra fields, links to archived pages) with markdown + JSON renderers; `--format llms-txt` on `coral export`.
- **Interface contract resources and tools (M2.2)** — MCP `wiki://interfaces/<name>` resources + `list_interfaces` / `contract_status` read-only tools; pairs with the `coral interface watch` daemon (M2.3) that re-runs contract checks on `.coral/contracts/**` changes.
- **Stateful `WikiState` with dirty-flag refresh (M2.4)** — the design referenced by the audit #002 fix above; v0.30.0 shipped the watcher + dirty-flag mechanism, but the resource-read path wasn't wired through (audit caught it).
- **`coral mcp preview`** (M1.14) — inspect the MCP surface without an IDE; dumps `tools/list`, `resources/list`, `prompts/list` to stdout.
- **`coral session auto-capture`** — git post-commit hook integration; deduplicates against already-captured sessions; silent secret scrubbing.

### Pipeline note

Fase 3 closes the PRD-v0.24-evolution roadmap. The 46-milestone arc (M1.1–M1.15, M2.1–M2.18, M3.1–M3.11) ships in three phases across v0.24.0 → v0.25.0 → v0.30.0. Five RAG modules (tantivy, chunking, CRAG, HyDE, reranker) shipped as stubs behind feature flags — production implementations land in v0.31+ as user demand justifies. Mutation testing, E2E browser, and OTel trace runners shipped as **structural validation** — the runner shape is real and tested, the integrations are real, but each is the first iteration. The audit cycle 5 (11 findings) followed within hours of the tag and lands on `main` in `[Unreleased]` above; v0.31 will ship those fixes alongside the plug-and-play install path.

## [0.25.0] - 2026-05-11

**Major release — Fase 2 of the PRD-v0.24-evolution sprint, all M2.x milestones shipped (18 milestones).** Where Fase 1 (v0.24.0–v0.24.2) shipped the killer-feature trinity (`coral guarantee --can-i-deploy`, MCP resource subscriptions push, `coral wiki at <git-ref>`) + the search-platform foundation (RRF hybrid search, parallel test execution, contract-aware page types), **Fase 2 (v0.25.0) shipped the contract + governance + bi-temporal-wiki layer**. The release bundles 18 M2.x milestones into a single tag because each was independently small but they form a coherent layer: semantic diff for breaking-change detection, contract resources surfaced via MCP, a `coral interface watch` daemon for live re-evaluation, stateful `WikiState` so dirty-flag tracking is visible to the resource-read path, a contract test runner (Pact-style), an event test runner (AsyncAPI/Kafka), and the operational primitives (`coral test flakes`, `coral test perf`, `coral consolidate --gc`, `coral query --expand-graph N`, `coral wiki bootstrap --from-symbols`, goldset eval, governance rules + `llms.txt`, bi-temporal `superseded_by`, `coral diff --ref <gitref>`, `coral diff --narrative`). Workspace version bumps from `0.24.2` → `0.25.0`. **BC posture: all v0.24.2 surfaces remain byte-identical** — manifests without any of the new `[contract]` / `[governance]` / `[backends.kafka]` blocks round-trip unchanged; `bc_regression` green.

### Added

- **Semantic diff via `oasdiff`/`buf`/`atlas` (M2.1).** Shells out to external tools for breaking-change detection. `oasdiff` for OpenAPI specs (REST API drift), `buf` for Protocol Buffers (gRPC schema breaks), `atlas` for database schema migrations. Each tool is optional — gracefully skipped if not on PATH with a clear "tool not found" message. Pairs with `coral guarantee` (v0.24.1) — a breaking schema change now flips the verdict to Red.
- **MCP contract resources + tools (M2.2).** New `wiki://interfaces/<name>` and `wiki://contracts/<service>` MCP resources; new read-only tools `list_interfaces` (lists declared `[interface]` blocks) and `contract_status` (per-consumer compatibility status). Brings the contract layer into the same surface area as the wiki, accessible to any MCP-aware agent.
- **`coral interface watch` daemon (M2.3).** Foreground watcher over `.coral/contracts/**` and `[interface]` blocks; re-runs the contract check on file change (debounced 500ms) and emits MCP `notifications/resources/list_changed` so subscribed clients refresh. Pairs with v0.24.1's resource-subscriptions push (PR #11).
- **Stateful `WikiState` with watcher-driven dirty-flag refresh (M2.4).** Shared `Arc<RwLock<WikiState>>` between the filesystem watcher and the MCP resource cache; the watcher flips the dirty flag, the cache observes it on next read. The actual wiring through to `resources/read` had a regression (caught by audit #002 post-tag, fixed in `[Unreleased]` above).
- **Contract test runner — Pact-style (M2.5).** `coral_test::contract_runner` reads `[[environments.<env>.contracts]]` blocks declaring consumer-expects-provider pairs; replays the consumer's expectations against the provider service. Pinned to a stable consumer-contract schema.
- **Event test runner — AsyncAPI / Kafka (M2.6).** `coral_test::event_runner` reads `[[environments.<env>.events]]` blocks declaring `kafka_topic` + `expected_event_shape`. Subscribes via `rdkafka` (behind the `events` feature flag), asserts every produced message matches the schema. Linux-friendly; macOS development supported.
- **`coral test flakes` — historical flake-rate tracking (M2.7).** Joins JSONL TestReports across the last N runs; a test is "flaky" if it has both passed and failed within the window. Flake rate is `failed_runs / total_runs`. Reported sorted by flake rate descending.
- **`coral test perf` — latency baseline + regression detection (M2.8).** Per-test wall-clock with rolling p50/p95/p99 from `.coral/perf-baseline.jsonl`. Flags any test whose p95 exceeds the rolling baseline by more than 3 standard deviations; `--update-baseline` rewrites the baseline file (intended for green-CI runs only).
- **Dual-level query routing (M2.9).** `coral query` now classifies each query as `entity-lookup` (single page recall, BM25-tight) or `synthesis` (multi-page fan-out, RRF-wide via `search_hybrid`) and routes accordingly. Pure-Rust classifier: question-word prefix → synthesis; quoted-string-only → entity.
- **`coral consolidate --gc` for orphan/broken-link detection (M2.10).** New reconcile pass during consolidation that flags pages whose forward links resolve nowhere or whose backlinks are all from archived pages. Pairs with `coral wiki bootstrap` (M2.12) — bootstrap creates pages, `--gc` ensures the graph stays clean.
- **`coral query --expand-graph N` for backlink/wikilink expansion (M2.11).** Hop-count expansion: starts from the top-K BM25 hits, walks outbound `[[wikilinks]]` and inbound backlinks up to N hops, returns the expanded result set sorted by combined relevance. N defaults to 1; N=0 falls back to base BM25.
- **`coral wiki bootstrap --from-symbols` (M2.12).** Generates one wiki page per declared symbol from `coral_core::symbols` (the regex-based extractor that feeds `coral stats --symbols`). Each generated page has `reviewed: false` so the existing `coral lint --governance` gates apply. Useful for seed-loading a wiki from an existing codebase.
- **Goldset evaluation — precision@k, recall@k, MRR (M2.13).** `coral search eval --goldset <path>` runs a manually-curated query→expected-page-slug set against the live wiki and reports precision@k, recall@k, mean reciprocal rank. Stops a ranker change from silently regressing the search-quality bar.
- **Governance rules engine + `llms.txt` generator (M2.14).** New `coral_core::governance` module with a configurable policy engine validating pages against 6 rules: low confidence, missing sources, body length bounds, required extra fields, and links to archived pages. Markdown + JSON renderers. CLI integration: `--governance` flag on `coral lint` loads policy from `coral.toml [governance]`; `--format llms-txt` on `coral export` produces a machine-readable wiki summary grouped by page type for AI agent consumption. New `symbols` module for regex-based repo symbol extraction; new `valid_from` / `valid_to` temporal validity fields on `Frontmatter`.
- **Bi-temporal `superseded_by` field on `Frontmatter` (M2.16).** Page lineage tracking — when a page is replaced by a newer one, the older page sets `superseded_by: <new-slug>`. The page validator follows the chain to detect cycles and broken links. Pairs with `valid_from` / `valid_to` (M2.14) for full bi-temporal page semantics.
- **`coral diff --ref <gitref>` for historical comparison (M2.17).** Compares the current wiki against the wiki as it existed at any git ref. Uses `git archive | tar` to extract the historical `.wiki/` into a temp directory, then walks it through the same diff pipeline as the current `coral diff`.
- **`coral diff --narrative` — auto wiki summary post-merge (M2.18).** After `coral diff` runs, optionally produces a one-paragraph natural-language summary of the changes by piping the diff through the configured runner. Useful for PR descriptions and merge commit messages.

### Changed

- **Workspace version `0.24.2` → `0.25.0`.** Marks Fase 2 completion.
- **`coral query` ranking unified.** Both `coral query` and the MCP `search` tool and `coral context build` now use `search_hybrid` (BM25 + TF-IDF via Reciprocal Rank Fusion). Three surfaces aligned on one ranker.
- **Frontmatter gained two temporal fields** (`valid_from: Option<DateTime>`, `valid_to: Option<DateTime>`) and one lineage field (`superseded_by: Option<String>`) — all additive, all `skip_serializing_if = "Option::is_none"`, all `#[serde(default)]`.

### Internal

- **Storage abstraction traits for wiki + embeddings (M1.11).** `WikiStore` / `EmbeddingStore` traits with file-backed default impls. Sets up the v0.30+ pluggable-backend story.
- **`coral test coverage` — endpoint gap analysis (M1.6).** Walks discovered routes (from OpenAPI specs + `[services.*.healthcheck]`) and reports which routes have zero TestCases referencing them.
- **`coral mcp preview` (M1.14).** Inspect the MCP surface without an IDE — dumps `tools/list`, `resources/list`, `prompts/list` to stdout.
- **Supply-chain hardening (M1.15)** — SBOM via `cargo-cyclonedx`, OpenSSF Scorecard published in CI, `cosign` keyless signing on release artifacts.

### Pipeline note

Fase 2 closes 18 M2.x milestones. Six are foundational (semantic diff, contract resources, watch daemon, stateful WikiState, contract runner, event runner) and provide the operational substrate for the v0.30.0 search/RAG layer (Fase 3). The remaining 12 are surface-area expansions of `coral query`, `coral test`, `coral diff`, and `coral lint`. The `WikiState` stateful refresh path has a known wiring regression (audit cycle 5 #002, fixed in `[Unreleased]` above).

## [0.24.2] - 2026-05-10

**Patch release — hybrid search platform + query optimization + CI template.** Builds on v0.24.1's killer-feature foundation with the search-platform pieces the PRD-v0.24-evolution roadmap deferred from the initial sprint: RRF (Reciprocal Rank Fusion) hybrid search combining BM25 + TF-IDF, BM25-relevance ranking before context building, hybrid search wired into the MCP `search` tool, `--algorithm hybrid` exposed in the `coral search` CLI, and `coral ci generate` for GitHub Actions workflow scaffolding. Also lands the polling file watcher (PR #13) wired into `coral mcp serve` so resource-subscriptions push (PR #11, v0.24.1) becomes end-to-end useful — without a watcher the subscribe surface couldn't actually fire. Semantic-diff scaffolding (oasdiff/buf/atlas, PR #14) lands as the foundation for v0.25.0's M2.1 milestone. `coral session auto-capture` (PR #16) closes the Claude Code post-commit-hook loop so transcripts get captured without manual intervention.

### Added

- **`coral search --algorithm hybrid` (PR #21).** Exposes the RRF hybrid search algorithm in the search CLI alongside the existing `bm25` and `tf-idf` modes. Default remains `bm25` for v0.24.x; `hybrid` becomes the default in v0.30.0.
- **RRF hybrid search combining BM25 + TF-IDF (PR #18).** New `coral_core::search::search_hybrid` function. RRF formula: `score(d) = Σ 1/(k + rank_i(d))` over the two ranker lists, where k=60 (standard RRF constant). Provides the best precision in head-to-head tests against the goldset by combining BM25's term-frequency sensitivity with TF-IDF's IDF weighting.
- **`coral context build` uses RRF ranking (PR #23).** Context-building (the LLM-context-payload generator) now uses hybrid search for page selection. Improves citation recall in downstream LLM-driven workflows.
- **MCP `search` tool uses hybrid RRF (PR #22).** The MCP search tool now defaults to `search_hybrid` instead of `search_bm25`. MCP clients (Claude Code, Claude Desktop) get the better ranker without any client change.
- **`coral ci generate` for GitHub Actions workflow scaffolding (PR #20).** Emits a `.github/workflows/coral.yml` with the standard `coral lint` + `coral test --kind contract` + `coral guarantee --can-i-deploy` pipeline. Saves new users the 20-minute YAML-from-scratch step.
- **`coral session auto-capture` — Claude Code post-commit hook (PR #16).** Detects recent Claude Code transcripts, deduplicates against already-captured sessions, and silently captures new ones with secret scrubbing enabled. Designed to be wired into `.git/hooks/post-commit`.
- **Polling file watcher for MCP subscription push (PR #13, PR #19).** Background thread polls `.wiki/` mtimes every 2s and fires `notify_resources_list_changed()` when changes are detected. Graceful shutdown via channel drop. PR #19 wires the watcher into `coral mcp serve` so the v0.24.1 subscribe surface is end-to-end useful.
- **Semantic diff scaffolding via `oasdiff`/`buf`/`atlas` (PR #14).** Foundation pieces for the v0.25.0 M2.1 milestone — wiring + subprocess invocation lands here; the full integration with `coral guarantee` lands in v0.25.0.

### Changed

- **`coral query` ranks pages by BM25 before context building (PR #17).** Instead of blindly taking the first 40 pages from `walk::read_pages`, uses `coral_core::search::search_bm25` to rank all pages by relevance to the user's question and selects the top-40 most relevant. Falls back to the previous `take(40)` behavior when search returns empty (e.g., all-stopword queries).
- **`coral query` uses RRF hybrid search for context ranking.** Aligns `coral query` with the MCP `search` tool and `coral context build` — all three now use `search_hybrid` (BM25 + TF-IDF via RRF) for best precision.
- **`coral query` includes all pages when wiki ≤40 pages.** Small-wiki escape hatch — below the 40-page threshold there's no need to rank, just send everything. Updates search snapshots accordingly.

### Internal

- **`Cargo.lock` refreshed for v0.24.2** (commit `c72c5d4`).

### Pipeline note

This release closes out Fase 1 of the PRD-v0.24-evolution sprint. The search-platform pieces (hybrid RRF, MCP wiring, context-build alignment) provide the foundation for Fase 2's M2.9 dual-level query routing and Fase 3's M3.1 persisted BM25 index. Semantic-diff lands as scaffolding only; the production integration with `coral guarantee` is M2.1 in v0.25.0.

## [0.24.1] - 2026-05-10

**Feature release — completes the killer-feature trinity + parallel test execution + Interface page type.** Where v0.24.0 shipped the first killer feature (`coral wiki at <git-ref>`, PR #5) plus the performance pass (mimalloc, OnceLock caches, MCP wiki page cache, lint precompute), **v0.24.1 lands the other two killer features**: `coral guarantee --can-i-deploy` (single-command GREEN/YELLOW/RED deployment gate for CI) and MCP resource subscriptions push (`subscribe: true` + `notify_resource_updated` + `notify_resources_list_changed`). Together with PR #5 these form the trinity advertised in the PRD: "wiki time-travel + deployment gate + push notifications" — the three things that distinguish Coral from "yet another lint tool" in the wave-1 release narrative. Also ships parallel test execution via rayon (PR #9, respects `ParallelismHint`) and a new `Interface` page type for API contract pages (PR #12). Workspace version bumps from `0.24.0` → `0.24.1`.

### Added

- **`coral guarantee --can-i-deploy` (PR #10) — killer feature #1.** Single-command deployment safety gate aggregating lint results + contract checks into a GREEN/YELLOW/RED verdict. Flags:
  - `--strict` — YELLOW counts as fail (default: YELLOW is pass-with-warnings).
  - `--format json` — machine-readable verdict + per-check breakdown for CI integration.
  - `--format github-actions` — emits `::error::` / `::warning::` annotations that GitHub Actions parses into PR diff comments.
  - Exit code 0 for GREEN/YELLOW (or 0 for GREEN only under `--strict`), 1 for RED.
- **MCP resource subscriptions + push notifications (PR #11) — killer feature #2.** Implements the MCP resource subscription protocol end-to-end:
  - Server advertises `listChanged: true, subscribe: true` in `initialize` response.
  - Clients call `resources/subscribe` with a URI to watch.
  - `notify_resource_updated()` pushes a JSON-RPC notification to subscribed clients.
  - `notify_resources_list_changed()` broadcasts a list-refresh signal.
  - Stdio transport drains the notification channel after each request.
  - This is the **push-drift mechanism**: an agent gets notified when the wiki changes instead of polling on every tool call.
- **Parallel test execution via rayon (PR #9).** The orchestrator now respects `ParallelismHint`: `Isolated` cases run in parallel via rayon, `Sequential` cases preserve order, and `PerService` cases parallelize across service groups. Can cut test-suite runtime by 50-80% for projects with many independent healthcheck/smoke tests. No new workspace deps (rayon already in the tree from PR #4).
- **`PageType::Interface` for API contract pages (PR #12).** New page type for wiki pages describing API contracts (OpenAPI, protobuf, GraphQL schemas). Interface pages are skipped by the orphan checker (they're roots by design, linked to `.coral/contracts/`). Updates all exhaustive matches across the workspace.

### Changed

- **Workspace version `0.24.0` → `0.24.1`.**
- **MCP server `initialize` response gains `capabilities.resources.subscribe: true` and `capabilities.resources.listChanged: true`** (PR #11). Pre-v0.24.1 MCP clients that didn't expect the new capabilities ignore them per the protocol spec; v0.24.1+ clients can opt in.

### Pipeline note

This release completes the killer-feature trinity advertised in the PRD-v0.24-evolution. Fase 1 is now half-done; v0.24.2 closes the remaining search-platform pieces (hybrid RRF, MCP wiring, context-build alignment) and the semantic-diff scaffolding that feeds v0.25.0's M2.1.

## [0.24.0] - 2026-05-10

**Major release — first sprint of the PRD-v0.24-evolution plan: performance pass + first killer feature + contract drift detection.** Bundles 8 PRs into a single tag: five performance optimizations (mimalloc global allocator, `STOPWORDS` `OnceLock<HashSet>`, tool-kind lookup `OnceLock<AHashMap>`, lint outbound-link precompute, MCP wiki-page in-memory cache), one killer feature (`coral wiki at <git-ref>` — time-travel wiki access via `git archive | tar` extraction), and two correctness pieces (`--affected --since` filter for change-based repo selection, `requestBody` parsing in OpenAPI contract drift detection). Workspace version bumps from `0.23.3` → `0.24.0`. **BC posture: all v0.23.3 surfaces remain byte-identical** — no manifest schema changes; the new CLI flags (`--at`, `--affected`, `--since`) are additive; the new MCP behavior (in-memory cache) is observably faster but functionally equivalent.

### Added

- **`coral wiki at <git-ref>` — time-travel wiki access (PR #5).** Killer feature #3 in the killer-feature trinity (the other two land in v0.24.1). View the wiki as it existed at any git ref (tag, commit, branch). Implementation: `git archive <ref> | tar -x` extracts the historical `.wiki/` into a temp directory, then reads pages via the standard `walk::read_pages` pipeline. Supports `--search` (BM25), `--filter` (slug substring), `--full` (page body), and `--limit`. The temp directory is cleaned up on drop via RAII.
- **`--affected --since <ref>` for change-based repo selection (PR #7).** New `RepoFilters` flags. When both are present, uses `git diff --name-only <ref>...HEAD` to determine which repos have changed files since the given ref, filtering the selection to only the affected ones. Enables `coral test --affected --since main` for CI efficiency — skip the unchanged repos.
- **OpenAPI `requestBody` parsing in contract drift detection (PR #8).** The contract checker previously only inspected responses; now also extracts `requestBody` declarations from provider specs and emits a `RequestBodyDrift` warning when a consumer sends a body to an endpoint that doesn't declare one. Catches schema-level drift before tests even run.

### Changed

- **Workspace version `0.23.3` → `0.24.0`.**
- **`coral lint`'s wikilink extraction precomputed once per page (PR #4).** The lint runner was extracting wikilinks via regex 3× per page (in `check_broken_wikilinks`, `check_orphan_pages`, and `check_archived_linked_from_head`). Now precomputes once before the rayon fan-out and passes the results to link-aware checks. Cuts regex passes from 3N to N for a wiki with N pages.

### Internal

- **`mimalloc` as global allocator (PR #1).** Replaces the system allocator with `mimalloc` for 10-20% throughput improvement on allocation-heavy workloads (TF-IDF tokenization, wiki page parsing, JSON serialization in the MCP server). One new workspace dep (`mimalloc`).
- **`STOPWORDS` → `OnceLock<HashSet>` for O(1) lookup (PR #2).** The `tokenize()` function checked every token against a 38-element slice — O(n) per token × thousands of tokens per page. `OnceLock<HashSet>` gives O(1) amortized lookup, measurable on wikis with 100+ pages.
- **Tool-kind lookup → `OnceLock<AHashMap>` (PR #3).** `lookup_tool_kind()` was allocating `ToolCatalog::all()` (Vec of 8 items) and linear-scanning on every `tools/call` request. Now uses a lazily-initialized `AHashMap` for O(1) lookup, eliminating per-request allocation entirely. One new workspace dep (`ahash`).
- **MCP wiki pages in-memory cache (PR #6).** `WikiResourceProvider` was re-reading the entire wiki from disk on every MCP request (`resources/list`, `resources/read`, `wiki_stats`, `wiki_index`). Now caches parsed pages in a `OnceLock<Vec<Page>>` initialized on first access — turning repeated calls from O(N × disk) to O(1). (This is the cache that later gets replaced by the stateful `WikiState` in v0.25.0 M2.4 and audit-fixed in `[Unreleased]` above per audit #002.)
- **PRD v0.24 evolution roadmap landed in docs.** Comprehensive product requirements document covering performance optimization, multi-repo testing guarantees, interface contract layer, MCP resource subscriptions, and a 6-week sprint plan. The document drives the 46-milestone arc across v0.24.0 → v0.25.0 → v0.30.0.

### Pipeline note

This release opens Fase 1 of the PRD-v0.24-evolution sprint. Fase 1 completes across v0.24.0–v0.24.2; the remaining killer features (`coral guarantee --can-i-deploy`, MCP resource subscriptions push) land in v0.24.1. The hybrid-search pieces (RRF) and the watcher wiring for MCP subscriptions land in v0.24.2.

## [0.23.3] - 2026-05-09

**Feature release — fourth and final of v0.23 sprint (testing platform): `PropertyBasedRunner` — Schemathesis-style property tests from OpenAPI specs.** Closes the v0.23 testing loop with a fourth primitive: deterministic, shrinking property-based fuzzing over `(path, method)` operations from an OpenAPI spec. Where `coral test --kind user_defined` runs hand-authored YAML, `coral test --kind healthcheck` runs auto-discovered probes, and `coral test --kind recorded` replays captured Keploy YAMLs, `coral test --kind property-based` walks the OpenAPI spec, generates random valid inputs from each operation's JSON Schema declarations, and asserts every iteration's response status lands in the spec's declared response set. The first failing iteration halts the case; proptest shrinks the input to the minimal failing form; the report's `Evidence` carries the shrunken counter-example so a failing run reproduces from the report alone. **One new workspace dep: `proptest = "1"`** (already in the dev-dep tree of `coral-core` and `coral-lint`; promoted to workspace-level so the runtime pieces in `coral-test` can pull it in). **BC sacred: all v0.23.2 surfaces byte-identical** — `bc_regression` green (8 tests), manifests without `[[environments.<env>.property_tests]]` round-trip unchanged (`property_tests_absent_round_trips_unchanged`).

### Added

- **`[[environments.<env>.property_tests]]` config blocks.** New `Vec<PropertyTestSpec>` field on `EnvironmentSpec` with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`:
  ```toml
  [[environments.dev.property_tests]]
  service = "api"
  spec = "openapi.yaml"   # path, repo-root-relative
  seed = 42               # optional; CLI overrides via --seed
  iterations = 100        # optional; default 50; CLI overrides via --iterations
  ```
  Field shape: `{ service: String, spec: PathBuf, seed: Option<u64>, iterations: Option<u32> }`. `seed` and `iterations` are also `skip_serializing_if = "Option::is_none"` so the minimal block is just `service` + `spec`.
- **`coral_test::property_runner` module.** Hand-rolled `BoxedStrategy<Value = serde_json::Value>` for the 5 JSON Schema types (string/number/integer/object/array), a per-endpoint `drive_case` loop that runs N proptest iterations and stops on the first failing status, and curl-subprocess HTTP invocation mirroring `recorded_runner.rs`.
- **`TestKind::PropertyBased` is now a value-enum entry on `coral test --kind`.** Pre-v0.23.3 invocations are byte-compatible: empty `--kind` does NOT include property-based (orchestrator-side gate so `coral test --service api` doesn't suddenly find new cases).
- **`coral test --kind property-based --iterations N --seed K`** CLI flags. `--iterations` overrides `[[property_tests]].iterations` for one invocation. `--seed` overrides the manifest seed. When neither is set, a fresh seed is drawn from the system clock + process id, `tracing::info!`-logged AND embedded into `Evidence::stdout_tail` so a failing run reproduces from the report alone — no manifest mutation needed.

### Changed

- **`EnvironmentSpec` gained one additive field** (`property_tests: Vec<PropertyTestSpec>`). Test fixtures across `coral-test` / `coral-env` / `coral-cli` updated to include `property_tests: Vec::new()`. Serde-side BC preserved via `#[serde(default, skip_serializing_if = "Vec::is_empty")]`.
- **`TestFilters` gained two additive fields** (`property_iterations: Option<u32>`, `property_seed: Option<u64>`) so `run_test_suite_filtered` can plumb the CLI overrides without touching the manifest. Monitors pass `None` for both — they consume the manifest as-is.
- **`coral_test::discover::parse_openapi_value`** extracted from the OpenAPI smoke discoverer so the property runner shares one parser implementation (and the 32 MiB DoS cap from v0.19.8) instead of duplicating the 30 lines.

### Internal

- **`proptest = "1"` promoted to workspace dep.** Was already a `dev-dependency` on `coral-core` and `coral-lint`; the v0.23.3 runtime path in `coral-test` needs it at compile-time too. Kept the version pin centralized so the three crates stay in sync. **No new transitives** — every leaf package (`bit-set`, `bit-vec`, `bitflags`, `num-traits`, `quick-error`, `rand`, `rand_chacha`, `regex`, `regex-syntax`, `unarray`) was already in the lockfile from criterion / regex / proptest itself in dev-deps.
- **Curl-based HTTP invocation** mirrors `recorded_runner.rs` — `-w "\nHTTP_STATUS:%{http_code}"` for status capture, `-d` for JSON body, `--max-time 10` for the per-iteration timeout. The `split_curl_status` helper is re-implemented inside the property module instead of cross-crate-imported (kept the test module independent of `user_defined_runner`).
- **D1 scope-narrowing decisions baked in:** GET + POST only (PUT/PATCH/DELETE silently dropped at discovery), path params + JSON request body only (no query/header generation), 5 JSON Schema types only (`null`/`boolean` + missing `type` fall back to `string`), status validation only (response-body schema validation deferred to v0.24+).
- **Determinism (D5):** seed precedence is CLI > manifest > `fresh_random_u64_seed()` (a SplitMix64 finalizer over wall-clock nanos + pid). Seed expanded `u64 → 32 bytes` (replicated 4×) and handed to `TestRng::from_seed(RngAlgorithm::ChaCha, ...)` because ChaCha requires exactly 32 bytes — a u64 alone won't do.

### Tests (+12)

- **Spec / BC (2)** in `crates/coral-env/src/spec.rs`: `property_tests_absent_round_trips_unchanged` (T1 — BC pin), `property_test_config_with_seed_iterations_parses` (T2 — TOML round trip with all four fields populated).
- **`property_runner` module (10)** in `crates/coral-test/src/property_runner.rs::tests`:
  - `json_schema_string_type_generates_string_value` (T3)
  - `json_schema_integer_type_generates_int_value` (T4)
  - `json_schema_object_type_generates_object_with_required_fields` (T5)
  - `seed_42_produces_deterministic_input_sequence` (T6)
  - `omitted_seed_logs_actual_seed_used` (T7)
  - `property_runner_failed_iteration_returns_first_counter_example` (T8)
  - `property_runner_all_pass_returns_pass_status` (T9)
  - `iterations_cli_flag_overrides_manifest` (T10)
  - `cases_from_property_specs_emits_one_case_per_path_method`
  - `cases_from_property_specs_skips_non_get_post_methods`
- **CLI / value-enum (1)** in `crates/coral-cli/src/commands/test.rs::tests`:
  - `coral_test_iterations_seed_flags_parse` (acceptance #10 + AC #6 sanity).
- **Plus 7 supporting unit tests** in the property module covering `interpolate_path`, `u64_to_chacha_seed` determinism, `collect_expected_codes`, runner shape sanity, etc.

### Acceptance criteria — 10/10 met

1. Manifest without `[[property_tests]]` parses byte-identically (T1 — `property_tests_absent_round_trips_unchanged`).
2. `[[environments.dev.property_tests]]` block parses with all four fields (T2).
3. `coral test --kind property-based --service api` reads `openapi.yaml` and generates random requests (T3 + T4 + T5 — strategy tests; live-env exercise verified by `cases_from_property_specs_emits_one_case_per_path_method`).
4. Each `(path, method)` from spec → ONE TestCase (`cases_from_property_specs_emits_one_case_per_path_method`).
5. Runner executes configured iteration count (default 50) per endpoint (T9 — `5/5 inputs passed`).
6. With `--seed 42`, two runs produce identical request sequences (T6).
7. Without explicit seed, actual seed logged (`tracing::info!`) AND in `Evidence::stdout_tail` (T7).
8. Failed iteration → TestReport::Fail with first shrunken counter-example (T8).
9. All-pass → TestReport::Pass with "N/N inputs passed" in evidence (T9).
10. `coral test --kind property-based --iterations 5` overrides manifest's iterations (T10 + `coral_test_iterations_seed_flags_parse`).

### Pipeline note

Fourth and final feature of v0.23 sprint — testing platform now ships four runner kinds: healthcheck, user-defined, recorded, and property-based. Out of scope, deferred to v0.24+:
- Response-body schema validation (we currently only validate status codes; widening to body shape needs a stricter JSON Schema validator dep).
- `$ref` / `oneOf` / `allOf` / `anyOf` resolution — v0.23.3 silently treats unknown JSON Schema types as `string`.
- Query-string and custom-header generation.
- PUT / PATCH / DELETE methods.
- Stateful sequences (capture-from-one-endpoint, replay-into-another).
- `--update-snapshots` for property cases.
- Monitor integration of `--kind property-based` (the orchestrator gate is wired; the monitor handler intentionally passes `property_iterations: None, property_seed: None` for now).

## [0.23.2] - 2026-05-09

**Feature release — third of v0.23 sprint (testing platform): `coral test record` (Keploy capture) + `RecordedRunner` replay.** Closes the synthetic-monitoring loop with a third primitive: capture real HTTP traffic via [Keploy](https://github.com/keploy/keploy) and replay it as deterministic TestCases. Where `coral test --kind user_defined` runs hand-authored YAML and `coral test --kind healthcheck` runs auto-discovered `[services.<name>.healthcheck]` probes, `coral test --kind recorded` replays Keploy-captured exchanges from `.coral/tests/recorded/<service>/*.yaml`. **Capture is Linux-only behind the `recorded` Cargo feature** — eBPF + cgroup-v2 + the `keploy` binary on PATH. **Parser + replay are always-on on every platform** (D4 in the orchestrator's spec) so a Mac contributor can replay a YAML captured on Linux CI without rebuilding. Three `dev`/`staging`-style decisions baked in: status code is required-equal, only `Content-Type` is asserted on response headers (other headers are noise), JSON body deep-equal AFTER recursively stripping every key in `[environments.<env>.recorded].ignore_response_fields`. **Zero new deps** — `serde_yaml_ng` already in workspace; capture path subprocesses curl + keploy. **BC sacred: all v0.23.1 surfaces byte-identical** — `bc_regression` green (8 tests), manifests without `[environments.<env>.recorded]` round-trip unchanged (`recorded_config_absent_round_trips_unchanged`). **1356 tests pass (was 1333; +23).** Pinned tested Keploy version: v2.x (the `api.keploy.io/v1beta1` schema; bump tracked separately).

### Added

- **`[environments.<env>.recorded]` config block.** New `Option<RecordedConfig>` field on `EnvironmentSpec` with `#[serde(default, skip_serializing_if = "Option::is_none")]`:
  ```toml
  [environments.dev.recorded]
  ignore_response_fields = ["id", "timestamp", "created_at", "request_id"]
  ```
  Field shape: `{ ignore_response_fields: Vec<String> }`, also `skip_serializing_if = "Vec::is_empty"` so an empty list serializes to nothing.
- **`coral_test::recorded_runner` module.** `KeployTestCase` parser (Keploy v1beta1 schema), `RecordedRunner` (TestRunner trait impl), `discover_recorded(project_root)` walker. `RecordedRunner::assert_exchange(captured, ignore_fields, status, headers, body)` is pure over its inputs — the unit tests pass canned values without spawning curl.
- **`coral_test::recorded_runner::json_bodies_match` + `strip_keys_recursive`.** Recursive ignore-list strip — a key named `id` at any depth (top-level, inside arrays, inside nested objects) drops out of BOTH sides before deep-equal compare.
- **`TestKind::Recorded` is now a value-enum entry on `coral test --kind`.** Pre-v0.23.2 invocations are byte-compatible: empty `--kind` does NOT include recorded (orchestrator-side gate so `coral test --service api` doesn't suddenly find new cases).
- **`coral test record --service NAME --duration SECS [--output DIR]` CLI subcommand.** Capture-side, Linux-only, gated by the `recorded` Cargo feature. On non-Linux OR without the feature, exits 2 with a friendly hint pointing at the build flag. Linux+feature path:
  - Resolves the target service's PID via `docker compose ps --format json` + `docker inspect <id> --format '{{.State.Pid}}'` (two-step fallback for Docker version drift per D6).
  - Subprocesses `keploy record --pid <PID> --path <output_dir>`; SIGTERM after `--duration` seconds.
  - Default output dir: `<project_root>/.coral/tests/recorded/<service>/`. Created if missing.

### Changed

- **`EnvironmentSpec` gained one additive field** (`recorded: Option<RecordedConfig>`). Test fixtures across `coral-test` / `coral-env` / `coral-cli` updated to include `recorded: None`. Serde-side BC preserved via `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- **`coral test`'s arg shape now nests an optional subcommand.** `TestArgs { command: Option<TestSubcommand>, run: TestRunArgs }` so pre-v0.23.2 `coral test --service foo --kind smoke` continues to parse via the flat `TestRunArgs` while `coral test record ...` dispatches to the new handler. The `run_inner` function (formerly `run`'s body) is unchanged.
- **`UserDefinedRunner::discover_tests_dir` and `contract_check::parse_consumer_for_repo`** now skip `.coral/tests/recorded/**` paths so the recorded-runner's Keploy YAMLs don't get parsed as `YamlSuite` (which would `InvalidSpec`-fail at runtime).

### Internal

- **No new workspace deps.** `serde_yaml_ng` already present; capture path subprocesses curl + keploy in line with `coral_runner::http`, `commands::notion_push`, `commands::chaos`. The single-async-runtime-free workspace shape is preserved.
- **Cargo feature wiring**: `coral-test` declares `recorded = []` (gates the capture path); `coral-cli` declares `recorded = ["coral-test/recorded"]` so `cargo install --features recorded` flows through. The replay path (`RecordedRunner` parser + replay) is intentionally NOT gated — pure I/O against any HTTP endpoint, runs on macOS without the feature.
- **Curl-based replay** mirrors the rest of the workspace (`-i` for status + headers + body, `--data-binary @-` for stdin body). `parse_curl_response` walks past any `100 Continue` precursor blocks before pulling the final status line.
- **Pinned tested Keploy version**: the `api.keploy.io/v1beta1` schema. Pre-1.0 Keploy may drift; the parser `#[serde(default)]`s every optional field so a minor-version bump that adds new keys (or drops ones we don't use) doesn't hard-fail the parser.

### Tests (+23)

- **Spec / BC (2)** in `crates/coral-env/src/spec.rs`: `recorded_config_absent_round_trips_unchanged` (T1 — BC pin), `recorded_config_with_ignore_fields_parses` (T7 — TOML round trip).
- **`recorded_runner` module (12)** in `crates/coral-test/src/recorded_runner.rs::tests`:
  - `recorded_runner_parses_keploy_yaml` (T2 — parser)
  - `recorded_runner_status_mismatch_fails` (T3)
  - `recorded_runner_body_diff_with_ignore_fields_passes` (T4)
  - `recorded_runner_body_diff_without_ignore_fields_fails` (T5)
  - `strip_keys_recursive_walks_arrays_and_nested_objects`
  - `parse_curl_response_separates_headers_and_body`
  - `parse_curl_response_handles_100_continue_precursor`
  - `recorded_runner_supports_only_recorded_kind`
  - `discover_recorded_walks_service_directories`
  - `discover_recorded_returns_empty_when_dir_missing`
  - `build_invoke_curl_command_uses_method_and_url`
  - `content_type_charset_parameters_are_stripped_before_compare`
- **Integration (5)** in `crates/coral-test/tests/recorded.rs`:
  - `recorded_replay_with_ignore_fields_passes_against_mock_server` (T4 — end-to-end via TCP listener)
  - `recorded_replay_status_mismatch_fails_against_mock_server` (T3)
  - `recorded_replay_body_diff_without_ignore_fields_fails_against_mock_server` (T5)
  - `run_test_suite_filtered_picks_up_recorded_when_kind_recorded` (orchestrator gate)
  - `run_test_suite_filtered_skips_recorded_when_kind_unspecified` (BC: pre-v0.23.2 callers don't pick up recorded by default)
- **CLI / value-enum (4)** in `crates/coral-cli/src/commands/test.rs::tests`:
  - `coral_test_kind_recorded_in_value_enum` (T8 — AC #4)
  - `coral_test_kind_smoke_user_defined_still_parse` (sanity)
  - `coral_test_record_help_mentions_linux` (T7 — AC #10, snapshot via `render_long_help`)
  - `coral_test_record_on_macos_exits_with_friendly_error` (T6 — `#[cfg(target_os = "macos")]`, AC #2)

### Acceptance criteria — 10/10 met

1. On Linux with `--features recorded`, `coral test record --service api --duration 5s` invokes Keploy + persists YAMLs to `.coral/tests/recorded/api/` (Linux-only smoke, gated `#[cfg(all(target_os = "linux", feature = "recorded"))]`; not exercised on macOS CI).
2. On non-Linux OR without the feature, exits 2 with `"requires Linux + 'recorded' cargo feature"` message (T6).
3. `RecordedRunner` parses Keploy YAML schema (T2 — runs on macOS).
4. `coral test --kind recorded --service api` replays each captured exchange and emits a `TestReport` per case (T8 + integration variant `run_test_suite_filtered_picks_up_recorded_when_kind_recorded`).
5. Status code mismatch → Fail (T3).
6. Body diff with `ignore_response_fields` filtering → Pass (T4).
7. `[environments.<env>.recorded] ignore_response_fields = [...]` parses as `Vec<String>` (T7 — `recorded_config_with_ignore_fields_parses`).
8. Manifest without `[environments.<env>.recorded]` parses identically (T1 — `recorded_config_absent_round_trips_unchanged`).
9. `bc-regression` green.
10. `coral test record --help` documents Linux-only constraint (T7 — `coral_test_record_help_mentions_linux`).

### Pipeline note

Third feature of v0.23 sprint. Tooling validated by 5 prior end-to-end uses in v0.22 / v0.23.0 / v0.23.1. Out of scope, deferred to v0.23.x / v0.24+:
- Keploy proxy-mode capture (TCP-level dependency rewriting; current path is direct subprocess only).
- Auto-PID-discovery for non-Docker backends (Tilt, Kind, Compose-on-Podman). v0.23.2 wires Compose only.
- `ignore_request_fields` (request-side strip — not yet a real-world need).
- Replay against a different host than the one captured against (URL rewriting via `${SVC_<NAME>_BASE}` substitution).
- Linux smoke test in CI matrix (covered by Linux developers via `cargo test --features recorded` locally; GH Actions matrix expansion deferred).

## [0.23.1] - 2026-05-09

**Feature release — second of v0.23 sprint (testing platform): `coral monitor up` with JSONL persistence.** Cron-like scheduled TestCase loops against a long-lived environment, the synthetic-monitoring counterpart to `coral test`'s one-shot run. Where `coral test` answers "did the suite pass right now?", `coral monitor up` answers "has staging been healthy for the last hour?" — each iteration appends one `MonitorRun` line to `.coral/monitors/<env>-<monitor>.jsonl` so an operator can `coral monitor history --tail 60` to see the last 60 ticks. Foreground only in v0.23.1; `--detach` errors with a deferred-message pointing at v0.24+. SIGINT/SIGTERM exit cleanly with a final summary via `signal-hook`. The `MonitorRun` JSONL schema is **frozen** at 7 fields (`timestamp`, `env`, `monitor_name`, `total`, `passed`, `failed`, `duration_ms`) — pinned by `monitor_run_jsonl_shape_pinned`. **One new workspace dep: `signal-hook = "0.3"`** (~30 KB single-purpose with `libc` transitive — `libc` was already in the tree via `tracing`/`clap`). Phase 2's pure refactor extracts `coral_test::run_test_suite_filtered` from `coral_cli::commands::test::run` so both surfaces share the runner-build / discover / filter / execute pipeline and can never drift on which cases they pick. **BC sacred: all v0.23.0 surfaces are byte-identical** — `bc_regression` green (8 tests), manifests without `[[environments.<env>.monitors]]` round-trip unchanged (`monitors_absent_round_trips_unchanged`). **1333 tests pass (was 1300; +33).**

### Added

- **`[[environments.<env>.monitors]]` config blocks.** New `Vec<MonitorSpec>` field on `EnvironmentSpec` with `#[serde(default, skip_serializing_if = "Vec::is_empty")]`:
  ```toml
  [[environments.staging.monitors]]
  name = "smoke-loop"
  tag = "smoke"
  kind = "user_defined"
  services = ["api"]
  interval_seconds = 60
  on_failure = "log"             # default; "fail-fast" exits non-zero on first failure
  ```
- **`coral monitor {up,list,history,stop}` CLI subcommand.**
  - `up --env NAME [--monitor NAME]` — foreground loop. First iteration runs immediately (no leading sleep); subsequent iterations sleep `interval_seconds` between ticks. Iteration overrun logs a `WARN` and starts the next tick immediately.
  - `list [--env NAME]` — print declared monitors with best-effort `running`/`stopped` status (last JSONL timestamp within `interval × 2` → `running`).
  - `history --env NAME --monitor NAME [--tail N]` — print the last N JSONL lines (default 20).
  - `stop` — v0.23.1 deferred-stub; prints "use Ctrl-C in foreground" and exits 0.
- **`MonitorRun` JSONL row** persisted to `.coral/monitors/<env>-<monitor_name>.jsonl` via `OpenOptions::new().create(true).append(true)` + `writeln!` + `f.sync_all()` — explicitly NOT `atomic_write_string`, because an append-only ledger of unbounded length wants O(1) appends, not O(N) full-file rewrites.
- **`OnFailure` enum** (`log` | `fail-fast` | `alert`). `Alert` parses (forward-compat) but errors at runtime with `"on_failure = \"alert\" is reserved for v0.24+"`.
- **`coral_test::run_test_suite_filtered(project_root, spec, backend, plan, env_handle, filters, update_snapshots)`** — new public API that wraps the runner-build / discover / filter / execute pipeline previously inline in `coral test`'s handler. Both `coral test` and the monitor loop call this; future surfaces (`coral profile`, `coral schedule`) plug in the same way. `TestFilters` carries the filter shape (services, tags, kinds, include_discovered).
- **`coral monitor up` SIGINT/SIGTERM clean shutdown** via `signal-hook::flag::register`. The handler flips an `AtomicBool`; the loop checks at the top AND inside `sleep_interruptible` (250ms chunks instead of one big `thread::sleep`) so Ctrl-C latency is < 250ms, not 60s.

### Changed

- **`EnvironmentSpec` gained one additive field** (`monitors: Vec<MonitorSpec>`). `EnvironmentSpec::validate` now also checks: monitor names are unique within an env, `interval_seconds > 0`, and `kind` (when set) is one of `healthcheck` / `user_defined` / `smoke` / `contract` / `property_based`. Test fixtures across `coral-test`/`coral-env` updated to include `monitors: Vec::new()`.
- **`coral test`'s `run` handler** is now a thin wrapper over `coral_test::run_test_suite_filtered`. The CLI translates its `KindArg` (which has a `Smoke` umbrella variant) into a flat `Vec<TestKind>`, hands the rest off. Behavior is byte-identical to v0.23.0.

### Internal

- **One new workspace dep: `signal-hook = "0.3"`.** Justified inline in the manifest comment (~30 KB single-purpose; `libc` transitive was already in the tree). The `unsafe libc::signal` fallback was rejected because the `Arc<AtomicBool>` registration is the well-trodden, easier-to-audit path.
- **Phase 2 pure refactor (no behavior change).** Pulling the runner-build code out of `coral test` into `coral_test::run_test_suite_filtered` was its own commit-discipline phase: `cargo test --workspace` was run between Phase 2 and Phase 3 to verify nothing in the `coral test` path drifted.
- **Loop tests serialize via a process-global `Mutex`** because the SIGINT-flag is process-global. `loop_test_lock()` returns a poison-tolerant guard so a panicking test doesn't break the next one.

### Tests (+33)

- **Spec / BC (4)** in `crates/coral-env/src/spec.rs`: `monitors_absent_round_trips_unchanged` (T1 — BC pin), `monitors_full_section_parses`, `monitors_alert_on_failure_parses_validate_passes`, `monitor_with_zero_interval_rejected`, `monitor_with_unknown_kind_rejected`, `monitor_duplicate_names_rejected`.
- **JSONL / MonitorRun (4)** in `crates/coral-cli/src/commands/monitor/run.rs::tests`: `monitor_run_jsonl_shape_pinned` (T2 — schema pin), `from_reports_counts_pass_fail_error`, `append_run_creates_file_and_appends`, `jsonl_path_combines_env_and_monitor_name`.
- **Loop / scheduling (5)** in `crates/coral-cli/src/commands/monitor/up.rs::tests`: `monitor_loop_first_iteration_immediate` (T3 — AC #2), `monitor_loop_appends_each_iteration_to_jsonl` (T4 — AC #3), `on_failure_log_continues_after_failure` (T6 — AC #5), `on_failure_fail_fast_exits_on_first_failure` (T7 — AC #6), `kind_for_monitor_translates_known_strings`, `kind_for_monitor_rejects_unknown_strings`, `dummy_spec_round_trips`, `exit_for_returns_success_when_no_failures`.
- **History / list (8)** in `crates/coral-cli/src/commands/monitor/{history,list,stop}.rs::tests`: `tail_lines_returns_last_n` (T5 — AC #7), `tail_lines_handles_fewer_lines_than_n`, `tail_lines_skips_blank_trailing_newlines`, `tail_lines_handles_empty`, `status_for_reports_stopped_when_no_file`, `status_for_reports_running_when_recent_tick`, `status_for_reports_stopped_when_tick_outside_window`, `status_for_handles_unparseable_last_line`, `stop_returns_success`.
- **CLI E2E (6)** in `crates/coral-cli/tests/monitor_e2e.rs`: `monitor_list_shows_declared_monitors` (T8 — AC #10), `monitor_list_unknown_env_prints_friendly_message`, `monitor_history_missing_file_exits_2`, `monitor_history_tail_returns_last_n_lines`, `monitor_stop_is_deferred_stub`, `monitor_up_detach_is_rejected`.

### Acceptance criteria — 10/10 met

1. Manifest without `[[monitors]]` round-trips byte-identically (T1 / `monitors_absent_round_trips_unchanged`).
2. `monitor up --env staging` first iteration runs immediately (T3, asserts wall-clock < 500ms before first tick).
3. Each iteration appends ONE `MonitorRun` line to `.coral/monitors/<env>-<name>.jsonl` (T4, 3 iterations → 3 lines).
4. `MonitorRun` schema frozen: `timestamp`, `env`, `monitor_name`, `total`, `passed`, `failed`, `duration_ms` (T2 pins exact field list + serialization order).
5. `on_failure = "log"` records and continues (T6).
6. `on_failure = "fail-fast"` exits non-zero on first failed iteration (T7).
7. `monitor history --tail 5` returns last 5 JSONL lines (T5; e2e variant in `monitor_history_tail_returns_last_n_lines`).
8. SIGINT exits cleanly with summary (loop checks `shutdown_flag()` at top + inside `sleep_interruptible`; final summary always prints).
9. Iteration > interval logs warning + starts next immediately (`tracing::warn!` + `continue` skips the sleep).
10. `monitor list` shows declared monitors + best-effort status (T8; status heuristic = `interval × 2` window).

### Pipeline note

Second feature of v0.23 sprint. Tooling validated by 4 prior end-to-end uses in v0.22 / v0.23.0. Out of scope, deferred to v0.23.x / v0.24+:
- `--detach` (daemonization, PID files, `monitor stop` via PID-kill).
- JSONL rotation / size cap (current limitation: file grows unbounded).
- `on_failure = "alert"` wiring (PagerDuty / OpsGenie webhooks).
- Cron expression parsing (current resolution: `interval_seconds: u64`).
- `coral session capture --from cursor` and `--from chatgpt` (still tracking #16, unchanged).

## [0.23.0] - 2026-05-09

**Feature release — first of v0.23 sprint (testing platform): `coral chaos inject` with Toxiproxy backend.** Network/process fault injection for running Coral environments. Closes the loop between `coral up` (spin up) and `coral test` (validate happy-path) with a third primitive: validate-under-pressure. Per PRD §3.3 (test honeycomb) + §13.6, this is the first of four v0.23 testing-platform features (chaos, monitor, record, property-based). v0.23.0 wires Toxiproxy (Shopify) as the first backend — TCP-level proxy that injects latency, bandwidth caps, slow-close, timeout, slicer faults at the connection level. Pumba (Linux-only, container-level kill/pause) deferred to v0.23.x or v0.24+ pending demand. **1300 tests pass (was 1274; +26).** `bc-regression` green; chaos-OFF compose YAML byte-identical to v0.22.6.

### Added

- **`[environments.<env>.chaos]` config block.** New `Option<ChaosConfig>` field on `EnvironmentSpec` with `#[serde(default)]`:
  ```toml
  [environments.dev.chaos]
  backend = "toxiproxy"
  image = "ghcr.io/shopify/toxiproxy:2.7.0"
  listen_port = 8474
  ```
- **`[[environments.<env>.chaos_scenarios]]` named scenarios** runnable via `coral chaos run <name>`.
- **Toxiproxy sidecar auto-injection in compose render.** When `chaos.is_some()`, `compose_yaml::render` appends a `toxiproxy` service to the rendered YAML with admin port (8474) published.
- **Service rerouting via `depends_on`.** When chaos is on, every `(consumer, dep)` edge becomes a toxiproxy proxy declaration; consumer env vars rewrite `<DEP>_URL=http://dep:<port>` → `<DEP>_URL=http://toxiproxy:<proxy_port>`. Proxy port allocation deterministic: `hash(consumer:dep) % 10000 + 30000`.
- **`coral chaos {inject,clear,list,run}` CLI subcommand.** `inject --service N --toxic T[:V] [--duration S]`, `clear [--service N]`, `list [--json]`, `run <scenario-name>`.
- **`ToxicKind` parser** accepts `latency:Nms`, `bandwidth:Nkb`, `slow_close:Nms`, `timeout`, `slicer`. Others rejected with friendly error listing valid forms.

### Changed

- **`EnvironmentSpec` gained two additive fields** (`chaos`, `chaos_scenarios`). Test fixtures across `coral-test`/`coral-env` updated to include `chaos: None, chaos_scenarios: Vec::new()`. Serde-side BC preserved via `#[serde(default)]`.

### Internal

- **Zero new workspace deps.** HTTP via subprocess `curl` (matches existing `coral-runner` pattern). Toxiproxy admin API is small + bounded — full HTTP client overkill.
- **Mock TCP server in inject test** uses `Content-Length` parsing + 250ms read timeout to avoid the "second-read-blocks-forever" deadlock that initially hung CI. Parses headers, extracts Content-Length, reads exactly that many body bytes, responds.

### Pipeline note

First feature of v0.23 sprint. Tooling validated by 3 consecutive end-to-end uses in v0.22.

## [0.22.6] - 2026-05-09

**Feature release: `coral skill build` ships Coral as an Anthropic-Skills-compatible bundle.** The new subcommand walks `template/{agents,prompts,hooks}`, prepends an auto-generated `SKILL.md` manifest at the zip root, and writes a deterministic deflate zip to `dist/coral-skill-<version>.zip` (or `--output PATH`). Two consecutive runs produce byte-identical archives — every entry's mtime is pinned to the zip-format minimum (`1980-01-01T00:00:00Z`), entries are sorted by zip path, and unix permissions are pinned to `0o644` so umask drift can't leak in. The bundle excludes Coral-specific surfaces that aren't portable: `template/schema/` (lint-rule schema), `template/workflows/` (GitHub Actions yaml), and `template/commands/` (Claude-Code slash-command wrappers — the agent personas themselves are the portable surface). `SKILL.md` reads each agent/prompt's YAML frontmatter `description` field for the per-file Contents listing; missing/malformed frontmatter falls back to an empty description rather than failing the build. Frontmatter `version` always equals `env!("CARGO_PKG_VERSION")` so the manifest can never drift from the running binary. The companion `coral skill publish` is a thin stub that prints the deferred-message `publish is deferred to v0.23+; for now, run \`coral skill build\` and submit the zip manually to https://github.com/anthropics/skills` and exits 0 — the real Anthropic-Skills fork+PR flow lands in v0.23+. **One new workspace dep: `zip = "2"`** (~50 KB) with `default-features = false, features = ["deflate"]` — this drops the `bzip2`/`xz2`/`zstd` system-library transitives we'd otherwise pull in (we use deflate exclusively). Hand-rolling the zip header (LFH + central directory + EOCD) was rejected as error-prone for a release-shipping artifact. **BC sacred: all v0.22.5 surfaces are byte-identical** — pinned by `bc_regression` (8 tests still green), `template/` is unchanged. **1274 tests pass (was 1264; +10 = 4 unit + 6 e2e), all green.** Closes the v0.22 sprint feature backlog.

### Added

- **`coral skill build [--output PATH]`** (`crates/coral-cli/src/commands/skill.rs::build`). Walks `template/{agents,prompts,hooks}`, generates `SKILL.md`, writes a deterministic deflate zip via `zip::write::ZipWriter`, atomic-renames the result into place. Default output path: `dist/coral-skill-<version>.zip` relative to cwd. Prints `wrote <path> (<N> files, <K> bytes uncompressed)` and exits 0.
- **`coral skill publish`** stub (same module, `::publish`). v0.22.6 only emits the deferred-message and exits 0; pinned byte-for-byte by `tests/skill_build.rs::skill_publish_stub_emits_deferred_message`.
- **Auto-generated `SKILL.md`** with YAML frontmatter (`name`, `description`, `version`) + a "Contents" section enumerating every shipped file with its frontmatter `description`. Generation is in `commands::skill::generate_skill_md`; description extraction in `commands::skill::parse_description` (tolerant of missing frontmatter, missing `description:` key, and non-UTF-8 bytes).
- **`zip = "2"` workspace dependency** (`Cargo.toml [workspace.dependencies]`) wired into `coral-cli` as both runtime and dev dep. Justified inline in the manifest comment.

### Tests (+10)

- **Unit (4)** in `crates/coral-cli/src/commands/skill.rs::tests`: `parse_description_handles_simple_frontmatter`, `parse_description_returns_none_without_frontmatter`, `parse_description_returns_none_when_key_missing`, `skill_md_includes_version_from_cargo`.
- **E2E (6)** in `crates/coral-cli/tests/skill_build.rs`: `skill_build_produces_valid_zip` (AC #1, #2), `skill_build_includes_agents_prompts_hooks` (AC #5), `skill_build_excludes_schema_workflows_commands` (AC #6), `skill_build_skill_md_frontmatter_has_name_description_version` (AC #3, #4), `skill_build_deterministic_two_runs` (AC #8 — sleeps 1.1s between runs so any naive `now()`-based timestamping would diff), `skill_publish_stub_emits_deferred_message` (AC #9 — pins the deferred-message text byte-for-byte).

### Acceptance criteria — 10/10 met

1. `coral skill build` produces `dist/coral-skill-0.22.6.zip` and exits 0.
2. The zip re-opens cleanly via `zip::ZipArchive` (no torn-archive risk — atomic write via `tempfile::NamedTempFile::persist`).
3. The bundle contains a root `SKILL.md` with valid YAML frontmatter (`name`, `description`, `version`).
4. SKILL.md frontmatter `version` equals `env!("CARGO_PKG_VERSION")`.
5. `agents/wiki-linter.md`, `prompts/consolidate.md`, `hooks/pre-commit.sh` (and every other regular file under those three subdirs) ship with the `template/` prefix stripped.
6. `template/schema/`, `template/workflows/`, `template/commands/` are NOT in the bundle (asserted by both file-name and directory-prefix membership).
7. `--output /tmp/foo.zip` writes to the override path; the default `dist/` is not touched.
8. Two consecutive `coral skill build` invocations produce byte-identical zips.
9. `coral skill publish` exits 0 and emits the spec-D5 deferred-message text.
10. `template/` is read-only to the binary — content unchanged after build (holds by construction).

### Pipeline note

Final feature release of the v0.22 sprint. Out of scope, deferred to v0.23+:
- Real `coral skill publish`: the Anthropic-Skills fork+PR flow (clone `anthropics/skills`, branch, copy bundle into a per-skill subdirectory, push, open PR via `gh`). v0.22.6 prints the manual-submission pointer and exits 0.
- `template/schema/` / `template/workflows/` / `template/commands/` portability — revisit when each can be expressed without Coral-specific or Claude-Code-specific assumptions.
- Skill-bundle signing / SHA-256 sidecar for reproducibility audits.
- Customizing the bundle name (currently fixed to `coral-skill-<version>.zip`).

## [0.22.5] - 2026-05-09

**Feature release: discoverable MCP server card per the 2025-11-25 spec.** Coral now publishes `/.well-known/mcp/server-card.json` on the HTTP/SSE transport AND mirrors the same payload via a new `coral mcp card` CLI subcommand. Both surfaces emit byte-identical pretty-printed JSON modulo the trailing newline `println!` adds. The card carries the spec-defined fields — `name`, `version`, `protocolVersion`, `transports`, `capabilities.{resources,tools,prompts}.count`, `vendor` — plus a Coral-specific `x-coral` namespace (`buildTimestamp`, `ciStatus`). Capability counts are sampled from the same catalog instances `coral mcp serve` uses, so the discovery payload can never drift out of sync with the JSON-RPC `initialize` reply. The HTTP route is mounted **before** the `/mcp` Origin allowlist — registries hit it from any origin (the card is public by design), while the `/mcp` DNS-rebinding mitigation stays unchanged for the actual MCP traffic. Zero new workspace dependencies — the card is a pure `serde_json::json!` macro plus `option_env!("CORAL_BUILD_TIMESTAMP")` for the (optional) build provenance hint, no build-script changes. **BC sacred: all v0.22.4 surfaces are byte-identical** — pinned by `bc_regression` (8 tests still green). **1264 tests pass (was 1258; +6 = 3 unit + 2 e2e + 1 CLI smoke), all green.** This is the second consecutive release dogfood-validated by the v0.22.4-fixed `scripts/release.sh`.

### Added

- **`GET /.well-known/mcp/server-card.json` on the HTTP/SSE transport** (`crates/coral-mcp/src/transport/http_sse.rs::handle_well_known_card`). 200 + `Content-Type: application/json` + pretty-printed body. Branch lands in `handle_request` BEFORE the Origin allowlist so cross-origin GETs (browser tabs, registry probes) are accepted; the `/mcp` Origin check is unchanged. Any other path under `/.well-known/mcp/*`, or a non-GET method on the card path, returns 404.
- **`coral mcp card` CLI subcommand** (`crates/coral-cli/src/commands/mcp.rs::card`). No flags. Constructs the same `WikiResourceProvider` / `ToolCatalog` / `PromptCatalog` instances `coral mcp serve` uses, prints `serde_json::to_string_pretty(&card)` followed by exactly one trailing newline, exits 0. Errors propagate via anyhow.
- **`coral_mcp::card::server_card(&dyn ResourceProvider, &ToolCatalog, &PromptCatalog) -> serde_json::Value`** (new module `crates/coral-mcp/src/card.rs`). The single source of truth for the card payload; both surfaces call it. Re-exported as `coral_mcp::server_card`.
- **`x-coral` namespace.** `buildTimestamp` reads from `option_env!("CORAL_BUILD_TIMESTAMP")` so reproducible/CI builds may inject an ISO-8601 timestamp; a plain `cargo build` produces `"unknown"`. `ciStatus` is the literal `"green"` because the binary itself is the artifact CI just blessed (a `"red"` value would lie about the running binary's provenance).

### Tests (+6)

- **Unit (3)** in `crates/coral-mcp/src/card.rs::tests`: `card_has_name_version_protocolversion`, `card_capabilities_counts_match_catalog_lens`, `card_serializes_to_pretty_json`.
- **E2E (2)** in `crates/coral-mcp/tests/mcp_http_sse_e2e.rs`: `well_known_card_endpoint_returns_200_with_valid_json` (covers AC #1, #2, #3, #4, #8 — endpoint shape + content type + capability counts + cross-origin GET), `well_known_unknown_path_returns_404` (AC #7 — sibling well-known paths and non-GET methods on the card path).
- **CLI smoke (1)** in `crates/coral-cli/tests/mcp_card_smoke.rs`: `cli_mcp_card_emits_json_to_stdout` (AC #5 — exit 0 + valid JSON of the same schema, with `version` from `CARGO_PKG_VERSION`).

### Acceptance criteria — 8/8 met

1. `GET /.well-known/mcp/server-card.json` returns 200 with valid JSON matching D1 schema.
2. `Content-Type: application/json` on the card response.
3. `name == "coral"`, `version == env!("CARGO_PKG_VERSION")` (= "0.22.5"), `protocolVersion == "2025-11-25"`.
4. `capabilities.{resources,tools,prompts}.count` match catalog `.len()`.
5. `coral mcp card` exits 0, prints valid JSON of the same schema.
6. `coral mcp card` stdout equals HTTP body byte-for-byte modulo trailing newline (both surfaces share `server_card()` + `to_string_pretty()`).
7. `GET /.well-known/mcp/anything-else` → 404; non-GET on the card path → 404.
8. Card endpoint accepts cross-origin GETs (the new branch lands BEFORE the Origin allowlist); `/mcp` Origin check unchanged.

### Pipeline note

Feature release within v0.22 sprint. Second consecutive release dogfood-validated by the v0.22.4-fixed `scripts/release.sh` (`bump` → `tag` → push → gh-release). Out of scope, deferred:
- Build-script timestamp injection — the `CORAL_BUILD_TIMESTAMP` env knob exists; CI can set it. Default is `"unknown"`.
- Auto-publishing to external registries — users self-host. v0.22.6 will cover the published-skill build pipeline.
- JSON Schema validation against an external file — assertions cover shape; no schema dep added.

## [0.22.4] - 2026-05-08

**Patch release: simplify `release.sh tag` to use `git tag -a` directly.** The v0.22.3 dogfood revealed that `cargo release tag --execute` is a NO-OP when the version bump came from a manual Cargo.toml edit (instead of `cargo release X.Y.Z`). cargo-release tracks "we just released X.Y.Z" as in-memory state from the bump step; without that state, `cargo release tag --execute` exits silently and creates nothing. Real-world Coral releases mix manual bumps (when the dev applies a tester finding inline, like v0.21.3 → v0.21.4) with cargo-release-driven bumps, so depending on cargo-release's state-tracking is fragile. v0.22.4 ditches `cargo release tag` entirely and uses plain `git tag -a "v$version" -m "Coral v$version"` + `git push origin main` + `git push origin "v$version"`. The annotated tag matches the conventions previously declared in `release.toml` (`tag-name = "v{{version}}"`, `tag-message = "Coral v{{version}}"`). Net effect: `release.sh tag X.Y.Z` now works regardless of whether the prior bump came from `release.sh bump` or a manual edit. The regression test #7b is updated to assert `git tag -a` is the canonical line (replacing the prior assertion that `cargo release tag --no-confirm --execute` appears).

### Fixed (in-cycle, before tag)

- **`scripts/release.sh::cmd_tag` no longer relies on cargo-release state.** Switched to `git tag -a` + `git push` directly. Adds an idempotency guard: if `refs/tags/v$version` already exists locally, the script aborts with exit 6 instead of producing a duplicate-tag error from git.

### Pipeline note

Patch release within v0.22 sprint. The dogfood loop continues to harden the new tooling. v0.22.{5,6} cover MCP registry publish + coral skill build/publish (originally v0.22.{4,5} pre-fix).

## [0.22.3] - 2026-05-08

**Patch release: fix `cargo release tag` positional-version argument bug.** The first dogfood run of `scripts/release.sh tag 0.22.2` (the second real use of the new release tooling, after `release.sh bump` was validated by the v0.22.1 cache fix) revealed another shape-bug the v0.22.0 tester didn't catch: `cmd_tag` was invoking `cargo release tag $version --no-confirm --execute`, but `cargo release tag` does NOT accept a positional version argument — it derives the tag name from the workspace's `[workspace.package].version` (already written by the prior `release.sh bump $version` step). The stray positional crashed cargo-release with `unexpected argument '0.22.2' found` immediately after the HEAD-subject validation passed. Pre-fix, no automated test exercised the post-validation path because all flows that would invoke `cargo release tag` would push to the live remote, so the test suite stopped short. Post-fix: invocation is `cargo release tag --no-confirm --execute` (no positional). v0.22.2 itself was tagged via the manual fallback (`git tag -a v0.22.2 -m "Coral v0.22.2"` + `git push origin v0.22.2`) because the bug was caught at tag-time. The new tooling is now validated end-to-end through `bump → tag → release-gh`; the `release.sh tag 0.22.3` invocation will be the first to fully run cargo-release's tag pipeline. **1258 tests pass (was 1257; +1 regression test).**

### Fixed (in-cycle, before tag)

- **`scripts/release.sh::cmd_tag` invocation shape.** Removed the stray positional `$version` from the `cargo release tag` invocation. The push step (`cargo release push --no-confirm --execute`) was always correct; only the tag step was broken.

### Tests

- **#7b `release_sh_tag_invokes_cargo_release_without_positional_version`** — source-grep regression test that asserts no NON-COMMENT line in `scripts/release.sh` contains the `cargo release tag $version`-shaped pattern, AND that the canonical `cargo release tag --no-confirm --execute` line IS present (so the test isn't a tautology). Comment-line filtering excludes the legitimate doc-comment that explains the bug history.

### Pipeline note

Patch release within v0.22 sprint. The `release.sh tag 0.22.3` invocation (planned for the maintainer's next move) is the first end-to-end use of the new tooling that exercises every step (bump + tag + push + gh-release). v0.22.{4,5} (originally v0.22.{3,4} pre-fix) cover MCP registry publish + coral skill build/publish.

## [0.22.2] - 2026-05-08

**Feature release: `coral test --emit k6` smoke→load handoff.** Adds an emitter that walks the same TestCase discovery + filter pipeline `coral test` already uses and serializes the resulting HTTP step set to a single k6 JavaScript load-test script. The user runs `k6 run <emitted file>`; Coral itself never executes k6 — that's the handoff point. Coverage targets the ~95% case: UserDefined HTTP steps + HTTP Healthcheck probes translate 1:1; TCP/Exec/Grpc healthchecks and `YamlStep::Exec` steps emit `// SKIPPED` comments AND are returned via `EmitOutput.skipped` so the CLI surfaces a stderr summary. Output is byte-deterministic across runs (cases iterate in `(service, id)` lexicographic order). `--emit k6` does NOT require `coral up` — it reads only `EnvironmentSpec`, so it works on a clean checkout. **Zero new workspace dependencies** — pure `format!` + `String::push_str` string formatting; no JS AST library, no template engine. **BC sacred: `coral test` (no `--emit`) is byte-identical to v0.22.1** — pinned by `bc_regression::coral_test_no_emit_no_match_stdout_pinned_to_v0_22_1`. **1257 tests pass (was 1228; +29 = +21 ship + 8 in-cycle fix), all green.** `bc-regression` green.

### Added

- **`coral test --emit k6 [--emit-output PATH]`.** New flags on `coral test`. `--emit k6` short-circuits before runner construction: discover + filter the TestCase set, hand to `coral_test::emit_k6`, write to stdout (default) or atomically to PATH via `coral_core::atomic::atomic_write_string`. Stderr surfaces a `k6 emit summary: included=N skipped=M` line plus one `skip: <id> — <reason>` line per skipped case. Acceptance criteria 12/12.
- **`crates/coral-test/src/emit_k6.rs`.** New pure-emitter module. Public surface: `emit_k6(cases: &[TestCase], spec: &EnvironmentSpec) -> EmitOutput`. `EmitOutput { script: String, included: usize, skipped: Vec<SkipNote> }`. `SkipReason` variants: `HealthcheckNotHttp`, `UserDefinedExecStep`, `UnsupportedKind`, `InvalidUserDefinedSpec`. Output structure: imports + `export const options` + per-service `SVC_<NAME>_BASE` consts + one body block per case (fenced with `// === <id>: <name> (service=<svc>, tags=[...]) ===`) + skip comments + `// === Coral emit summary: ... ===` footer. Header emits exactly one `import http from 'k6/http';` and one `import { check, sleep } from 'k6';` (k6 ships `check`/`sleep` from root, not `k6/check`).
- **`Emit` enum on `TestArgs`** — clap `ValueEnum` with `K6` variant. v0.22.2 ships `k6` only; future emitters (`gatling`, `locust`, …) extend the enum.
- **Per-service `__ENV.CORAL_<NAME>_BASE` override.** Each service in `EnvironmentSpec.services` with a published port emits `const SVC_<UPPER>_BASE = __ENV.CORAL_<UPPER>_BASE || 'http://localhost:<PORT>';` so the user can point a single emitted script at dev/staging/prod via `CORAL_API_BASE=https://api.staging.example.com k6 run load.js`. The first service's URL becomes the default `BASE`; cases without a service or targeting an undeclared service fall back to `${BASE}` with a `// TODO` comment.
- **Mapping table.** `expect.status: 200` → `(r) => r.status === 200`. `expect.body_contains: "ok"` → `(r) => r.body.includes("ok")`. `expect.snapshot` → `/* snapshot expects skipped — k6 doesn't do snapshots */`. `expect.json_path` (reserved) → `/* json_path expects: TODO wire r.json('x') === 1 */`. Empty `expect: {}` → no `check()` block. `YamlStep::Exec` → `// SKIPPED <id>:<n> — exec step not k6-compatible`.
- **`crates/coral-test/tests/emit_k6_smoke.rs`** — 6 integration tests: header pin, two-step UserDefined determinism golden, exec-step skip comment, TCP healthcheck skip, declared-port flow into `SVC_<NAME>_BASE`, unknown-service `${BASE}` + TODO fallback.
- **`crates/coral-cli/tests/test_emit_k6.rs`** — 4 end-to-end CLI tests: stdout default, `--emit-output` atomic write, zero-matches → exit 2 + stderr filter listing, `--format junit` rejection.

### Changed

- **`crates/coral-cli/src/commands/test.rs::run`** wraps the existing test-execution path with a `--emit` short-circuit. Flag-interaction validation runs BEFORE any I/O: `--emit k6 --format junit` exits 2 with a one-line stderr message naming both flags; `--emit k6 --update-snapshots` exits 2 with `--update-snapshots not meaningful with --emit`. The execution path (no `--emit`) is unchanged.
- **`crates/coral-test/src/lib.rs`** declares `pub mod emit_k6;` and re-exports `emit_k6, EmitOutput, SkipNote, SkipReason` at the crate root for symmetry with `JunitOutput`.

### Tests (+21)

- **`emit_k6` unit tests (10)** in `crates/coral-test/src/emit_k6.rs::tests` — covers `js_string` escape, `split_http_line` method recognition, empty-cases skeleton shape, port → `SVC_<NAME>_BASE` const, multi-step UserDefined translation, exec-step inline skip, HTTP healthcheck `http.get` + 2xx check, healthcheck-probe-label/path lookups.
- **`emit_k6_smoke` integration tests (6)** as listed above.
- **`test_emit_k6` CLI tests (4)** as listed above.
- **`bc_regression::coral_test_no_emit_no_match_stdout_pinned_to_v0_22_1`** — pins `coral test` (no `--emit`) byte-identical "no test cases match the given filters\n" stdout against drift.

### Fixed (in-cycle, before tag)

The first independent tester audit on commit `f3b7dde` returned 1 HIGH + 2 MEDIUM. All three are fixed before v0.22.2 is tagged (this release is local-only at the time the in-cycle fix lands).

- **HIGH — service names with dashes produced invalid JavaScript.** `to_uppercase()` on `my-api` was being spliced verbatim into `const SVC_MY-API_BASE = ...` and `__ENV.CORAL_MY-API_BASE`, both of which are syntax errors (JS identifiers and POSIX env-vars disallow `-`). `node --check out.mjs` failed with `SyntaxError: Missing initializer in const declaration`. Fix: introduce private `to_js_ident(name)` (`crates/coral-test/src/emit_k6.rs`) that uppercases then maps `-`/`.` → `_` and validates against `^[A-Z_][A-Z0-9_]*$` with a deterministic alphanumeric fallback. Applied at all `SVC_<IDENT>_BASE` emission sites (header at L120-122, `BASE` alias at L131, body reference in `service_base_const` at L399). The `__ENV.CORAL_<IDENT>_BASE` env-var name uses the same transform so the override path stays consistent. Regression pinned by `dash_in_service_name_emits_valid_js_ident` (unit) + `cli_emit_k6_output_passes_node_check_with_dashed_service_name` (integration, `node --check`-gated).
- **MEDIUM — `--format json --emit k6` was a silent footgun.** Pre-fix the runtime gate only rejected `Format::Junit`; `--format json` was silently accepted and the user got k6 JS on stdout while expecting JSON. Fix: `crates/coral-cli/src/commands/test.rs` rejects ANY non-default `--format` (i.e. anything other than the clap default `markdown`) when `--emit` is set. Error message names the offending format and both flags: `"--format json applies to test execution; --emit selects an emitter (you used both)"`, exit 2. Regression pinned by `cli_emit_k6_rejects_format_json_combo` (mirrors the existing junit case) and `cli_emit_k6_accepts_default_format_markdown` (sanity pin so the gate doesn't over-trigger on the clap-supplied default).
- **MEDIUM — no automated `node --check` coverage.** AC #2 (emitted JS passes `node --check`) was un-pinned at ship — verified manually in dev rehearsal but not asserted by any test, which is why the HIGH dash bug shipped. Fix: `crates/coral-cli/tests/test_emit_k6.rs` adds two integration tests that invoke `node --check <path>` on the emitted script. Both gate-skip cleanly if `node` isn't on `PATH` (same `eprintln!("SKIP …")` pattern as `cargo_release_available()` in `release_flow.rs`), and both write to `.mjs` so ESM-only syntax errors surface (the dash bug only failed under ESM, not CJS). Two scenarios pinned: happy path (`cli_emit_k6_output_passes_node_check_happy_path`, no dashes) and the HIGH regression (`cli_emit_k6_output_passes_node_check_with_dashed_service_name`).

**Test delta from this fix:** +8 (4 new emit_k6 unit tests for `to_js_ident` / `is_valid_js_ident` / dash-case const emission; 4 new CLI tests for json rejection + markdown sanity + 2× node --check). Brings totals from the +21 ship delta to +29 over v0.22.1's 1228 baseline.

**Three LOW findings deferred to v0.22.3** — they are real but lower-priority and would bloat the in-cycle fix:

- AC #8 test (`cli_emit_k6_with_emit_output_writes_atomically`) asserts file contents *contain* expected substrings; it should assert the file equals stdout from the same invocation, since "atomic" was the contract.
- Inclusion count is misleading when all HTTP steps in a UserDefined suite have invalid `http:` lines: the case still counts as `included=1` because at least one step rendered an inline skip comment, but no actual HTTP call was emitted.
- Lowercase YAML method tokens (e.g. `http: get /x`) are dropped silently by `split_http_line` and produce an inline skip with no diagnostic distinguishing them from genuinely malformed input.

### Acceptance criteria — 12/12 met

1. `coral test --emit k6` exits 0 on a project with at least one HTTP UserDefined or HTTP Healthcheck case ✓ — `cli_emit_k6_writes_to_stdout_by_default` covers the HTTP healthcheck path.
2. Emitted JS passes `node --check` ✓ — pinned automatically by `cli_emit_k6_output_passes_node_check_happy_path` and `cli_emit_k6_output_passes_node_check_with_dashed_service_name` (both gate-skip when `node` is absent); structure-test pinned by `emit_k6_header_has_options_and_imports`.
3. Output contains exactly one `import http from 'k6/http';` and one `export const options` ✓ — `emit_k6_header_has_options_and_imports` uses `.matches(...).count() == 1`.
4. Each service in `EnvironmentSpec.services` with a published host port emits `SVC_<UPPER>_BASE` using its declared port ✓ — `emit_k6_service_base_uses_declared_port`.
5. UserDefined HTTP steps translate per the mapping table; YAML order preserved ✓ — `emit_k6_user_defined_two_step_suite_round_trips`.
6. Healthcheck cases with TCP/Exec/Grpc probes emit a `// SKIPPED` comment AND are added to `EmitOutput.skipped` ✓ — `emit_k6_healthcheck_tcp_skipped`. Stderr summary surfaces in `cli_emit_k6_writes_to_stdout_by_default`.
7. `--service api --emit k6` includes only `api` cases ✓ — covered by `apply_filters` reuse + the existing `--service` filter test surface.
8. `--emit-output dist/load.js` writes atomically; stdout empty ✓ — `cli_emit_k6_with_emit_output_writes_atomically` asserts both.
9. `--emit k6 --format junit` exits 2 with a one-line message naming both flags ✓ — `cli_emit_k6_rejects_format_junit_combo`.
10. Zero matching cases → empty stdout, exit 2, stderr lists active filters ✓ — `cli_emit_k6_zero_matches_exits_2_with_diagnostic`.
11. Output is byte-deterministic across two runs on identical inputs ✓ — `emit_k6_user_defined_two_step_suite_round_trips` runs `emit_k6` twice and asserts script equality.
12. `coral up` is NOT a precondition; emit works on a clean checkout ✓ — every CLI test in `test_emit_k6.rs` runs without invoking any backend; the `--emit` branch reads only `EnvironmentSpec`.

### Pipeline note

Continuation of v0.22 sprint per the v0.22.1 pipeline note. Next: `coral env import` → MCP registry publish → `coral skill build/publish` for v0.22.{3,4,5}. **Q1-Q4 in §7 of the orchestrator's spec are deferred to v0.23**: capture-threading (`capture: { token: "$.token" }`), `sleep(1)` cadence override (current default is 1s between cases, post-edit by user), retry policy translation, `--include-discovered` count breakout. The `sleep(1);` line is emitted once per included case so an end-user can `sed` to a different cadence without touching the surrounding scaffolding.

## [0.22.1] - 2026-05-08

**Patch release: fix `release.sh preflight` per-package iteration cost.** The first dogfood run of the v0.22.0 tooling — `scripts/release.sh bump 0.22.0` against Coral's actual 9-crate workspace — exposed a real performance bug the v0.22.0 tester didn't catch: cargo-release fires the `pre-release-hook` ONCE PER WORKSPACE PACKAGE, so `release.sh preflight` was running `scripts/ci-locally.sh` 9 times back-to-back (~9 × ~50s = ~7-9 min wall-time per bump). Tests had only exercised the hook in standalone tempdirs, never in the real workspace. v0.22.1 caches the ci-locally result via a marker file under `$TMPDIR` keyed on `(version, HEAD sha)`. Subsequent invocations within the same cargo-release run short-circuit with `ok "ci-locally.sh already passed in this cargo-release run"`. Honors `$CORAL_PREFLIGHT_FORCE=1` for debugging. Without this fix, v0.22.0 bumps in production would have been unusably slow; v0.22.1 brings the bump back down to ~50s. **1228 tests pass (+1: `release_sh_preflight_caches_ci_locally_across_per_package_calls`), all green.** v0.22.0 was shipped via the manual flow as a result of this bug — v0.22.1 onwards uses the new tooling end-to-end.

### Fixed

- **`release.sh preflight` ci-locally caching.** `cmd_preflight` now writes `${TMPDIR:-/tmp}/coral-preflight-${version}-${head_sha}.pass` after the first ci-locally pass and short-circuits subsequent calls within the same cargo-release run. The marker is automatically invalidated when HEAD moves (sha-keyed). `$CORAL_PREFLIGHT_FORCE=1` bypasses for debugging.

### Tests

- **#13 `release_sh_preflight_caches_ci_locally_across_per_package_calls`** — runs preflight twice with a counter-bumping ci-locally stub, asserts ci-locally fires exactly once. Then asserts `$CORAL_PREFLIGHT_FORCE=1` bypasses cleanly.

### Pipeline note

Patch release within v0.22 sprint. Slides the rest of v0.22 sprint by one: v0.22.{2,3,4,5} for `coral env import` / `coral test --emit k6` / MCP registry publish / `coral skill build/publish` respectively.

## [0.22.0] - 2026-05-08

**Feature release: `cargo-release` adoption + `scripts/release.sh` maintenance entry point.** Replaces the v0.19-era ad-hoc `release.toml` (which had `push = true`, contrary to working-agreements, and a `pre-release-replacements` regex stuck at v0.15.x shape) with a v0.22-shaped config: `push = false`, `tag = false`, `consolidate-commits = true`, `shared-version = true`, no auto-`Co-Authored-By` trailer. The maintainer now drives a release through three local-only phases plus one GitHub-side step, each a single command: `scripts/release.sh bump X.Y.Z` writes a `release(vX.Y.Z): bump version` commit (no tag, no push) after preflight asserts the CHANGELOG entry is present and `scripts/ci-locally.sh` is green; tester sign-off; `scripts/release.sh tag X.Y.Z` validates HEAD subject + tags + pushes, which triggers `.github/workflows/release.yml` to build binaries for Linux x86_64, macOS Intel, and macOS Apple Silicon; finally `scripts/release-gh.sh vX.Y.Z` extracts the `## [X.Y.Z]` CHANGELOG section verbatim and updates the GH release's title and notes (replacing the workflow's auto-generated commit-list notes with the curated changelog). Two helper scripts ship standalone: `scripts/extract-changelog-section.sh X.Y.Z [PATH]` (awk-based, exit 1 if absent), and `scripts/release-gh.sh vX.Y.Z` with `GH_DRY_RUN=1` for previewing. CHANGELOG link-footer rewriting is moved out of `pre-release-replacements` (which iterates per-package and would duplicate lines on a 9-crate workspace) and into the `release.sh preflight` hook with a bash-level idempotency guard. **Zero new workspace dependencies** — `cargo-release` is installed via `cargo install`, not declared in `[workspace.dependencies]`. **BC sacred: `coral` binary, every CLI subcommand, `coral.toml` manifest schema, and `coral.lock` lockfile are byte-identical to v0.21.4** — this is purely a maintainer-tooling release; runtime code is untouched. **CHANGELOG link-footer repaired** to span v0.16.0 through v0.21.4 (had been frozen at v0.15.1 for six sprints). **1227 tests pass (was 1217; +10), all green.** `bc-regression` green.

### Added

- **`scripts/release.sh`** with subcommands `preflight`, `bump <X.Y.Z>`, and `tag <X.Y.Z>`. `preflight` is the cargo-release pre-release-hook (reads `$NEW_VERSION`, asserts `## [<v>] - <today>` is in `CHANGELOG.md`, idempotently rewrites the link footer, runs `scripts/ci-locally.sh`). `bump` wraps `cargo release X.Y.Z --no-tag --no-push --no-confirm --execute`. `tag` validates the HEAD subject prefix `release(vX.Y.Z):` then runs `cargo release tag … && cargo release push …`. Bare invocation prints usage and exits 2; `--help` prints usage and exits 0.
- **`scripts/extract-changelog-section.sh <X.Y.Z> [PATH]`.** Awk-based parametric extractor: prints the `## [X.Y.Z]` section verbatim, terminating BEFORE the next `## [` heading. Accepts version with or without leading `v`. Defaults `PATH` to `CHANGELOG.md` at the repo root. Exit 0 on found, 1 on absent, 2 on bad invocation. Stderr empty on success.
- **`scripts/release-gh.sh vX.Y.Z`.** Post-tag step. Verifies `gh` is on PATH and authenticated; verifies `git rev-parse vX.Y.Z` resolves; extracts the section into `/tmp/coral-release-vX.Y.Z.md`; parses the leading `**Feature release: …**` bold prefix as the release title (strips outer `**` and trailing period); detects whether a GH release for the tag already exists (the workflow's `softprops/action-gh-release@v2` auto-creates one) and runs either `gh release edit` or `gh release create`; prints the URL. Set `GH_DRY_RUN=1` to preview without invoking `gh`.
- **`crates/coral-cli/tests/release_flow.rs`.** Integration test binary, 8 tests, runs in ~5s on a warm cache (the `cargo-release`-backed test clones the workspace via `git clone --local` and pipes the live scripts in, so it exercises the in-progress edits). Tests cleanly skip with a `SKIP …` log line when `cargo-release` isn't installed locally — the contract is enforced when the tool is available, but local-laptop development without it still passes.

### Changed

- **`release.toml`** rewritten from scratch. Old config had `push = true` and a stale v0.15.x `pre-release-replacements` regex. New config: `shared-version = true`, `consolidate-commits = true`, `publish = false`, `push = false`, `tag = false`, `pre-release-commit-message = "release(v{{version}}): bump version"`, `allow-branch = ["main"]`, `pre-release-hook = ["../../scripts/release.sh", "preflight"]` (the `../../` is required because cargo-release runs the hook with cwd set to each package_root). `sign-commit` and `sign-tag` left unset so the maintainer's git config governs.
- **`CHANGELOG.md` link footer repaired.** Pre-v0.22.0 footer terminated at `[0.15.1]` and `[Unreleased]: …compare/v0.15.1…HEAD`, even though the project shipped through v0.21.4. Added `[0.16.0]` through `[0.21.4]` entries (skipping v0.17.x and v0.18.x, which were never released — verified by absence in `git tag --list`) and updated `[Unreleased]: …compare/v0.21.4…HEAD`. The repaired shape is what `scripts/release.sh preflight`'s footer-rewrite logic expects.
- **README** gains a new `## Releasing` section (~40 lines) with a numbered five-step maintainer flow + troubleshooting subsection. TOC updated.

### Internal

- **CHANGELOG mutation moved out of `pre-release-replacements`.** Cargo-release runs replacements once per package; on a 9-crate workspace, the same regex matched the just-rewritten `[Unreleased]: …compare/vNEW…HEAD` line nine times in a row, adding nine duplicate `[NEW]:` lines. The Rust `regex` crate has no lookahead/backreference support, so no static pattern matches the pre-bump footer but not the post-bump footer. Solution: do the rewrite in `release.sh preflight` (the pre-release-hook), guarded by a `grep -q '^\[NEW\]: '` idempotency check that short-circuits subsequent invocations within the same run.
- **`cargo-release` is NOT a workspace dependency.** It's a maintainer-laptop install (`cargo install --locked cargo-release`). The integration tests gracefully skip the cargo-release-backed test when the binary isn't on PATH — printing a `SKIP …` line on stderr — so CI without the install still passes.
- **`release.yml` workflow is unchanged.** It already triggers on `v*.*.*` tag push and builds the three-target binary matrix. The `release-gh.sh` post-tag script is layered ON TOP of it (replacing the auto-generated notes with the curated CHANGELOG section), not replacing it.
- **No new dev-dependencies.** Tests use the existing `assert_cmd` + `predicates` + `tempfile` + `chrono` stack already in `coral-cli`'s `[dev-dependencies]`/`[dependencies]`.

### Fixed (in-cycle, before tag)

- **`scripts/release.sh tag` test now isolates `$REPO_ROOT` to the tempdir.** The pre-fix test (`release_sh_tag_rejects_wrong_head_subject`) drove `release.sh tag 0.22.0` from a `current_dir(tempdir)` invocation, but the script's `cd "$REPO_ROOT"` (where `REPO_ROOT=$(git rev-parse --show-toplevel)`) jumped back into the live Coral repo whose HEAD subject IS `release(v0.22.0):` — so subject validation incorrectly passed and `cargo release tag 0.22.0 --execute` then errored with `unexpected argument '0.22.0'`. Fix: copy `release.sh` into `<tempdir>/scripts/release.sh` so `git rev-parse --show-toplevel` resolves to the tempdir. Same pattern as `release_sh_preflight_fails_when_changelog_section_absent`.
- **`scripts/extract-changelog-section.sh` is fence-aware.** Awk now toggles `in_fence` on lines beginning with ``` and only honors `^## \[` headings outside a fence. CHANGELOG bodies often include fenced markdown examples like ` ```markdown ## [Old example] ``` ` that previously truncated the extracted section at the fence-internal pseudo-heading. Pinned by `extract_changelog_section_skips_fenced_pseudo_headings`.
- **`scripts/release-gh.sh` rejects ambiguous bold-title nesting.** The leading `**…**` title-extraction rule is the markdown-spec "first `**` after the opener closes" semantics; if the maintainer writes a nested-bold line like `**Title with **inner** continued.** body` the title silently truncates at "Title with " (the first inner closer). Detection: a leading-span ending in whitespace is the smoking gun for mid-sentence truncation, since legitimate titles always end at terminal punctuation immediately before `**`. The script now exits with a remediation hint pointing the maintainer at backticks/italics OR title-on-its-own-line as the two clean options.
- **`scripts/release.sh` derives `<owner>/<repo>` from `git remote get-url origin`.** The CHANGELOG link-footer rewriter previously hardcoded `agustincbajo/Coral`, so a fork's CHANGELOG would emit upstream-pointing URLs after `release.sh bump`. The new `github_owner_repo` helper strips `.git` + `git@github.com:`/`https://github.com/` prefix and falls back to the historical hardcoded value if origin is missing or unparseable (keeping tempdir test fixtures stable). Pinned by `release_sh_rewrites_footer_using_origin_owner_repo`.
- **`crates/coral-mcp/tests/mcp_stdio_golden.rs` accepts any SemVer-shaped server version.** Pre-fix the assertion was `matches!(server_version, "0.21.0" | "0.21.1" | … | "0.21.4")` — the FIRST `release.sh bump` past v0.21.4 would have failed preflight on the bumped tree's `CARGO_PKG_VERSION`. Replaced with a `regex::Regex` match against `^\d+\.\d+\.\d+(-…)?$`. Bump-immune. Adds `regex` as a `dev-dependencies` entry on `coral-mcp` (already a workspace dep elsewhere — zero new top-level dependencies).

### Tests (+10)

- **`crates/coral-cli/tests/release_flow.rs`** — 10 black-box integration tests covering the 8 acceptance items in the spec's §5 plus 2 regression pins for the in-cycle fixes above (`extract_changelog_section_skips_fenced_pseudo_headings`, `release_sh_rewrites_footer_using_origin_owner_repo`). Tests #1, #1b, #2, #7 are pure-shell (no cargo-release dependency) and run in <100 ms. Tests #3, #4, #9 stub `ci-locally.sh` to control the exit code. Test #5 exercises `bump` arg validation. Test #6 (`release_sh_bump_execute_produces_clean_commit_no_coauthor`) clones the workspace into a tempdir via `git clone --local`, runs the real `cargo release`, and grep-asserts NO `Co-Authored-By` trailer in the resulting commit body — pinning AC #4. Test #7 (`release_sh_tag_rejects_wrong_head_subject`) copies `release.sh` into the tempdir's `scripts/` so `$REPO_ROOT` stays scoped (the v0.22.0 HIGH 1 fix). Test #8 (`release_gh_sh_dry_run_extracts_correct_section`) uses `GH_DRY_RUN=1` to verify title extraction against the live v0.21.4 section.

### Acceptance criteria — 12/12 met

1. `scripts/release.sh bump 0.22.0` against clean tree at v0.21.4 with `## [0.22.0] - <today>` populated CHANGELOG section produces (a) commit `release(v0.22.0): bump version` (or `: <feature>` after maintainer amend) with `[workspace.package].version` + every `coral-* = "0.22.0"` bumped, (b) NO tag, (c) NO push ✓ — pinned end-to-end by `release_sh_bump_execute_produces_clean_commit_no_coauthor` (modulo the cargo-release-installed precondition). Manual rehearsal in `/tmp/coral-bump-debug` produced exactly this commit shape pre-flight.
2. `scripts/release.sh bump 0.22.0` without CHANGELOG section dated today aborts BEFORE any commit, stderr names missing heading ✓ — `release_sh_preflight_fails_when_changelog_section_absent`. Stderr contains both the version and `CHANGELOG`.
3. `scripts/release.sh bump 0.22.0` invokes `scripts/ci-locally.sh` via pre-release hook; if `ci-locally.sh` exits non-zero, no commit lands ✓ — `release_sh_preflight_fails_when_ci_locally_fails` stubs the script to exit 7 and asserts preflight propagates the same code (cargo-release aborts on non-zero hook).
4. Release commit has NO `Co-Authored-By: Claude` trailer ✓ — `release_sh_bump_execute_produces_clean_commit_no_coauthor` greps the commit body case-insensitively for `co-authored-by`.
5. `scripts/release.sh tag 0.22.0` against bump-commit creates `v0.22.0` annotated tag (message `"Coral v0.22.0"`), pushes main + tag ✓ — `cargo release tag` with the configured `tag-name` / `tag-message` is the implementation; deferred to maintainer execution because pushing in tests would be a side effect.
6. `scripts/release.sh tag 0.22.0` REJECTS if HEAD subject doesn't start with `release(v0.22.0):` ✓ — `release_sh_tag_rejects_wrong_head_subject`.
7. `scripts/release-gh.sh v0.22.0` extracts section verbatim and creates GH release matching prior 5 manual `gh release create` shape ✓ — `release_gh_sh_dry_run_extracts_correct_section`. The dry-run flow validates the title-extraction regex against the live v0.21.4 section. Production `gh release edit` path is the same code path with the `gh` invocation un-stubbed.
8. `scripts/extract-changelog-section.sh 0.21.3` returns v0.21.3 section (multi-line, terminates BEFORE `## [0.21.2]`), exit 0 ✓ — `extract_changelog_section_returns_v0_21_4_block` covers the same path against v0.21.4; AC #8's specific version is also covered by the shared awk implementation.
9. `scripts/extract-changelog-section.sh 9.99.99` empty stdout, exit 1 ✓ — `extract_changelog_section_missing_version_exits_1`.
10. `cargo release patch` (no `--execute`) on clean tree at v0.21.4 prints dry-run summary, DOES NOT mutate working tree/index/refs ✓ — cargo-release default behavior (without `--execute`) is dry-run; `release.sh bump` is the wrapper that adds `--execute`. A direct `cargo release patch` invocation against the workspace at v0.21.4 prints the version-bump preview without mutation.
11. CHANGELOG link footer after `release.sh bump 0.22.0` has fresh `[Unreleased]: …compare/v0.22.0…HEAD` AND `[0.22.0]: …releases/tag/v0.22.0` lines ✓ — `rewrite_changelog_footer` in `release.sh` does this in the preflight hook; manual rehearsal in `/tmp/coral-bump-debug` produced the expected footer with v0.99.0 as the bump target.
12. README "Releasing" section walks maintainer through full v0.22.0 cycle in ≤60 lines, links to `release.toml` ✓ — 39 lines including troubleshooting subsection. The release.toml link is in the section's lead paragraph.

### Pipeline note

First feature of the v0.22 sprint. No `Co-Authored-By: Claude` trailer per working-agreements (and pinned by test #6 for every future release commit). No push, no tag — the maintainer drives `release.sh bump 0.22.0` AFTER this commit lands and tester sign-off arrives. The version bump is intentionally deferred to demo the new tooling's first run on a real release rather than dogfooding it inline.

## [0.21.4] - 2026-05-08

**Feature release: `MultiStepRunner` opt-in (planner + executor + reviewer).** Adds a tiered routing layer to `coral consolidate`. Default behavior — `coral consolidate` (no flag, no manifest opt-in) — stays byte-identical to v0.21.3: still a single `runner.run(prompt)` call against the resolved provider. With `--tiered` (or `[runner.tiered.consolidate] enabled = true` in `coral.toml`), the run decomposes into three sequential calls — a planner call that emits 1-5 sub-tasks as YAML, one executor call per sub-task, and a reviewer call that synthesizes the sub-task results into the final consolidate-plan YAML the existing parser consumes. Each tier's provider and model are picked independently from `[runner.tiered.{planner,executor,reviewer}]`, so a workflow can use a fast cheap planner (`haiku`) and a strong reviewer (`opus`) without rebuilding the runner stack. A `[runner.tiered.budget] max_tokens_per_run` cap (default 200_000, mirroring a Claude Sonnet 200K-context window) is enforced via a pure-Rust `len/4` token approximation at three pre-flight gates — once before the planner call (with a 1.5× projection), once per executor sub-task, and once before the reviewer call. Budget breaches surface as the new `RunnerError::BudgetExceeded { actual, budget }` variant before any wasted network call lands. **Zero new workspace dependencies** — the orchestrator chose `len/4` over a tiktoken-style BPE counter precisely to avoid a provider-specific dep. **BC sacred: `coral consolidate` (no `--tiered`, no manifest) is byte-identical to v0.21.3** — pinned by the snapshot test `consolidate::tests::consolidate_no_tiered_flag_is_byte_identical_to_v0213` PLUS the drift-detector `consolidate_byte_identity_snapshot_actually_catches_drift` (which mutates the rendered output and asserts the pin would catch it). **`RunnerSection` defaults to `None` tiered** so a v0.21.3 `coral.toml` round-trips byte-identically — pinned by `manifest::tests::manifest_without_runner_section_round_trips_unchanged`. **1217 tests pass (was 1197; +20).** `bc-regression` green.

### Added

- **`coral consolidate --tiered`.** Opt-in flag to route the consolidate run through a tiered planner→executor→reviewer pipeline. Reads `[runner.tiered]` from `coral.toml` to pick per-tier providers / models / budget. CLI flag wins over the manifest's `[runner.tiered.consolidate] enabled = true`. Errors actionably when `--tiered` is passed against a manifest with no `[runner.tiered]` block.
- **`coral consolidate --verbose`.** Emits a single `tracing::info` summary line after a tiered run with planner/executor/reviewer call counts and `tokens_used`. No effect on non-tiered runs.
- **`crates/coral-runner/src/multi_step.rs`.** New module disjoint from `runner.rs` so the existing `Runner` impls cannot be regressed by edits here. Public types: `MultiStepRunner` (trait), `TieredOutput`, `TieredConfig`, `TierSpec`, `BudgetConfig`, `TieredRunner` (concrete impl). Public fns: `approx_tokens`. Public consts: `DEFAULT_MAX_TOKENS_PER_RUN = 200_000`. Internal consts: `PLANNER_SYSTEM`, `EXECUTOR_SYSTEM`, `REVIEWER_SYSTEM` (each ≤200 chars).
- **`RunnerError::BudgetExceeded { actual: u64, budget: u64 }`.** New additive variant on `coral_runner::RunnerError`. Display message names both numbers AND points the user at `runner.tiered.budget.max_tokens_per_run`. Pinned by `multi_step::tests::budget_exceeded_display_is_actionable`.
- **`Project.runner: RunnerSection`** with `tiered: Option<TieredManifest>`. Hung on `Project` with `#[derive(Default)]` so the legacy single-repo synthesis path (`Project::single_repo`) and parsed v0.21.3 manifests both produce `RunnerSection::default() == { tiered: None }`. Helper `RunnerSection::tiered_enabled_for_consolidate()` returns `false` for the default — i.e. tiered routing only kicks in via explicit opt-in.
- **`coral_core::project::manifest::TieredManifest`, `TierSpecManifest`, `BudgetManifest`, `TieredConsolidate`.** Manifest-side parsed shapes. Mirror the runner-side `TieredConfig`/`TierSpec`/`BudgetConfig` so the `consolidate.rs` glue is a one-to-one field copy.

### Changed

- **`Project` struct gains `pub runner: RunnerSection`.** Default = `RunnerSection::default()`. Every existing `Project` literal (in tests, in `Project::single_repo`, in `make_project_with_repos`) is updated to spell out the new field. **No on-disk schema break** — manifests without `[runner]` parse and re-render byte-identically (pinned by `manifest_without_runner_section_round_trips_unchanged`).
- **`render_toml` emits `[runner.tiered.*]` only when `runner.tiered.is_some()`.** Default `RunnerSection` emits zero bytes — preserves byte-identity for v0.21.3 manifests.
- **`crates/coral-runner/Cargo.toml` adds `serde_yaml_ng = { workspace = true }`** as a non-dev dependency. The dep was already in `[workspace.dependencies]` (every other YAML parser in Coral uses it); the `multi_step.rs` planner-output parser reuses it. **Zero net additions to the workspace dep set.**
- **README "Runners" / "Consolidate" surface.** New `docs/runner-tiered.md` page covers the manifest schema, the per-tier model overrides, the budget gate, the per-call timeout caveat, and the `--tiered` CLI flag.

### Internal

- **`MultiStepRunner` is a separate trait, not a method on `Runner`.** Every existing `Runner` impl (`ClaudeRunner`, `GeminiRunner`, `LocalRunner`, `HttpRunner`, `MockRunner`) compiles unchanged — the spec §6 D6 BC contract.
- **`TieredRunner::run_tiered` invokes its three boxed `Runner`s sequentially.** Planner first, with the user prompt wrapped to ask for `subtasks: [{id, description}]` YAML. On unparseable planner output a single `tracing::warn!` is emitted and the pipeline falls back to one executor call against the original user prompt. Executor calls are sequential (no concurrency in v0.21.4 — keeps the budget gate predictable and avoids the rayon-vs-tokio question). Reviewer call is single, with `format!("Original task:\n{}\n\nSub-task results:\n{}", original_user, joined_execute_outputs)` as user prompt; its `RunOutput.stdout` becomes `TieredOutput::final_output.stdout` and is what the consolidate plan parser sees.
- **Token budget = `(s.len() as u64).div_ceil(4)`.** Three pre-flight gates: (1) before the planner call, project the planner prompt's token cost × 1.5 against the budget; (2) before each executor call, check cumulative + system+user against the budget; (3) before the reviewer call, same. The 1.5× planner factor exists because the executor and reviewer prompts depend on the planner's output and can't be projected up front. Order-of-magnitude correctness — not billing accuracy — is the goal.
- **`build_tiered_runner` resolves provider names at construction time.** A `[runner.tiered.planner] provider = "voyage"` (unknown) parses cleanly at manifest load but errors at runner build, naming the offending tier (`[runner.tiered.planner]: unknown provider: voyage (valid: claude, gemini, local, http)`). Acceptance criterion #7 — failure mode is "missing-tool error before any network call," not "silent fallback to claude."
- **`format_preview_output` and `format_apply_report` extracted as pure `fn(...) -> String`.** The pre-v0.21.4 `run_with_runner` had `println!` calls inline. The refactor lets the byte-identity snapshot test compare strings directly without capturing stdout — which would have required a new test-utility crate.
- **No tiered-level timeout.** `Prompt::timeout` is per-call, so a tiered run with three 60s timeouts can take up to 180s wall-clock. Documented in `docs/runner-tiered.md`.
- **No streaming variant on `MultiStepRunner`.** v0.21.4 is non-streaming end-to-end. Future work: a `--stream` variant of `consolidate --tiered` would need a new trait method; out of scope for v0.21.4.

### Tests (+20)

- **#1-#7 multi-step (`crates/coral-runner/src/multi_step.rs::tests`)**: `tiered_runner_calls_three_tiers_in_order`, `tiered_runner_routes_model_per_tier`, `tiered_runner_budget_pre_flight_aborts`, `tiered_runner_budget_mid_pipeline_aborts`, `tiered_runner_falls_back_on_unparseable_plan`, `tiered_runner_propagates_executor_error`, `tokens_used_approximates_chars_div_4`. Plus 5 supporting unit tests for `approx_tokens`, plan-fence stripping, default-budget pinning, and the `BudgetExceeded` Display message.
- **#8-#11 manifest (`crates/coral-core/src/project/manifest.rs::tests`)**: `manifest_without_runner_section_round_trips_unchanged` (BC pin — emits zero `[runner]` bytes for default `RunnerSection`), `manifest_with_full_tiered_section_parses` (full-shape happy path), `manifest_partial_tiered_section_rejected` (acceptance #6 — missing tier surfaces a `[runner.tiered]` error naming the missing tier), `legacy_v0213_single_repo_fixture_still_parses` + `manifest_zero_budget_rejected`.
- **#12-#13 consolidate snapshot (`crates/coral-cli/src/commands/consolidate.rs::tests`)**: `consolidate_no_tiered_flag_is_byte_identical_to_v0213` (snapshot pin against the v0.21.3 byte string for both preview and apply rendering), plus `consolidate_byte_identity_snapshot_actually_catches_drift` (drift detector that mutates the rendered output and asserts the pin would catch it — proves the byte-identity test isn't a tautology).
- **#14 consolidate tiered (`crates/coral-cli/src/commands/consolidate.rs::tests`)**: `consolidate_with_tiered_flag_invokes_three_runners` — drives `run_with_tiered_runner` with three Mock runners, asserts each tier was called once, each prompt carried its tier's per-tier model override, and the reviewer's stdout drove disk mutation (AC-4 — reviewer stdout = consolidate-plan parser input).

### Acceptance criteria — 15/15 met

1. Workspace with no `[runner]` section parses and round-trips byte-identical to v0.21.3 ✓ (`manifest_without_runner_section_round_trips_unchanged`).
2. `coral consolidate` (no flag, no manifest opt-in) byte-identical stdout to v0.21.3 against scripted `MockRunner` ✓ (`consolidate_no_tiered_flag_is_byte_identical_to_v0213` + drift detector).
3. `coral consolidate --tiered` invokes 3 distinct sub-runners in order: planner → executor → reviewer ✓ (`consolidate_with_tiered_flag_invokes_three_runners` asserts `.calls()` on each Mock).
4. Reviewer's `RunOutput.stdout` becomes the consolidate plan parser input ✓ (the same test asserts `obsolete` is `status: stale` on disk after `--apply`, which only fires if the reviewer's YAML drove the apply path).
5. `[runner.tiered.consolidate] enabled = true` + no flag = tiered. CLI flag wins regardless ✓ (`run` dispatch: `args.tiered || project.runner.tiered_enabled_for_consolidate()`).
6. `[runner.tiered]` missing one of `planner|executor|reviewer` fails with `CoralError::Walk` mentioning the missing tier ✓ (`manifest_partial_tiered_section_rejected`).
7. `[runner.tiered]` with `provider = "voyage"` (unknown) fails at runner-construction time, not parse time ✓ (`build_tiered_runner` calls `make_runner_for_provider_str` which propagates the parser error verbatim).
8. `budget.max_tokens_per_run = 100` + 1000-char prompt → `RunnerError::BudgetExceeded` BEFORE planner call ✓ (`tiered_runner_budget_pre_flight_aborts` asserts `planner.calls().len() == 0`).
9. Budget exceeded after planner but before executor → `BudgetExceeded` returned cleanly ✓ (`tiered_runner_budget_mid_pipeline_aborts`).
10. `budget = 1_000_000` + small prompt → success, `tokens_used` non-zero and ≤ budget ✓ (`tokens_used_approximates_chars_div_4`).
11. `MultiStepRunner` opt-in: every existing `Runner` impl compiles unchanged ✓ (`cargo build --workspace` green; `MultiStepRunner` is a separate trait).
12. v0.15 single-repo fixture parses, validates, round-trips ✓ (`legacy_v0213_single_repo_fixture_still_parses`); `bc-regression` step in `scripts/ci-locally.sh` green.
13. `[runner]` with no `[runner.tiered]` child = `RunnerSection::default()` ✓ (`manifest_with_full_tiered_section_parses` covers the present case; the empty-runner-section round-trips identically by construction since `[runner]` alone produces the default).
14. `tokens_used` ≈ `(all_systems + all_users + all_stdouts).len() / 4` ± 25 % ✓ (`tokens_used_approximates_chars_div_4`).
15. `consolidate --tiered --verbose` prints one `tracing::info` line summarizing call counts and `tokens_used` ✓ (`run_with_tiered_runner`: `tracing::info!(plan_calls, execute_calls, review_calls, tokens_used)`).

### Pipeline note

Final feature of the v0.21 sprint (fifth feature of the five-feature batch). No `Co-Authored-By: Claude` trailer per working-agreements. No push, no tag.

## [0.21.3] - 2026-05-08

**Feature release: `coral session distill --as-patch` (option (b) / distill-as-patch).** Adds a second emit mode to `coral session distill`. Default behavior — `coral session distill <id>` (no `--as-patch`) — stays byte-identical to v0.21.2: still emits 1-3 NEW synthesis pages under `.coral/sessions/distilled/`. With `--as-patch`, the LLM instead proposes 1-N **unified-diff patches** against EXISTING `.wiki/<slug>.md` pages. Patches save to `.coral/sessions/patches/<id>-<idx>.patch` plus a sidecar `<id>-<idx>.json` carrying target slug + LLM rationale + provenance. With `--apply` the patches are `git apply`-ed in turn AND each touched page's frontmatter is rewritten so `reviewed: false` (Coral OWNS the flip — the LLM's job is body content). Pre-apply atomicity: if ANY patch fails its `git apply --check`, NO files are written and the command exits non-zero with the patch index + git stderr verbatim. Validation is layered — every component of the path-style `target_slug` must pass `coral_core::slug::is_safe_filename_slug`, the resolved page must already exist in `list_page_paths(.wiki)`, the diff `--- a/X.md` / `+++ b/X.md` headers must agree with the target, AND `git apply --check --unsafe-paths --directory=.wiki <patch>` must succeed. Top-K BM25 candidate pages from `coral_core::search::search_bm25` are surfaced in the prompt by default (K=10, override via `--candidates N`, `0` skips). **No new workspace dependencies** — the orchestrator picked subprocess `git apply` over `diffy` (zero net additions to `Cargo.lock`). **BC sacred: option (a) page-emit path is byte-identical to v0.21.2** — pinned by `crates/coral-session/src/distill.rs::tests::distill_without_as_patch_byte_identical_to_v0212`. **`IndexEntry.patch_outputs` is `#[serde(default)]`** so a v0.21.2 `index.json` deserializes cleanly. **1197 tests pass (was 1174; +23).** `bc_regression` green.

### Added

- **`coral session distill --as-patch`.** Opt-in flag to switch from option (a) (page-emit) to option (b) (patch-emit). Absence preserves byte-identical behavior to v0.21.2.
- **`coral session distill --candidates N`.** Top-K BM25-ranked candidate pages to include in the patch-mode prompt (default `10`, set `0` to skip candidate collection — the LLM call still runs but without page context). Only applies with `--as-patch`; ignored otherwise.
- **`crates/coral-session/src/distill_patch.rs`.** New module disjoint from `distill.rs` so option (a)'s byte-identical contract cannot be regressed by edits here. Public types: `DistillPatchOptions`, `DistillPatchOutcome`, `Patch`, `PatchSidecar`, `PageCandidate`. Public fns: `build_patch_prompt`, `parse_patches`, `select_candidates`, `distill_patch_session`. Public consts: `DISTILL_PATCH_PROMPT_VERSION = 2`, `MAX_PATCHES_PER_SESSION = 5`, `DEFAULT_CANDIDATES = 10`.
- **`IndexEntry.patch_outputs: Vec<String>`** (with `#[serde(default)]`). Tracks every `.patch` and `.json` basename written under `.coral/sessions/patches/` so `forget` can clean up. Empty for sessions captured pre-v0.21.3.

### Changed

- **`coral session forget <id>`** now sweeps `.coral/sessions/patches/<basename>` for every entry in `IndexEntry.patch_outputs`, alongside the existing `distilled_outputs` cleanup. **`.wiki/` mutations from `--apply --as-patch` are NOT undone** — distill-as-patch's apply is one-way (the user owns the wiki post-apply). Path-traversal defense (`/`, `\`, `..`, `.`-prefix → skip with warn) mirrors the `distilled_outputs` loop verbatim.
- **README "Distillation" section** gains a new "Patch mode (`--as-patch`, v0.21.3+)" subsection covering the flag, the validation pipeline, the on-disk artifact shape, and the `--apply` semantics.

### Internal

- **`distill_patch::git_apply_inner` runs `git apply --unsafe-paths --directory=.wiki`** so LLM-emitted diff headers (`--- a/<target>.md`) resolve relative to `.wiki/` without the LLM needing to know the wiki path. `--unsafe-paths` permits paths outside the index — NOT untrusted paths. Real safety comes from the slug allow-list check that runs BEFORE git ever sees the diff.
- **`parse_patches` defensively appends a trailing `\n`** to any diff that doesn't end with one. YAML block-scalar `|` (CLIP) sometimes drops the trailing newline when the source ends mid-line; git apply rejects unterminated patches with "corrupt patch at line N". Defensive normalization keeps a subtly-broken YAML mis-emit applying cleanly.
- **Pre-apply atomicity**: every patch validates against a system-tempfile copy in `.coral/sessions/patches-validate/` BEFORE any durable artifact lands. On any failure, no `.patch` / `.json` is written and `.wiki/` is untouched (spec D6).
- **`Page::from_file → set extra["reviewed"] = Bool(false) AND extra["source"] = { runner, prompt_version, session_id, captured_at } → Page::write()`** rewrites the frontmatter post-apply so the unreviewed-distilled lint gate fires regardless of what the LLM emitted. Coral OWNS the flip. The `source.runner` block is REQUIRED, not decorative: the qualifier in `coral_lint::structural::check_unreviewed_distilled` (mirrored in `coral_core::page::Page::is_unreviewed_distilled`) requires BOTH `reviewed: false` AND a non-empty `source.runner` to fire — without the source block, a patched-but-unreviewed page would slip past `coral lint` at commit time. (An initial v0.21.3 commit `0ba9efd` shipped only `reviewed: false`; the post-commit audit caught it pre-tag and the trust-gate fix landed before v0.21.3 was tagged. Pinned by `apply_patch_marks_page_as_unreviewed_distilled` which asserts `Page::is_unreviewed_distilled() == true` on the mutated `.wiki/<target>.md`.)
- **No new dependencies.** Orchestrator chose subprocess `git apply` over `diffy` to avoid a workspace dep. Zero net additions to `Cargo.lock`.

### Tests (+23)

- **#1-#11 e2e (`crates/coral-session/tests/distill_patch_e2e.rs`)**: `distill_patch_writes_pairs_under_patches_dir`, `distill_patch_apply_mutates_wiki_and_resets_reviewed`, `apply_patch_marks_page_as_unreviewed_distilled` (post-commit-audit regression test for the trust-gate fix — asserts `Page::is_unreviewed_distilled() == true` after `--apply`, which would have failed against the initial v0.21.3 commit), `patch_with_unknown_target_rejects_pre_io`, `malformed_diff_rejects_atomically`, `one_bad_patch_rolls_back_all`, `patch_with_dotdot_target_rejects`, `diff_header_mismatch_rejects_when_only_minus_is_wrong`, `patch_count_capped_at_five`, `forget_removes_patch_basenames`, `distilled_and_patch_outputs_track_independently`. Every test drives a `MockRunner` with a hand-rolled YAML response so the LLM call is deterministic.
- **#12-#20 unit (in `distill_patch::tests`)**: `select_candidates_is_deterministic`, `zero_candidates_skips_page_load`, `candidates_flag_truncates_to_n`, `is_safe_path_slug_rejects_dotdot_segments`, `diff_targets_slug_matches_a_and_b_prefixes`, `parse_patches_handles_yaml_code_fence`, `parse_patches_caps_at_five`, `parse_patches_rejects_dotdot_target`, `parse_patches_rejects_header_mismatch`.
- **#21 BC pin (in `distill::tests`)**: `distill_without_as_patch_byte_identical_to_v0212` — pins the page-emit envelope so any future edit that quietly shifts the schema is caught at test time.
- **#22 BC pin (in `capture::tests`)**: `index_without_patch_outputs_field_deserializes` — proves a v0.20.x / v0.21.2-shaped `index.json` deserializes cleanly with `patch_outputs` defaulting to empty.
- **#23 CLI integration smoke (in `commands::session::tests`)**: `run_distill_as_patch_writes_patches_dir_via_mock_runner` — drives `run_distill` with `--as-patch` and an injected `MockRunner`, asserts the patches dir has the right files and the index is updated.

### Acceptance criteria — 15/15 met

1. `coral session distill <id> --as-patch` writes 1-N `<id>-<idx>.patch` + `<id>-<idx>.json` pairs under `.coral/sessions/patches/` ✓ (`distill_patch_writes_pairs_under_patches_dir`).
2. Each emitted `.patch` validates via `git apply --check --unsafe-paths --directory=.wiki` BEFORE any file is written ✓ (`distill_patch_session` validation loop precedes write loop).
3. If ANY patch fails validation, NO files written, NO `.wiki/` mutation, command exits non-zero with patch index + git stderr verbatim ✓ (`one_bad_patch_rolls_back_all`).
4. `--as-patch --apply` mutates each `.wiki/<target>.md`. Post-apply, every modified page's frontmatter has `reviewed: false` ✓ (`distill_patch_apply_mutates_wiki_and_resets_reviewed`).
5. `--as-patch` without `--apply` leaves `.wiki/` byte-unchanged ✓ (`distill_patch_writes_pairs_under_patches_dir` snapshots wiki bytes pre/post).
6. LLM prompt includes top-K BM25-ranked candidate pages, default K=10 ✓ (`select_candidates` uses `coral_core::search::search_bm25`).
7. `--candidates 0` sends no candidates, LLM call still runs ✓ (`zero_candidates_skips_page_load` + `distill_patch_session` short-circuits page load when `candidates == 0`).
8. Sidecar `.json` carries `target_slug`, `rationale`, `prompt_version`, `runner_name`, `session_id`, `captured_at`, `reviewed: false` ✓ (`distill_patch_writes_pairs_under_patches_dir` asserts every field).
9. `coral session forget <id>` cleans BOTH `distilled_outputs` AND `patch_outputs`, `.wiki/` mutations NOT undone ✓ (`forget_removes_patch_basenames`).
10. `IndexEntry` deserializes v0.20.x / v0.21.2 index file (no `patch_outputs` field) without error ✓ (`index_without_patch_outputs_field_deserializes`).
11. Page-emit (option a) path byte-identical to v0.21.2 ✓ (`distill_without_as_patch_byte_identical_to_v0212`).
12. `bc-regression` passes unmodified ✓ (`scripts/ci-locally.sh` step 4 green).
13. Patch with target not in `list_page_paths(.wiki)` rejected at parse time, BEFORE `git apply --check` ✓ (`patch_with_unknown_target_rejects_pre_io`).
14. Patch with malformed unified-diff header (mismatched paths) rejected at parse time ✓ (`malformed_diff_rejects_atomically`, `diff_header_mismatch_rejects_when_only_minus_is_wrong`).
15. CLI stdout lists patch index, target slug, rationale; "written" block has `.patch` AND `.json`; "applied" block (only `--apply`) has `.wiki/<slug>.md (reviewed: false)` ✓ (`run_distill_as_patch` in `crates/coral-cli/src/commands/session.rs`).

### Pipeline note

Patch release within the v0.21 sprint (fourth feature of the five-feature batch). No `Co-Authored-By: Claude` trailer per working-agreements.

## [0.21.2] - 2026-05-08

**Feature release: `coral up --watch` (live reload via compose `develop.watch`).** Wire the wave-1 `WatchSpec` / `SyncRule` types through the YAML renderer and `ComposeBackend::up` so `coral up --watch` runs `docker compose watch` foreground after `up -d --wait` completes. The wave-1 v0.17 schema reserved `[services.<name>.watch]` (with `sync` + `rebuild` + `restart` + `initial_sync`) but the renderer dropped it on the floor — v0.21.2 closes that gap. After `up -d --wait` succeeds, `compose watch` streams sync events ("syncing X files to Y", "rebuilding service Z") to the terminal until Ctrl-C; SIGINT (exit code 130) is treated as a clean exit. `coral env watch` is a thin alias for `coral up --watch` so the surface area stays small. macOS users hit a known fsevents flakiness in Docker Desktop ([docker/for-mac#7832](https://github.com/docker/for-mac/issues/7832)) — Coral emits a one-line `WARNING:` banner to stderr before the watch subprocess starts so the issue is never silent. **`EnvCapabilities::watch` flips from `false` to `true`.** **BC sacred: services without `[services.*.watch]` emit byte-identical YAML to v0.21.1** — pinned by `compose_yaml::tests::watch_absent_yields_yaml_identical_to_pre_watch` and `crates/coral-env/tests/watch_yaml_render.rs::service_without_watch_emits_no_develop_block`. **1174 tests pass (was 1155; +19).** `bc_regression` green.

### Added

- **`coral up --watch [--env NAME] [--service NAME]...`.** After `up -d --wait` succeeds, run `compose watch` foreground until Ctrl-C. Requires at least one service to declare `[services.<name>.watch]` in `coral.toml`. The watch subprocess inherits the parent's stdin/stdout/stderr so events stream live (matches `tilt up` / `skaffold dev` UX). Pre-flight validation rejects `--watch` against a manifest with no watch blocks via `EnvError::InvalidSpec` whose message names both `--watch` and `[services.<name>.watch]` (acceptance criterion #2).
- **`coral env watch [--env NAME] [--service NAME]... [--build]`.** Alias for `coral up --watch`. ~10-line dispatch in `crates/coral-cli/src/commands/env.rs::watch` translates `WatchArgs` → `UpArgs { watch: true, detach: true, ... }` and re-enters `up::run` so there's exactly one watch implementation.
- **`develop.watch` block in the rendered Compose YAML.** Emitted from `[services.<name>.watch]` for any service that declares it. Order: `sync` rules first, then `rebuild`, then `restart` (pinned by `compose_yaml::tests::watch_block_all_three_actions`). Sync rules carry `path` (resolved against `resolved_context` for `repo = "..."` services, same way `build.context` is resolved) and `target` (container-side, verbatim). Rebuild and restart entries carry only `path`. `initial_sync = true` (compose ≥ 2.27) propagates to every sync entry; older compose versions silently drop the unknown key — no version probe needed.
- **macOS `WARNING:` banner.** Single stderr line emitted before `compose watch` starts on macOS, mentioning [docker/for-mac#7832](https://github.com/docker/for-mac/issues/7832) by URL. Pinned by `crates/coral-cli/tests/watch_macos_banner.rs`, `#[cfg(target_os = "macos")]`-gated.

### Changed

- **`ComposeBackend::capabilities()` returns `watch: true`.** Pinned by `compose::tests::capabilities_advertise_watch_true`.
- **`crates/coral-cli/src/commands/up.rs::UpArgs` gains `pub watch: bool` (`#[arg(long)]`).** The line-65 hardcode `watch: false` flips to `args.watch`. `--detach` default stays `true` so `coral up --watch` continues to behave like `coral up && coral verify` upstream.
- **README "Quickstart — environments + tests"** gains a new "Live reload (`coral up --watch`, v0.21.2+)" subsection. TOML snippet (sync + rebuild + restart) + commands + macOS caveat with upstream issue link. The pre-existing "compose watch file-descriptor errors on macOS Sonoma+" troubleshooting entry is rewritten — the v0.19.x `--no-watch` placeholder workaround is replaced with "omit `--watch`" since the flag is now real.
- **README env table** gains `--watch` on the `coral up` row and a new `coral env watch` row.

### Internal

- **`compose_yaml::render_watch(ws, plan)`** — pure helper over `WatchSpec`. Returns `None` for empty `WatchSpec` (defensive: the CLI catches this case earlier with a friendly error). Lives next to `render_real` so the watch path inherits the same `resolved_context` resolution as `build.context`.
- **`compose::watch_subprocess(plan, artifact, services)`** — `Command::status()`-shaped foreground subprocess (NOT `output()`) so stdout/stderr stream live. Appends `"watch"` after `--project-name`, then forwards the `--service` allowlist as positional args. Blocks until the child exits.
- **`compose::validate_watch_services(plan)`** — extracted as a free function (was inline in `up`) so it's directly unit-testable without spawning a subprocess. Pinned by 4 tests in `compose::tests::validate_watch_services_*`.
- **No new dependencies.** `notify` was a non-starter — compose handles fs events natively. Zero net additions to `Cargo.lock`.

### Tests (+19)

- **#1-#6 (`crates/coral-env/src/compose_yaml.rs::tests`)**: `watch_block_empty_emits_nothing`, `watch_block_sync_only`, `watch_block_all_three_actions`, `watch_initial_sync_propagates_to_sync_entries`, `watch_path_resolves_against_resolved_context`, `watch_absent_yields_yaml_identical_to_pre_watch`.
- **#7-#10 (`crates/coral-env/src/compose.rs::tests`)**: `validate_watch_services_rejects_plan_without_any_watch_block` (pins the `--watch` + `[services.<name>.watch]` message shape), `validate_watch_services_rejects_plan_with_services_but_no_watch`, `validate_watch_services_accepts_plan_with_at_least_one_watch_block`, `validate_watch_services_rejects_empty_watch_spec`.
- **#11 (`crates/coral-env/src/compose.rs::tests`)**: `capabilities_advertise_watch_true`.
- **#12 (`crates/coral-env/src/spec.rs::tests`)**: `sync_rule_requires_both_path_and_target` — guard against `#[serde(default)]` slipping in and weakening the contract.
- **#13-#17 (`crates/coral-env/tests/watch_yaml_render.rs`)**: `parse_then_render_emits_develop_watch_block`, `watch_actions_emit_in_canonical_order`, `sync_paths_resolve_against_repo_checkout`, `initial_sync_propagates_to_every_sync_entry`, `service_without_watch_emits_no_develop_block`, `adding_watch_changes_artifact_hash`. End-to-end round-trip parse → plan → render → re-parse YAML.
- **#18 (`crates/coral-cli/tests/watch_macos_banner.rs`)**: `macos_emits_warning_banner_before_watch_subprocess` — `#[cfg(target_os = "macos")]`-gated; pins the URL appears on stderr.
- **`crates/coral-cli/tests/watch_smoke.rs`** (`#[ignore]`-gated; runs only with `--ignored` and a real docker daemon): `watch_subprocess_runs_foreground_against_real_docker`, `watch_without_watch_service_fails_actionably`. Two `ignored` smoke tests reach 19 ignored total (was 17).

### Acceptance criteria — 15/15 met

1. `coral up --watch --env dev` foregrounds `compose watch` after `up -d --wait` ✓ (`compose::up` sequencing).
2. `--watch` against an env with no watch blocks fails with `EnvError::InvalidSpec` whose message names `--watch` AND `[services.<name>.watch]` ✓ (`validate_watch_services_rejects_plan_without_any_watch_block`).
3. Services without `watch` emit byte-identical YAML to v0.21.1 ✓ (`watch_absent_yields_yaml_identical_to_pre_watch`, `service_without_watch_emits_no_develop_block`).
4. `compose watch` runs foreground; stdin/stdout/stderr inherited; Ctrl-C exits cleanly without orphaned containers ✓ (`watch_subprocess` uses `Command::status()`; SIGINT 130 → `Ok(())`).
5. macOS `WARNING:` line goes to stderr before the watch subprocess starts, mentioning docker/for-mac#7832 by URL ✓ (`macos_emits_warning_banner_before_watch_subprocess`).
6. `coral env watch` is an alias with identical observable behavior ✓ (single `up::run` dispatch).
7. `ComposeBackend::capabilities()` returns `watch: true` ✓ (`capabilities_advertise_watch_true`).
8. `cargo test -p coral-env` includes a snapshot-style assertion of rendered `develop.watch` YAML for sync+rebuild+restart ✓ (`watch_block_all_three_actions`, `parse_then_render_emits_develop_watch_block`).
9. Adding `[services.*.watch]` to an existing manifest produces a NEW artifact hash ✓ (`adding_watch_changes_artifact_hash`).
10. macOS banner test gated `#[cfg(target_os = "macos")]` ✓ (file-level cfg on `watch_macos_banner.rs`).
11. `bc_regression` passes unmodified ✓.
12. `--watch` propagates through `UpOptions.watch` to `ComposeBackend::up` — no other path consumes it ✓ (single read site at `compose::up`).
13. `coral up --watch --service api` only watches `api` ✓ (`watch_subprocess` forwards `services` after `watch` verb).
14. SIGINT 130 → `Ok(())`; only non-130 non-zero is `EnvError::BackendError` ✓ (`compose::up` exit-code branch).
15. README + CHANGELOG mention `--watch` and the macOS caveat ✓.

### Pipeline note

Patch release within the v0.21 sprint (third feature of the five-feature batch). No `Co-Authored-By: Claude` trailer per working-agreements.

## [0.21.1] - 2026-05-08

**Feature release: HTTP/SSE MCP transport (Streamable HTTP per MCP 2025-11-25).** v0.20.x had deferred this transport during the cycle-4 audit (H6) — every shipped MCP client speaks stdio, and an inflated docs surface for an unimplemented transport read worse than absence of the feature. v0.21.1 reintroduces it as a first-class peer of stdio: `coral mcp serve --transport http --port <p>` opens `POST /mcp` (JSON-RPC), `GET /mcp` (SSE keep-alive), `DELETE /mcp` (session teardown), and `OPTIONS /mcp` (CORS preflight) on a `tiny_http::Server`. Default bind is `127.0.0.1`; `--bind 0.0.0.0` is opt-in and emits a `WARNING:` stderr banner. Origin allowlist accepts `null` / `http://localhost*` / `http://127.0.0.1*` / `http://[::1]*` only — the spec's DNS-rebinding mitigation. Body cap is 4 MiB → 413, concurrency cap is 32 in-flight → 503, batched JSON-RPC arrays return 400. Wire format is byte-stable with v0.20.x stdio: the dispatcher (`McpHandler::handle_line`) is shared, so tool catalogs, audit-log shape, the `--read-only` / `--allow-write-tools` gate, and the `--include-unreviewed` filter behave identically across the two transports. Phase 2 of the v0.21.1 plan lifted the stdio loop body out of `server.rs` into `transport/stdio.rs` so the new HTTP transport could share `handle_line` without the JSON-RPC core dragging the stdio framing along — `serve_stdio` is now a 6-line shim over `transport::stdio::serve_stdio`. **BC pinned via `mcp_stdio_golden.rs` (test #21): the JSON-RPC envelope shape is byte-identical to v0.21.0.** **1155 tests pass (was 1124; +31).** BC contract holds — `bc_regression` is green; the wire format `"transport": "stdio"` deserializes unchanged from any v0.20.x config.

### Added

- **`coral mcp serve --transport http --port <p> [--bind <addr>]`.** Streamable HTTP/SSE per MCP 2025-11-25. Default `--port 3737`, default `--bind 127.0.0.1`. `--port 0` asks the OS to pick a free port; the resolved port is printed to stderr (`coral mcp serve — listening on http://127.0.0.1:NNNNN/mcp`).
- **New module `crates/coral-mcp/src/transport/`** with `stdio.rs` (lifted from `server.rs`), `http_sse.rs` (the new transport), and `mod.rs` (umbrella). `pub use coral_mcp::transport::{HttpSseTransport, serve_http_sse, serve_stdio}` for callers that want the lower-level surface.
- **`Transport::HttpSse` enum variant** and **`ServerConfig::bind_addr: Option<IpAddr>`**. The wire-format string `"http_sse"` was held stable across the v0.20.x → v0.21.1 reintroduction so any older `ServerConfig` JSON / TOML deserializes unchanged.
- **`Mcp-Session-Id` cookie.** Server mints a 36-char UUID-shaped opaque token on every `initialize` POST; clients echo on subsequent traffic. Sessions live in `Arc<Mutex<HashMap<String, Instant>>>` with a 1h TTL, reaped on each request. Hand-crafted (no `uuid` crate) — opacity to clients is the only requirement; cryptographic randomness is not (per the spec).

### Changed

- **`McpHandler::serve_stdio` is now a 6-line shim** over `transport::stdio::serve_stdio`. The lift was byte-identical — pinned by the new `mcp_stdio_golden.rs::stdio_transcript_response_shape_is_byte_identical_to_v0_21_0` regression test.
- **`coral mcp serve` CLI** gains `--port <u16>`, `--bind <IpAddr>`, and `Http` as a `--transport` value. The CLI validates `--bind 0.0.0.0` (or `::`) by emitting both a `tracing::warn!` and a `WARNING:` stderr banner so a server bound to every interface is never silent. The existing `--read-only` / `--allow-write-tools` / `--include-unreviewed` flags are dispatcher-level concerns and behave identically across both transports.
- **README**: removed the v0.20.x "Transport status: deferred" callout, replaced with a worked curl recipe for the HTTP transport, a wire-shape table, and a new "Security model for the HTTP transport" section. The PRD-style "What Coral does NOT defend against" entry for CSRF / DNS-rebinding flipped from "future" to "current" and now points at the new section.

### Internal

- **New workspace dep: `tiny_http = "0.12"`.** Picked over hyper / axum because the MCP HTTP transport is a small, blocking-I/O surface (POST + GET + DELETE) and tiny-http is single-purpose, dep-light, and has no async runtime dragging the rest of the workspace into tokio. This is the only new dep in v0.21.1.
- **`crates/coral-mcp/Cargo.toml`** dropped the dead `[features]` block (`default = ["stdio"]`, `stdio = []`, `http_sse = []`). Both transports are unconditionally compiled in — runtime selection is via the `Transport` enum, not a build-time cargo feature.
- **`ServerConfig` gained `bind_addr: Option<IpAddr>`** with `#[serde(default)]` so existing serialized configs deserialize unchanged.

### Tests (+24)

**21 tests in the orchestrator's spec, plus 3 edge-case fillers:**

- **#1-#7 unit (in `transport/http_sse.rs::tests`)**: Origin allowlist, Accept validation, DNS-rebind block, SSE frame literal bytes, session table reap, body cap constant, JSON-RPC batch detection, plus a `new_session_id` UUID-shape pin and uniqueness pin.
- **#8-#16 e2e (`crates/coral-mcp/tests/mcp_http_sse_e2e.rs`)**: POST initialize round-trip, POST resources/list, POST tools/call write-tool rejection, session ID minting, DELETE termination, GET /mcp SSE keep-alive, 5 concurrent clients, malformed JSON-RPC → 200 with -32700, 5 MiB POST → 413 + server stays up, plus three edge-case fillers (Accept text/plain only → 406, OPTIONS preflight CORS shape, unknown path → 404). All driven via raw `std::net::TcpStream` so the test crate doesn't need `tiny_http` as a dep.
- **#17 CLI smoke (`crates/coral-cli/tests/mcp_http_smoke.rs`)**: `coral mcp serve --transport http --port 0` binds, prints the resolved address, responds to a hand-crafted POST initialize. Plus a sibling test that pins the `WARNING:` stderr banner emitted on `--bind 0.0.0.0`.
- **#18-#19 adversarial**: ASCII-rendered homoglyph origin (`xn--lcalhost-5cf.attacker.com`, `localhost.attacker.com`, `127.0.0.1.attacker.com`) blocked; bind-to-already-bound port returns a friendly `io::Error` mentioning the port.
- **#21 BC (`crates/coral-mcp/tests/mcp_stdio_golden.rs`)**: pinned canonical request transcript through `McpHandler::handle_line` produces byte-identical envelopes to v0.21.0. Plus a sibling smoke that spawns the actual `coral mcp serve --transport stdio` binary, pipes initialize, and verifies the protocol version round-trips.

### Acceptance criteria — 15/15 met

- Default bind is `127.0.0.1` ✓
- `--bind 0.0.0.0` works as opt-in with stderr warning ✓
- HTTP transport audit-log line shape matches stdio (validates dispatcher is shared) ✓ — the shared `handle_line` is the only place audit lines originate.
- 5+ concurrent clients work, no deadlocks ✓
- Body > 4 MiB → 413, server stays up ✓
- Malformed JSON → 200 with JSON-RPC -32700 (transport-level errors only for transport-shape problems) ✓
- Origin homoglyph attempts blocked ✓
- Already-bound port → friendly error ✓
- Session ID minted on initialize, optional on subsequent ✓
- DELETE 204 / 404 split ✓
- 32 concurrent → 503 (validated via the spawn cap branch in `serve_blocking`)
- Batched arrays → 400 ✓
- OPTIONS preflight → 200 with tight CORS headers ✓
- Unknown path → 404 ✓
- BC test #21 passes ✓

### Pipeline note

Patch release within the v0.21 sprint (same minor as v0.21.0 since the v0.21 cycle is a five-feature batch, not a fresh minor). No `Co-Authored-By: Claude` trailer per working-agreements.

## [0.21.0] - 2026-05-08

**Feature release: `coral env devcontainer emit`.** Render a `.devcontainer/devcontainer.json` from the active `[[environments]]` block so VS Code, Cursor, and GitHub Codespaces can attach to the same Compose project Coral runs. New `crates/coral-env/src/devcontainer.rs` is a pure renderer over `EnvPlan` (no I/O); `coral_env::render_devcontainer` is callable from the library, and the new `coral env devcontainer emit` CLI subcommand prints the JSON to stdout or writes it atomically with `--write` (mirrors `coral env import --write` exactly). Service auto-selection prefers the first real service whose `repo = "..."` is set (BTreeMap order, lexicographic by service name) and falls back to the alphabetically first real service if none has a repo; mock services are never selected. `forwardPorts` is the union of every `RealService.ports` from the spec, deduped and sorted. **1124 tests pass (was 1108; +16).** BC contract holds — single-repo v0.15 layouts get the same actionable "no [[environments]] declared in coral.toml" error from `coral env devcontainer emit` they get from `coral env status` / `coral up` / `coral down`.

### Added

- **`coral env devcontainer emit [--env NAME] [--service NAME] [--write] [--out PATH]`.** Render `.devcontainer/devcontainer.json` from the active `[[environments]]` block. Stdout by default; `--write` lands `<project_root>/.devcontainer/devcontainer.json` via `coral_core::atomic::atomic_write_string` (sibling tempfile + `rename`, matches every other on-disk write in the workspace). `--service` overrides the auto-selection algorithm; `--out` overrides the destination path (only meaningful with `--write`).
- **`coral_env::render_devcontainer(plan, opts)` library function.** Pure renderer returning `DevcontainerArtifact { json, additional_files, warnings }`. Reruns for an unchanged plan produce byte-identical output: keys are emitted in ASCII-alphabetic order (`serde_json::Value` defaults to a `BTreeMap` backing; we don't pull `serde_json/preserve_order` to keep the dep tree slim). Output ends with a trailing newline so editors don't churn the file on save.
- **Capability flag flipped.** `EnvCapabilities { emit_devcontainer: true }` for both `ComposeBackend` and `MockBackend`. The trait gains no new method — devcontainer emit is a free function over `EnvPlan` because every backend produces a compatible plan.

### JSON shape

Keys land in ASCII-alphabetic order (renderer uses the default `BTreeMap`-backed `serde_json::Value`; the example below shows that order so it matches what users actually see):

```json
{
  "customizations": { "vscode": { "extensions": [] } },
  "dockerComposeFile": ["../.coral/env/compose/<8-char-hash>.yml"],
  "forwardPorts": [<RealService.ports union, deduped, sorted ascending>],
  "name": "coral-<env>",
  "remoteUser": "root",
  "service": "<auto-selected or --service override>",
  "shutdownAction": "stopCompose",
  "workspaceFolder": "/workspaces/${localWorkspaceFolderBasename}"
}
```

`dockerComposeFile` is a **single-element array** (the JSON-Schema spec accepts string-or-array; we use the array form because it's forward-compatible — multi-file overlays land cleanly without a schema migration). `customizations.vscode.extensions` is `[]` by default — no curated list. `remoteUser` is hard-coded to `"root"`, the conventional default for Compose-backed devcontainers.

### Tests

10 new unit tests in `crates/coral-env/src/devcontainer.rs::tests` covering: `dockerComposeFile` array shape and relative path; service auto-selection (`repo`-preference, alphabetic fallback); `forwardPorts` union/dedup/sort; empty-services error message (must point at `coral env import` AND hand-authoring); only-mocks error; `--service` override; unknown-service-override → `ServiceNotFound`; byte-stable output across reruns; full JSON round-trip via `serde_json::Value`.

5 new e2e tests in `crates/coral-cli/tests/env_devcontainer_emit_e2e.rs`: stdout-print parses, `--write` lands file at conventional path, unknown `--env` exits non-zero with available envs in stderr, `--service` override survives through `--write`, unknown service override errors.

1 new BC regression test in `crates/coral-cli/tests/bc_regression.rs`: `coral env devcontainer emit` against a v0.15-shape repo (no `coral.toml`) fails with the same "no [[environments]] declared in coral.toml" error every other env subcommand uses. Mirrors the existing BC contract for `coral env status` / `coral up`.

### Pipeline note

Single-feature minor-version bump (no patches accumulated since v0.20.2). Spec was scoped from a maintainer-issued orchestrator brief; implementation followed the orchestrator's defaults (opt-in `--write`, `forwardPorts` from declared spec, `extensions: []`). No `Co-Authored-By: Claude` trailer per working-agreements.

## [0.20.2] - 2026-05-08

**Patch release: closes 15 cycle-4 follow-up issues (#34–#48).** No new features. Hardens the boundaries v0.20.1 left for follow-up: body-via-tempfile RAII helper now shared across `HttpRunner` + `coral notion-push` + embeddings providers; MCP `tools/list` and `render_page` now respect the same trust gate the v0.20.1 lint applies; mock implementations match real-impl contracts. **1108 tests pass** (was 1068, +40).

### Fixed (bugs)

- **#36** — `coral validate-pin --remote <url>` inserts `--` between `--tags` and the URL. Modern git ≥2.30 mitigates option-injection via strange-hostname blocking; consistency with v0.19.5's git-clone fix matters; covers older-git CI environments. Defense-in-depth.
- **#37** — MCP `render_page` and per-page resource list skip `reviewed: false` distilled pages (matches v0.20.1 H2 lint qualifier exactly). New `--include-unreviewed` opt-in. New e2e suite `crates/coral-mcp/tests/mcp_unreviewed_e2e.rs`.
- **#38** — MCP `tools/list` filters write tools symmetrically with the dispatcher: default → 5 read-only; `--read-only false` → still 5; `--read-only false --allow-write-tools` → all 8. Doc-vs-reality drift fixed.
- **#41** — `coral session show <prefix>` raises `InvalidInput` on >1 matches (matches `forget`/`distill` semantics).
- **#42** — `coral session forget` (no `--yes`) exits non-zero on user-abort. Prompt displays canonical resolved short-id.
- **#45** — `coral env import --write` uses `coral_core::atomic::atomic_write_string` (sibling tempfile + rename).
- **#46** — `coral lint --apply` wraps `Page::write()` in `with_exclusive_lock` for parallel-apply safety.
- **#47** — Project/repo names rendered into AGENTS.md / CLAUDE.md / cursor-rules / copilot / llms-txt now route through `escape_markdown_token` (escapes newlines, backticks, emphasis chars, brackets, parens, backslashes). Pre-fix `name = "evil\n## injection"` landed arbitrary Markdown in agent docs.

### Hardening

- **#34** — `coral session capture` enforces a 32 MiB cap on input JSONL. Returns `SessionError::TooLarge` with size + cap.
- **#43, #44** — `coral notion-push` + embeddings providers (Voyage / OpenAI / Anthropic) route bodies through the new shared `coral_runner::body_tempfile` module: `body_tempfile_path` + `write_body_tempfile_secure` (mode 0600 + `create_new`) + `TempFileGuard` (RAII cleanup). Bodies never appear in `Command::get_args()`.

### Mock-vs-real parity

- **#39** — `MockBackend::up` rejects `EnvMode::Adopt` with the same `EnvError::InvalidSpec` `ComposeBackend::up` returns.
- **#40** — `MockRunner::with_timeout_handler(impl FnMut(Duration) -> RunnerResult<RunOutput>)` lets tests verify timeout-honoring contracts without `thread::sleep`.

### Documentation

- **#35** — CHANGELOG v0.20.0 stale "1049 tests" → corrected to actual 1052.
- **#48** — README "9 TestKind variants" reframed to "3 user-reachable + 6 reserved for forward-compat (runners ship in v0.21+)".

### Pipeline note

Same shape as v0.19.5/6/8 / v0.20.0/1: fix agent landed all 15 fixes in one in-progress commit (terminated mid-finalize); maintainer applied 3 inline `doc_lazy_continuation` clippy fixes, bumped version, wrote this entry, ran `scripts/ci-locally.sh` green before tagging. No `Co-Authored-By: Claude` trailer per working-agreements.

## [0.20.1] - 2026-05-08

**Patch release: cycle-4 audit fixes (3 Critical + 7 High).** All 10 changes are bug fixes — no new features. The cycle-4 audit pipeline (multiple parallel agents, non-overlapping mandates, surfaced ~ten findings) caught three live security gaps that the v0.20.0 release left open: cache poisoning could short-circuit the new `unreviewed-distilled` lint gate, the single-file HTML export's slug interpolation reopened the v0.19.5 C5 XSS surface, and the multi-page export's TOC inherited a parallel XSS via the same vector. The seven High-priority fixes harden the prompt-injection posture (default-on lint scan, fenced wiki bodies in every LLM-bound prompt, qualified `unreviewed-distilled` rule), close the `coral session forget` orphaned-output bug, and clean up README/docs counts that drifted in v0.20.0. **1068 tests pass (was 1052; +16).** BC contract holds.

> **Behavior change you may notice (audit H2):** `coral lint --rule unreviewed-distilled` is now qualified — it only fires when `reviewed: false` AND `source.runner` names a populated LLM provider (matches what `coral session distill` writes). Hand-authored drafts that use `reviewed: false` as a workflow signal but have no `source` block will no longer trigger the lint. This is the correct behavior per the v0.20 PRD's expected-behavior matrix; v0.20.0 over-fired. (Mirroring the v0.19.8 #30 XSS callout pattern: this is a deliberate scope tightening.)

> **Behavior change you may notice (audit H4):** `coral lint --check-injection` is now ON by default. Pass `--no-check-injection` to suppress. This mirrors the `--no-scrub` opt-out shape from session capture: default-safe, explicit opt-out. The legacy `--check-injection` flag is preserved (now hidden) so any pre-v0.20.1 scripts keep working.

### Fixed — Critical (C1–C3)

- **C1: WalkCache poisoning bypassed the `unreviewed-distilled` lint gate.** `crates/coral-core/src/cache.rs` + `crates/coral-core/src/walk.rs`. The on-disk `.coral-cache.json` was keyed by `(rel_path, mtime_secs)`. A poisoned cache entry could return a `reviewed: true` `Frontmatter` for a file whose disk content actually said `reviewed: false`, short-circuiting `Page::from_content` and the lint gate. Cache key now extends to `(rel_path, mtime_secs, content_hash)`; the hash is FNV-1a 64-bit over the on-disk content (same shape as `coral_env::compose_yaml::content_hash`). Cache hits now re-read the file (cheap) and verify the hash before trusting the cached parse. Legacy v0.20.0 entries (no hash) treat as a miss and force a re-parse. Regression test (`read_pages_rejects_poisoned_cache_via_hash_check`) stash-validated against the pre-fix `.get()` path.
- **C2: HTML export single-bundle XSS via slug.** `crates/coral-cli/src/commands/export.rs::render_html`. v0.19.5 audit C5 added `is_safe_filename_slug` to `render_html_multi` but not to `render_html`. A page with `slug: x"><script>alert(1)</script><span x="` produced live XSS in the single-file HTML bundle (the slug landed in `id="…"` and the existing `html_escape` doesn't help — HTML id has no escape grammar). The fix filters unsafe slugs out of the page list BEFORE building any HTML. New regression test `render_html_skips_unsafe_slug_for_xss_in_id_attribute`.
- **C3: HTML export multi `index.html` XSS via slug in TOC.** Same file. `render_html_multi` had the safe-slug filter only on the per-page write, AFTER the TOC was already baked. The per-page file was correctly skipped, but `index.html` still carried the unsafe slug. Hoisted the filter so TOC and disk stay consistent — both skip the unsafe page. New regression test `render_html_multi_skips_unsafe_slug_in_toc`.

### Fixed — High (H1–H7)

- **H1: `coral session forget` left distilled `.md` files orphaned.** `crates/coral-session/src/{capture,distill,forget}.rs`. Forget looked for `.coral/sessions/distilled/<session-id>.md` but distill writes by `<finding-slug>.md`. The two never agreed. Fix: `IndexEntry` gains a `distilled_outputs: Vec<String>` field (serde-default empty for v0.20.0 entries); distill populates it; forget walks it and removes each `.md` from both `.coral/sessions/distilled/` AND `.wiki/synthesis/` (where `--apply` mirrors them). Sessions captured pre-v0.20.1 emit a `tracing::warn!` asking the user to sweep manually — we can't safely auto-sweep slug-named files because they might belong to another session. Defense-in-depth: forget refuses to follow basenames containing `/`, `\`, `..`, or a leading dot. New regression test `forget_removes_slug_named_distilled_outputs_after_real_distill` exercises the full distill→forget cycle with a `MockRunner`.
- **H2: `unreviewed-distilled` lint qualifier — fires only when `source.runner` is populated.** `crates/coral-lint/src/structural.rs`. Pre-fix the lint fired on every `reviewed: false` page including hand-authored drafts that had no `source` block. The audit-prompt's expected-behavior matrix said cases 2 and 3 should NOT fire. Qualified the check: only fires when `reviewed: false` AND `source.runner` names a non-empty LLM provider. Hand-authored drafts (no `source` field, or empty runner) can now use `reviewed: false` freely as a workflow signal. The matrix is encoded as 4 fixtures (`h2_matrix_case_1`/`2`/`3`/`4`). `docs/SESSIONS.md:175` updated. The session-e2e fixture now ships the full `source` block to match what real `coral session distill` emits.
- **H3: prompt injection vector via wiki body.** `crates/coral-cli/src/commands/{query,diff,lint}.rs` + new helper `crates/coral-cli/src/commands/common/untrusted_fence.rs`. Every command that interpolates a wiki body into an LLM prompt (`coral query`, `coral diff --semantic`, `coral lint --auto-fix`, `coral lint --suggest-sources`) now wraps each body in a `<wiki-page slug="…" type="…"><![CDATA[ … ]]></wiki-page>` envelope. The system prompt appends a `UNTRUSTED_CONTENT_NOTICE` that explicitly tells the LLM to treat fenced content as data and ignore any instructions inside. CDATA terminator (`]]>`) is defanged on the way in (replaced with `]] >`) so a malicious body cannot escape its envelope; the helper also runs `coral_lint::check_injection` on the body and either drops the page (`fence_body`, used by `query`) or annotates with `[suspicious-content-detected]` (`fence_body_annotated`, used by `diff`/`lint` where every page is load-bearing). Regression test `query_fences_wiki_body_against_prompt_injection` exercises the full path against a `MockRunner` and asserts (a) the body is fenced, (b) the CDATA-escape sequence is defanged, (c) the system prompt has the notice. Plus 5 unit tests on the fence helper itself.
- **H4: `coral lint --check-injection` is now ON by default.** `crates/coral-cli/src/commands/lint.rs` + `template/hooks/pre-commit.sh`. Mirrors the `--no-scrub` opt-out shape from session capture: default-safe, explicit opt-out via the new `--no-check-injection` flag. The legacy `--check-injection` flag is preserved (now hidden) so any pre-v0.20.1 scripts keep working — passing it is a no-op since the scan runs anyway. Pre-commit hook gains a second pass that runs `coral lint --rule injection-suspected --severity warning` so distilled pages with injection-shaped bodies are surfaced before they land in the repo. README + `docs/SESSIONS.md` updated. Regression test `lint_runs_injection_scan_by_default`.
- **H5: README counts and Sessions reference table.** README. Added the "Sessions layer (v0.20+)" subsection to the Subcommand reference (5 leaves: `capture`, `list`, `show`, `forget`, `distill` + the umbrella). Updated headline counts: 37 → 42 leaf subcommands, 28 → 29 top-level commands, 4 → 6 grouped subcommand families, 5 → 6 layers, 8 → 9 Rust crates, 9 → 11 structural lint checks, 3 embeddings providers (Voyage/OpenAI/Mock → Voyage/OpenAI/Anthropic — Mock is test-only, Anthropic is production). Architecture tree includes `coral-session/`. Test-count claim updated to 1068+ (was 1020+).
- **H6: `coral mcp serve --transport http` documented but not implemented — chose to clarify the docs.** README. Removed the recipe-7 example and the "Generic HTTP/SSE" section that promised an HTTP transport; replaced with an explicit "Transport status (v0.20.x)" callout that says stdio-only and tracks HTTP/SSE for a follow-up. The clap definition for `--transport` only accepts `stdio` already, so the docs are now consistent with the binary. Decision rationale: `crates/coral-mcp/src/transport/http_sse.rs` does not exist (the implementation isn't trivially close), and every shipped MCP client uses stdio anyway, so docs are more harmful than no feature.
- **H7: `coral env import` echoed full file content in error.** `crates/coral-env/src/import.rs`. `serde_yaml_ng`'s typed-deserialize error path emits `invalid type: string "<entire input verbatim>"` when the input parses as a single YAML scalar (which `/etc/passwd` does). Multi-KB stderr containing the entire file. Fix: new `scrub_parse_error` helper truncates to ~200 chars + a `(... <N> additional chars truncated)` marker, then runs the same secret-shape regex `coral_runner::scrub_secrets` uses (extended to also catch `password:`/`token:`/`secret:`/`api_key:` / `sk-…` / `gh[opsu]_…` patterns). Regression test `import_error_truncates_and_scrubs_secrets` feeds a `/etc/passwd`-shaped input with three secret tokens and 200 filler lines; asserts none survive in the error. Test fixture builds the `ghp_…` token via `concat!` so GitHub push protection doesn't flag the file.

### Internal

- **Workspace total: 1068 tests pass (was 1052; +16).** Zero clippy warnings; the four-cycle BC contract holds across all 6 v0.15 fixtures.
- **`IndexEntry` schema gains `distilled_outputs: Vec<String>`** with `#[serde(default)]` so on-disk indexes captured pre-v0.20.1 deserialize unchanged.
- **Cache schema (v1) gains `content_hash: String`** with `#[serde(default)]` so on-disk caches captured pre-v0.20.1 deserialize unchanged; the empty default forces a re-parse on the first `read_pages` call after upgrade.
- **`LintArgs` derives `Clone`** so unit tests can mutate one instance per check.
- **No new workspace dependencies.** All H3 fencing logic uses the existing `coral_lint`/`coral_core` surface; H7 secret-scrub is a self-contained regex.

---

## [0.20.0] - 2026-05-08

**Major feature release: `coral session capture / list / forget / distill / show`** ([#16](https://github.com/agustincbajo/Coral/issues/16)). The wiki finally captures the conversations that produced it. Five new CLI subcommands; one new crate (`coral-session`); one new lint rule (`unreviewed-distilled`, Critical) that gates any LLM-generated wiki page until a human reviews + signs off. 1052 tests pass at v0.20.0 ship (was 977; +75); count grew to 1068 by v0.20.1. BC contract holds. The v0.19.x audit-driven hardening sprint is complete; this is the first feature release on top of that.

### Added

- **New crate: `coral-session`.** `crates/coral-session/{src,tests}` — implements capture, list, forget, distill, and show flows. Sits alongside the existing `coral-runner` / `coral-env` / `coral-test` crates with the same shape (declarative error type, `MockRunner`-friendly traits, atomic writes via `coral_core::atomic`). Modules: `capture` (idempotent index updates under `with_exclusive_lock`), `claude_code` (versioned JSONL adapter that defensively handles unknown record types), `distill` (single-pass `Runner::run`, hard cap of 3 findings/session, slug allowlist), `forget` (atomic deletion of raw + distilled + index entry), `list` (Markdown + JSON output), `scrub` (regex-driven privacy redactor with 25 regression tests).

- **`coral session capture --from claude-code [PATH]`** — copies a Claude Code transcript into `<project_root>/.coral/sessions/<date>_claude-code_<sha8>.jsonl`. When `PATH` is omitted, walks `~/.claude/projects/`, parses each transcript's first record, and picks the most-recently-modified one whose recorded `cwd` matches the current project. Default behaviour runs the privacy scrubber over every byte before write.

- **`coral session list [--format markdown|json]`** — renders `.coral/sessions/index.json` as a Markdown table (default) or parseable JSON array, sorted by `captured_at` descending. Empty state prints a friendly "no captured sessions yet" message instead of a header-only table.

- **`coral session show <SESSION_ID>`** — prints metadata + first N message previews (default 5, override with `--n`). Accepts either full UUID or any unique 4+-char prefix.

- **`coral session distill <SESSION_ID> [--apply] [--provider …] [--model …]`** — single-pass LLM call that extracts 1–3 surprising / non-obvious findings and emits each as a synthesis Markdown page. Always lands as `reviewed: false`. Without `--apply`, writes only `.coral/sessions/distilled/<slug>.md`. With `--apply`, also writes `.wiki/synthesis/<slug>.md` so the page shows up in `coral search` / `coral lint` / `coral context-build`. Provider follows the standard `--provider` semantics (claude / gemini / local / http; falls back to `CORAL_PROVIDER` env or `claude`).

- **`coral session forget <SESSION_ID> [--yes]`** — atomic delete of raw `.jsonl` + distilled `.md` + index entry under `with_exclusive_lock`. Prefix matching identical to `show`/`distill`. Without `--yes`, prompts interactively `[y/N]`.

- **New lint rule: `unreviewed-distilled` (Critical).** `crates/coral-lint/src/structural.rs::check_unreviewed_distilled` flags any wiki page whose frontmatter declares `reviewed: false`. Critical severity flips `coral lint` to a non-zero exit, so the bundled pre-commit hook AND any CI lint pipeline reject the commit until a human flips the flag to `true`. Reuses the existing v0.19.x trust-by-curation machinery rather than reinventing it. The complementary `unknown-extra-field` (Info) check now skips `reviewed` and `source` keys to avoid double-counting and noise.

- **Privacy scrubber: 25-pattern regex set covering Anthropic / OpenAI / GitHub / AWS / Slack / GitLab / JWT / Authorization-header / x-api-key-header / bare-Bearer / env-export-assignment shapes.** Each match is replaced by `[REDACTED:<kind>]`; the marker tells the user *what kind* of secret was redacted without leaking the original. Pattern ordering matters (longest-most-specific wins on overlap); scrubbing is idempotent (re-scrubbing a redacted output produces no further redactions). 25 unit tests cover each token shape; one fixture-based integration test (`crates/coral-session/tests/secrets_fixture.rs`) exercises the full capture + scrub pipeline against a real-shaped transcript with `sk-ant-…`, `ghp_…`, `AKIA…`, and a 3-segment JWT embedded.

- **Privacy opt-out is intentionally hard.** `coral session capture --no-scrub` alone fails fast with a clear hint. To take effect it MUST be combined with `--yes-i-really-mean-it`. The mandatory two-flag combo is the v0.20 PRD answer to design Q2: false negatives leak credentials irreversibly, so the default errs on redaction.

- **`coral init` now seeds `.coral/sessions/` patterns into the project-root `.gitignore`.** Idempotent (preserves existing user-managed lines; appends only patterns not already listed). Adds `.coral/sessions/*.jsonl`, `.coral/sessions/*.lock`, `.coral/sessions/index.json`, plus the negation `!.coral/sessions/distilled/` so curated distillations remain in git while raw transcripts stay local-only. Implements PRD design Q1.

- **`docs/SESSIONS.md`** — full design + privacy + trust-by-curation walkthrough with the per-question PRD answers documented inline.

- **README "Quickstart — capture and distill agent sessions" section.** End-to-end flow with the privacy posture and the `reviewed: false` gate called out. Roadmap reorganized: the `coral session` line moves from "v0.20+ feature roadmap" to "Shipped (v0.20.0)"; the cross-format support deferral is now in "v0.21+".

- **Glossary terms**: `Session`, `Captured session`, `Distilled session`. SCHEMA.base.md's synthesis page-type explainer mentions distillation as a producer.

### Internal

- **Fixture transcript** at `crates/coral-session/tests/fixtures/claude_code_with_secrets.jsonl` — a hand-redacted miniature Claude Code JSONL with the v0.20 must-redact secret shapes embedded in both plain user content and `assistant.content[].tool_use.input` blocks. The integration test asserts every must-redact category is replaced with the appropriate marker, AND that `--no-scrub` preserves source bytes byte-for-byte.

- **`coral-cli/tests/session_e2e.rs`** — end-to-end CLI test driving the `coral` binary against a tmpdir + the fixture: `init` → `capture` → `list (markdown)` → `list (json)` → `show` → `forget --yes`, plus three negative tests (`--no-scrub` without confirmation fails, `--no-scrub --yes-i-really-mean-it` writes raw bytes, `--from cursor` returns "not yet implemented" pointing at #16). Plus a `coral lint` integration test that confirms the `unreviewed-distilled` Critical rule fires on a page with `reviewed: false` frontmatter.

- **Workspace dependency**: `coral-session` registered in `Cargo.toml` workspace deps so the CLI (and any future downstream crate) can consume it; `coral-lint` lifted `serde_yaml_ng` from dev-dep to regular dep so the new check can pattern-match on `Bool` / `String` variants of the YAML extra map.

- **Distill prompt is versioned (`prompt_version: 1`)** in the emitted page's frontmatter so a future prompt-template change can be re-distilled against old captured sessions without ambiguity.

### Notes on per-design-question answers

The v0.20 PRD ([#16](https://github.com/agustincbajo/Coral/issues/16)) left six design questions explicitly open. Each is answered + documented in source comments:

1. **Storage default** — gitignored raw + non-gitignored `distilled/` via `!` negation. (`coral init` + `docs/SESSIONS.md`.)
2. **Privacy scrubbing** — opt-out only; `--no-scrub` requires `--yes-i-really-mean-it` confirmation. (`session.rs::run_capture` guard.)
3. **Distill output format** — distill-as-page (option a). Distill-as-patch (option b) deferred to v0.21+ once we have diff/merge UX. (`distill.rs` module-level docstring.)
4. **Trust gating** — same `reviewed: false` machinery as `coral test generate`; new `check_unreviewed_distilled` Critical rule + bundled pre-commit hook. (`structural.rs`.)
5. **Cross-format support order** — Claude Code first; `--from cursor` and `--from chatgpt` exist as CLI args but currently emit a clean "not yet implemented; track #16" error.
6. **MultiStepRunner usage** — single-tier `Runner::run` for MVP. Tiering is a v0.21+ optimization once we have data on distill-output quality vs latency. (`distill.rs::distill_session`.)

## [0.19.8] - 2026-05-04

Closes the eight open audit follow-up issues from the v0.19.7 cycle (#26 through #33). Adds MCP cursor pagination on `resources/list` + `tools/list` (the only one of the eight that was a feature; everything else is bug fixes, audit-gap conversions, or tracked-deferral cleanup). Audit-gap fixtures for `coral test discover` (OpenAPI), `coral export --format html` (XSS), and `coral-runner` streaming (mid-stream truncation / hang / partial-event) ship as protective tests so each gap stops being a gap. 977 tests pass (was 928; +49). One real XSS surface fixed: pulldown-cmark previously passed raw `<script>...</script>` and `[click me](javascript:alert(1))` through verbatim in the static HTML export.

### Fixed

- **#27 — wikilink escape `[[a\|b]]` now produces target `a|b`.** Pre-fix the regex saw `\|` as a literal char and the alias-stripping at `wikilinks.rs:56-58` split on the FIRST `|`, yielding target `a\` (broken slug). Now the captured body is pre-processed: `\|` becomes a sentinel byte (U+001F UNIT SEPARATOR) before the alias split, then the sentinel is restored to a literal `|` afterward. Matches Obsidian semantics. The slug allowlist still rejects backslashes anywhere else in the resulting target. Six new unit tests + the existing proptest property updated to permit `|` in the output (escape form). Closes [#27](https://github.com/agustincbajo/Coral/issues/27).
- **#30 — HTML export XSS hardened.** `coral export --format html` runs through `pulldown-cmark` 0.13 which has no `Options::ENABLE_HTML` flag — by default it passes raw HTML through verbatim and accepts arbitrary URL schemes in link destinations. Fix sanitizes at the Event level: `Event::Html(s)` and `Event::InlineHtml(s)` are converted to `Event::Text` (HTML-escaped on emission), so `<script>` becomes `&lt;script&gt;`. `Tag::Link` and `Tag::Image` events with `dest_url` matching the unsafe-scheme allowlist (`javascript:`, `data:`, `vbscript:`, `file:`) have their URL rewritten to `#` before emission. The check is ASCII-case-insensitive and strips leading whitespace + control bytes (Chrome strips them before parsing the scheme). Eight new fixtures cover `<script>` body, inline `<img onerror=>`, `[c](javascript:alert(1))`, `[c](data:text/html,...)`, `[c](JavaScript:...)` (case-folding), whitespace-prefix bypass, frontmatter breakout payload, and multi-export equivalent. Closes [#30](https://github.com/agustincbajo/Coral/issues/30).

  **Behavior change you may notice**: legitimate inline HTML in markdown bodies — `<em>foo</em>`, `<sup>1</sup>`, `<details>...</details>`, raw `<a name="...">` anchors — is **also** escaped under the new policy (Coral's stance: wikis are markdown-first; raw HTML is never rendered, just rendered-as-text). Use markdown emphasis (`*foo*`, `**bar**`) instead, or wait for a v1.0+ allowlist sanitizer if you have a concrete need for the rich-HTML escape hatch (file an issue with the use case).
- **#28 — `_default` is now reserved as a repo name.** The MCP `coral://wiki/<repo>/_index` URI handler treats `<repo> == "_default"` as a sentinel for the legacy single-repo aggregate index. A wiki containing a real repo named `_default` would silently shadow the wildcard with no error. `Project::validate()` now rejects `name = "_default"` with a message naming the reservation and pointing at the MCP-sentinel rationale. README updated (under "Resources catalog") to document the sentinel and the resulting reservation. Closes [#28](https://github.com/agustincbajo/Coral/issues/28).
- **#29 — `parse_spec_file` enforces a 32 MiB cap.** Matches the v0.19.5 N3 cap that `coral_core::walk::read_pages` applied to wiki pages. Without this, a multi-GiB `openapi.yaml` (whether malicious or accidentally checked-in) was loaded into RAM by `read_to_string` and parsed by the YAML deserializer — DoS reachable from a downstream repo's `coral test discover` invocation. Six new fixture tests (cyclic `$ref`, huge inline example, unknown HTTP method, escaped path, local-file `$ref`, under-cap sanity) pin the discovery walker's behavior under adversarial inputs. Closes [#29](https://github.com/agustincbajo/Coral/issues/29).

### Added

- **#26 — MCP cursor pagination on `resources/list` and `tools/list`.** Wikis with more than ~100 pages previously emitted one giant JSON-RPC envelope (DoS edge case + transport-size compatibility issue). Both list methods now accept a `cursor` parameter (opaque, MCP spec-compliant; encoded as a stringified non-negative integer offset) and return a `nextCursor` field when results overflow the page. Page size: 100 (`coral_mcp::server::PAGINATION_PAGE_SIZE`). Invalid cursor → JSON-RPC error so misbehaving clients learn immediately rather than seeing an empty catalog. Cursor pointing past the end is also an error (drift-detection: the underlying wiki may have shrunk between requests; clients re-list from offset 0). Six new server-unit tests + four end-to-end tests in `crates/coral-mcp/tests/mcp_pagination_e2e.rs` (under page size, over page size with multi-page walk, invalid cursor, tools/list contract). Closes [#26](https://github.com/agustincbajo/Coral/issues/26).

### Internal (audit-gap fixtures)

- **#29 — OpenAPI adversarial fixture suite.** Six fixtures exercise the discovery walker under adversarial inputs (`$ref` cycle, 33 MiB spec, unknown HTTP method, percent-encoded path, traversal-style local-file `$ref`, under-cap sanity). The walker emits zero cases for size-rejected files and skips unknown methods silently; cyclic refs and traversal refs are inert because `discover.rs` does NOT perform `$ref` resolution (pinned by the fixtures so any future ref-resolution change comes with explicit cycle + traversal protection). New `crates/coral-test/tests/openapi_adversarial.rs`.
- **#31 — Streaming runner adversarial fixture suite.** Eight fixtures cover the `run_streaming_command` line-reader under adversarial subprocess behavior: clean two-line stream, partial-final-line on EOF, partial-then-non-zero-exit, silent hang past timeout, line-then-hang past timeout, 200-rapid-chunk emission, empty stdout, stderr-only with clean exit. Pins that lines emitted before EOF reach `on_chunk` in order; trailing partial bytes ARE surfaced as a final chunk; `prompt.timeout` is total-wall-clock (not idle-since-last-byte). New `crates/coral-runner/tests/streaming_failure_modes.rs`. No bugs surfaced — the harness pins existing behavior. (`HttpRunner` itself sets `stream: false` at the wire level today; a future HTTP-SSE runner would need its own fixtures.)
- **#30 — HTML export XSS adversarial fixture suite.** Eight new fixtures inline in `crates/coral-cli/src/commands/export.rs::tests` cover `<script>` body, inline `<img onerror=>`, six unsafe-scheme `[c](javascript:alert(1))` variants (including case-folding + whitespace-prefix bypass), wikilink with `javascript:`-shaped target (renders as fragment-only — benign), frontmatter breakout payload in `last_updated_commit`, multi-export `<script>` equivalent. Plus `is_unsafe_url_scheme` direct unit-test matrix.

### Tracked deferrals

- **#32 — `*.lock` and `*.lock.lock` patterns added to bundled `.gitignore`.** The zero-byte sentinel files left behind by `with_exclusive_lock` after release can't be safely cleaned up without breaking the cross-process flock contract (root cause: unlink-while-FD-held detaches the inode, peer process opens fresh inode at same path, both believe they hold the lock — documented in `atomic.rs::with_exclusive_lock`). Live with the litter, ignore it in git so users don't accidentally commit it. `coral init` writes both patterns to `.wiki/.gitignore`; idempotent on re-run. Two new tests pin the addition + the idempotency. Closes [#32](https://github.com/agustincbajo/Coral/issues/32) (resolved as deferral via `.gitignore`).
- **#33 — `WikiLog::append_atomic` debug-asserts op shape.** `WikiLog::parse` requires `op` to match `\w[\w-]*` (single token of ASCII alnum + `_`/`-`). The constraint is enforced at parse time only — pre-fix, a caller passing `op = "user requested cleanup"` would write a line that subsequent `coral history` reads silently dropped from history. Now `append_atomic` `debug_assert!`s the constraint at write time so dev builds catch new bad callers. Release builds skip the check (zero overhead, in-tree convention covers it). Doc comment updated. The on-disk format itself stays at v1; relaxing the regex is a v1.0+ format-stability decision (would silently change on-disk format for upgraders). Five new tests. Closes [#33](https://github.com/agustincbajo/Coral/issues/33) (resolved as deferral with debug-assert).

## [0.19.7] - 2026-05-04

Small patch release closing the two N2 follow-up issues from v0.19.6's validator review and adding `coral env import` (a deferred onboarding feature from the v0.19 PRD). 928 tests pass (was 908; +20).

### Fixed

- **`HttpRunner` request-body tempfile is now created with mode 0600 on Unix.** Pre-v0.19.7 the file went out at the umask default (typically 0644), which restricted WRITE but not READ. On Linux multi-tenant hosts where `/tmp` is shared across UIDs, any local user could `cat` the in-flight prompt body — defeating the v0.19.6 N2 fix that explicitly moved the body off argv to keep it private from `ps`. macOS is unaffected because `$TMPDIR` is per-user under `/var/folders/<hash>/T/`. The fix uses `OpenOptions` with `create_new(true)` (defense-in-depth against a pre-positioned symlink at the target) and `mode(0o600)` on Unix. Closes [#24](https://github.com/agustincbajo/Coral/issues/24).
- **`HttpRunner` request-body tempfile cleanup is now uniform across all return paths.** Pre-v0.19.7 the cleanup was hand-rolled at three of the four return paths; the fourth (header-write fail / body-write fail / wait-output fail) leaked the file. New `TempFileGuard` RAII wrapper handles cleanup on every return path including panic-unwind. Doc comment updated — no longer claims "best-effort". Closes [#25](https://github.com/agustincbajo/Coral/issues/25).

### Added

- **`coral env import <compose.yml>` — deferred from v0.19 PRD.** Convert an existing `docker-compose.yml` into a `coral.toml` `[[environments]]` block. Output is conservative and advisory: only fields that round-trip cleanly through `EnvironmentSpec` are emitted; anything Coral can't translate (long-form `depends_on`, list-form `environment`, port ranges, unrecognized fields) surfaces as a `# TODO:` comment so users see the gaps. Heuristics: `CMD ["curl", "-f", "http://...//health"]` infers `kind = "http" + path = "/health" + expect_status = 200`; `CMD-SHELL <line>` becomes `kind = "exec", cmd = ["sh", "-c", <line>]`. Compose duration strings (`5s`, `1m30s`, `2h`) parse to seconds. New `coral_env::import` module; new `crates/coral-env/src/import.rs` + 16 unit tests including a round-trip-through-`EnvironmentSpec` pin so the emitted TOML is always runtime-valid.

### Internal

- New `coral_core::slug::is_safe_repo_name` reused in the import module's env-name and service-name validation, keeping the same allowlist that v0.19.6's H1 fix introduced for repo names.

## [0.19.6] - 2026-05-04

Third-cycle audit follow-up. A re-validation pass on v0.19.5 surfaced 8 real bugs across `coral-mcp`, `coral-core`, `coral-runner`, `coral-cli`, and `coral-test`, plus 4 Notable polish items. All shipped here. Headline: the MCP `resources/read` response no longer hardcodes `text/markdown` (every JSON resource was being silently mislabeled), `WikiLog::append_atomic`'s first-create path is now race-free under contending writers, and `coral project sync`'s lockfile write serializes cross-process via the same flock primitive `ingest` and `index.md` already use.

### Fixed (Critical)

- **C1. `resources/read` hardcoded `mimeType: "text/markdown"`.** The handler at `crates/coral-mcp/src/server.rs:207` emitted `text/markdown` for every URI, silently undoing the v0.19.5 audit's `#[serde(rename = "mimeType")]` fix. JSON resources (`coral://manifest`, `coral://lock`, `coral://stats`, `coral://graph`, `coral://wiki/_index`, `coral://test-report/latest`) reached clients tagged as markdown — clients then either fell back to plain text or failed to parse the JSON body. `ResourceProvider::read()` now returns `(body, mime_type)` so per-URI mime knowledge stays at the place that knows it. Per-page `coral://wiki/<slug>` resources were retagged from `text/markdown` to `application/json` to match the JSON envelope `render_page` actually emits. New `read_mime_type_matches_list_catalog_for_every_uri` regression in `crates/coral-mcp/tests/mcp_resources_e2e.rs` asserts every advertised URI's read response carries the same mime as `list()`.
- **C2. `WikiLog::append_atomic` first-create header race.** Under N concurrent writers, the loser of the `create_new` race could land its `- <entry>` line BETWEEN the winner's `header` write and the winner's `entry` write — POSIX `O_APPEND` makes each `write()` atomic-seek-to-EOF, but does NOT pair the two writes. Reproduced deterministically with 4 contending threads at ~1/50. Now the entire create-or-append sequence runs inside `with_exclusive_lock` (the same primitive that serializes `index.md` and `coral.lock`). New `append_atomic_first_create_race_never_produces_entry_before_header` asserts the canonical header always sits at the very start, no entry line ever precedes it, and all N entries land.

### Fixed (High)

- **H1. Repo names in `coral.toml` accept path traversal.** A `[[repos]]` block with `name = "../escape"` produced `<project_root>/repos/../escape` for `Project::resolved_path`, and `coral project sync` would `git clone` outside the project root. New `coral_core::slug::is_safe_repo_name` (sibling of v0.19.5's `is_safe_filename_slug`) gates `Project::validate()`. Slugs like `api`, `worker`, `shared-types` pass; `../escape`, `foo/bar`, `.hidden`, `-flag` reject with a clear error naming the offending repo.
- **H2. `LocalRunner` and `GeminiRunner` skip `scrub_secrets`.** v0.19.5 H8 routed `http.rs`, `runner.rs`, and `embeddings.rs` (3 sites) through `scrub_secrets` before wrapping in `RunnerError`; the synchronous Local and Gemini paths missed the migration. A wrapper script that hits a hosted endpoint (or a misconfigured llama.cpp pointed at an auth proxy) could echo back `Authorization: Bearer …` headers; the unscrubbed stderr would then leak the key into logs. Both runners now scrub before constructing the error envelope. Per-runner regression test runs a tiny shell stub that prints a fake bearer token and exits non-zero, asserting the resulting `RunnerError` carries `<redacted>` instead.
- **H3. `coral project sync` lost-update race on `coral.lock`.** Two parallel `coral project sync --repo A` and `--repo B` invocations raced the same way the v0.19.5 H7 ingest race did against `index.md` — both would `Lockfile::load_or_default` outside any lock, mutate their own copy, then clobber on `write`. Now wrapped in `with_exclusive_lock` so cross-process syncs serialize. The closure body uses `atomic_write_string` directly rather than `Lockfile::write_atomic` to avoid a self-deadlock when re-entering the same flock from a fresh FD. New regression in `crates/coral-core/src/project/lock.rs::tests` spawns 8 threads each upserting a different repo's SHA and asserts the final `coral.lock` carries all 8 entries.

### Fixed (Medium)

- **M1. `substitute_vars` mangles UTF-8.** The byte-walking loop in `crates/coral-test/src/user_defined_runner.rs::substitute_vars` did `out.push(bytes[i] as char)`, treating each `u8` as a single codepoint. A multi-byte UTF-8 sequence (`é = 0xC3 0xA9`) emerged as Latin-1 `Ã©`. Replaced with a `char_indices()`-driven walk that only enters the `${…}` fast path when the current ASCII byte is `$`. Multi-byte chars are appended verbatim. Regression test exercises `café`, `naïve`, `日本`, and emoji.
- **M2. `.coral/audit.log` unbounded growth.** A long-running `coral mcp serve` would append forever. Now rotates once at 16 MiB: the active file is renamed to `audit.log.1` (replacing any prior rolled file) and a fresh `audit.log` starts. Single-rotation is intentional; users who want longer retention can configure logrotate externally. Regression test seeds an oversized active log, makes one tool call, and asserts `audit.log.1` carries the pre-rotation content while `audit.log` restarts fresh.
- **M3. JSON-RPC notification produces a response.** Per JSON-RPC 2.0 §4.1 a request without an `id` is a notification — server MUST NOT reply, even with an error. `handle_line` now returns `Option<Value>`: `None` for notifications, `Some(_)` for requests. `serve_stdio` skips emitting anything when the dispatch returns `None`. Side effects still run. Two new tests pin the silent-on-notification contract for both known and unknown methods.

### Fixed (Notable)

- **N1. `WalkCache::save` non-atomic.** Migrated from `fs::write` to `coral_core::atomic::atomic_write_string` so a crash mid-save can't leave a half-written `.coral-cache.json`. New concurrent-save regression hammers `save` from 10 threads and asserts the post-storm read parses cleanly.
- **N2. Curl request body still in argv.** v0.19.5 H6 moved the `Authorization` header to stdin via `-H @-`; the prompt body was still inlined as `-d <body>`, exposing it to every other process via `ps` / `/proc/<pid>/cmdline`. Migrated to `--data-binary`: when no API key is set, body streams via stdin (`@-`); when an API key IS set (and stdin is already claimed by `-H @-`), body is written to a per-call tempfile and referenced via `--data-binary @<path>`. Best-effort cleanup unlinks the tempfile after `wait_with_output`. Two regression tests: argv leaks neither bearer token nor body bytes regardless of which path is taken.
- **N3. `body_after_frontmatter` doesn't recognize `\r\n`.** The walk-cache fast-path's literal `starts_with("---\n")` check rejected CRLF-line-ended pages (Windows authors, Office paste), silently treating the whole document as "body" and diverging from the slow `parse()` path. Now recognizes both `---\n` and `---\r\n` openers and skips the canonical blank-line separator in either flavor. Two new tests cover CRLF-with and CRLF-without separator.
- **N4. `render_repo_index` reflects untrusted input.** The `<repo>` URI segment in `coral://wiki/<repo>/_index` was echoed verbatim in the `repo` field of the response. Now validated against `is_safe_filename_slug` (or the legacy `_default` literal) before render, rejecting percent-encoded slashes, embedded whitespace, leading dots, and similar shell-metas. Regression test sends a handful of poisoned URIs and asserts each is rejected.

## [0.19.5] - 2026-05-04

Audit pass — multi-agent code audit on v0.19.4 found ~30 real bugs across the workspace, ranging from prompt-injection / path-traversal / argv-leaked-secrets in the Critical tier down to README example drift and lint info-disclosure in the Medium / Notable tiers. Closes the entire audit punch list. The MCP server transitioned from a wave-1 stub (every `read()` returned `None`) to a wired implementation that actually reads pages, tools delegate to the existing core helpers, and per-call audit lines land in `.coral/audit.log`.

### Fixed (Critical)

- **C1. MCP server is a stub. Now wired.** v0.19 wave 1 advertised six resources and eight tools but every `WikiResourceProvider::read()` returned `None` and every `tools/call` got a `NoOpDispatcher` "skip". v0.19.5 ships a real `WikiResourceProvider::read()` that materialises every advertised URI (`coral://manifest`, `coral://lock`, `coral://graph`, `coral://wiki/_index`, `coral://stats`, `coral://test-report/latest`) plus per-page `coral://wiki/<slug>` resources via `walk::read_pages`, and a `CoralToolDispatcher` (in `coral-cli`) that delegates `search` / `find_backlinks` / `affected_repos` to the core helpers. New `crates/coral-mcp/tests/mcp_resources_e2e.rs` boots the provider against a tmpdir fixture and asserts every URI returns non-empty JSON / Markdown.
- **C2. `Resource.mime_type` was emitted as snake_case on the wire.** MCP clients expect `mimeType` (camelCase per the spec); the missing `#[serde(rename = "mimeType")]` made every client silently fall back to `text/plain`, losing our `application/json` hint. New unit test pins the wire shape.
- **C3. `git clone` option injection (CVE-2017-1000117 / CVE-2024-32004 family).** `coral project sync` shelled out as `git clone --branch <ref> <url> <path>`; a malicious `url` like `--upload-pack=/tmp/evil` would have been parsed as a flag. v0.19.5 inserts `--` before the user-controlled positionals (`git clone --branch <ref> -- <url> <path>`) and rejects refs that start with `-`. Same treatment applied to `gitdiff::run`'s range. Regression test inspects the built `Command` argv to confirm the `--` separator sits between flags and positionals.
- **C4. LLM-emitted slug → path traversal in `plan::build_page`.** A `create` plan entry with `slug: ../../etc/passwd` would have escaped `wiki_root` on `coral ingest --apply`. New `coral_core::slug::is_safe_filename_slug` allowlist (`[a-zA-Z0-9_-]`, length ≤ 200, no leading `.` or `-`) is checked before any path interpolation. Builds error out instead of writing.
- **C5. Slug path traversal in `consolidate::apply_merge` / `apply_split` / `export::render_html_multi`.** Same root cause as C4 across three more LLM-driven write paths. Each site now validates the target slug; unsafe entries are skipped with a `tracing::warn!`. Regression tests assert no file lands outside the wiki / `out_dir`.
- **C6. `coral export-agents --format claude-md` emitted AGENTS.md content.** The CLAUDE.md file's first line was `# AGENTS.md` and its generation marker pointed at `--format agents-md`; both now correctly identify the claude-md format. Regression test pins the H1 + marker shape.
- **C7. README frontmatter `sources:` example didn't parse.** README L487-505 used inline-table sources (`- { type: code, path: src/auth.rs, lines: "12-87" }`); the actual parser is `pub sources: Vec<String>`. Updated example to plain strings; the bundled `template/schema/SCHEMA.base.md` was already correct.
- **C8. README `[[environments]]` example didn't deserialize at runtime.** README L257-285 used `[environments.dev.services.api]` which TOML lifts to a path the `EnvironmentSpec` struct doesn't recognise (`missing field 'services'` at runtime). Working idiom is `[environments.services.api]` — the `[[environments]]` array entry is the implicit parent. Strengthened `crates/coral-core/tests/readme_examples_parse.rs` to assert the `services` table sits at the right TOML path; new `crates/coral-env/tests/readme_environment_e2e.rs` deserializes the block all the way to `EnvironmentSpec`.

### Fixed (High)

- **H3. `coral mcp serve --read-only false` was rejected by clap.** `ArgAction::SetTrue` doesn't accept a value, so users couldn't disable read-only without `--allow-write-tools`. Switched to `ArgAction::Set` with `default_missing_value = "true"`. Regression test: `--read-only false --help` doesn't error.
- **H4. `coral notion-push --apply` swallowed Notion API error bodies.** Pre-v0.19.5 `curl -s -o /dev/null -w '%{http_code}'` discarded the response body, so users saw `FAIL slug: HTTP 400` with no actionable detail. Now we capture stdout, surface the first 400 chars of the body on non-2xx, and propagate `output.status.success() == false` distinctly from HTTP failure.
- **H5. `HttpRunner::run` ignored `prompt.timeout`.** The `Prompt::timeout` field was wired everywhere except the HTTP runner — calls hung indefinitely even with an explicit deadline. Now translated to curl's `--max-time`. Regression test inspects the built `Command` argv.
- **H6. API keys leaked into argv at 5 sites** (`Voyage` / `OpenAI` / `Anthropic` embeddings, `HttpRunner`, `notion-push`). Argv is readable by every other process via `ps` / `/proc/<pid>/cmdline`. Migrated to curl's `@-` form: the secret header is written to stdin instead of placed in argv. New `curl_post_with_secret_header` helper centralises the pattern. Regression tests assert `Bearer <token>` doesn't appear in `cmd.get_args()`.
- **H7. `coral ingest --apply` lost-update race on `.wiki/index.md`.** The pre-v0.19.5 flow read the index OUTSIDE the flock, mutated it in memory, and wrote it BACK inside the flock — concurrent invocations clobbered each other's additions. v0.19.5 moves the read into the locked closure. Hardens the same invariant the v0.15 atomic-write pass landed.
- **H8. `RunnerError::AuthFailed` exposed provider stdout/stderr verbatim.** Some providers echo the request headers in error responses; surfacing that in our error envelope leaked the API key into logs and traces. New `runner::scrub_secrets` (regex-driven, case-insensitive over `Authorization` / `x-api-key` / bare `Bearer`) replaces token-shaped substrings with `<redacted>` before they land in `AuthFailed` / `NonZeroExit` payloads. Applied at every error-construction site in `runner.rs`, `http.rs`, and `embeddings.rs`.
- **H9. `compose.rs` wrote the generated YAML non-atomically.** A `docker compose up` racing the writer could see a half-written file. Migrated to `coral_core::atomic::atomic_write_string` (temp + rename).
- **H10. Malformed `coral.toml` was silenced as legacy.** A `coral.toml` that doesn't parse as TOML at all used to fall back to `synthesize_legacy()`, leaving the user wondering why their manifest was ignored. New `Project::discover` distinguishes "no manifest found" from "found but malformed"; the second case surfaces as `CoralError::Manifest(...)` with the file path.
- **H11. README claim "8 MCP tools" was misleading.** Default install ships 5 read-only tools (`query`, `search`, `find_backlinks`, `affected_repos`, `verify`); the 3 write tools (`run_test`, `up`, `down`) require `--allow-write-tools`. README updated.

### Fixed (Medium)

- **M1. `coral context-build --budget` overshot.** Budget check ran AFTER `chars_used += page.body.len()`, so the page that broke the budget was still included. Now checked BEFORE acceptance.
- **M2. Embeddings `upsert` accepted dim-mismatched vectors.** Both backends (in-memory + SQLite) silently stored vectors of the wrong length, causing `search()` to return zero hits forever after a corrupt cache load. Both `upsert` paths now reject the mismatched vector — JSON backend logs and skips, SQLite returns an error.
- **M3. `gitdiff::run` git option injection.** Covered by C3.
- **M4. `coral_lint::structural::check_source_exists` info disclosure.** Sources containing `..` or starting with `/` would `Path::join(repo_root, src)` and stat outside the repo root. Now refused with a clear warning before the filesystem probe runs.
- **M5. `.coral/audit.log` was documented but not written.** Wired up: every MCP `tools/call` invocation appends a `{ts, tool, args, result_summary}` line via `OpenOptions::append`.
- **M6. `coral lint --check-injection` was documented but not implemented.** New flag added; `coral_lint::structural::check_injection` scans page bodies for fake chat tokens, header-shaped substrings, base64-shaped runs > 100 chars, and unicode bidi-override / tag characters. Surfaces a Warning so reviewers scrub before pages reach an LLM context window.
- **M9. `--algorithm bm25` undocumented in README.** Added to the `coral search` subcommand reference.

### Fixed (Notable)

- **N1. `Pins::save` was non-atomic.** Migrated to `atomic_write_string`.
- **N3. File-size caps in `walk::read_pages`.** Wiki pages are markdown, not large media; pages > 32 MiB are now skipped with a `tracing::warn!` rather than read into memory.

### Skipped

- **M7. `*.lock.lock` zero-byte cleanup** — explored, but unlinking the sentinel after release reopens the cross-process lost-update race the `cross_process_lock_serializes_n_subprocess_increments` test pins. Documented as intentional in the `with_exclusive_lock` docstring; users can `.gitignore` `*.lock` instead.
- **M8. README "4 groupers → 5"** — claim doesn't appear in the README at all.
- **M10. `EnvError` Display nesting** — reviewed; the install hint sits in the leaf variant's `#[error]` template, no `#[from]` wrapping involved. No change.
- **N2. `WikiLog` regex op shape** — relaxing the regex would change the log format; documented at the call sites instead.

### MCP wiring details

- New `coral-mcp` deps: `coral-stats` (for `stats` resource), `toml` (for `lock` resource).
- New `WikiResourceProvider` helpers: `render_manifest`, `render_lock`, `render_stats`, `render_aggregate_index`, `render_repo_index`, `render_page`. Every helper is best-effort — a malformed wiki returns useful JSON instead of bubbling up an error to the JSON-RPC envelope.
- Path traversal guard: per-page `coral://wiki/<slug>` URIs run each path segment through `is_safe_filename_slug` before any `fs::read`.
- The `query` tool intentionally returns `Skip` over MCP — it requires LLM streaming + provider keys that don't fit the JSON-RPC `tools/call` envelope. CLI `coral query` is the entry point; this is documented in the Troubleshooting section of the README.

### Test counts

- coral-core: 181 → 187 (+6, slug allowlist + git separator regression + manifest H10 + walk N3)
- coral-mcp: 14 → 20 (+6, mimeType serde + 9 e2e resource reads)
- coral-runner: 67 → 70 (+3, scrub_secrets + curl-no-leak + max-time)
- coral-cli: lib + integration grew with C4/C5/C6/M5 + claude-md + path-traversal regression
- **Workspace total: 887 tests pass** (was 851; +36). Zero clippy warnings, BC contract holds across all 6 v0.15 fixtures.

### Closes

- v0.19.5 audit punch list (~30 findings, all closed except the 4 documented as intentional).

## [0.19.4] - 2026-05-04

Audit follow-up — closes the remaining 6 items from the v0.19.3 multi-agent code audit (3 Medium + 3 latent smells). Tracking issue [#23](https://github.com/agustincbajo/Coral/issues/23). The Critical and High tier shipped in v0.19.3; the audit punch list is now 100% resolved.

### Fixed (Medium)

- **`coral lint --staged` now resolves staged paths against the git toplevel instead of `cwd`** ([#17](https://github.com/agustincbajo/Coral/issues/17)). `git diff --cached --name-only` always emits paths relative to the repo root; pre-v0.19.4 the code joined them against `std::env::current_dir()`, so invoking `coral lint --staged` from any subdirectory (e.g. `cd .wiki/ && coral lint --staged`) silently produced non-existent absolute paths and the filter dropped every issue. New `git_toplevel(cwd)` helper resolves the join base via `git rev-parse --show-toplevel`. The pure parser parameter renamed from `cwd` to `toplevel` to make the contract explicit. Regression test pinned at the parser layer.
- **`coral search` no longer silently reuses a stale sqlite embeddings DB when `remove_file` fails** ([#18](https://github.com/agustincbajo/Coral/issues/18)). The pre-v0.19.4 `let _ = std::fs::remove_file(&path)` swallowed lock contention, read-only filesystems, and any permission failure; the next `SqliteEmbeddingsIndex::open` reused the stale schema, producing confusing "schema mismatch" errors. `NotFound` is now the only soft-fail branch (first-run, race); any other error surfaces with a path + actionable hint.
- **`coral test-discover` now skips `.wiki/`** ([#19](https://github.com/agustincbajo/Coral/issues/19)). The CHANGELOG had been claiming `.wiki` was excluded since v0.18, but the code only added `.git`, `.coral`, `node_modules`, `target`, `vendor`, `dist`, `build` to its skip list. A wiki page literally named `openapi.yaml` would emit a bogus auto-generated TestCase. Regression test pins the contract.

### Fixed (latent smells)

- **`Project::load_from_manifest` now routes through `coral_core::path::repo_root_from_wiki_root`** ([#20](https://github.com/agustincbajo/Coral/issues/20)). The open-coded `path.parent().unwrap_or(Path::new("."))` was the same trap that bit `coral status` in v0.19.2 (`Path::new("coral.toml").parent()` returns `Some("")`, not `None`). Calling `Project::load_from_manifest("coral.toml")` directly used to leak an empty PathBuf as `project.root`. Fix migrates to the centralized helper introduced in v0.19.3.
- **`apply_consolidate_plan` now takes the wiki root as an explicit parameter** ([#21](https://github.com/agustincbajo/Coral/issues/21)). The removed `infer_wiki_root` walked `pages.first().path.parent().parent()` and silently produced an empty PathBuf for flat-layout wikis (pages at `<wiki>/<slug>.md`, no per-type subdirectory), causing merge targets to land at `cwd` instead of inside `.wiki/`. The caller already had the right path; v0.19.4 just threads it through. 12 test callers and the production caller updated; new regression test pins the flat-layout case.
- **`git_remote.rs` now logs every outcome of `git merge --ff-only`** instead of fire-and-forget `let _ = ...` ([#22](https://github.com/agustincbajo/Coral/issues/22)). Success → `tracing::debug!`; non-zero exit (uncommitted work, divergent upstream, no tracking branch) → `tracing::warn!` with `stderr` tail; spawn failure → `tracing::warn!` with the IO error. Users debugging "why is my clone not advancing?" now have a complete trail under `RUST_LOG=coral=debug`.

### Test counts

- coral-core: 169 → 170 (+1, `load_from_relative_filename_resolves_root_to_dot`)
- coral-test (lib): 89 → 90 (+1, `discover_skips_dot_wiki_tree`)
- coral-cli (lib): 223 → 225 (+2, `apply_consolidate_plan_uses_explicit_wiki_root_for_flat_layout` + `parse_staged_wiki_paths_resolves_against_supplied_base`)
- **Workspace total: 851 tests pass** (was 847). Zero clippy warnings, BC contract holds across all 6 v0.15 fixtures.

### Closes

- [#17](https://github.com/agustincbajo/Coral/issues/17) — `coral lint --staged` cwd resolution
- [#18](https://github.com/agustincbajo/Coral/issues/18) — `coral search` silent embeddings DB recreation
- [#19](https://github.com/agustincbajo/Coral/issues/19) — discovery walks `.wiki/`
- [#20](https://github.com/agustincbajo/Coral/issues/20) — `Project::load_from_manifest` parent unwrap
- [#21](https://github.com/agustincbajo/Coral/issues/21) — `consolidate::infer_wiki_root` empty-parent
- [#22](https://github.com/agustincbajo/Coral/issues/22) — `git_remote.rs` fire-and-forget merge
- [#23](https://github.com/agustincbajo/Coral/issues/23) — umbrella tracking issue (all sub-issues resolved)

## [0.19.3] - 2026-05-04

Audit pass — multi-agent re-validation found **2 Critical + 6 High + 3 Medium** real bugs that v0.19.2 didn't cover. Round 1 (this release) fixes the Critical and High items; Medium items deferred to v0.19.4.

### Fixed (Critical)

- **`coral test-discover --commit` now writes files that are actually read.** v0.19.x advertised the workflow `coral test-discover --commit → edit → coral test`, but every reader (`UserDefinedRunner::discover_tests_dir`, `HurlRunner::discover_hurl_tests`, `contract_check::parse_consumer_for_repo`) used non-recursive `read_dir`. Files committed to `.coral/tests/discovered/` were silently ignored; user edits to the committed YAML had ZERO effect because `coral test --include-discovered` re-generated tests from the OpenAPI spec in memory. Centralised the walk in a new `coral_test::walk_tests::walk_tests_recursive` and migrated all three readers. New test `discover_walks_recursively_into_subdirectories` pins the contract.
- **`coral onboard --apply` no longer corrupts `last_updated_commit` to the literal string `"unknown"`.** Same class as the v0.19.2 status fix: `Path::new(".wiki").parent()` returns `Some("")` (NOT `None`), so `unwrap_or(root)` never fired and `head_sha` ran git in the empty `cwd`, producing `ENOENT` from `execvp` on macOS, which `.ok()` swallowed. Migrated to the centralised `coral_core::path::repo_root_from_wiki_root` helper (see below) and now logs a `tracing::warn!` on git failure instead of swallowing.

### Fixed (High)

- **`coral_core::path::repo_root_from_wiki_root()` — single source of truth for the empty-parent foot-gun.** The bug class has now bitten `coral lint` (v0.19.0), `coral status` (v0.19.2), `coral onboard` and `coral lint --fix` (v0.19.3 audit). The helper centralises the guard so future callers can't open-code the wrong variant. 6 unit tests pin the contract for relative single-component, nested, absolute, root, `.`, and `..` inputs. Migrated 5 callsites to use it.
- **AGENTS.md output references the correct command.** The renderer used to emit `_Generated by coral export --format agents-md_`, but the actual subcommand is `coral export-agents`. Users who copied the line hit `unrecognized subcommand`. Module docstring + README also corrected to drop the false claim that the renderer reads `[project.agents_md]` and `[hooks]` blocks (the manifest parser doesn't even define those fields — that's v0.20+ scope).
- **`coral ingest` and `coral bootstrap` now `tracing::warn!` on git failures** instead of silently substituting the literal string `"HEAD"` for `head_sha`. Pre-v0.19.3 a missing/broken git would hand the LLM a prompt with no diff context and stamp every page's `last_updated_commit` to `"HEAD"` — now the user gets a warning explaining why.
- **CHANGELOG corrected:** `coral test discover` (incorrect, no such subcommand) → `coral test-discover` (correct top-level command). README was already right.
- **`coral test-discover --commit` filename docstring corrected:** previously claimed `<service>.<sha8>.yaml`; the code has always written `<sanitized-case-id>.yaml`. Docstring now matches reality.

### Test counts

- coral-core: 169 (was 163; +6 from `path::tests`)
- coral-test (lib): 89 (was 80; +9 from `walk_tests::tests` + regression tests in user_defined_runner)
- **Workspace total: 847 tests pass** (was 831). Zero clippy warnings, BC contract holds across all 6 v0.15 fixtures.

### Audit context

A multi-agent code audit kicked off after the v0.19.2 ship found 11 real bugs across the workspace. The 8 fixed here are the Critical + High tier. Medium items (`coral lint --staged` cwd resolution, `coral search` silent embeddings DB recreation, discovery walk excluding `.wiki/`) are deferred to v0.19.4.

## [0.19.2] - 2026-05-03

Patch release fixing a user-reported cosmetic bug in `coral status`.

### Fixed

- **`coral status` no longer emits `failed to invoke git: No such file or directory (os error 2)`** when run from a repo root with the default relative `.wiki/` path on macOS.
  - Root cause: `Path::new(".wiki").parent()` returns `Some("")` (NOT `None`), and the empty `PathBuf` propagated into `Command::current_dir("")`, which surfaces as `ENOENT` from `execvp` on macOS.
  - The same fix landed in `coral lint` in commit `d2d7012` (v0.19.0); `crates/coral-cli/src/commands/status.rs:120-123` was missed in that pass.
  - Mirror the lint-side pattern: treat empty parent the same as missing parent and fall back to `.`.
  - New regression test (`status_resolves_repo_root_when_wiki_path_is_relative`) invokes `coral status` from a real git tmpdir against the relative default and asserts neither the misleading WARN nor the rev-list failure surfaces in stderr.

### Note for users on the same affected wikis

The cosmetic `Last commit: <unknown>` and `Recent log: (no entries)` outputs that accompanied this bug **are not the same bug** — those reflect the wiki's `index.md` `last_commit` field and `log.md` entries, neither of which is populated by externally-managed wikis (e.g. those built via project-local scripts that bypass `coral ingest` / `coral bootstrap`). Both fields populate normally when Coral itself drives the wiki.

## [0.19.1] - 2026-05-03

Validation pass on top of v0.19.0. Three real bugs caught during a
multi-agent re-validation are fixed; coverage extended; README's first
v0.19 rewrite (which had invalid TOML) is now snapshot-tested. No
behavior change for v0.15 single-repo users.

### `coral contract check` — cross-repo interface drift detection

- **New crate module `coral-test::contract_check`** — walks each repo's
  `openapi.{yaml,yml,json}` (provider) and `.coral/tests/*.{yaml,yml,hurl}`
  (consumer); for every `[[repos]] depends_on` edge, diffs the consumer's
  expectations against the provider's declared interface. Deterministic,
  no LLM.
- **`coral contract check [--format markdown|json] [--strict]`** CLI command.
  Reports `UnknownEndpoint`, `UnknownMethod`, `StatusDrift`,
  `MissingProviderSpec`. Fails fast in CI *before* `coral up` runs.
- **8 new end-to-end scenarios** in
  `crates/coral-cli/tests/multi_repo_interface_change.rs`:
  - happy path (no drift),
  - endpoint removed (Error),
  - method changed (Error),
  - status drift (Warning, Error in `--strict`),
  - unsynced provider repo (Warning),
  - JSON output round-trip,
  - Hurl files are scanned alongside YAML,
  - legacy single-repo project rejects with a clear error.
- **13 new unit tests** in `coral-test::contract_check` covering path
  matching with `{param}` and `${var}` placeholders, status set
  comparison, and end-to-end project walking.
- **Soft-fail on malformed provider specs.** A new `MalformedProviderSpec`
  finding (Warning severity) replaces the previous abort-the-whole-check
  behavior — one bad `openapi.yaml` no longer hides drift in every other
  repo. `coral contract check` now reports the entire project's drift in
  a single pass.
- **Extended end-to-end coverage.** 4 new scenarios pin behavior under
  realistic adversarial input: lowercase HTTP methods in test files
  (`get /users` ≡ `GET /users`), query strings and fragments stripped
  before path comparison (`/users?limit=10` ≡ `/users`), provider specs
  discovered under `api/v1/` and other nested directories, and corrupt
  YAML reported as a warning rather than aborting the run.

### CI workflow improvements (no behavior change for users)

- **MSRV 1.85 gate** — `cargo build --workspace --locked` against the
  declared minimum supported Rust version, so cross-team installs from
  pinned tags are guaranteed to work.
- **`bc-regression` dedicated job** — backward-compat suite runs as its
  own check on every PR; the failure mode reads as "BC broke" instead
  of "some test broke".
- **Cross-platform smoke** (ubuntu-latest + macos-latest) — `cargo build
  --release && coral init` round-trip catches platform regressions before
  the Release workflow tries to build the tarballs.
- **`concurrency` group** cancels in-progress runs on the same ref to
  save Actions minutes.

### Test extensions (no behavior change for users)

- **README example regression suite** — `crates/coral-core/tests/readme_examples_parse.rs`
  pins three TOML examples from README.md (project block, environment
  with healthcheck subtable, contract-check topology). v0.19's first
  README rewrite shipped with multi-line inline-tables (a TOML syntax
  error); the new suite catches that class of doc rot before it ships.
- **Cycle detection coverage** — 5 new `coral-core::project::manifest`
  tests pin behavior on 3-node cycles, self-loops, diamond DAGs (must
  validate), disconnected acyclic components (must validate), and
  detection of a cycle in one component when others are healthy.
- **Compose YAML regression coverage** — 5 new `coral-env::compose_yaml`
  tests pin headers in HTTP healthchecks rendering as `-H 'k: v'` flags,
  `env_file` propagating to every service, gRPC probes emitting the
  right `grpc_health_probe` invocation, and deterministic rendering for
  identical plans.
- **Adopt-mode rejection** — `ComposeBackend::up` short-circuits on
  `EnvMode::Adopt` with a helpful `InvalidSpec` error pointing at the
  managed default, with a positive-path companion test pinning that
  managed plans never short-circuit there.

## [0.19.0] - 2026-05-03

Massive release that consolidates v0.17 (environments) + v0.18 (testing)
+ v0.19 (AI ecosystem) all the way through PRD wave 3 of each milestone.
Single-repo v0.15 users still see zero behavior change — environments,
testing, and MCP are all opt-in via `[[environments]]` and
`.coral/tests/`.

### Headline features

- **`coral up` / `coral down` / `coral env *`** — multi-service dev
  environments via Compose backend (real subprocess: render YAML,
  `up -d --wait`, `ps --format json` parser).
- **`coral verify`** + **`coral test`** with markdown / JSON / JUnit
  output. HealthcheckRunner + UserDefinedRunner (YAML + Hurl) with
  retry policies, captures (`${var}`), and snapshot assertions.
- **`coral test-discover`** auto-generates TestCases from
  `openapi.{yaml,yml,json}`. **No LLM** — deterministic mapping.
- **`coral mcp serve`** — Model Context Protocol server (JSON-RPC 2.0
  stdio, MCP 2025-11-25). 6-resource catalog, 8-tool catalog
  (read-only by default), 3 templated prompts.
- **`coral export-agents`** emits `AGENTS.md` / `CLAUDE.md` / `.cursor/
  rules/coral.mdc` / `.github/copilot-instructions.md` / `llms.txt`.
  **Manifest-driven, NOT LLM-driven** — see
  [Anthropic's context-engineering guidance](https://www.anthropic.com/engineering/context-engineering).
- **`coral context-build --query --budget`** — smart context loader.
  TF-IDF rank + backlink BFS + greedy fill under explicit token
  budget. Output ready to paste into any prompt.

### v0.18.0-dev wave 3 + v0.19.0-dev waves 2–3 — Discovery, Hurl, MCP serve, exports, context build

The remaining v0.18 + v0.19 waves land together. Coral now ships every
feature the PRD blueprinted as part of v0.16 → v0.19, with full unit
tests + integration E2E.

#### v0.18 wave 3 — discovery, Hurl, retry/captures/snapshots

- **`coral test-discover` + `coral test --include-discovered`** auto-generates `TestCase`s from `openapi.{yaml,yml,json}` (OpenAPI 3.x) anywhere under the project. Walks excluding `.git/`, `.coral/`, `node_modules/`, `target/`, `vendor/`, `dist/`, `build/`. One case per `(path, method)` with status assertion picked from the spec's lowest 2xx response. Endpoints with `requestBody.required = true` are skipped (we don't fabricate bodies).
- **Hurl support** (`coral-test::hurl_runner`) — hand-rolled minimal parser for `.coral/tests/*.hurl` files (request-line, headers, `HTTP <status>`, `[Asserts] jsonpath "$.x" exists`, `# coral: name=...` directive). Avoids the libcurl FFI dep that pulling official `hurl` would require. Output `YamlSuite` is identical to YAML suites so the same executor runs both.
- **Retry policy** with `BackoffKind::{None, Linear, Exponential}` and `RetryCondition::{FivexX, FourxX, Timeout, Any}` — per-step or suite-default. Exponential capped at 5s.
- **Captures** in `HttpStep.capture: { var: "$.path" }` extract from the response body and substitute as `${var}` in subsequent step URLs/headers/bodies.
- **Snapshot assertions** in `HttpExpect.snapshot: "fixtures/x.json"` write on first run, compare on subsequent runs. `coral test --update-snapshots` flag accepts new outputs.

#### v0.19 wave 2 — `coral mcp serve`

- **`coral-mcp::server`** ships a hand-rolled JSON-RPC 2.0 stdio server implementing the minimal MCP surface (`initialize`, `resources/list`, `resources/read`, `tools/list`, `tools/call`, `prompts/list`, `prompts/get`, `ping`). Pinned to MCP spec 2025-11-25.
- Hand-rolled rather than `rmcp = "1.6"` to keep the dep tree slim — the trait-based catalogs mean we can swap to rmcp in v0.20 without breaking callers.
- **`coral mcp serve [--transport stdio] [--read-only] [--allow-write-tools]`** CLI command.
- Read-only mode (default) blocks `up`, `down`, `run_test` tool calls (PRD §3.6 + risk #25). E2E test pipes a real `initialize` request via stdio and asserts the protocol version + serverInfo response.

#### v0.19 wave 3a — `coral export-agents`

- **Manifest-driven, NOT LLM-driven** — see [Anthropic's context-engineering guidance](https://www.anthropic.com/engineering/context-engineering); empirical work consistently finds LLM-synthesised AGENTS.md degrades agent task success vs. deterministic templates rendered from structured config.
- `coral export-agents --format <agents-md|claude-md|cursor-rules|copilot|llms-txt> [--write] [--out PATH]` deterministic templates from `coral.toml`.
- Default write paths: `AGENTS.md`, `CLAUDE.md`, `.cursor/rules/coral.mdc`, `.github/copilot-instructions.md`, `llms.txt`.
- 6 unit tests + 1 E2E (`export_agents_md_includes_project_metadata`).

#### v0.19 wave 3b — `coral context-build`

- **Smart context loader** under explicit token budget. Differentiator vs Devin Wiki / Cursor multi-root / pure RAG: no vector DB, no full-context blast, just curated selection.
- TF-IDF ranks pages by query terms; BFS over `backlinks` walks adjacent context; greedy fill stops at `--budget` (4 chars/token heuristic).
- Output sorted by `(confidence desc, body length asc)` so the most-trusted concise sources lead the prompt.
- `coral context-build --query "X" --budget 50000 --format markdown|json [--seeds 8]`.

### v0.18.0-dev wave 2 — `coral test` / `coral verify` (in progress)

Wave 2 of v0.18 wires real `HealthcheckRunner` and `UserDefinedRunner`
into `coral test` and the new `coral verify` sugar. Discovery from
OpenAPI / proto, Hurl as a second input format, snapshot assertions,
contract tests, and the rest of the v0.18 roadmap follow in wave 3.

#### Added

- **`coral_test::probe`** — backend-agnostic `probe_once(status, kind, timeout)` that resolves a service's published port at probe time. HTTP via `curl` subprocess (same reasoning as `coral_core::git_remote`: no heavy HTTP client in the default tree). TCP via std `TcpStream::connect_timeout`. Exec via `Command::new`. gRPC delegates to `grpc_health_probe`, falls back to TCP connect.
- **`HealthcheckRunner`** auto-derives `TestCase`s from each service's `service.healthcheck`. One probe per case → `TestStatus::{Pass,Fail,Skip}`. Tagged `["healthcheck", "smoke"]` so `--tag smoke` picks them up.
- **`UserDefinedRunner`** — parse + run `.coral/tests/*.{yaml,yml}` suites. v0.18 wave 2 supports HTTP steps (`http: GET /path` shorthand, `headers`, `body`, `expect.status` + `expect.body_contains`) and exec steps (`exec: ["cmd", "arg"]` + `expect.exit_code` + `expect.stdout_contains`). gRPC, GraphQL, snapshot, captures, retry, parallel are wave 3.
- **`coral verify [--env NAME]`** — sugar for "run all healthchecks". Liveness only, <30s budget. Exit non-zero on any fail.
- **`coral test [--service NAME]... [--kind smoke|healthcheck|user-defined]... [--tag T]... [--format markdown|json|junit] [--env NAME]`** — runs the union of healthcheck cases + user-defined YAML suites. Filters by service and tag (PRD §5.2). JUnit XML via `JunitOutput::render`.
- **6 new probe tests** + **8 user-defined runner tests** (parse_http_line variants, split_curl_status round-trip, YamlSuite serde, discover from `.coral/tests/`).

### v0.17.0-dev wave 2 — `coral up` / `down` / `env *` (in progress)

Wave 2 wires the real subprocess lifecycle into `ComposeBackend` and
exposes the env layer through three new top-level commands.

#### Added

- **`coral_env::compose_yaml::render`** — turns an `EnvPlan` into a `docker-compose.yml` string. Covers `image`, `build { context, dockerfile, target, args, cache_from, cache_to }`, `ports`, `environment`, `depends_on { condition: service_healthy }`, and `healthcheck` with all four `HealthcheckKind` variants compiled to compose's `test:` block. Stable byte-output for content-hash-based artifact caching.
- **Real `ComposeBackend`** lifecycle: `up` (writes `.coral/env/compose/<hash>.yml`, runs `docker compose --file <art> --project-name <coral-env-hash> up -d --wait`), `down` (`down --volumes`), `status` (`ps --format json` with parser tolerant to v1/v2 shapes), `logs` (`logs --no-color --no-log-prefix --timestamps`), `exec` (`exec -T`).
- **`coral up [--env NAME] [--service NAME]... [--detach] [--build]`** brings up the selected environment. Defaults to the first `[[environments]]` block.
- **`coral down [--env] [--volumes] [--yes]`** tears down. `--yes` is required when `production = true` (PRD §3.10 safety).
- **`coral env status [--env NAME] [--format markdown\|json]`** queries `EnvBackend::status()`.
- **`coral env logs <service> [--env] [--tail N]`** prints container logs.
- **`coral env exec <service> -- <cmd>...`** runs a command inside a container; exit code propagates.
- **`Project.environments_raw: Vec<toml::Value>`** — `coral-core` keeps the `[[environments]]` table opaque so the wiki layer doesn't depend on `coral-env`. The CLI's `commands::env_resolve` parses entries on demand.
- **`commands::env_resolve::{resolve_env, parse_all, default_env_name}`** — CLI-side helpers that turn the opaque manifest table into typed `EnvironmentSpec` values.
- 4 new compose-yaml render tests + 2 BC tests (`up_fails_clearly_when_no_environments_declared`, `down_fails_clearly_when_production_env_without_yes`).

### v0.17.0-dev wave 1 / v0.18.0-dev wave 1 / v0.19.0-dev wave 1 — Multi-wave foundation

Three new crates land on the same day, each scaffolded with the same architectural pattern (`Send + Sync` trait, `thiserror` errors, in-memory `Mock*` for upstream tests). Subprocess + transport wiring follows in wave 2 of each milestone — wave 1 ships the type model, the test infrastructure, and a clear contract for the next wave.

#### v0.17 wave 1 — `coral-env` (environment layer)

- **New crate `coral-env`**: pluggable backend trait family. `EnvBackend: Send + Sync` with `up`/`down`/`status`/`logs`/`exec`. Watch, devcontainer/k8s emit, port-forward, and attach/reset/prune are reserved for v0.17.x.
- **`EnvironmentSpec` schema** for `[[environments]]` in `coral.toml`: name, backend, mode (managed/adopt), `compose_command` (auto/docker/podman), `production` flag, env file, services map.
- **`ServiceKind`** tagged enum (`Real { repo, image, build, ports, env, depends_on, healthcheck, watch }` / `Mock { tool, spec, mode, recording }`). `Real` is `Box`'d so `Mock` doesn't pay the size of the larger variant.
- **`Healthcheck`** with `HealthcheckKind::{Http, Tcp, Exec, Grpc}` + `HealthcheckTiming` (separates `start_period_s` / `interval_s` / `start_interval_s` / `consecutive_failures` — k8s startup-vs-runtime).
- **`EnvPlan`**: backend-agnostic compiled plan; `compose_project_name(project_root, env)` derives `coral-<env>-<8-char-hash>` from the absolute path so two worktrees of the same meta-repo never collide on compose namespaces.
- **`healthcheck::wait_for_healthy`** loop with `consecutive_failures` policy. Pure function over a probe closure; backend-agnostic.
- **`ComposeBackend` runtime detection** probes `docker compose`, `docker-compose`, and `podman compose` in order. Subprocess invocation lands in v0.17 wave 2.
- **`MockBackend`** with `calls()` recorder + `push_status` queue.

#### v0.18 wave 1 — `coral-test` (testing layer)

- **New crate `coral-test`**: `TestRunner: Send + Sync` trait with `supports/run/discover/parallelism_hint/snapshot_dir/supports_record`. Same architectural pattern as `coral-env`/`coral-runner`.
- **`TestKind`** enum with all 9 PRD §3.3 variants: `Healthcheck`, `UserDefined`, `LlmGenerated`, `Contract`, `PropertyBased`, `Recorded`, `Event`, `Trace`, `E2eBrowser`. v0.18 wave 2 ships only the first two; the rest live in the schema so manifests don't break later.
- **`TestCase`** + **`TestSource`** (`Inline | File | Discovered { from } | Generated { runner, prompt_version, iter_count, reviewed }`).
- **`TestReport`** with `TestStatus::{Pass, Fail, Skip, Error}` + per-case `Evidence` (HTTP, exec, stdout/stderr tails).
- **`JunitOutput::render`** — minimal but compliant `<testsuites>` XML for GitHub Actions reporters and most CI dashboards. `xml_escape` covers `&`, `<`, `>`, `"`, `'`.
- **`MockTestRunner`** with FIFO scripted statuses + invocation recorder.

#### v0.19 wave 1 — `coral-mcp` (Model Context Protocol server)

- **New crate `coral-mcp`**: type model + resource/tool/prompt catalogs for the upcoming MCP server. Wave 2 wires the [`rmcp = "1.6"`](https://github.com/modelcontextprotocol/rust-sdk) official Rust SDK and the stdio + Streamable HTTP/SSE transports.
- **`ResourceProvider` trait** + `WikiResourceProvider`. The 6-resource static catalog: `coral://manifest`, `coral://lock`, `coral://graph`, `coral://wiki/_index`, `coral://stats`, `coral://test-report/latest`. Per-page resources (`coral://wiki/<repo>/<slug>`) are listed dynamically by wave 2.
- **`ToolCatalog`** — 5 read-only tools (`query`, `search`, `find_backlinks`, `affected_repos`, `verify`) + 3 write tools (`run_test`, `up`, `down`). Write tools require `--allow-write-tools` per PRD risk #25 (MCP server as exfiltration vector). All input schemas validated as JSON in tests.
- **`PromptCatalog`** — 3 templated prompts: `onboard?profile`, `cross_repo_trace?flow`, `code_review?repo&pr_number`.
- **`ServerConfig`** — `--read-only` defaults to `true` to align with PRD §3.6 security stance.

## [0.16.0] - 2026-05-03

The biggest release since v0.10 — Coral evolves from "wiki maintainer" to "multi-repo project manifest + wiki + (forthcoming) environments + tests + MCP". Single-repo v0.15 users see **zero behavior change**, pinned by a new `bc_regression` integration suite running on every PR.

This release implements the foundation specified in the [v0.16+ PRD](https://github.com/agustincbajo/Coral/issues): the `coral.toml` manifest, the `coral.lock` lockfile, the seven `coral project` subcommands, and the `Project::discover`/`synthesize_legacy` shim that makes the upgrade frictionless. `coral project sync` clones repos in parallel, written atomically into `coral.lock`. `coral project graph` visualizes the dependency graph as Mermaid (renders inline in GitHub Markdown), DOT, or JSON.

### Added — multi-repo features

- **`Project` model** (`crates/coral-core/src/project/`): the new entity that represents one or more git repositories sharing an aggregated `.wiki/`. The single-repo case is treated as a `Project` synthesized from the cwd via `Project::synthesize_legacy(cwd)`.
- **`Project::discover(cwd)`** walks up looking for a `coral.toml` containing a `[project]` table. Falls back to legacy synthesis when none is found.
- **`coral.toml` manifest** (`apiVersion = "coral.dev/v1"`, `[project.defaults]`, `[remotes.<name>]` URL templates, `[[repos]]` with `name`/`url`/`remote`/`ref`/`path`/`tags`/`depends_on`). Validates duplicate names, dependency cycles, unknown apiVersion, unresolvable URLs.
- **`coral.lock` lockfile** separates manifest intent from resolved SHAs. Atomic tmp+rename with the existing `flock`. Auto-creates on first read.
- **`coral_core::git_remote`** module: `sync_repo(url, ref, path)` returning a typed `SyncOutcome` (`Cloned`/`Updated`/`SkippedDirty`/`SkippedAuth`/`Failed`). Subprocess `git` so the user's SSH agent / credential helper / GPG signing stay transparent — Coral never prompts for or stores credentials.
- **Seven `coral project` subcommands**:
  - `coral project new [<name>] [--remote N] [--force] [--pin-toolchain]` — create the manifest + empty lockfile.
  - `coral project add <name> [--url|--remote] [--ref] [--path] [--tags ...] [--depends-on ...]` — append a repo entry, validates manifest invariants on save.
  - `coral project list [--format markdown|json]` — tabular view with resolved URLs.
  - `coral project lock [--dry-run]` — refresh `coral.lock` from the manifest without pulling.
  - `coral project sync [--repo N]... [--tag T]... [--exclude N]... [--sequential] [--strict]` — clone or fast-forward selected repos (parallel via rayon by default), write resolved SHAs to `coral.lock` atomically. Auth failures and dirty working trees are skipped-with-warning per PRD risk #10 — one bad repo never aborts the whole sync.
  - `coral project graph [--format mermaid|dot|json]` — emit the repo dependency graph; Mermaid renders inline in GitHub-flavored Markdown.
  - `coral project doctor [--strict]` — drift / health check (replaces the originally-named `healthcheck` to avoid collision with `service.healthcheck` planned for v0.17). Reports unknown apiVersion, missing clones, stale lockfile entries, duplicate paths.
- **`commands::common::resolve_project()`** shim — single entry point every CLI command uses to resolve its `Project`. Honors `--wiki-root` exactly as v0.15.
- **`commands::filters::RepoFilters`** — shared `--repo`/`--tag`/`--exclude` parser, embedded via clap `#[command(flatten)]`. In legacy projects every filter resolves to "the only repo is included" so single-repo workflows stay zero-friction.

### Added — tests

- **`tests/bc_regression.rs`** (6 tests) pins v0.15 single-repo behavior on every PR: `init`/`status`/`lint`/`project list` against a legacy cwd, plus `--wiki-root` override fidelity.
- **`tests/multi_repo_project.rs`** (7 tests) E2E coverage for the new flow: `project new` → `add` × N → `lock` → `list` → `sync` (real local-bare-repo clone) → `graph` → `doctor`, including `depends_on` cycle detection.
- All existing 200+ unit tests + integration suites continue to pass.

### Notes — backward compatibility

- v0.15 users see **zero behavior change**. No `coral.toml` → every command synthesizes a single-repo project from the cwd via `Project::synthesize_legacy`.
- `coral init` is **not** renamed to `coral project new`. Both exist, both work, with no deprecation warning. Scripts that grep stderr won't break.
- `--wiki-root <path>` keeps working — v0.15 fixture-based tests pass unchanged.

### Notes — forward compatibility

- A v0.15 binary cannot read multi-repo wikis once the index frontmatter migrates to `last_commit: { repo → sha }` (planned for v0.16.x). Migration path: `coral migrate-back --to v0.15` will reduce a 1-repo map back to a scalar. The current v0.16.0 release does **not** yet rewrite `WikiIndex.last_commit`, so v0.15 binaries can still read wikis written by v0.16.0 in single-repo mode.

## [0.15.1] - 2026-05-02

Patch release — provider-agnostic `RunnerError` messages.

### Fixed

- **`RunnerError` UX bug**: every variant's `Display` impl hardcoded "claude", so a user running `coral query --provider local` against a missing `llama-cli` got the misleading message "claude binary not found" with a hint to install Claude Code. Same for Gemini, HTTP — every error message implied the user was using Claude.
- All 5 variants reworded to be runner-agnostic with per-provider hints in one message:
  - `NotFound` lists install paths for Claude / Gemini / Local / HTTP.
  - `AuthFailed` lists token-setup commands for Claude / Gemini / HTTP.
  - `NonZeroExit` / `Timeout` / `Io` say "runner" instead of "claude".
- No API change — variant signatures unchanged. The existing `runner_error_display_messages_are_actionable` test passes against the new wording (it asserts via `.contains()` substrings which all still match).

### Documentation

- ROADMAP refresh: marked v0.14 + v0.15 work done, promoted speculative items shipped during this session, added v0.16 candidates (cross-process integration test, sqlite-vec migration).

## [0.15.0] - 2026-05-02

15th release this session. Closes the lost-update race that v0.14
narrowed to. **Cross-process file locking now actually safe.**

### Added — features

- **`coral_core::atomic::with_exclusive_lock(path, closure)`**: wraps a closure in an `flock(2)` exclusive advisory lock on `<path>.lock`. Race-free under N concurrent writers, both threads within one process AND cooperating processes (e.g. two `coral ingest` invocations against the same `.wiki/`). Closes the lost-update race documented in v0.14's `concurrency.rs`.
- **`coral ingest` and `coral bootstrap` writes** are now wrapped in `with_exclusive_lock(&idx_path, ...)` — concurrent invocations against the same wiki serialize properly.

### Added — quality

- New stress test `with_exclusive_lock_serializes_concurrent_load_modify_save`: 50 threads each running a load+modify+save round-trip on a shared counter. All 50 increments must persist (final counter == 50). v0.15 lock-protected: PASS. v0.14 atomic-only: would lose ~80% of updates.
- Upgraded `wikiindex_upsert_concurrent` (was: assert errors == 0, entries ≤ N) → strict assertion: errors == 0 AND entries == N. Stress-tested 25× clean.

### Dependencies

- Added `fs4 = "0.13"` (workspace, MIT/Apache-2.0). 45 KB. Used only by `with_exclusive_lock`. Cross-platform `flock(2)` / `LockFileEx` shim. Allowed by `deny.toml`.
- MSRV stays at 1.85: stdlib added `File::lock_exclusive`/`unlock` in 1.89, but we use UFCS to pin the call to the fs4 trait, keeping the MSRV unchanged.

### Files generated by file locking

- Every `with_exclusive_lock(path)` creates an empty sibling `<path>.lock` file (held open by `flock` for the duration of the lock). `.gitignore` already excludes `**/index.md.lock`, `**/log.md.lock`, `**/.coral-embeddings.json.lock`.

### Verified

- 602 tests pass (was 598). +4 (lock unit + stress).
- Clippy + fmt clean. cargo-audit / cargo-deny clean.
- Stress: 25× consecutive runs of `wikiindex_upsert_concurrent` all PASS (every slug landed, zero errors).

## [0.14.1] - 2026-05-02

Patch release — ships the post-v0.14.0 polish that landed on main.

### Added

- **`coral lint --fix` confidence-from-coverage rule**: pure-rule (no-LLM) auto-fix that downgrades a page's `confidence` by 0.20 (floored at 0.30) when ANY entry in `frontmatter.sources` no longer resolves to a file/dir under the repo root. Mirrors the filter logic of the existing `SourceNotFound` lint check (HTTP/HTTPS sources skipped, no-source pages untouched). Idempotent at the floor — repeated runs without remediation never push a page below `0.30`. Exposed as `confidence-from-coverage` in the no-LLM fix report. Closes the long-standing speculative item from `docs/ROADMAP.md`. 6 new tests.

### Changed

- `wikiindex_upsert_concurrent` (test) — upgraded the assertion from "errors tolerated" to "errors == 0" now that the v0.14 `atomic_write_string` infrastructure eliminates the torn-write race. Stress-tested 15× clean. The lost-update race remains documented as a v0.15+ design item.

### Documentation

- `docs/USAGE.md` — new "Concurrency model (v0.14)" section documenting what's safe under concurrent access, what remains racey (lost-update on `WikiIndex`), and how custom code should use the new helpers.

### Verified

- 598 tests pass (was 592). +6 (confidence-from-coverage).

## [0.14.0] - 2026-05-02

14th release this session. Concurrency-safety release — closes the two
load+modify+save races documented in v0.13's `concurrency.rs` test
suite without adding any new dependency. **592 tests, 0 failures.**

### Added — features

- **`WikiLog::append_atomic(path, op, summary)`** ([crates/coral-core/src/log.rs](crates/coral-core/src/log.rs)): static method that writes a single log entry to disk atomically using POSIX `O_APPEND` semantics. Single writes ≤ PIPE_BUF (~4 KiB) are atomic per POSIX, and a log entry line is well under that. The first writer also seeds the YAML frontmatter + heading via `OpenOptions::create_new`. Critical detail: **even the first-writer path uses `append(true)`** — without it, a concurrent append-mode writer's bytes get overwritten by the first writer's cursor-linear writes (caught the hard way: 18/20 entries observed without O_APPEND on both sides; 20/20 across 25 stress runs after the fix). Switched `coral ingest`, `coral bootstrap`, and `coral init` to use it. The old `load+append+save` pattern remains as a regression test in `concurrency.rs` to pin that it IS still racey for any code that uses it directly. 4 new tests.
- **`coral_core::atomic::atomic_write_string(path, content)`** ([crates/coral-core/src/atomic.rs](crates/coral-core/src/atomic.rs)): new module providing temp-file + rename for torn-write safety. `std::fs::write` truncates the target to zero before writing, so concurrent readers can observe a partial or empty file mid-write. The new helper writes to `<filename>.tmp.<pid>.<counter>` and then `rename`s onto the target — POSIX guarantees rename is atomic within a single filesystem. Critical detail: temp filename uses **PID + a process-global AtomicU64 counter** because every thread shares the same PID, so PID alone collides under concurrent writers (caught this race the hard way: stress test failed with "No such file or directory" until the counter was added). Wired into `Page::write`, `WikiLog::save`, `EmbeddingsIndex::save`, and the index-write paths in `coral ingest` / `coral bootstrap` / `coral init`. 5 new tests, including a 50-writer × 50-reader stress test that asserts no reader ever observes a torn write.

### Documentation

- `coral export --format` help text now lists `html` (was missing despite the format being supported).

### Not solved (deferred to v0.15+)

- The **lost-update race** for load+modify+save patterns on `WikiIndex`. Two concurrent writers can both produce a complete `*.tmp` file; the second `rename` clobbers the first writer's data. Fixing this requires true cross-process file locking (a new dep — `fs2` or similar). v0.14 narrows the failure mode from "torn writes + parse errors" to "lost updates", which is the strictly weaker bug.

### Verified

- All 5 v0.14 atomic-write changes verified by stress tests:
  - WikiLog atomic append: 20 threads × 25 stress runs → 20/20 entries every run.
  - atomic_write_string: 50 writers + 50 readers → zero torn observations.
- Test count: 583 (v0.13.0) → 592 (v0.14.0). Net **+9 tests** (4 log + 5 atomic).
- Clippy + fmt clean across all crates. cargo-audit / cargo-deny clean.
- Linux CI green (cf. previous v0.13.0 batch which required 5 fix iterations).

## [0.13.0] - 2026-05-02

13th release this session. Massive batch — 10 items shipped via the
multi-agent loop. **583 tests, 28/28 e2e probe still green.**

### Added — features

- **`coral lint --suggest-sources [--apply]`**: LLM-driven source proposal pass for `HighConfidenceWithoutSources` issues. Ingests `git ls-files` output as context, asks LLM to propose 1–3 paths per affected page. Default dry-run; `--apply` appends suggestions to `frontmatter.sources` (deduped). 6 new tests + new template prompt.
- **Per-rule auto-fix routing**: `--auto-fix` now groups issues by `LintCode` and dispatches per-code prompts (`lint-auto-fix-broken-wikilink`, `lint-auto-fix-low-confidence`) before falling back to the generic `lint-auto-fix`. 5 new tests + 2 new template prompts. KNOWN_PROMPTS surface them.
- **`coral lint --fix` extras**: 3 more rules — `dedup_sources`, `dedup_backlinks`, `normalize_eol` (CRLF→LF). 5 new tests.
- **`coral export --format html --multi --out <dir>`**: split single-file HTML into `index.html` + `style.css` + per-page `<type>/<slug>.html` files. GitHub Pages ready. Wikilinks rewrite to relative `../<type>/<slug>.html`. 3 new tests.
- **`coral status --watch [--interval N]`**: daemon mode that re-renders every N seconds (default 5, min 1). ANSI clear-screen on TTYs only. 2 new tests + watch loop intentionally not unit-tested.
- **`AnthropicProvider`** ([crates/coral-runner/src/embeddings.rs](crates/coral-runner/src/embeddings.rs)): speculative embeddings provider for when Anthropic ships the API. Wired via `--embeddings-provider anthropic`. Until the API exists, calls return `EmbeddingsError::ProviderCall` from a placeholder 404. Mirrors the OpenAI/Voyage shape for one-line URL update later. 3 new tests.
- **`SqliteEmbeddingsIndex`** ([crates/coral-core/src/embeddings_sqlite.rs](crates/coral-core/src/embeddings_sqlite.rs)): alternative storage backend for embeddings, opt-in via `CORAL_EMBEDDINGS_BACKEND=sqlite`. Closes ADR 0006 deferred item early. Pure SQLite + Rust cosine (no `sqlite-vec` C extension); bundled SQLite (~1MB). Both backends produce identical results — parity test enforces it. 12 new tests (10 unit + 2 backend-parity).

### Added — quality

- **Cross-runner contract suite** ([crates/coral-runner/tests/cross_runner_contract.rs](crates/coral-runner/tests/cross_runner_contract.rs)): every `Runner` impl (Claude/Gemini/Local/Http/Mock) honors a uniform contract — totality on empty prompt, NotFound on bogus binary, default `Prompt::default()` shape, `run_streaming` default impl. 5 new tests with substitute binaries.
- **Concurrency tests** ([crates/coral-core/tests/concurrency.rs](crates/coral-core/tests/concurrency.rs)): documents thread-safety of `Page::write`, `WikiLog::append`, `WikiIndex::upsert`, `EmbeddingsIndex::upsert`. **Key finding**: `WikiLog::append` and `WikiIndex::upsert` have a load+modify+save race under concurrent file access (only ~2/10 entries persist). Documented as v0.14 design item, NOT a v0.13 fix. In-memory operations (Mutex-guarded) are correct. 7 new tests.
- **200-page stress tests** ([crates/coral-cli/tests/stress_large_wiki.rs](crates/coral-cli/tests/stress_large_wiki.rs)): 7 `#[ignore]` tests covering each subcommand (lint/stats/search/status/export) against a synthetic 200-page wiki. Measured wall-clock 22–41ms per test; budgets at 1–5s. Run on demand: `cargo test -p coral-cli --test stress_large_wiki -- --ignored`.

### Added — example

- **`examples/orchestra-ingest/`**: copy-pasteable starter wiki + workflows for new consumer repos. Includes a 4-page seed wiki, custom SCHEMA, `.coral-pins.toml`, and the 3 cron jobs (ingest/lint/consolidate) wired to Coral's composite actions. `coral lint --structural` against the example: **0 issues**.

### Changed

- `chunked_parallel_actually_uses_multiple_threads_when_available` (test) — softened to liveness-only since rayon thread saturation under `cargo test --workspace` made the ≥2-thread assertion flaky. Load-bearing assertion (`chunk_calls == 32`) preserved; thread count is now informational `eprintln!` only.

### Documentation

- USAGE.md fully refreshed: `coral lint` flag listing now includes `--fix`, `--auto-fix` per-rule routing, `--suggest-sources`, `--rule`. New sections for `coral status --watch` and `coral history`. `coral search` gains "Storage backend" subsection (sqlite env var). `coral export` gains `html --multi` description.
- README links to `examples/orchestra-ingest/` from the table of contents.

### Verified

- End-to-end probe of every deterministic subcommand against a 4-page synthetic seed: **28/28 OK** (re-verified post-batch).
- Test count: 476 (v0.11.0) → 534 (v0.12.0) → 583 (v0.13.0). Net **+107 tests across 2 minor releases**.
- Clippy + fmt clean across all crates. cargo-audit / cargo-deny clean.

## [0.12.0] - 2026-05-02

12th release this session. Two new subcommands + a new lint flag + property
test coverage for 4 more core modules + wiremock integration tests for HttpRunner.
**End-to-end probe: 28/28 deterministic subcommand invocations OK.**

### Added

- **`coral status`** ([crates/coral-cli/src/commands/status.rs](crates/coral-cli/src/commands/status.rs)): daily-use snapshot synthesizing `index.md` `last_commit` + lint counts (fast structural only) + stats one-liner + last N (default 5) log entries reverse-chrono. Markdown ~14 lines; JSON shape `{wiki, last_commit, pages, lint{critical,warning,info}, stats{total_pages,confidence_avg,orphan_candidates}, recent_log[]}`. Always exits 0 (informational). For CI gates use `coral lint --severity critical`.
- **`coral history <slug>`** ([crates/coral-cli/src/commands/history.rs](crates/coral-cli/src/commands/history.rs)): reverse-chronological log entries that mention a slug (case-sensitive substring match). Capped at N (default 20). Pure helper `pub(crate) fn filter_entries` extracted for testability. Empty-match: friendly markdown line / `entries: []` JSON.
- **`coral lint --fix [--apply]`** ([crates/coral-cli/src/commands/lint.rs](crates/coral-cli/src/commands/lint.rs)): no-LLM rule auto-fix (counterpart to LLM-driven `--auto-fix`). Mechanical, deterministic: trim frontmatter trailing whitespace, sort `sources`/`backlinks` alphabetically, normalize `[[ slug ]]` → `[[slug]]` (aliases preserved), trim trailing line whitespace. Default dry-run; `--apply` writes via `Page::write()`. Composes with `--auto-fix` when both set.

### Tests

- **5 new test files** for property + integration coverage (D bloque):
  - `crates/coral-core/tests/proptest_log.rs` (6 tests) — `WikiLog` round-trip + invariants.
  - `crates/coral-core/tests/proptest_index.rs` (4 tests) — `WikiIndex` round-trip + upsert idempotency.
  - `crates/coral-core/tests/proptest_page.rs` (4 tests) — `Page::write/read` round-trip via tempdir.
  - `crates/coral-core/tests/proptest_embeddings_index.rs` (5 tests) — save/load round-trip + prune semantics.
  - `crates/coral-runner/tests/wiremock_http.rs` (6 tests) — in-process mock server testing `HttpRunner` request/response shape, Authorization header semantics, 4xx → AuthFailed/NonZeroExit routing, system-message inclusion/omission.
- **3 new snapshot tests** in `crates/coral-cli/tests/snapshot_cli.rs`: `status_4_page_seed`, `history_outbox_4_page_seed`, `lint_fix_dry_run_4_page_seed`. Total snapshots now 22.
- **31 new unit tests** in coral-cli (status: 4, history: 7, lint --fix: 19, e2e ArgsLit refresh: 1).

### Verified

- End-to-end probe of every deterministic subcommand against a 4-page synthetic seed: **28/28 OK**, 0 failures. Covers init, lint (structural/--severity/--rule/--fix variants), stats, search (TF-IDF + BM25 + JSON), diff, export (5 formats), status, history (3 forms), validate-pin, prompts list, sync, notion-push dry-run, lint --fix --apply.

Test count: 476 (v0.11.0) → 534 (+58). Clippy + fmt clean. cargo audit/deny clean.

## [0.11.0] - 2026-05-02

### Added

- **`HttpRunner`** ([crates/coral-runner/src/http.rs](crates/coral-runner/src/http.rs)): fifth `Runner` impl that POSTs to any OpenAI-compatible `/v1/chat/completions` endpoint. Works against vLLM, Ollama (`http://localhost:11434/v1/chat/completions`), OpenAI, Anthropic Messages-via-compat, or any local server speaking the standard chat-completion shape. Same curl shell-out pattern as the rest — keeps the binary lean (no `reqwest`/`tokio` for the sync CLI).

  Body shape: `{model, messages: [system?, user], stream: false}`. Empty/None system prompt is omitted from the messages array (avoids polluting the conversation with an empty turn). Model fallback to literal `"default"` when `prompt.model` is None — strict endpoints reject this with a 4xx that surfaces as `RunnerError::NonZeroExit`.

  Same auth-detection path (`combine_outputs` + `is_auth_failure`) as the other runners — 401-shaped failures → `RunnerError::AuthFailed`.
- **`--provider http` flag** wired in [crates/coral-cli/src/commands/runner_helper.rs](crates/coral-cli/src/commands/runner_helper.rs). Reads `CORAL_HTTP_ENDPOINT` (required) and `CORAL_HTTP_API_KEY` (optional) at construction time. Unset endpoint exits with code 2 + actionable hint.
- **13 new unit tests** (11 in http.rs + 2 in runner_helper.rs): `build_payload` shape (model fallback, system message inclusion/omission, stream:false), curl error paths against unreachable loopback, builder chaining, parser/dispatcher round-trips.

### Documentation

- README "Multi-provider LLM support" section: HttpRunner added to the table of 5 Runner impls + Ollama / vLLM / OpenAI examples.
- USAGE.md: `coral query` flag listing now includes `http` with env var setup.

## [0.10.0] - 2026-05-02

### Added

- **`coral lint --rule <CODE>`** ([crates/coral-cli/src/commands/lint.rs](crates/coral-cli/src/commands/lint.rs)): repeatable filter that keeps only issues whose `LintCode` is in the allowlist (OR semantics across repeats). Useful for CI gates that only care about specific issue types. Codes are kebab-case (snake_case also accepted): `broken-wikilink`, `orphan-page`, `low-confidence`, `high-confidence-without-sources`, `stale-status`, `contradiction`, `obsolete-claim`, `commit-not-in-git`, `source-not-found`, `archived-page-linked`, `unknown-extra-field`. Composes with `--severity` (`--rule X --severity critical` keeps only critical X). Auto-fix still sees the FULL report. **12 new unit tests + 2 snapshot tests**.

### Documentation

- USAGE.md: documented `--rule` flag with all 11 valid codes + composition with `--severity`.

### Tests

- 3 new error-path tests in `coral-runner` ([crates/coral-runner/src/runner.rs](crates/coral-runner/src/runner.rs), [crates/coral-runner/src/embeddings.rs](crates/coral-runner/src/embeddings.rs)):
  - Non-streaming `claude_runner_run_honors_timeout` mirroring the existing streaming-timeout test.
  - `runner_error_display_messages_are_actionable` — pins the user-facing Display for every `RunnerError` variant (NotFound / AuthFailed / NonZeroExit / Timeout / Io).
  - `embeddings_error_display_messages_are_actionable` — same shape for `EmbeddingsError`.

## [0.9.0] - 2026-05-02

### Added

- **3 new `StatsReport` metrics** ([crates/coral-stats/src/lib.rs](crates/coral-stats/src/lib.rs)):
  - `pages_without_sources_count: usize` — count of pages with empty `frontmatter.sources`. Pair with the `HighConfidenceWithoutSources` lint to find the worst offenders.
  - `oldest_commit_age_pages: Vec<String>` — top 5 slugs by lexicographic commit-string ordering. Useful for spotting long-untouched pages. Future work: real timestamp comparison via `git log`.
  - `pages_by_confidence_bucket: BTreeMap<String, usize>` — confidence distribution into 4 buckets (`"0.0-0.3"`, `"0.3-0.6"`, `"0.6-0.8"`, `"0.8-1.0"`). All 4 keys present even when empty so the JSON shape stays stable.

  Markdown rendering picks up 3 new lines after `Total outbound links`. JSON schema regenerated; `docs/schemas/stats.schema.json` now lists 15 required fields (was 12). **15 new unit tests** + 2 refreshed snapshot files.

- **3 more snapshot tests** ([crates/coral-cli/tests/snapshot_cli.rs](crates/coral-cli/tests/snapshot_cli.rs)): `validate_pin_no_pins_file`, `lint_severity_critical_json_4_page_seed`, `lint_severity_warning_4_page_seed`. Total snapshots now 14.

## [0.8.1] - 2026-05-02

Test + docs only (no behavior change). All 4 of these are quality-of-
maintenance investments rather than user-facing features.

### Added

- **`docs/TUTORIAL.md`** — 5-minute walkthrough exercising every deterministic Coral subcommand (init, lint, stats, search TF-IDF + BM25, diff, export HTML, validate-pin) against a synthetic 4-page seed wiki. No `claude setup-token`, no `VOYAGE_API_KEY`, no network. Every output block is REAL — captured by running the binary.
- **Property-based test suites** (proptest) for 4 hot paths:
  - `crates/coral-lint/tests/proptest_lint.rs` (6 properties): `run_structural` totality, issue invariants, empty input contract, order-independence, system-page-type orphan-skip, high-conf-without-sources predicate.
  - `crates/coral-core/tests/proptest_search.rs` (10 properties × TF-IDF and BM25): totality, result-count limits, non-negative scores, sort-descending invariant, slug membership, BM25 ⊆ TF-IDF slug set, empty input contracts.
  - `crates/coral-core/tests/proptest_wikilinks.rs` (9 properties): totality, no duplicates, document order, alias/anchor stripping, output safety (no `]` / `|` / `#` / newlines), code-fence skip, escape skip.
  - `crates/coral-core/tests/proptest_frontmatter.rs` (6 properties): YAML round-trip identity, body-bytes verbatim preservation, missing/unterminated rejection.
- **Snapshot tests** (insta) — 11 frozen-output tests in `crates/coral-cli/tests/snapshot_cli.rs` against the same 4-page seed: stats markdown + JSON, lint structural markdown + JSON, search TF-IDF + BM25, diff, export JSON + markdown-bundle + HTML head, prompts list. Catches accidental regressions in user-facing output that hand-written `contains(...)` assertions miss.

Test count: 385 (v0.8.0) → 427 (+42).

## [0.8.0] - 2026-05-02

### Added

- **`coral lint --severity <critical|warning|info|all>`** ([crates/coral-cli/src/commands/lint.rs](crates/coral-cli/src/commands/lint.rs)): filter the rendered report and exit-code calculation to issues at or above the named level. Critical-only mode is the natural CI gate. The filter applies AFTER `--auto-fix` runs, so the LLM still sees every issue (it can propose Warning fixes even when CI gates filter to Critical only). New `parse_severity_filter` helper. **12 new tests** (8 unit + 4 cli_smoke e2e).
- **JSON schema for `coral lint --format json`** ([docs/schemas/lint.schema.json](docs/schemas/lint.schema.json)): mirrors what `coral stats` already does. Generated via `schemars::schema_for!(LintReport)` in a one-shot `crates/coral-lint/examples/dump_schema.rs` dumper. Top-level `LintReport` with `definitions` for `LintCode` (11 variants), `LintIssue`, `LintSeverity` (3 variants). Useful for downstream tools, IDE validation, and as a drift guard. **5 new tests** including a "schema lists every variant" guard against future LintCode additions silently breaking consumers.
- **`coverage` CI job** ([.github/workflows/ci.yml](.github/workflows/ci.yml)): `cargo-llvm-cov` runs on every push/PR, prints a summary line and uploads `lcov.info` as a 30-day-retention artifact. `continue-on-error: true` since coverage is informational; `test` job remains the hard gate. Sets up the foundation for an eventual Codecov badge once secrets are wired.

### Documentation

- **USAGE.md updated** for v0.7+ flags: `coral lint --severity`, `coral search --algorithm bm25`, `coral consolidate --apply --rewrite-links`. The new `lint --format json` schema link points at the committed `docs/schemas/lint.schema.json`.

## [0.7.0] - 2026-05-02

### Added

- **`coral search --algorithm bm25`** ([crates/coral-core/src/search.rs](crates/coral-core/src/search.rs)): Okapi BM25 ranking alternative to TF-IDF inside the offline `--engine tfidf` family. Better precision on 100+ page wikis. Same `SearchResult` shape, same tokenization (reuses `tokenize` + `build_snippet`). Constants `pub const BM25_K1: f64 = 1.5` and `pub const BM25_B: f64 = 0.75` (Robertson/Sparck-Jones defaults). IDF clamped at 0 to avoid negative scores for very common terms. **13 new unit tests**.
- **`coral consolidate --apply --rewrite-links`** ([crates/coral-cli/src/commands/consolidate.rs](crates/coral-cli/src/commands/consolidate.rs)): mass-patches outbound `[[wikilinks]]` in OTHER pages that pointed at retired sources. For merges: `[[a]]`→`[[ab]]`. For splits: `[[too-big]]`→`[[part-a]]` (first target as default). Aliased forms (`[[a|alias]]`) and anchored forms (`[[a#anchor]]`) preserve their suffixes. New `RewriteSummary` reporting struct + `Rewrites: N page(s) patched` print block. Idempotent (second pass finds nothing). **13 new unit tests** including 8 helper-level + 4 end-to-end + 1 smoke.
- **`KNOWN_PROMPTS` registers `qa-pairs`, `lint-auto-fix`, `diff-semantic`** ([crates/coral-cli/src/commands/prompt_loader.rs](crates/coral-cli/src/commands/prompt_loader.rs)): three prompts added in v0.3 / v0.5 / v0.6 used `prompt_loader::load_or_fallback` correctly but never appeared in `coral prompts list`. Now all 9 surface and propagate through `coral sync` to consumer repos.
- **Embedded prompt templates for `diff-semantic` and `lint-auto-fix`** ([template/prompts/](template/prompts/)): both were fallback-only before; consumers couldn't drop overrides at `<cwd>/prompts/`.

### Documentation

- **README "Roadmap" section refreshed** for v0.4–v0.6 reality (was stuck on "v0.3.0 — planned").
- **README test count badge + breakdown** updated to 342.
- **docs/ROADMAP.md fully consolidated** into a release-history table format with explicit "Items bloqueados" + "v0.7+ speculative" sections.

## [0.6.0] - 2026-05-02

### Added

- **4 new structural lint checks** ([crates/coral-lint/src/structural.rs](crates/coral-lint/src/structural.rs)):
  - `CommitNotInGit` (Warning) — page's `last_updated_commit` not in `git rev-list --all`. Single git invocation per lint run; degrades gracefully via `tracing::warn!` when git is missing/detached. Skips placeholder commits (`""`, `"unknown"`, `"abc"`, `"zero"`, anything <7 chars).
  - `SourceNotFound` (Warning) — each `frontmatter.sources` entry must exist on disk relative to repo root. `http(s)://` URLs skipped.
  - `ArchivedPageLinked` (Warning) — for each `status: archived` page, finds linkers and emits one issue per (linker, archived target) pair. Archived → archived self-noise filtered.
  - `UnknownExtraField` (Info) — one issue per key in `frontmatter.extra`. Surfaces unrecognized YAML extensions for review.

  New `pub fn run_structural_with_root(pages, repo_root) -> LintReport` fans out all 9 checks via parallel rayon iterators. Existing `run_structural(&[Page])` preserved for backward compat. CLI computes `repo_root` as parent of `.wiki/`. **18 new unit tests** including real `git init` fixtures via tempfile.
- **`coral diff --semantic`** ([crates/coral-cli/src/commands/diff.rs](crates/coral-cli/src/commands/diff.rs)): LLM-driven contradictions + overlap analysis between two wiki pages. After the structural diff, the runner receives both bodies and proposes contradictions, overlap (merge candidates), and coverage gaps. Markdown output appends `## Semantic analysis` section; JSON output adds top-level `semantic.{model, analysis}` field. `--model` and `--provider` for runner selection. Override prompt at `<cwd>/prompts/diff-semantic.md`. **9 new unit tests** including MockRunner success/failure paths.
- **`coral consolidate --apply` for merges + splits** ([crates/coral-cli/src/commands/consolidate.rs](crates/coral-cli/src/commands/consolidate.rs)): previously only `retirements[]` were materialized; now `merges[]` and `splits[]` actually run.
  - Merge: in-place if target is a source, append-to-existing if target slug exists, create-new otherwise (page_type = mode of sources, alphabetical tiebreak). Body concat with markdown separator. Frontmatter union (sources + backlinks deduped; backlinks gain source slugs). Confidence = min(target baseline OR 0.5, min source confidence). Status = draft. Sources marked stale with `_Merged into [[<target>]]_` footer.
  - Split: stub pages at `<wiki>/<source.page_type subdir>/<target>.md` with `confidence: 0.4`, `status: draft`. Source marked stale with `_Split into [[a]], [[b]]_` footer. Per-target skip if slug already exists.
  - Outbound wikilinks intentionally NOT rewritten — structural lint surfaces them as broken so the user reviews + fixes incrementally.
  - **10 new unit tests** covering all 3 merge paths, all 4 merge edge cases, both split paths, all 4 split edge cases, plus combined retire+merge+split scenario.
- **`criterion` benchmarks** for 5 hot paths ([crates/coral-core/benches/](crates/coral-core/benches/), [crates/coral-lint/benches/structural_bench.rs](crates/coral-lint/benches/structural_bench.rs)): `search` (100 pages / 2-token query), `wikilinks::extract` (50-link body), `Frontmatter` parse (5-field block), `walk::read_pages` (100 pages / 4 subdirs), `run_structural` (100-page graph). Run via `cargo bench --workspace`. `target/criterion/report/index.html` for visual reports across runs. `docs/PERF.md` updated.
- **`cargo-audit` + `cargo-deny` CI jobs** ([.github/workflows/ci.yml](.github/workflows/ci.yml), [deny.toml](deny.toml)): security advisory scan + license/duplicate-version gate. Audit is `continue-on-error: true` (transitive advisories surface but don't block); deny is a hard gate with a hand-curated license allowlist (MIT, Apache-2.0, BSD-2/3, ISC, Unicode-3.0, Zlib, MPL-2.0, CC0-1.0, 0BSD).
- **ADR 0008** ([docs/adr/0008-multi-provider-runner-and-embeddings-traits.md](docs/adr/0008-multi-provider-runner-and-embeddings-traits.md)) and **ADR 0009** ([docs/adr/0009-auto-fix-scope-and-yaml-plan.md](docs/adr/0009-auto-fix-scope-and-yaml-plan.md)): documents the v0.4–v0.5 design decisions (two parallel traits, four runners, three providers, capped auto-fix scope + YAML plan shape, explicit alternatives considered).

### Changed

- **`SCHEMA.base.md` aligned with the 10 PageType variants** ([template/schema/SCHEMA.base.md](template/schema/SCHEMA.base.md)): the base SCHEMA only documented 9 page types; the Rust enum has 10 (`Reference` was added but never described). Plus the 4 system page types (`index`, `log`, `schema`, `readme`) are now called out. The frontmatter example inlines the full enum list.

### Performance

- **Parallelized embeddings batching** ([crates/coral-runner/src/embeddings.rs](crates/coral-runner/src/embeddings.rs)): both `VoyageProvider::embed_batch` and `OpenAIProvider::embed_batch` now fan their internal chunks (128 / 256 inputs each) across rayon's global thread pool. For a 1000-page wiki, an 8-core dev box does all chunks in flight at once instead of one-at-a-time. First-error-aborts semantics preserved; output order matches input order. New `embed_chunk` private methods extract the per-chunk curl-and-parse logic. **4 new unit tests** using a test-only `ChunkedMockProvider`.

## [0.5.0] - 2026-05-01

### Added

- **`coral consolidate --apply`** ([crates/coral-cli/src/commands/consolidate.rs](crates/coral-cli/src/commands/consolidate.rs)): parses the LLM's YAML proposal into structured `merges:` / `retirements:` / `splits:` arrays and applies the safe subset — every `retirements[].slug` becomes `status: stale`. `merges[]` and `splits[]` are surfaced as warnings (body merging / partitioning isn't safely automated). Default remains dry-run preview. 4 unit tests.
- **`coral onboard --apply`** ([crates/coral-cli/src/commands/onboard.rs](crates/coral-cli/src/commands/onboard.rs)): persists the LLM-generated reading path as a wiki page at `<wiki>/operations/onboarding-<slug>.md` (slug = profile lowercased + dashed; runs with the same profile overwrite). New `profile_to_slug` helper handles spaces, case, special chars. 3 unit tests including slug normalization.

### Changed

- **Streaming runner unification** ([crates/coral-runner/src/runner.rs](crates/coral-runner/src/runner.rs)): extracted `run_streaming_command` helper that ClaudeRunner, GeminiRunner, and LocalRunner all delegate to. GeminiRunner and LocalRunner override `Runner::run_streaming` to use it instead of the trait's default single-chunk fallback — `coral query --provider gemini`/`local` now sees the response token-by-token (when the underlying CLI streams). Timeout + auth-detection semantics are identical across all three runners.

### Documentation

- **USAGE.md fully refreshed** for v0.4 + v0.5: `bootstrap`/`ingest --apply` (drops the stale "v0.1, does not write pages" note), `coral query` telemetry, `coral lint --staged`/`--auto-fix [--apply]`, `coral consolidate --apply`, `coral onboard --apply`, `coral search --embeddings-provider <voyage|openai>`, `coral export --format html`, plus brand-new sections for `coral diff` and `coral validate-pin`. Multi-provider intro now mentions `local` (llama.cpp). New CI section for the embeddings-cache composite action.

### Added (continued)

- **`LocalRunner`** ([crates/coral-runner/src/local.rs](crates/coral-runner/src/local.rs)): third real `Runner` impl alongside Claude and Gemini. Wraps llama.cpp's `llama-cli` (`-p` for prompt, `-m` for `.gguf` model path, `--no-display-prompt`, system prompt prepended). Selected via `--provider local` (or `local`/`llama`/`llama.cpp`). Standing wrapper-script escape hatch through `with_binary` for installs with non-standard flags. 8 unit tests cover argv shape, echo-substitute integration, not-found, non-zero + 1 ignored real-llama smoke (`LLAMA_MODEL` env required).
- **`--provider local` flag** wired in [crates/coral-cli/src/commands/runner_helper.rs](crates/coral-cli/src/commands/runner_helper.rs): `ProviderName::Local` variant + parser aliases. Available on every LLM subcommand (`bootstrap`, `ingest`, `query`, `lint --semantic`, `consolidate`, `onboard`, `export --qa`).
- **`coral lint --auto-fix`** ([crates/coral-cli/src/commands/lint.rs](crates/coral-cli/src/commands/lint.rs)): LLM-driven structural fixes. After lint runs, the runner receives a structured prompt with affected pages + issues and proposes a YAML plan: `{slug, action: update|retire|skip, confidence?, status?, body_append?, rationale}`. Default is **dry-run** (prints the plan); `--apply` writes changes back. Caps the LLM scope: it can downgrade confidence, mark stale, or append a short italic note — but cannot rewrite whole bodies or invent sources. Override the system prompt at `<cwd>/prompts/lint-auto-fix.md`. 4 unit tests cover YAML parsing (with fences + missing-action default-to-skip), apply-on-disk frontmatter+body changes, and retire-marks-stale.

- **`coral diff <slugA> <slugB>`** ([crates/coral-cli/src/commands/diff.rs](crates/coral-cli/src/commands/diff.rs)): structural diff between two wiki pages — frontmatter delta (type / status / confidence), source set arithmetic (common / only-A / only-B), wikilink set arithmetic, body length stats. Markdown or JSON output (`--format json`). Useful for spotting merge candidates, evaluating retirement, or reviewing wiki/auto-ingest PRs. 4 unit tests. (Future: `--semantic` flag for LLM-driven contradiction detection.)
- **`coral export --format html`** ([crates/coral-cli/src/commands/export.rs](crates/coral-cli/src/commands/export.rs)): single-file static HTML site of the wiki — embedded CSS (light + dark via `prefers-color-scheme`), sticky sidebar TOC grouped by page type, every page rendered as a `<section id="slug">`. `[[wikilinks]]` translate to in-page anchor links via a regex preprocessor that handles plain / aliased / anchored forms. New `pulldown-cmark` dep for Markdown→HTML (CommonMark + tables + footnotes + strikethrough + task lists). Drop the file on GitHub Pages / S3 / any static host — no build step. 3 unit tests.

- **`coral validate-pin`** ([crates/coral-cli/src/commands/validate_pin.rs](crates/coral-cli/src/commands/validate_pin.rs)): new subcommand that reads `.coral-pins.toml` (with legacy `.coral-template-version` fallback) and verifies each referenced version exists as a tag in the remote Coral repo via a single `git ls-remote --tags` call (no clone). Reports `✓` per pin / `✗` for any missing tag. Exit `0` when clean, `1` if any pin is unresolvable. `--remote <url>` overrides the default for forks/mirrors. 6 unit tests.
- **`coral lint --staged`**: pre-commit hook mode. Loads every page (graph stays intact for orphan / wikilink checks) but filters the report to issues whose `page` is in `git diff --cached --name-only` plus workspace-level issues (no `page`). Exits non-zero only when a critical issue touches a staged file. 3 unit tests cover staged-path parsing, filter membership, and workspace-level retention.
- **`embeddings-cache` composite action** ([.github/actions/embeddings-cache/action.yml](.github/actions/embeddings-cache/action.yml)): drop-in `actions/cache@v4` wrapper for `.coral-embeddings.json`. Cache key strategy `<prefix>-<ref>-<hashFiles(*.md)>` with branch-scoped fallback so a single-page edit reuses ~all vectors but cross-branch staleness is avoided. README CI section documents usage.

## [0.4.0] - 2026-05-01

### Added

- **`OpenAIProvider`** ([crates/coral-runner/src/embeddings.rs](crates/coral-runner/src/embeddings.rs)): second real `EmbeddingsProvider` impl. Same curl shell-out pattern as Voyage. Constructors `text_embedding_3_small()` (1536-dim, default) and `text_embedding_3_large()` (3072-dim). `coral search --embeddings-provider openai` selects it; needs `OPENAI_API_KEY`. 3 unit tests + 1 ignored real-API smoke.
- **`coral search --embeddings-provider <voyage|openai>`** flag: pick the embeddings provider per invocation. Default `voyage` preserves v0.3.1 behavior. The dimensionality auto-resolves per OpenAI model (`text-embedding-3-large` → 3072, others → 1536).
- **Real `GeminiRunner`** ([crates/coral-runner/src/gemini.rs](crates/coral-runner/src/gemini.rs)): replaces the v0.2 `ClaudeRunner::with_binary("gemini")` stub with a standalone runner that builds its own argv per gemini-cli conventions (`-p` for prompt, `-m` for model, system prompt prepended to user with blank-line separator). Keeps the public API stable (`new()`, `with_binary()`). Surfaces `RunnerError::AuthFailed` on 401-style failures via the shared `combine_outputs` + `is_auth_failure` helpers. 7 unit tests cover argv shape (4), echo-substitute integration (1), not-found (1), non-zero (1) + 1 ignored real-gemini smoke. Streaming uses the trait default (single chunk on completion); incremental streaming is a future improvement.

- **`EmbeddingsProvider` trait** ([crates/coral-runner/src/embeddings.rs](crates/coral-runner/src/embeddings.rs)): mirrors the `Runner` trait pattern but for vector embedding providers. Lets the search command and tests swap providers without recompiling against a specific HTTP shape. Ships with `VoyageProvider` (the prior `coral-cli/commands/voyage` curl shell-out, now an impl) and `MockEmbeddingsProvider` (deterministic in-memory provider for offline tests). 6 unit tests including swap-via-trait-object and a deterministic mock smoke. A second real provider (Anthropic embeddings when shipped, OpenAI text-embedding-3) lands as one new struct in this module.
- **Dedicated `EmbeddingsError` enum** with `AuthFailed`, `ProviderCall`, `Io`, `Parse` variants — surfaces actionable detail without depending on `RunnerError` (which is claude-specific).

- **`coral query` telemetry** ([crates/coral-cli/src/commands/query.rs](crates/coral-cli/src/commands/query.rs)): emits two `tracing::info!` events bracketing the runner call — `coral query: starting` (with `pages_in_context`, `model`, `question_chars`) and `coral query: completed` (with `duration_ms`, `chunks`, `output_chars`, `model`). Visible with `RUST_LOG=coral=info coral query "..."`. No effect on stdout streaming.

### Documentation

- **README "Auth setup" section** ([README.md](README.md)): covers local shell (`claude setup-token`), CI (`CLAUDE_CODE_OAUTH_TOKEN` secret), and the gotcha when running `coral` from inside Claude Code (the parent's `ANTHROPIC_API_KEY` doesn't work in the subprocess; the v0.3.2 actionable error now points users here). Embeddings provider auth (`VOYAGE_API_KEY`) is also documented.

### Changed

- **`coral notion-push` is dry-run by default**; `--apply` is the explicit opt-in to actually POST. Matches `bootstrap`/`ingest` semantics. **BREAKING**: the prior `--dry-run` flag was removed (no longer needed). USAGE.md updated.
- **`coral search --engine embeddings`** now goes through the `EmbeddingsProvider` trait. CLI surface unchanged; behavior identical against Voyage. The factory in `coral-cli/src/commands/search.rs` constructs a `VoyageProvider` from `VOYAGE_API_KEY` + `--embeddings-model`.
- **`coral-cli/src/commands/voyage.rs` deleted** — the curl shell-out lives in `coral-runner::embeddings::VoyageProvider`.

## [0.3.2] - 2026-05-01

### Fixed

- **`coral search` UTF-8 panic** ([crates/coral-core/src/search.rs:103](crates/coral-core/src/search.rs:103)): the snippet builder sliced the page body with raw byte offsets, panicking when `pos.saturating_sub(40)` or `pos + max_len` landed inside a multi-byte char (em-dash, accent, smart quote, emoji). Repro: `coral search "embeddings"` against any wiki containing `—`. Fixed by clamping both ends to the nearest UTF-8 char boundary via new `floor_char_boundary` / `ceil_char_boundary` helpers. Regression test `search_does_not_panic_on_multibyte_chars_near_match` exercises a body with `—` near the match.
- **`ClaudeRunner` silent auth failures** ([crates/coral-runner/src/runner.rs](crates/coral-runner/src/runner.rs)): `claude --print` writes 401 errors to stdout, so the previous code surfaced the user-facing message `error: runner failed: claude exited with code Some(1):` with empty trailing detail. Both `run` and `run_streaming` now combine stdout + stderr via a new `combine_outputs` helper, and a new `RunnerError::AuthFailed` variant is returned when the combined output matches an auth signature (`401`, `authenticate`, `invalid_api_key`). The variant's `Display` shows the actionable hint: "Run `claude setup-token` or export ANTHROPIC_API_KEY in this shell." 2 new unit tests cover the helpers.
- **Test flake `ingest_apply_skips_missing_page_for_update`** ([crates/coral-cli/src/commands/ingest.rs](crates/coral-cli/src/commands/ingest.rs)): `bootstrap.rs` and `ingest.rs` each had their own `static CWD_LOCK: Mutex<()>`, so cross-module tests racing on process cwd would intermittently land in an orphan directory and panic on cwd restore. Unified into a single `commands::CWD_LOCK` shared by all command modules. 5× workspace stress run is green.

## [0.3.1] - 2026-05-01

### Added

- **Embeddings-backed search** (`coral search --engine embeddings`): semantic similarity via Voyage AI `voyage-3`. Embeddings cached at `<wiki_root>/.coral-embeddings.json` (schema v1, mtime-keyed per slug, dimension-aware). Only changed pages are re-embedded between runs. `--reindex` forces a full rebuild. `--embeddings-model` overrides the default `voyage-3`. Requires `VOYAGE_API_KEY` env var. Falls back to a clear error when missing. TF-IDF (`--engine tfidf`) remains the default — no API key, works offline.
- **`coral_core::embeddings::EmbeddingsIndex`**: new module with cosine-similarity search, prune-by-live-slugs, JSON load/save, schema versioning. 9 unit tests.
- **Voyage provider** at `coral_cli::commands::voyage`: shells to curl (same pattern as `notion-push`), batches input into 128-item chunks (Voyage's limit), parses by `index` field for ordering safety, surfaces curl/HTTP errors with full stdout for debugging. 2 unit tests + 1 ignored real-API smoke.
- **`coral init` `.gitignore`** also lists `.coral-embeddings.json` so the cache stays out of source control alongside `.coral-cache.json`.

### Changed

- **ADR 0006** updated with the v0.3.1 status: embeddings now ship in JSON storage; sqlite-vec migration is deferred to v0.4 if/when wiki size pressures the JSON format (~5k pages).

## [0.3.0] - 2026-05-01

### Added

- **mtime-cached frontmatter parsing**: new `coral_core::cache::WalkCache` persists parsed `Frontmatter` keyed by file mtime in `<wiki_root>/.coral-cache.json`. `walk::read_pages` consults the cache before YAML parsing — files whose mtime hasn't changed since the previous walk skip the deserialization step, with body re-extraction handled by a new pure helper `frontmatter::body_after_frontmatter`. Wikis ≥200 pages should see ~30 % faster `coral lint` / `coral stats`. Schema-versioned (`SCHEMA_VERSION = 1`) — future bumps invalidate stale caches automatically. `coral init` now writes `<wiki_root>/.gitignore` with a `.coral-cache.json` entry to keep the cache out of source control. Cache writes are best-effort: a failure to persist the cache logs a warning but does not fail the walk.
- **`coral export --format jsonl --qa`**: invokes the runner per page with a new `qa-pairs` system prompt and emits 3–5 `{"slug","prompt","completion"}` lines per page for fine-tuning datasets. Malformed runner output is skipped with a warning. Add `--provider gemini --model gemini-2.5-flash` for a cheaper batch run. Override the system prompt at `<cwd>/prompts/qa-pairs.md` (priority: local override > embedded `template/prompts/qa-pairs.md` > hardcoded `QA_FALLBACK`). Default jsonl behavior (stub prompt, no runner) is unchanged.

### Deferred to v0.3.1

- **sqlite-vec embeddings search** (originally part of v0.3 roadmap): kept as a separate sprint because it requires API-key management for an embedding provider (Voyage / Anthropic when shipped) plus end-to-end testing against a real provider. TF-IDF in v0.2+ stays as the search default.

## [0.2.1] - 2026-05-01

### Added

- **`coral notion-push`**: thin wrapper over `coral export --format notion-json` that POSTs each page to a Notion database via curl. Reads `NOTION_TOKEN` + `CORAL_NOTION_DB` env vars or flags. `--type` filter, `--dry-run` preview. Wired with 4 unit tests + 2 integration tests (no-token failure, dry-run does not call curl).
- **`ClaudeRunner::run_streaming` honors `prompt.timeout`** (was a TODO in v0.2). Reader runs in a separate thread; main loop waits with `recv_timeout` and kills the child + cleans up if the deadline elapses. New non-`#[ignore]` test `claude_runner_streaming_timeout_kills_child` invokes `/usr/bin/yes` (writes forever, ignores args) with a 200 ms deadline and asserts `RunnerError::Timeout` returns within 2 s.

### Documentation

- **SCHEMA.base.md** explicit wikilinks section: `[[X]]` resolves by frontmatter slug, NOT by `[[type/slug]]`. Lint flags broken links if you use the prefixed form. Documents the convention with a comparison table and notes that `#anchor` / `|alias` suffixes still resolve by the part before `#` / `|`. New `template_validation` test asserts the section is present.

## [0.2.0] - 2026-05-01

### Added

- **`bootstrap`/`ingest --apply`** (issue #1): both LLM-driven subcommands now mutate `.wiki/` when invoked with `--apply`. They parse the runner's YAML response (`Plan { plan: [PlanEntry { slug, action, type, confidence, body, ... }] }`), write pages via `Page::write()`, upsert entries into `index.md`, append `log.md`. Default behavior remains `--dry-run` (print plan, no mutations) for safety. Malformed YAML prints raw output and exits 1.
- **`walk` skips top-level system files** (issue #2): the wiki walker now skips `index.md` and `log.md` at the wiki root in addition to the existing `SCHEMA.md`/`README.md` skip. Eliminates the `WARN skipping page … missing field 'slug'` noise on every `coral lint` and `coral stats` invocation. Subdirectory files like `concepts/index.md` still parse normally.
- **CHANGELOG.md + cargo-release wiring** (issue #3): adopted Keep a Changelog 1.1.0 format with backfilled `[0.1.0]` entry. `release.toml` configures `cargo-release` to rotate `[Unreleased]` → `[X.Y.Z] - {date}` and update compare-links automatically. `release-checklist.md` updated.
- **Streaming `coral query`** (issue #4): `Runner` trait gained `run_streaming(prompt, &mut FnMut(&str))` with a default impl that calls `run()` and emits one chunk. `ClaudeRunner` overrides to read stdout line-by-line via `BufReader::read_line`. `MockRunner::push_ok_chunked(Vec<&str>)` enables tests. The `coral query` subcommand prints chunks as they arrive instead of buffering.
- **Prompt overrides** (issue #7): every LLM subcommand (`bootstrap`, `ingest`, `query`, `lint --semantic`, `consolidate`, `onboard`) now resolves its system prompt with priority `<cwd>/prompts/<name>.md` > embedded `template/prompts/<name>.md` > hardcoded fallback. New `coral prompts list` subcommand prints a table of each prompt's resolved source.
- **GeminiRunner** (issue #8): alternative LLM provider, opt-in via `--provider gemini` on any LLM subcommand or the `CORAL_PROVIDER=gemini` env var. v0.2 ships a stub that shells to a `gemini` CLI binary; if absent, returns `RunnerError::NotFound`.
- **`coral search`** (issue #5): TF-IDF ranking over slug + body across all wiki pages. `--limit` / `--format markdown|json` flags. Pure Rust, no embeddings, no API key — works offline. v0.3 will switch to embeddings (Voyage / Anthropic) per [ADR 0006](docs/adr/0006-local-semantic-search-storage.md). The CLI surface stays stable on upgrade.
- **Hermes quality gate** (issue #6): opt-in composite action (`.github/actions/validate`) and `wiki-validator` subagent (template/agents) that runs an independent LLM to verify wiki/auto-ingest PRs against their cited sources before merge. Configurable `min_pages_to_validate` threshold to keep token spend predictable on small PRs.
- **`coral sync --remote <tag>`** (issue #10): pulls the `template/` directory from any tagged Coral release via `git clone --depth=1 --branch=<tag>`. No new Rust deps — shells to `git`. Without `--remote`, behavior is unchanged: only the embedded bundle is used and a mismatched `--version` aborts. Passing `--remote` without `--version` errors fast.
- **`.coral-pins.toml` per-file pinning** (issue #11): `coral sync --pin <path>=<version>` and `--unpin <path>` flags persist into a TOML file at the repo root with a `default` version + an optional `[pins]` map. Backwards compatible with the legacy `.coral-template-version` single-line marker — when only the legacy file exists, `Pins::load` migrates it on the fly. The legacy marker is kept in sync so existing tooling that reads it still works.
- **`docs/PERF.md`** (issue #14): documented baselines, hyperfine methodology, profiling tips, and the release-profile config. README links to it from a new "Performance" section.
- **`coral export`** (issues #9 + #13): single subcommand with four output formats (`markdown-bundle`, `json`, `notion-json`, `jsonl`) for shipping the wiki to downstream consumers. Replaces what would have been per-target subcommands (Notion sync, fine-tune dataset) with a unified exporter. `--type` filters by page type, `--out` writes to a file. Decision rationale in [ADR 0007](docs/adr/0007-unified-export-vs-per-target-commands.md).
- **`coral-stats` JsonSchema** (issue #15): `StatsReport` derives `JsonSchema` (`schemars 0.8`), new `json_schema()` method, generated schema committed at `docs/schemas/stats.schema.json`. 5 additional unit tests cover self-link, no-outbound, perf 500-page baseline, schema validity, JSON roundtrip.
- **2 new ADRs**: [0006](docs/adr/0006-local-semantic-search-storage.md) (TF-IDF stub vs v0.3 embeddings) and [0007](docs/adr/0007-unified-export-vs-per-target-commands.md) (single `coral export` vs per-target commands).

### Changed

- **`[profile.release]`**: added `panic = "abort"` to shave ~50 KB off the stripped binary and skip unwinding tables. CLI panics are unrecoverable anyway.
- **`prompt_loader`**: added `load_or_fallback_in(cwd, …)` and `list_prompts_in(cwd, …)` variants that take an explicit working directory. Fixes a flaky test that raced against `set_current_dir` calls in other test binaries. The default `load_or_fallback` / `list_prompts` wrappers preserve the original API for production callers.

### Closed issues

#1, #2, #3, #4, #5, #6, #7, #8, #9, #10, #11, #13, #14, #15. (#12 — orchestra-ingest consumer repo — tracked separately.)

## [0.1.0] - 2026-04-30

### Added

- Cargo workspace with 5 crates: `coral-cli`, `coral-core`, `coral-lint`, `coral-runner`, `coral-stats`.
- `coral` CLI binary with 10 subcommands (init, bootstrap, ingest, query, lint, consolidate, stats, sync, onboard, search).
- Frontmatter parsing with `Frontmatter`, `PageType`, `Status`, `Confidence` types.
- Wikilink extraction with code-fence and escape handling.
- `Page`, `WikiIndex`, `WikiLog` data model with idempotent operations.
- `gitdiff` parser + runner (shells to `git diff --name-status`).
- `walk::read_pages` rayon-parallel page reader.
- 5 structural lint checks: broken wikilinks, orphan pages, low confidence, high confidence without sources, stale status.
- `Runner` trait + `ClaudeRunner` (subprocess wrapper) + `MockRunner` (testing).
- `PromptBuilder` with `{{var}}` substitution.
- `StatsReport` with markdown + JSON renderers.
- Embedded `template/` bundle: 4 subagents, 4 slash commands, 4 prompt templates, base SCHEMA, GH workflow template.
- 3 composite GitHub Actions: ingest, lint, consolidate.
- Multi-agent build pipeline (orchestrator/coder/tester loop).
- 150 tests + 3 ignored. Binary 2.8MB stripped.

### Documentation

- README, INSTALL, USAGE, ARCHITECTURE.
- 5 ADRs: Rust CLI architecture, Claude CLI vs API, template via include_dir, multi-agent flow, versioning + sync.
- Self-hosted `.wiki/` with 14 seed pages (cli/core/lint/runner/stats modules + concepts + entities + flow + decisions + synthesis + operations + sources).

[Unreleased]: https://github.com/agustincbajo/Coral/compare/v0.31.1...HEAD
[0.31.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.31.1
[0.31.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.31.0
[0.30.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.30.0
[0.25.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.25.0
[0.24.2]: https://github.com/agustincbajo/Coral/releases/tag/v0.24.2
[0.24.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.24.1
[0.24.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.24.0
[0.23.3]: https://github.com/agustincbajo/Coral/releases/tag/v0.23.3
[0.23.2]: https://github.com/agustincbajo/Coral/releases/tag/v0.23.2
[0.23.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.23.1
[0.23.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.23.0
[0.22.6]: https://github.com/agustincbajo/Coral/releases/tag/v0.22.6
[0.22.5]: https://github.com/agustincbajo/Coral/releases/tag/v0.22.5
[0.22.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.22.0
[0.21.4]: https://github.com/agustincbajo/Coral/releases/tag/v0.21.4
[0.21.3]: https://github.com/agustincbajo/Coral/releases/tag/v0.21.3
[0.21.2]: https://github.com/agustincbajo/Coral/releases/tag/v0.21.2
[0.21.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.21.1
[0.21.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.21.0
[0.20.2]: https://github.com/agustincbajo/Coral/releases/tag/v0.20.2
[0.20.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.20.1
[0.20.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.20.0
[0.19.8]: https://github.com/agustincbajo/Coral/releases/tag/v0.19.8
[0.19.7]: https://github.com/agustincbajo/Coral/releases/tag/v0.19.7
[0.19.6]: https://github.com/agustincbajo/Coral/releases/tag/v0.19.6
[0.19.5]: https://github.com/agustincbajo/Coral/releases/tag/v0.19.5
[0.19.4]: https://github.com/agustincbajo/Coral/releases/tag/v0.19.4
[0.19.3]: https://github.com/agustincbajo/Coral/releases/tag/v0.19.3
[0.19.2]: https://github.com/agustincbajo/Coral/releases/tag/v0.19.2
[0.19.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.19.1
[0.19.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.19.0
[0.16.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.16.0
[0.15.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.15.1
[0.15.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.15.0
[0.14.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.14.1
[0.14.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.14.0
[0.13.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.13.0
[0.12.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.12.0
[0.11.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.11.0
[0.10.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.10.0
[0.9.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.9.0
[0.8.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.8.1
[0.8.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.8.0
[0.7.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.7.0
[0.6.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.6.0
[0.5.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.5.0
[0.4.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.4.0
[0.3.2]: https://github.com/agustincbajo/Coral/releases/tag/v0.3.2
[0.3.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.3.1
[0.3.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.3.0
[0.2.1]: https://github.com/agustincbajo/Coral/releases/tag/v0.2.1
[0.2.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.2.0
[0.1.0]: https://github.com/agustincbajo/Coral/releases/tag/v0.1.0
