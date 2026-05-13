# Publishing the Coral plugin

There are three ways to distribute the Coral plugin to Claude Code
users. **Path A is already live** — anyone with Claude Code can
install Coral today by running two commands. Path B (Anthropic's
official curated marketplace) is the discovery-focused submission
route. Path C is the fallback for non-GitHub hosts.

---

## Path A — Self-hosted marketplace (LIVE today)

Anyone can install the Coral plugin right now via the standard
Claude Code marketplace flow. This works because:

- `github.com/agustincbajo/Coral` is public.
- `.claude-plugin/marketplace.json` declares the `coral` plugin.
- `.claude-plugin/plugin.json` declares the plugin metadata + MCP
  server + SessionStart hook.
- Releases are tagged on GitHub with cross-platform binaries +
  Sigstore provenance attestation.

**Install commands** (paste into Claude Code, one at a time):

```
/plugin marketplace add agustincbajo/Coral
/plugin install coral@coral
/reload-plugins
```

Or one-step with the binary install via the project's own installer:

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh \
  | bash -s -- --with-claude-config

# Windows PowerShell
& ([scriptblock]::Create((iwr -useb https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.ps1).Content)) -WithClaudeConfig
```

(The installer can patch `.claude/settings.json` with
`extraKnownMarketplaces` so even step 1 is skipped — see FR-ONB-26 in
`docs/PRD-v0.34-onboarding.md`.)

The `coral.dev/install` shortcut referenced in the PRD sketch is an
aspirational rustup-style URL (`curl rustup.rs | sh` pattern). The
domain is not currently owned by this project. The decision to
acquire a brandable domain (`coral.sh`, `coral.run`, `getcoral.dev`,
…) is deferred — see BACKLOG.md item #11. The GitHub raw URL above
is the operational install path either way, and most users will
install via the Anthropic curated marketplace (Path B below) once
approved, in which case the binary install is handled by the plugin
itself and no curl-pipe-bash command runs.

**Discovery surface:** README + social + word-of-mouth. The plugin is
NOT in the Claude Code "Discover" tab unless users follow Path B
below.

---

## Path B — Anthropic official curated marketplace

Anthropic ships an official marketplace browsable at
[claude.com/plugins](https://claude.com/plugins) and via the
`/plugin` → **Discover** tab inside Claude Code. Listing here is
discovery-driven: users find Coral without needing the GitHub URL.

### Submission process

Submit via one of the two official forms:

- **Claude.ai users**: [claude.ai/settings/plugins/submit](https://claude.ai/settings/plugins/submit)
- **Console / Workbench users**: [platform.claude.com/plugins/submit](https://platform.claude.com/plugins/submit)

The form fields are not publicly documented (no machine-readable
schema), but you will be asked for at minimum:

- Plugin name (`coral`)
- GitHub repository (`agustincbajo/Coral`)
- Description (already in `plugin.json` + `marketplace.json`)
- License (MIT — already declared)
- Category (`developer-tools` — already in `marketplace.json`)
- Contact e-mail / preferred maintainer channel

### Pre-submission checklist

Before clicking submit, run `claude plugin validate .` from a machine
where the `claude` CLI is installed. The validator flags:

- `marketplace.json` JSON syntax + required fields
- Plugin name kebab-case (Coral's `coral` passes)
- Duplicate plugin names within a marketplace
- YAML frontmatter in skill / agent / command files
- Malformed `hooks/hooks.json`
- Relative paths containing `../` (escape attempt)
- Reserved marketplace names (e.g. `claude-code-marketplace`,
  `anthropic-plugins` — Coral uses `coral`, safe)

Warnings (non-blocking but worth fixing):

- Missing marketplace description (Coral has one)
- Plugin name not in kebab-case (`coral` is fine)
- Empty plugins array (not Coral's case)

### What Anthropic likely reviews

Based on the curated nature of the marketplace and the existing
listings (the 11 LSP plugins documented in
`code.claude.com/docs/en/discover-plugins`):

- Security model: Coral has docs/SLSA-VERIFICATION.md + Sigstore
  provenance on every release.
- License clarity: MIT, declared everywhere.
- Plugin quality: validate output clean, README has install +
  examples.
- Maintenance velocity: regular tagged releases, CI green.

If submission asks for a "security model summary", point them at:
- `docs/SLSA-VERIFICATION.md` (artifact verification flow)
- `docs/audits/AUDIT-SECURITY-2026-05-12.md` (threat model)
- `docs/adr/0010-blocking-io-substrate.md` (transport rationale)
- ADR-0010..0012 (architectural decision records)

### After submission

Per Anthropic's docs:

- The plugin auto-syncs from the GitHub repo, so future tagged
  releases land in the official marketplace without re-submission.
- Users update via `/plugin update` (auto-update enabled by default
  for the official marketplace).
- The `plugin.json` `version` field drives discovery: bump it on
  each release. (Coral does this via `release.yml`'s
  `sync-plugin-manifests` job.)

---

## Path C — Non-GitHub hosts

Claude Code supports two additional marketplace transports beyond
GitHub:

- **Git (any provider)**:
  `/plugin marketplace add https://gitlab.com/your-org/plugins.git`
- **Direct URL to `marketplace.json`**:
  `/plugin marketplace add https://example.com/coral/marketplace.json`

These are useful for private mirrors (corporate marketplaces),
GitLab/Bitbucket hosts, or read-only S3-served manifests. Coral
itself uses GitHub, but a fork could republish via Path C.

---

## Versioning + update flow

Coral pins explicit versions in both `plugin.json` and
`marketplace.json` (currently `0.37.0`). The release pipeline keeps
them synced automatically via the `sync-plugin-manifests` job in
`.github/workflows/release.yml` (see PRD §7.6 FR-ONB-5).

| User action | What happens |
|---|---|
| `/plugin update` (manual) | Marketplace refreshes, plugin bumps to latest pinned version |
| `/plugin marketplace update <name>` | Force marketplace metadata refresh |
| Auto-update (official marketplace only) | Plugin auto-bumps on next session start when `version` differs |

If you omit `version` in `plugin.json`, Claude Code falls back to
the git commit SHA, so every commit is a new version. Coral pins
explicit versions to avoid surprising users with mid-development
churn.

---

## What's NOT done yet

The submission to Anthropic's official marketplace is **manual** —
this guide describes the process, but the form submission needs to
happen via a browser by the maintainer (you). The repo is fully
ready: validate-clean, MIT-licensed, with Sigstore provenance,
audit docs, and ADRs. The only blocker is filling the submission
form.

Once submitted and approved, this doc gets updated with the
official marketplace name (likely `coral` or `coral-official`) and
the install snippet shrinks to:

```
/plugin install coral@anthropic-plugins
```

---

## References

- [Discover and install prebuilt plugins](https://code.claude.com/docs/en/discover-plugins)
- [Create and distribute a plugin marketplace](https://code.claude.com/docs/en/plugin-marketplaces)
- [Plugins reference](https://code.claude.com/docs/en/plugins-reference)
- Coral install scripts: `scripts/install.sh`, `scripts/install.ps1`
- Coral release pipeline: `.github/workflows/release.yml`
- Coral SLSA verification: `docs/SLSA-VERIFICATION.md`
