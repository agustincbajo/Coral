---
name: coral-bootstrap
description: Bootstrap the Coral wiki for the current repository, with cost confirmation. Use when the user asks to "set up Coral", "initialize a wiki", "scaffold a wiki", "bootstrap the Coral wiki", "create a .wiki/ directory", "compile a wiki for this repo", or wants a first-time install of Coral in a fresh repo. The skill ALWAYS runs `coral bootstrap --estimate` first and shows the cost upper-bound before asking for confirmation.
disable-model-invocation: false
allowed-tools: Bash(coral:*), Bash(ls:*), Bash(test:*)
---

# Coral bootstrap

Run a cost-confirmed wiki bootstrap for the current repo.

## Steps

1. Run `coral self-check --format=json --quick`. Parse the JSON.
2. If `wiki_present == true`, ask user: *"Wiki already exists. Re-bootstrap? (y/n)"*. If `n`, exit.
3. If `providers_configured == []`, hand off to the `coral-doctor` skill (it has the provider mini-wizard). Do NOT continue here — `coral-doctor` will route back to this skill once a provider is configured.
4. Run `coral bootstrap --estimate`. Capture stdout. The output looks like:
   ```
   Repo size: 10,247 LOC across 142 files
   Estimated pages: 47
   Estimated tokens: ~120k input + ~80k output
   Provider: claude-sonnet-4-5
   Estimated cost: $0.42 (up to $0.53 — margin ±25%)
   ```
5. Show the user:
   - Estimated cost **with upper-bound**: *"$0.42 (up to $0.53)"*. Do NOT show a range like "$0.30–$0.50" — show the upper bound explicitly so the user knows the worst case.
   - Pages count + token totals.
   - Provider label.
   - If `estimate.upper_bound > $5`: include the FR-ONB-12 large-repo hint:
     ```
     ⚠️  This is a large repo (estimate > $5). Consider starting with:

         coral bootstrap --apply --max-pages=50 --priority=high

     This bootstraps the 50 most-referenced modules first. You can run again
     later with --resume to continue or re-run without --max-pages to do all.
     ```
   - Mention the prompt-caching disclaimer that `--estimate` already prints: *"Actual cost may be 30-50% lower if prompt caching is enabled (M2 will calibrate)."*
6. Ask the user one question with four options:
   > *"Run? Options:
   >   - **yes** — run with no cap
   >   - **yes --max-cost=X** — abort mid-flight if running cost exceeds $X
   >   - **yes --max-pages=N** — limit scope to the first N pages (useful for huge repos)
   >   - **cancel** — abort"*
7. On confirm, run the chosen variant:
   - `coral bootstrap --apply` (no cap)
   - `coral bootstrap --apply --max-cost=<USD>` (cap, aborts mid-flight with checkpoint)
   - `coral bootstrap --apply --max-pages=<N>` (scope limit)
   - Both flags can be combined.
8. If the run is interrupted (exit code 2 = `--max-cost` hit, or any other failure mid-flight): tell the user *"Bootstrap halted. Run `coral bootstrap --resume` to continue from the last checkpoint."* The checkpoint lives at `.wiki/.bootstrap-state.json`.
9. On success:
   - Suggest spawning the WebUI: invoke `coral-ui` skill (background spawn).
   - Mention *"Your wiki is in `.wiki/`. Try queries like 'show me the architecture' or open http://localhost:3838/pages."*
   - Suggest CI integration snippet (pre-commit hook, GitHub Actions). The user can opt in later.

## Failure modes

- **`coral` not in PATH** → suggest `/coral:coral-doctor`. Do NOT attempt to install Coral.
- **No provider configured** (`providers_configured == []`) → hand off to `coral-doctor` (which has the provider mini-wizard with 4 paths: Anthropic API key, Gemini, Ollama, install `claude` CLI).
- **`--apply` fails mid-flight** → tell the user about `coral bootstrap --resume`. The state file is at `.wiki/.bootstrap-state.json`.
- **Estimate upper-bound exceeds `--max-cost`** → the pre-flight gate prints a clear message; suggest `--max-pages=N` to limit scope, or removing `--max-cost`.
- **Existing wiki present** → never overwrite. Ask the user whether to refresh via `coral ingest --apply` (incremental), start over (they `rm -rf .wiki/` first), or skip.

## Reference

- Subcommand reference: `coral bootstrap --help`.
- Cost transparency: every cost number is an **upper bound** (estimate × 1.25, margin ±25% in M1). Real cost may be 30-50% lower with prompt caching (M2 calibration).
- Checkpoint schema: `.wiki/.bootstrap-state.json` is versioned (`schema_version`). A version mismatch is surfaced as an actionable error.
- Lockfile: `.wiki/.bootstrap.lock`. Held for the whole apply / resume phase; two concurrent runs cannot interleave.
