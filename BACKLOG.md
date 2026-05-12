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

**Status v0.34.x:** CLOSED. `coral-core::search_index::{save_index,
load_index}` migrated to bincode 2.x via the serde integration
(`bincode::serde::encode_to_vec` + `decode_from_slice`,
`bincode::config::standard()`). `deny.toml` no longer ignores
RUSTSEC-2025-0141 and `cargo audit` runs without the `--ignore` flag.

On-disk format migration: 2.x is NOT wire-compatible with 1.x. Legacy
`.coral/search-index.bin` files written by v0.33.x silently fail to
decode → `load_index` emits a single `tracing::warn!` and
`search_with_index` rebuilds the cache from the in-memory corpus. No
user action required; the second `coral` invocation hits the freshly-
written 2.x cache. Two new regression tests pin the contract:
`bincode2_encode_decode_roundtrip` (round-trip equality) and
`load_index_rebuilds_on_legacy_format_mismatch` (garbage-bytes →
InvalidData → transparent rebuild).

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

### 9. `coral wiki serve` deprecation timeline

The legacy v0.25.0 HTML/Mermaid server is preserved for BC. It works
but is "stuck in time" — no dark mode, no graph, no filtering. As
adoption of `coral ui serve` grows, consider a `--deprecated` banner
on `coral wiki serve` startup and a removal target (v0.40.0?).

Decision: not urgent until WebUI adoption metrics suggest the legacy
path is no longer used.

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
