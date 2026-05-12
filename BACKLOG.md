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

`crates/coral-ui/assets/src/e2e/` ships 14 Playwright tests across 5
spec files (nav, pages, graph, query, manifest). They run locally
against `coral ui serve --no-open --port 38400`.

A `.github/workflows/playwright-ci.yml.disabled` workflow exists but
the `.disabled` suffix keeps it out of the active set. To enable:

1. Add a "fixture bootstrap" step that creates a wiki, ingests fake
   pages, and spawns `coral ui serve` in the background.
2. Run `npx playwright install --with-deps chromium` (Linux) /
   `chromium firefox webkit` (Windows).
3. `npm run test:e2e`.
4. Rename `.disabled` → no suffix.

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

`RUSTSEC-2025-0141`: bincode 1.x is unmaintained. Currently ignored
in `deny.toml` and `cargo audit --ignore`. Not a security
vulnerability, just no longer accepting bug fixes upstream.

Used by `coral-core::search_index` for persisting the BM25 index to
disk. Migration is non-trivial: bincode 2.x changed its encode/decode
API (no more `serialize`/`deserialize`, now `encode`/`decode` with
explicit `Configuration`).

Acceptance: `coral-core::search_index::{save_index, load_index}` use
bincode 2.x, deny.toml stops ignoring the advisory, the on-disk
format is migrated cleanly (or invalidated + rebuilt on first
v0.34.0 boot).

Cost: 4–8 hours including writing the format-migration codepath.

---

## 🔵 Nice-to-haves surfaced during the sprint

### 8. Cross-module `coral_runner::test_script_lock()` proper serialiser

The current `pub fn test_script_lock() -> MutexGuard<'static, ()>` in
`coral-runner/src/lib.rs` works *within a single test binary*, but
cargo's parallelism across binaries (lib-test vs integration-test
binaries) still races. The current workaround is
`RUST_TEST_THREADS=1` in the CI Test (stable) and Coverage jobs.

A real fix would use file-locking (`fs4::FileExt::lock_exclusive`) on
a well-known path, or a `serial_test`-style crate-attribute macro.
The cost of the current workaround is ~20s extra wallclock on each
CI run; if that ever matters, file-lock the spawn point in the
`Runner` trait test fixtures.

Cost: ~3 hours to design + ~1 hour to implement.

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
