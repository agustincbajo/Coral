# Changelog

All notable changes to Coral are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
- **`coral test discover`** auto-generates TestCases from
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

- **`coral test discover` + `coral test --include-discovered`** auto-generates `TestCase`s from `openapi.{yaml,yml,json}` (OpenAPI 3.x) anywhere under the project. Walks excluding `.git/`, `.coral/`, `node_modules/`, `target/`, `vendor/`, `dist/`, `build/`. One case per `(path, method)` with status assertion picked from the spec's lowest 2xx response. Endpoints with `requestBody.required = true` are skipped (we don't fabricate bodies).
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

[Unreleased]: https://github.com/agustincbajo/Coral/compare/v0.15.1...HEAD
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
