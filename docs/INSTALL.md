# Install

## Prerequisites

- **Rust** 1.85+ (stable). Install via [rustup](https://rustup.rs/).
- **Git** 2.30+ (for `git diff --name-status` and `git rev-parse`).
- **Claude Code CLI** (`claude` in `PATH`). Install via [claude.com/code](https://claude.com/code). Required only for the LLM-backed subcommands (`bootstrap`, `ingest`, `query`, `consolidate`, `onboard`, `lint --semantic`). Structural lint, init, sync, and stats work without it.

## Install Coral CLI

### Option A — from a tagged release (recommended)

```bash
cargo install --locked --git https://github.com/agustincbajo/Coral --tag v0.1.0 coral-cli
```

### Option B — from main (latest)

```bash
cargo install --locked --git https://github.com/agustincbajo/Coral coral-cli
```

### Option C — from source

```bash
git clone https://github.com/agustincbajo/Coral && cd Coral
cargo install --locked --path crates/coral-cli
```

Verify:

```bash
coral --version    # coral 0.1.0
coral --help
```

## CI setup (GitHub Actions)

For automated wiki maintenance in your consumer repo, you need:

1. **Claude Code OAuth token** — generate once via `claude setup-token` on your machine.
2. **GitHub secret** — add the token as `CLAUDE_CODE_OAUTH_TOKEN` at the **organization** level (so all consumer repos inherit it).

Then either:

- **Use the Coral composite actions** in your `.github/workflows/wiki.yml`:
   ```yaml
   - uses: agustincbajo/Coral/.github/actions/ingest@v0.1.0
     with:
       claude_code_oauth_token: ${{ secrets.CLAUDE_CODE_OAUTH_TOKEN }}
   ```

- **Or copy the workflow template** that `coral sync` lays at `template/workflows/wiki-maintenance.yml`.

## Uninstall

```bash
cargo uninstall coral-cli
rm -rf .wiki/                      # if you want to discard the wiki
rm -f .coral-template-version
```
