# Install

> Full reference for `coral`-binary install, plugin wiring, upgrade, and
> uninstall. For the 60-second copy-paste onboarding, see
> [`README.md` § Getting Started in 60 seconds](../README.md#getting-started-in-60-seconds).

## Prerequisites

- **Git** 2.30+ — `coral init` requires a git repo; `coral diff` and
  `coral affected` shell out to `git`.
- **Claude Code CLI** (`claude` on PATH) — required only for the LLM-backed
  subcommands (`bootstrap`, `ingest`, `query`, `consolidate`, `onboard`,
  `lint --semantic`). If you don't have it yet, `coral doctor --wizard`
  walks you through the four supported provider paths (Anthropic API key,
  Gemini, local Ollama, or installing the `claude` CLI). Structural lint,
  `init`, `sync`, and `stats` work without it.
- **Optional:** `docker compose` v2.22+ for the `coral up` / `coral env`
  family. `podman compose` and `docker-compose` v1 are also detected.
- **Build-from-source only:** Rust 1.89+ (stable) via [rustup](https://rustup.rs/).
  MSRV bumped from 1.85 to 1.89 in v0.35.0 (ADR-0011 — required for
  stabilized `let_chains` + `is_multiple_of`).

## Linux & macOS — one-line installer

The Bash installer fetches the right release tarball for your platform/arch,
verifies the SHA-256, drops `coral` on `$PATH`, and prints the plugin
paste-block (or skips it under `--with-claude-config`).

```bash
curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh | bash
```

### Flags

| Flag                          | Purpose                                                                                                                                                                                  |
|-------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `--version vX.Y.Z`            | Pin to a specific release tag (skips the GitHub `releases/latest` API lookup; faster, deterministic).                                                                                    |
| `--with-claude-config`        | After the binary lands, run `coral self-register-marketplace` to patch the project-scope `.claude/settings.json` so Claude Code already knows about the Coral marketplace (FR-ONB-26).   |
| `--skip-plugin-instructions`  | Don't print or write the 3-paste-line snippet at the end. Use this in CI / Dockerfile installs where the plugin lines are noise.                                                         |
| `--help` / `-h`               | Print the inline help block and exit.                                                                                                                                                    |

Examples:

```bash
# Pin a version.
curl -fsSL .../install.sh | bash -s -- --version v0.38.0

# Auto-register the marketplace into THIS repo's .claude/settings.json
# (idempotent; atomic backup of any pre-existing file alongside).
curl -fsSL .../install.sh | bash -s -- --with-claude-config

# CI/Docker — quiet install, no stray paste-files.
curl -fsSL .../install.sh | bash -s -- --skip-plugin-instructions
```

### Install location

- Writes to `/usr/local/bin/coral` if it's writable; otherwise falls back
  to `~/.local/bin/coral` and reminds you to put that directory on PATH.
- Idempotent — re-running over the same release is a no-op.

### WSL2 note (FR-ONB-31)

If `install.sh` detects WSL2 (`/proc/version` contains `microsoft`) it
prints:

```
Detected WSL2. Coral binary installed for Linux.
  If you use Claude Code on Windows host (not in WSL),
  install the Windows binary instead via install.ps1.
```

It does NOT abort — running `coral` inside WSL is a supported configuration.
The warning is for users whose Claude Code lives on the Windows side.

## Windows — one-line installer

The PowerShell installer fetches the same release artifact, drops it under
`%LOCALAPPDATA%\Coral\bin`, and prepends that directory to your user PATH if
missing. Run from a regular PowerShell prompt (no admin needed):

```powershell
iwr -useb https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.ps1 | iex
```

To pass parameters, fetch the script first:

```powershell
$installer = (iwr -useb https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.ps1).Content
& ([scriptblock]::Create($installer)) -Version v0.38.0 -WithClaudeConfig
```

### Parameters

| Parameter             | Equivalent of                          |
|-----------------------|----------------------------------------|
| `-Version vX.Y.Z`     | `--version vX.Y.Z`                     |
| `-WithClaudeConfig`   | `--with-claude-config`                 |
| `-SkipPluginInstructions` | `--skip-plugin-instructions`       |
| `-InstallDir <path>`  | Override the default `%LOCALAPPDATA%\Coral\bin` |

### Windows Defender SmartScreen (FR-ONB-31)

On a fresh Windows machine, the first run of `coral.exe` may be blocked by
SmartScreen because Coral does not (yet) carry an Authenticode signature.
The installer prints, in yellow:

```
Windows Defender SmartScreen may block coral.exe on first run.
  If so: right-click coral.exe -> Properties -> check "Unblock" -> OK.
  Code signing is on the roadmap (tracked in BACKLOG).
```

### PATH refresh (FR-ONB-31)

A user-scope PATH update doesn't propagate to a shell that started **before**
the install. The installer prints, in yellow:

```
PATH updated for new sessions. Open a NEW PowerShell window to use 'coral'
  (current shell still has old PATH outside this script).
```

If you absolutely need it now, `refreshenv` (from Chocolatey's `chocolateyProfile`)
or restarting the shell are the two options.

## Manual install (pre-built tarball)

Each tagged release ships pre-built binaries on the [Releases page](https://github.com/agustincbajo/Coral/releases):

- Linux x86_64 → `coral-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`
- macOS Apple Silicon → `coral-vX.Y.Z-aarch64-apple-darwin.tar.gz`
- macOS Intel → `coral-vX.Y.Z-x86_64-apple-darwin.tar.gz`
- Windows MSVC → `coral-vX.Y.Z-x86_64-pc-windows-msvc.zip`

Each artifact has a `.sha256` sidecar. Verify, extract, place `coral`
(or `coral.exe`) on PATH:

```bash
VERSION=v0.38.0
TARGET=aarch64-apple-darwin
curl -L -o coral.tar.gz \
  "https://github.com/agustincbajo/Coral/releases/download/${VERSION}/coral-${VERSION}-${TARGET}.tar.gz"
shasum -a 256 -c coral.tar.gz.sha256
tar -xzf coral.tar.gz
sudo mv "coral-${VERSION}-${TARGET}/coral" /usr/local/bin/
coral --version
```

## Build from source

```bash
git clone https://github.com/agustincbajo/Coral
cd Coral
cargo build --release
./target/release/coral --version

# Or via cargo install (idempotent; no clone needed):
cargo install --locked --git https://github.com/agustincbajo/Coral --tag v0.38.0 coral-cli
```

Windows GNU toolchain users: see the [README "Windows — extra prereqs"
section](../README.md#windows--extra-prereqs-before-cargo-build) for the
MSVC vs MinGW-w64 setup notes.

## Verification

```bash
coral --version            # coral 0.37.0
coral self-check --quick   # < 100 ms; reports binary, providers, wiki, CLAUDE.md state
```

`coral self-check --format=json` emits the full diagnostic envelope. Its
schema is a frozen contract (see [`docs/PRD-v0.34-onboarding.md` Appendix F](PRD-v0.34-onboarding.md#19-apéndice-f-selfcheck-json-schema-nuevo-en-v14--frozen-contract)).
`coral self-check --print-schema` emits the matching JSON Schema for CI
contract checks.

### Verifying release provenance (SLSA / Sigstore)

From v0.33.0 onward, every release artifact carries an in-toto
attestation signed via Sigstore public-good (provenance generated by
`actions/attest-build-provenance@v2`). Verify locally:

```bash
gh attestation verify coral-v0.38.0-x86_64-unknown-linux-gnu.tar.gz \
  --repo agustincbajo/Coral
```

Full attestation policy + cosign-based verification: see
[`docs/SLSA-VERIFICATION.md`](SLSA-VERIFICATION.md).

## Plugin install (Claude Code)

Inside Claude Code, paste these three lines (one at a time — Claude Code's
prompt parser does NOT honor `&&` chains):

```
/plugin marketplace add agustincbajo/Coral
/plugin install coral@coral
/reload-plugins
```

…or skip this step entirely by passing `--with-claude-config` to the
installer (Linux/macOS) / `-WithClaudeConfig` (Windows), which writes the
same `extraKnownMarketplaces` entry to `.claude/settings.json` for you.

### What the plugin gives you

The Coral plugin registers, into the current Claude Code session:

- **5 auto-invoked skills** that fire when the SessionStart hook
  detects the right intent in user prompts:
  - `coral-bootstrap` — first-time wiki compilation
  - `coral-query` — answer questions about the codebase from the wiki
  - `coral-onboard` — new-contributor walkthrough
  - `coral-ui` — background-spawn `coral ui serve` and open the browser
  - `coral-doctor` — provider / health / "is this broken?" routing
- **2 slash commands** for the cases where you already know what you
  want and want to skip the natural-language routing:
  - `/coral:coral-bootstrap`
  - `/coral:coral-doctor`
- **SessionStart hook** — at the start of every Claude Code session
  the plugin runs a platform-aware dispatcher
  (`hooks/session-start.{sh,ps1,bat}`) that calls
  `coral self-check --quick` and seeds session context with the
  current wiki / provider / runner state. Empirical latency budget:
  ≤800 ms macOS/Linux, ≤1200 ms Windows (enforced in CI via hyperfine).

### `coral ui serve` security note (v0.35.0+)

`coral ui serve` auto-mints a 256-bit CSPRNG bearer token when bound
to a non-loopback address without an explicit `--token`. Explicit
tokens are validated against a 128-bit entropy floor (NIST SP 800-63B:
32 hex chars minimum) and rejected with exit code 2 if a short or
weak value is pasted. Loopback (127.0.0.1 / ::1) stays
unauthenticated for plug-and-play local dev. Same flow applies to
`coral mcp serve --transport http`.

## Upgrade

```bash
coral self-upgrade                    # default: latest same-major (v0.38.x -> v0.38.y)
coral self-upgrade --check-only       # report-only; never mutate
coral self-upgrade --version v0.38.0  # pin to a specific same-major release
```

- **Major bumps** (e.g. v0.37 → v0.38) require re-running the install
  script explicitly. The deliberate friction is AF-9 of the PRD —
  schemas may change across majors and the `self-upgrade` cannot prove
  the on-disk state is forward-compatible. The error message tells you
  the exact next command.
- **Windows**: the binary is replaced via `MoveFileEx` rename-then-replace
  (Windows can't overwrite a `.exe` while it's executing). Post-upgrade
  message reminds you that the next invocation in a new shell will use the
  upgraded binary; the old one is unlinked on next reboot if locked.
- **Linux/macOS**: atomic rename of `coral.new` over `coral` (the running
  process keeps its file-descriptor — the next invocation is the new
  binary).
- Post-upgrade runs `coral self-check` and reports success/fail with the
  new binary path. The Claude Code plugin auto-updates via the marketplace
  on the next `/reload-plugins`; `self-upgrade` does NOT touch it.

## Uninstall

```bash
coral self-uninstall            # remove binary + ~/.coral/ (config + logs)
coral self-uninstall --keep-data  # remove binary, keep ~/.coral/
```

`self-uninstall` deliberately does NOT touch `.wiki/` inside a repo — that
content belongs to your repo, not to the binary. After binary removal it
prints:

```
Plugin still registered in Claude Code.
Remove with: /plugin uninstall coral@coral
```

## Troubleshooting

### "Plugin shows Errors in Claude Code"

```bash
coral self-check --full
```

The output names the failing probe and gives an actionable `action:`
command. Most common: `coral` not on PATH, or `claude` CLI version mismatch.

### "No provider configured" / wizard didn't run

```bash
coral doctor --wizard
```

Four interactive paths: Anthropic API key, Gemini API key, local Ollama
endpoint, or installing the `claude` CLI. The wizard writes
`.coral/config.toml` per repo (chmod 600 on Unix; FR-ONB-27 + PRD Appendix E).

### "`coral bootstrap` is expensive on this repo"

```bash
coral bootstrap --estimate                          # see the upper-bound first
coral bootstrap --apply --max-cost=5.00             # hard cap; aborts mid-flight if exceeded
coral bootstrap --apply --max-cost=5.00 --resume    # resume from checkpoint after a cap hit
coral bootstrap --apply --max-pages=50              # cap by page count, not USD
```

The checkpoint lives at `.wiki/.bootstrap-state.json` (gitignored by
`coral init`; FR-ONB-34). `--resume` skips the planner call and re-tries
every page that is NOT `Completed`.

### "Windows: `coral.exe` blocked / silently fails to launch"

See the [Windows Defender SmartScreen](#windows-defender-smartscreen-fr-onb-31)
section above. Right-click → Properties → "Unblock" → OK. A second symptom
is "command not found" in a shell that pre-dates the install — open a new
PowerShell window.

### "WSL2: which binary do I want?"

If you use Claude Code **inside WSL**, the Linux binary (`install.sh`) is
correct. If your Claude Code is on the Windows host and your project tree
is mounted via `\\wsl$\`, install the Windows binary (`install.ps1`) instead.
Mixing them works but is harder to reason about — pick one host.

### "Bootstrap exit code 2 — what does that mean?"

`coral bootstrap --apply` exits 2 (PRD FR-ONB-29) when `--max-cost` halted
the run mid-flight with a partial checkpoint on disk. The exit is distinct
from 0 (success), 1 (findings), or 3 (internal error) so CI can detect it.
Run `coral bootstrap --resume` to continue.

## CI setup (GitHub Actions)

For automated wiki maintenance in your consumer repo:

1. **Claude Code OAuth token** — generate once via `claude setup-token`.
2. **GitHub secret** — add the token as `CLAUDE_CODE_OAUTH_TOKEN` at the
   **organization** level so all consumer repos inherit it.

Then either:

- **Use the Coral composite actions** in your `.github/workflows/wiki.yml`:
  ```yaml
  - uses: agustincbajo/Coral/.github/actions/ingest@v0.38.0
    with:
      claude_code_oauth_token: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}
  ```

- **Or copy the workflow template** that `coral sync` lays at
  `template/workflows/wiki-maintenance.yml`.

The Hermes quality gate (`/.github/actions/validate`) is **opt-in** — wire
it up explicitly when you want an independent LLM to validate wiki PRs
before merge. See [USAGE — CI: Hermes quality gate](./USAGE.md#ci-hermes-quality-gate).
