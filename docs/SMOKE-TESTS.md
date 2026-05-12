# Coral cross-platform smoke tests

This document captures the manual + automated smoke test plan for the
v0.34.0 onboarding flow (PRD §6.8 FR-ONB-23, §6.10 FR-ONB-32 for self-
upgrade, §6.4 FR-ONB-31 for Windows-specific friction). Two audiences:

- **Release manager** — runs through the manual list below on each
  release-candidate tag before promoting it to `latest`.
- **CI** — the `.github/workflows/post-release-smoke.yml` workflow
  fires automatically on a published GitHub Release and runs the
  subset of the manual list that can be automated.

The plan is split into three blocks: install, baseline diagnostic
(`self-check` + `doctor`), and the upgrade-in-place path (`self-
upgrade`). All three are gated against the same matrix:
`ubuntu-latest`, `macos-latest`, `windows-latest`.

---

## 1. Install via the one-line installers

| Platform | Command | Expected |
|---|---|---|
| Linux x86_64 / aarch64 | `curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh \| bash` | exits 0; `coral --version` prints `coral 0.34.0` (or later) |
| macOS x86_64 / arm64 | same `install.sh` URL | same |
| Windows x86_64 | `iwr -useb https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.ps1 \| iex` | exits 0; `coral --version` (in a new shell because PATH gets refreshed) prints the version |

Optional flag combos to spot-check:

- `install.sh --with-claude-config` — patches `.claude/settings.json`
  in the current dir, prints the backup path. Idempotent.
- `install.sh --skip-plugin-instructions` — suppresses the 3-paste-line
  hint when running inside a CI pipeline where stdout is parsed.

---

## 2. Baseline diagnostic — `self-check` + `doctor`

After install, in any directory (even an empty `mktemp -d`):

```bash
coral self-check --format=json --quick
```

**Pass conditions** (parse the JSON; all consumers should tolerate
extra keys per FR-ONB-6):

- `schema_version` == `1`
- `coral_status` ∈ {`"ok"`, `"degraded"`} — never `"binary_missing"` /
  `"check_failed"` on a fresh install
- `coral_version` starts with `0.34`
- `in_path` == `true`
- `platform.os` matches the runner (`linux` / `macos` / `windows`)
- envelope total length ≤ 8000 chars (the SessionStart hook cap)

Then `coral doctor`. On a host with no provider configured, expect
warnings + a suggestion to run `coral doctor --wizard`. Exit code 0 in
both cases (warnings do NOT promote to failure unless `--strict` is
passed, which the smoke flow does not).

---

## 3. `coral self-upgrade --check-only`

```bash
coral self-upgrade --check-only
```

On a freshly-installed `0.34.0` binary, `--check-only` should print
**one** of these two states:

- `up_to_date` — `coral self-upgrade` would no-op.
- `update_available` — the GitHub `latest` release is ahead. The smoke
  test prints + records this for the release-manager to confirm it
  matches the tag they just promoted.

Exit code 0 in both cases. We do NOT run `coral self-upgrade` itself in
the automated smoke (it would mutate the runner image and the next
matrix shard would see an inconsistent state). Manual smoke on a
spare VM should exercise the full path **including** the Windows
rename-then-replace dance (renames `coral.exe` → `.old`, places the new
binary, deletes `.old` on next reboot).

---

## 4. Windows-specific manual smoke

Because `windows-latest` runners can't always reproduce end-user
friction (SmartScreen warnings, PATH refresh in a third-party shell),
the release manager runs these by hand on a Windows VM before
promotion:

1. **Defender SmartScreen** — first-run of `coral.exe` after fresh
   download triggers the SmartScreen banner. Click "More info" →
   "Run anyway". Subsequent runs are silent.
2. **PATH refresh** — open a fresh PowerShell after install (do NOT
   reuse the one that ran `install.ps1`); `coral --version` should
   resolve. If it doesn't, `install.ps1` regressed on PATH update.
3. **WSL2 detection** — run `install.sh` (the bash installer) from
   inside WSL2 and confirm the warning "you appear to be in WSL2;
   consider running install.ps1 from Windows host directly" prints.
4. **rename-then-replace** — `coral self-upgrade --version v0.34.0-rc.1`
   (an earlier tag) on a Windows host while a second `coral`
   subprocess holds the .exe locked (`coral mcp serve --transport stdio`
   in a sidecar shell). The upgrade should rename the locked .exe to
   `.old` and place the new one alongside; the sidecar continues
   running until restarted.

---

## 5. Ollama-path manual smoke (FR-ONB-28)

Run on a dev box with Ollama installed:

```bash
ollama serve &
ollama pull llama3.1:8b
cd /tmp && mktemp -d && cd ...   # use the tempdir
git clone https://github.com/.../tiny-rust-crate  # any small repo
cd tiny-rust-crate
coral doctor --wizard   # pick Ollama
coral init
coral bootstrap --apply --provider=http
```

Pass conditions: bootstrap completes within 10 minutes on a laptop
without GPU; `.wiki/` contains at least one `.md` page outside
`index.md` / `log.md` / `SCHEMA.md`; the body of that page is
non-empty and not an HTTP-error string.

This path is also covered by the automated test in
`crates/coral-cli/tests/ollama_bootstrap.rs` (run with
`cargo test --test ollama_bootstrap -- --ignored`); the manual
checklist exists for the release-manager's spot check.

---

## 6. Automation map

| Smoke item | Automated? | Where |
|---|---|---|
| install.sh → `coral --version` | yes | `.github/workflows/post-release-smoke.yml` (ubuntu, macos) |
| install.ps1 → `coral --version` | yes | same workflow (windows) |
| `coral self-check --quick` envelope shape | yes | same workflow + `crates/coral-cli/tests/quick_output_size.rs` |
| `coral doctor --non-interactive` JSON | yes | same workflow + `crates/coral-cli/tests/naming_collision.rs` |
| `coral self-upgrade --check-only` | yes | same workflow |
| Windows SmartScreen banner | no | manual |
| Windows PATH refresh in new shell | no | manual |
| WSL2 detection warning | no | manual (but `install.sh` has unit-level coverage) |
| rename-then-replace under file lock | no | manual |
| Ollama bootstrap | partial | `tests/ollama_bootstrap.rs` (gated) |
