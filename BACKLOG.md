# Coral Backlog

Items deferred from the v0.32.x → v0.33.0 WebUI sprint that did not reach
production. None block users today; all are polish, infrastructure, or
GTM follow-ups. Listed by category, not priority — the maintainer
picks the order.

Last updated: 2026-05-12, **post v0.34.1 patch**. The M1 onboarding
stack shipped as v0.34.0 (weeks 1–6 + tag + post-release smoke matrix
verde across Linux/macOS/Windows) plus a same-day patch v0.34.1
closing three post-release tech-debt items (GITHUB_TOKEN auth in
self-upgrade, Windows hook latency rewrite, Ollama config bridge).
None of the 10 backlog entries below was within scope of M1; they
remain open against v0.35.0+ unless re-prioritised. M1's own
follow-up list lives in the PRD §15 timeline rather than here; the
3 items that closed in the v0.34.1 patch are documented at the
bottom of this file under "v0.34.0 sprint status (shipped)".

---

## 🟡 Polish — visible to users

### 1. Screenshots for the 5 M2/M3 views

`docs/UI.md` and `README.md` currently embed captures for the four M1
views only (Pages, Graph, Query, Manifest). The Interfaces, Drift,
Affected, Tools, and Guarantee views from v0.33.0 are documented in
prose but have no images.

Acceptance: each of the 5 new views has at least one PNG under
`docs/assets/` referenced from `docs/UI.md` and (optionally) the README
WebUI section.

How: same recipe as the M1 captures —

```bash
./target/release/coral.exe ui serve --no-open --port 38400 &
DOCS=$(cygpath -w docs/assets)
CHROME='/c/Program Files/Google/Chrome/Application/chrome.exe'
for view in interfaces drift affected tools guarantee; do
  "$CHROME" --headless=new --enable-webgl --use-gl=swiftshader \
    --window-size=1400,900 --hide-scrollbars --virtual-time-budget=10000 \
    --lang=en-US --screenshot="$DOCS\\ui-${view}-en.png" \
    "http://127.0.0.1:38400/${view}"
done
```

Cost: ~15 min including auth-gated tools (needs `--token` to populate
the Tools view non-empty).

---

### 2. GTM communication for v0.33.0

The v0.33.0 release was published with full Sigstore provenance but
there's been zero communication: no tweet, no blog post, no message in
the Claude Code marketplace, no Discord/HN.

Decision blocker: tone is the maintainer's call. Once decided, drafts
of the technical content are largely ready in `CHANGELOG.md` and the
PRD §17 post-mortem.

Suggested channels:
- Twitter / Mastodon thread highlighting the bi-temporal slider as the
  unique-vs-LightRAG feature.
- README badge update with new install line (already at v0.32.0 in
  the install snippets; bump to v0.33.0 for users copy-pasting).
- `/plugin marketplace` push for Claude Code (`.claude-plugin/plugin.json`
  is at v0.32.3; bump and re-publish).

---

## 🟠 Infrastructure — invisible to users, valuable for contributors

### 3. Real binary smoke on macOS and Linux

The release CI matrix produces `.tar.gz` artifacts for
`x86_64-unknown-linux-gnu`, `x86_64-apple-darwin`, and
`aarch64-apple-darwin`, and the cross-platform smoke job runs
`coral --version` + `coral init` on every release. Nobody has actually
extracted the artifact and run `coral ui serve` with real `.wiki/`
data on a macOS or Linux machine.

Risk: glibc compatibility on older distros, codesign issues on
darwin, libssl ABI surprises.

Acceptance: someone runs the v0.33.0 release `.tar.gz` on each of:
- macOS 14+ (both x86 and Apple Silicon)
- Ubuntu 22.04 / 24.04
- Debian 12

…and confirms `coral ui serve` serves the SPA + REST endpoints
end-to-end.

Cost: ~30 min per platform if a machine is available.

---

### 4. Playwright E2E in CI matrix (Linux + Windows)

**Status v0.34.x:** STILL DEFERRED. Audit summary below.

`crates/coral-ui/assets/src/e2e/` ships 14 Playwright tests across 5
spec files (nav, pages, graph, query, manifest). They run locally
against `coral ui serve --no-open --port 38400` and exercise mostly
static chrome (search inputs, table headers, banner copy, tab
labels) — they do NOT require an LLM token or a populated wiki.

**Workflow status (v0.34.x audit):**

`.github/workflows/playwright-ci.yml.disabled` exists with the
scaffolding (checkout, node setup, cargo build, npm ci, playwright
browser install, background `coral ui serve`, `npm run test:e2e`).
The only TODO is the "Seed temp workspace" step — currently a
`mkdir -p $RUNNER_TEMP/coral-e2e` placeholder with a comment.

The minimal seed is now feasible because `coral init` (post FR-ONB-25)
no longer needs interactive input — it just needs a git repo. A
runnable fixture would be:

```yaml
- name: Seed temp workspace
  shell: bash
  run: |
    mkdir -p "$RUNNER_TEMP/coral-e2e"
    cd "$RUNNER_TEMP/coral-e2e"
    git init --initial-branch=main --quiet
    git config user.email t@e.com && git config user.name T
    echo "# seed" > README.md && git add . && git commit -m init --quiet
    "$GITHUB_WORKSPACE/target/debug/coral" init
```

That gets `.wiki/index.md`, `.wiki/SCHEMA.md`, `.wiki/log.md` written.
For `/pages` to have any rows the seed would also need to drop a
handful of `pages/*.md` with valid frontmatter — but most of the
existing specs assert *chrome* (filters sidebar input, table
headers, tab labels) which renders against an empty page set.

**Why still deferred:** the existing Playwright suite has not been
validated against a CI runner end-to-end and the failure mode of
"works on local Chrome on Windows, fails headless Chromium on Linux"
is a known frustration. Landing the workflow without a validation
round-trip risks the maintainer waking up to red CI on every PR
until someone tracks down a locale or timing nit.

**Recommended next step (not landed v0.34.x):** rename the file to
`playwright-ci.yml`, push to a feature branch (NOT main), iterate
on the workflow_dispatch trigger until the suite goes green on a
single runner, THEN merge. Estimated ~2 hours of CI round-trips
(maintainer time, not engineering effort).

Cost: ~2 hours for the fixture bootstrap + one round-trip to validate
on actual Actions runners.

---

### 5. Formal `llvm-cov` ≥ 70% threshold enforcement

**Status v0.34.x:** PARTIALLY LANDED. The Coverage job in
`.github/workflows/ci.yml` now enforces a line-coverage floor via
`cargo llvm-cov report --fail-under-lines $CORAL_COV_MIN_LINES`, and
`continue-on-error` has been dropped. The floor is currently set to
**55%** (env `CORAL_COV_MIN_LINES` in the job definition) as a
conservative starting point we know the workspace clears today; the
PRD KPI target of **70%** remains the destination.

**Escalation plan** (bump the env var, no code changes needed):
- v0.34.x  — floor at 55% (current).
- v0.35.0  — bump to 60% after adding `coral-runner::http` golden tests
  and exercising the SSE-push path (currently the lowest covered file
  per `cargo llvm-cov --summary-only`).
- v0.36.0  — bump to 65% after the `coral-cli::commands::doctor` wizard
  paths get end-to-end coverage (currently only the happy-path is
  exercised).
- v0.37.0  — reach the PRD 70% KPI; revisit only if coverage measurably
  regresses on a refactor.

Each bump should land in its own PR with a brief CHANGELOG entry; never
*lower* the floor without a written justification.

**Why not 70% immediately?** Without a freshly-measured baseline in CI,
the safe default is "set the floor strictly below the current measured
value so legitimate PRs aren't artificially blocked, then tighten via
the ratchet above." Measure once `Coverage` lands on `main`, then bump
the env var in a follow-up commit.

---

### 6. `include_docs_flag_enables_pdf_scanning` test fix

`crates/coral-cli/src/commands/ingest.rs:705` —
`#[ignore = "flaky on Linux CI: ..."]`. Pre-existing race between
`CWD_LOCK` and `MockRunner`. Surfaced once `ci.yml` started running
again post-v0.32.2 unblock.

The test exercises the `--include-docs` flag; the production code
path is also covered by `slug_from_pdf_filename` /
`pdf_text_extraction_*` siblings which pass.

Root-cause analysis needed: it could be unrelated to PDF logic
entirely (the panic is on `assert_eq!(exit, ExitCode::SUCCESS)`).
Reproduce locally on Linux with `cargo test --
--test-threads=1` and bisect.

Cost: 2–4 hours of investigation. Low priority — no production
impact.

---

### 7. `bincode` 1.x → 2.x migration

**Status v0.34.x:** PARTIALLY LANDED. `coral-core::search_index::{save_index,
load_index}` migrated from bincode 1.x to 2.x via the serde integration
(`bincode::serde::encode_to_vec` + `decode_from_slice`,
`bincode::config::standard()`). On-disk format migration is automatic:
legacy `.coral/search-index.bin` files written by v0.33.x silently fail
to decode → `load_index` emits a single `tracing::warn!` and
`search_with_index` rebuilds the cache from the in-memory corpus. No
user action required.

Two new regression tests pin the contract:
`bincode2_encode_decode_roundtrip` (round-trip equality) and
`load_index_rebuilds_on_legacy_format_mismatch` (garbage-bytes →
InvalidData → transparent rebuild).

**RUSTSEC-2025-0141 still applies.** The advisory covers ALL bincode
versions (the upstream team stopped maintenance 2025-12-16 — see
https://git.sr.ht/~stygianentity/bincode). Migration to 2.x didn't
clear the advisory; the ignore stays in `deny.toml` + `cargo audit`
with the rationale that 2.x is on a more recently-maintained fork and
the API we use is stable. Long-term swap to one of the suggested
alternatives (postcard / bitcode / rkyv) is a follow-up item:

- **postcard**: postcard 1.x is stable, no_std-friendly, similar size
  to bincode. Best fit for the small, schema-stable `SearchIndex`
  struct. Wire format is varint-based like bincode 2.x.
- **bitcode**: optimised for compactness (~10% smaller than bincode
  for typical structs), but the API requires bitcode-specific derive
  macros — bigger blast radius.
- **rkyv**: zero-copy deserialisation, much faster load. Overkill for
  a 5 KB index file; would require switching off `serde::Deserialize`
  and learning rkyv's `Archive` model.

Recommended next swap: postcard, ~1 hour of work modelled on the
bincode 2.x migration. Drop the RUSTSEC ignore in the same PR.

---

## 🔵 Nice-to-haves surfaced during the sprint

### 8. Cross-module `coral_runner::test_script_lock()` proper serialiser

**Status v0.34.x:** EVALUATED, NOT MIGRATED. Audit summary below.

The current `pub fn test_script_lock() -> MutexGuard<'static, ()>` in
`coral-runner/src/lib.rs` works *within a single test binary*. Cargo
runs distinct test binaries (`coral-runner` lib vs its `tests/`
integration crate) as separate OS processes, so the in-process Mutex
can't coordinate them. The CI workaround is `RUST_TEST_THREADS=1` in
the `Test (stable)` and `Coverage` jobs, plus the lock for in-binary
serialisation.

**Audit (v0.34.x):** searched the whole workspace for callsites that
write a tempfile script + chmod +x + spawn it without holding the lock.

| Callsite                                    | Holds lock? | ETXTBSY risk?           |
| ------------------------------------------- | ----------- | ----------------------- |
| `coral-runner/src/runner.rs` (#[test])      | yes         | mitigated in-binary     |
| `coral-runner/src/local.rs` (#[test])       | yes         | mitigated in-binary     |
| `coral-runner/src/gemini.rs` (#[test])      | yes         | mitigated in-binary     |
| `coral-runner/tests/streaming_*` (8 tests)  | yes         | mitigated in-binary     |
| `coral-cli/tests/release_flow.rs`           | n/a         | none — pre-existing scripts on disk, no write-then-exec |
| `coral-test`, `coral-cli/commands/test.rs`  | n/a         | no fork-exec on tempfile scripts |

**Outcome:** every ETXTBSY-prone callsite is in `coral-runner` and
already holds the lock. The "cross-binary race" the BACKLOG entry
worried about is a paper risk: each test uses a fresh `tempfile::
TempDir`, so distinct inodes mean ETXTBSY cannot fire cross-process
even if both binaries run concurrently (the kernel checks per-inode
write-fd refcounts, not per-cargo-binary). The `RUST_TEST_THREADS=1`
workaround in CI is belt-and-suspenders, not load-bearing.

**Migration cost vs benefit:** an `fs4::FileExt::lock_exclusive()` on
`target/.coral-test-script.lock` would be ~30 LoC + edge-case work
(Windows uses LockFileEx with mandatory locking semantics that differ
from Linux's advisory `flock`; `fs4` papers this over but failure
modes diverge). It would let us drop `RUST_TEST_THREADS=1` from CI
saving ~20 s of wall-clock per run. Given the workaround is stable
and the in-process lock already covers the real ETXTBSY race window,
the migration is **not** scheduled for v0.35.

**Decision trigger:** revisit if (a) we add a new crate that
write-then-execs tempfile scripts outside `coral-runner` — in which
case the lock genuinely needs to cross crate boundaries — or (b) CI
wall-clock for `Test (stable)` becomes a measurable PR-cycle pain
point (>2 min total). Until then: leave the existing
in-process Mutex + `RUST_TEST_THREADS=1` workaround in place.

Cost (if/when scheduled): ~3 hours to design (Windows semantics,
poison recovery, well-known-path strategy) + ~1 hour to implement.

---

### 9. `coral wiki serve` deprecation timeline — DONE (v0.38.0)

The legacy v0.25.0 HTML/Mermaid server was deprecated in v0.34.1
(stderr banner + removal target v0.36.0) and **removed in v0.38.0**
after a 3-version window. `coral ui serve` is the full replacement
(same default port, modern SPA, graph, bi-temporal slider, filtering,
LLM query playground).

This was a breaking change pre-1.0, documented as such in the
CHANGELOG and commit message. Migration is a one-line edit: replace
`coral wiki serve` with `coral ui serve`.

Files removed: `crates/coral-cli/src/commands/serve.rs`. Cargo
feature `webui` and the `tiny_http` direct dep in `coral-cli` were
both retired (tiny_http remains a workspace dep — `coral-ui` and
`coral-mcp` still use it).

---

### 10. SLSA provenance verification doc

`actions/attest-build-provenance@v2` writes Sigstore-signed in-toto
attestations to every release artifact. Nothing in `docs/INSTALL.md`
or the README tells users how to verify them.

One-liner:

```bash
gh attestation verify coral-v0.33.0-x86_64-unknown-linux-gnu.tar.gz \
  --repo agustincbajo/Coral
```

Acceptance: a "Verifying release provenance" subsection in
`docs/INSTALL.md` with the command + expected output snippet.

Cost: 15 minutes.

---

## What's deliberately NOT in this backlog

These were considered and rejected (matches PRD §13 anti-features):

- ❌ SaaS / multi-tenant Coral
- ❌ WYSIWYG wiki editor (Git-native is the value prop)
- ❌ Multimodal ingest (PDF/image/audio) — the existing PDF flag is the
  furthest we go
- ❌ Telemetry / analytics
- ❌ A web-side "Coral Cloud" UI (lives in its own future PRD if it ever
  ships)

---

## Closing notes

Coral v0.33.0 is in production with:
- ✅ All M1 + M2 + M3 features from the PRD
- ✅ CI 100% green (first time in repo history)
- ✅ Sigstore-signed SLSA-shaped provenance on every release artifact
- ✅ Single-binary distribution preserved (~12 MB stripped Linux x86_64)
- ✅ Backward-compat sacred across v0.32.x and v0.33.0

The seven open items above are sequel work, not unfinished business.

---

### 11. Brandable install-URL domain (decision deferred)

The PRD (`docs/PRD-v0.34-onboarding.md` §1) sketches a rustup-style
shortcut for the install script:

```bash
curl -fsSL https://coral.dev/install | bash -s -- --with-claude-config
```

`coral.dev` itself is registered by a third party. Candidate
alternatives if we pursue the shortcut:

- `coral.sh` — terse, dev-aesthetic, ~$25/yr at typical registrars
- `coral.run` — same shape, ~$25/yr
- `getcoral.dev` — verb-prefixed (Stripe-style `stripe.com/docs`),
  cheaper TLD, ~$10/yr
- `coral-cli.dev` — explicit-tool branding

**Trade-offs auditados in-session:**

- **For**: tagline in marketing material is shorter
  (`coral.sh/install` vs raw GitHub URL); future repo moves don't
  break the install script; aesthetics matter post-launch.
- **Against**: most users install via Anthropic's curated marketplace
  (UI-driven, no URL), or via `/plugin marketplace add agustincbajo/Coral`
  (no domain involved). The `curl install.sh` flow is one-shot and
  the raw GitHub URL is acceptable verbosity. DNS + TLS + renewal is
  ongoing maintenance for a cosmetic win.

**Status**: decision deferred. Not blocking anything. Revisit if/when
a tagged marketing push or first conference talk gives Coral a name
to live up to. If acquired, set up a Cloudflare Pages redirect from
`<domain>/install` → `raw.githubusercontent.com/agustincbajo/Coral/
main/scripts/install.sh` and update `docs/PUBLISH.md` + README + the
PRD §1 mention.

Cost: ~$10-25/yr registrar + ~1 hour Cloudflare setup. No code work.

---

### v0.34.0 sprint status (shipped)

PRD `docs/PRD-v0.34-onboarding.md` v1.4 — onboarding-stack milestone M1.
Tagged `v0.34.0` on 2026-05-12; same-day patch `v0.34.1` closed three
post-release tech-debt items. Both releases published with full Sigstore
provenance + smoke matrix verified across Linux/macOS/Windows.

**v0.34.0 deliverables (M1 weeks 1–6, all 34 FRs):**

- ✅ `coral self-check` (App. F frozen schema), `coral self-upgrade`
  (cross-platform), `coral self-uninstall`, `coral self-register-marketplace`
- ✅ `coral doctor --wizard` (4-path provider mini-wizard)
- ✅ `coral bootstrap --estimate` / `--max-cost` / `--resume` (checkpointed)
- ✅ `coral init` writes `CLAUDE.md` template + `.gitignore` security entries (FR-ONB-25, FR-ONB-34)
- ✅ `install.sh --with-claude-config` + Windows SmartScreen/PATH hints + WSL2 detect (FR-ONB-31)
- ✅ `SessionStart` hook (cross-platform) + 4 new CI tests (hyperfine, output-size, naming-collision, Ollama gated)
- ✅ README "Getting Started in 60 seconds" + `docs/INSTALL.md` rewrite (FR-ONB-21, DoD M1 #14)
- ✅ Week 3 validator B2 closed (init outside git fails actionably) + week 2 nits 1 & 2 (resolve_provider 1×, comment alignment)
- ✅ Tag `v0.34.0` → `release.yml` ran the cross-platform smoke matrix (8/8 jobs green)
- ✅ Post-release-smoke matrix verde (Linux 5s / macOS 9s / Windows 25s)

**v0.34.1 patch deliverables (post-release tech debt):**

- ✅ `coral self-upgrade` authenticates via `$GITHUB_TOKEN` / `$GH_TOKEN` (raises GitHub API quota 60 → 5000 req/hour; eliminates 403 flake on smoke matrix)
- ✅ `on-session-start.ps1` rewritten without `Start-Job` double-host (`System.Diagnostics.Process` + `BeginOutputReadLine` + `WaitForExit`); latency dropped mean 644→367 ms, max 890→394 ms; CI threshold cut 1200 → 600 ms
- ✅ `[provider.ollama]` config from `coral doctor --wizard` now bridges into `--provider=http` automatically; BC preserved for v0.33 users with `CORAL_HTTP_ENDPOINT` env
- ✅ Post-release-smoke workflow auto-fires via `workflow_run` trigger (previously needed manual `workflow_dispatch`)

**Outstanding for v0.35:**

- Record + commit `docs/assets/getting-started.gif` (replaces the placeholder) — needs screen capture
- GTM communication (tweet, HN, Discord, marketplace push) — tone decision is maintainer's call
- Calibration crowd-sourced data n ≥ 30 for `--estimate` ±15% accuracy (M2 target per PRD §10)
- Anthropic official marketplace submission — human-to-human conversation required
- Gemini + Anthropic provider config bridge (extends the v0.34.1 Ollama bridge to the other two providers — in flight as of the v0.34.1-post quick-wins batch)

These outstanding items are scheduled v0.35 work, NOT regressions.

### v0.35 Phase C deferrals (ARCH-C1 follow-up) ✅ CLOSED v0.36-prep

Phase C audit demoted 6 zero-external-callers `pub mod` declarations
in `crates/coral-core/src/lib.rs` to `pub(crate)` (storage, vocab,
late_chunking, reranker, tantivy_backend, pgvector). That was 18% of
the 33 modules — below the ≥30% goal but the only safe set without
deeper refactors.

**v0.36-prep (commit `refactor(core): tighten public-mod surface
via curated re-exports`) demoted the remaining 10 candidates via
`pub use` shims at crate root.** Final public mod surface: **17/33
modules = 51% reduction from baseline**, comfortably exceeding the
original ≥30% goal. Each shim is mechanical and reversible.

Re-exports landed at crate root:
- `index` → `IndexEntry`, `WikiIndex`
- `cache` → `WalkCache`
- `embeddings` → `EmbeddingsIndex`
- `embeddings_sqlite` → `SqliteEmbeddingsIndex`, `SQLITE_FILENAME`
- `eval` → `evaluate`, `load_goldset`, `eval_render_markdown`
- `narrative` → `diff_wiki_states`, `generate_narrative`, `PageDiff`
- `llms_txt` → `llms_txt_generate`
- `gc` → `gc_analyze`, `gc_render_json`, `gc_render_markdown`
- `symbols` → `Symbol`, `SymbolKind`, `extract_from_dir`, `find_symbols_for_slug`
- `git_remote` → `SyncOutcome`, `sync_repo`
- `search_index` → `search_with_index`
- `wikilinks` → `wikilinks_extract`

### v0.35 Phase C deferrals (ARCH-C2 — test_script_lock)

`pub fn coral_runner::test_script_lock` exists because integration
tests in `coral-cli` need a process-wide mutex to coordinate the
fork-exec ETXTBSY race. Phase C kept the function `pub` with
heavy doc-comment + `#[doc(hidden)]` (option (c) in the spec) —
moving it to a `coral-test-utils` dev-dep crate (option (b)) was
considered but rejected as net-negative churn for v0.35: the
function is one line, the doc comment is the contract, and a
separate crate would add a workspace member for ~10 LoC.

Revisit only if the helper grows to 3+ functions or accidentally
becomes a runtime-relevant API.

### v0.35 Phase C deferrals (workspace clippy lints) ✅ v0.36 step CLOSED

Phase C added `[workspace.lints.clippy]` with `unwrap_used = "warn"`,
`expect_used = "warn"`, `panic = "warn"`. Initial count: **104
warnings** in libs + bins (production code, not tests).

**v0.36-prep ratchet result: 104 → 45 (57% reduction; target was
<50).** Categories attacked, in preference order:

- **D. Test fixtures gated with module-wide `#![allow]`** + doc-
  comment justification. Affected `coral-{runner,env,test}/src/
  mock.rs` (33 warnings combined). These mocks see `.lock().unwrap()`
  on single-threaded test Mutexes — the poisoned-mutex panic is
  exactly the surface we want in tests.
- **B. Safe-by-construction unwraps with per-function `#[allow]`**.
  Affected `coral-core/src/{symbols.rs,log.rs}` (22 warnings) and
  `coral-mcp/src/transport/http_sse.rs` (7 warnings). All sites
  match static regex literals or static-string header parses; the
  invariant is documented once per function with the allow attr.

No production logic refactored to dodge a lint; no lint levels
downgraded. Remaining 45 warnings tracked for v0.37 ratchet.

Next ratchet plan: aim for < 20 by v0.37 (15 sites probably mechanical
`?` operator + 5-10 contextual rewrites), < 5 + per-site
`#[allow(...)]` justifications by v1.0 (when promoting `warn` →
`deny` is safe).

### v0.35 Phase C deferrals (mimalloc baseline benchmark) ✅ CLOSED v0.36-prep

ADR-0012 kept `mimalloc` as `#[global_allocator]` with an unverified
"10-20% throughput" claim. The v0.36-prep work added a criterion
bench at `crates/coral-core/benches/allocator.rs` with three
representative workloads, ran it on Windows 11 MSVC, and captured the
full report at `docs/bench/MIMALLOC-BASELINE-2026-05-13.md`.

**Result: mimalloc DRAMATICALLY exceeds the claim:**

| Workload | mimalloc | system | speedup |
|----------|----------|--------|---------|
| A — TF-IDF 100 pages, 2-token query | 943 µs | 1.34 ms | +29.7% |
| B — page parse, 50 docs            | 268 µs | 465 µs  | +42.4% |
| C — JSON Value, 10 routes × 5 props | 92.5 µs| 162 µs  | +42.7% |

Criterion's change-detection on the system-allocator variant flagged
50.7-77.1% regressions vs the mimalloc baseline (p=0.00).
ADR-0012 promoted from "accepted, baseline-needed" to "accepted,
baseline measured." Decision trigger ≥10% met on all three workloads.

### v0.35 Phase C deferrals (gzip+brotli sibling generation hardening) ✅ CLOSED v0.36-prep

Both follow-ups landed in commit `build(ui): harden SPA sibling-gen
— drop raw >=100 KiB + always-Vary`:

1. **Raw bundle dropped from `include_dir!` for files ≥100 KiB.**
   `coral-ui/build.rs` removes the raw asset from `assets/dist` after
   writing both siblings. On the v0.35 SPA this drops 3 files (index.js
   535 KiB, sigma.js 172 KiB, markdown.js 167 KiB). Legacy-client
   fallback: `static_assets::decompress_any_sibling` brotli-then-gzip-
   decodes on the fly when the raw was dropped and the client
   advertised `Accept-Encoding: identity` (or omitted the header).
   `flate2`/`brotli` promoted from build-deps to runtime deps;
   flate2's default backend is pure-Rust miniz_oxide so the single-
   binary distribution invariant holds. Override knob:
   `CORAL_UI_KEEP_RAW=1` preserves raw on disk for debug builds.

2. **`Vary: Accept-Encoding` now always-on for content-negotiated
   paths.** `StaticResponse` gained a `vary_accept_encoding: bool`
   that's set whenever the path has at least one sibling — true on
   both the compressed and raw branches. `server::respond_static`
   emits Vary unconditionally on those responses, fixing the cache-
   poisoning hole where an intermediate cache keyed on URL alone
   could serve raw bytes to a brotli-capable client after a previous
   identity-only client warmed the cache. Per RFC 9110 §15.5.4.

Binary size delta: release `coral.exe` ~14.2 MiB vs ~14.8 MiB
baseline (Windows MSVC).
