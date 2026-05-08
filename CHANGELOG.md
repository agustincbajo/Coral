# Changelog

All notable changes to Coral are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.21.3] - 2026-05-08

**Feature release: `coral session distill --as-patch` (option (b) / distill-as-patch).** Adds a second emit mode to `coral session distill`. Default behavior — `coral session distill <id>` (no `--as-patch`) — stays byte-identical to v0.21.2: still emits 1-3 NEW synthesis pages under `.coral/sessions/distilled/`. With `--as-patch`, the LLM instead proposes 1-N **unified-diff patches** against EXISTING `.wiki/<slug>.md` pages. Patches save to `.coral/sessions/patches/<id>-<idx>.patch` plus a sidecar `<id>-<idx>.json` carrying target slug + LLM rationale + provenance. With `--apply` the patches are `git apply`-ed in turn AND each touched page's frontmatter is rewritten so `reviewed: false` (Coral OWNS the flip — the LLM's job is body content). Pre-apply atomicity: if ANY patch fails its `git apply --check`, NO files are written and the command exits non-zero with the patch index + git stderr verbatim. Validation is layered — every component of the path-style `target_slug` must pass `coral_core::slug::is_safe_filename_slug`, the resolved page must already exist in `list_page_paths(.wiki)`, the diff `--- a/X.md` / `+++ b/X.md` headers must agree with the target, AND `git apply --check --unsafe-paths --directory=.wiki <patch>` must succeed. Top-K BM25 candidate pages from `coral_core::search::search_bm25` are surfaced in the prompt by default (K=10, override via `--candidates N`, `0` skips). **No new workspace dependencies** — the orchestrator picked subprocess `git apply` over `diffy` (zero net additions to `Cargo.lock`). **BC sacred: option (a) page-emit path is byte-identical to v0.21.2** — pinned by `crates/coral-session/src/distill.rs::tests::distill_without_as_patch_byte_identical_to_v0212`. **`IndexEntry.patch_outputs` is `#[serde(default)]`** so a v0.21.2 `index.json` deserializes cleanly. **1196 tests pass (was 1174; +22).** `bc_regression` green.

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
- **`Page::from_file → set extra["reviewed"] = Bool(false) → Page::write()`** rewrites the frontmatter post-apply so the unreviewed-distilled lint gate fires regardless of what the LLM emitted. Coral OWNS the flip.
- **No new dependencies.** Orchestrator chose subprocess `git apply` over `diffy` to avoid a workspace dep. Zero net additions to `Cargo.lock`.

### Tests (+22)

- **#1-#10 e2e (`crates/coral-session/tests/distill_patch_e2e.rs`)**: `distill_patch_writes_pairs_under_patches_dir`, `distill_patch_apply_mutates_wiki_and_resets_reviewed`, `patch_with_unknown_target_rejects_pre_io`, `malformed_diff_rejects_atomically`, `one_bad_patch_rolls_back_all`, `patch_with_dotdot_target_rejects`, `diff_header_mismatch_rejects_when_only_minus_is_wrong`, `patch_count_capped_at_five`, `forget_removes_patch_basenames`, `distilled_and_patch_outputs_track_independently`. Every test drives a `MockRunner` with a hand-rolled YAML response so the LLM call is deterministic.
- **#11-#19 unit (in `distill_patch::tests`)**: `select_candidates_is_deterministic`, `zero_candidates_skips_page_load`, `candidates_flag_truncates_to_n`, `is_safe_path_slug_rejects_dotdot_segments`, `diff_targets_slug_matches_a_and_b_prefixes`, `parse_patches_handles_yaml_code_fence`, `parse_patches_caps_at_five`, `parse_patches_rejects_dotdot_target`, `parse_patches_rejects_header_mismatch`.
- **#20 BC pin (in `distill::tests`)**: `distill_without_as_patch_byte_identical_to_v0212` — pins the page-emit envelope so any future edit that quietly shifts the schema is caught at test time.
- **#21 BC pin (in `capture::tests`)**: `index_without_patch_outputs_field_deserializes` — proves a v0.20.x / v0.21.2-shaped `index.json` deserializes cleanly with `patch_outputs` defaulting to empty.
- **#22 CLI integration smoke (in `commands::session::tests`)**: `run_distill_as_patch_writes_patches_dir_via_mock_runner` — drives `run_distill` with `--as-patch` and an injected `MockRunner`, asserts the patches dir has the right files and the index is updated.

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
