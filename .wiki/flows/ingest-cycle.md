---
slug: ingest-cycle
type: flow
last_updated_commit: 721050563f1ed29954b279fe334bf6bc8c8e2c34
confidence: 0.85
sources:
- crates/coral-cli/src/commands/ingest.rs
- crates/coral-core/src/gitdiff.rs
backlinks:
- cli
- karpathy-wiki
- wiki-index
status: reviewed
---

# Ingest cycle

The end-to-end flow that keeps `.wiki/` in sync with `HEAD`.

## Sequence

1. Trigger: `git push origin main` lands a commit, OR a developer runs `coral ingest` locally.
2. Read `last_commit` from `.wiki/index.md` (see [[wiki-index]]).
3. Resolve `HEAD` via `git rev-parse HEAD`.
4. Compute the range `<last_commit>..HEAD`.
5. Run `git diff --name-status <range>` to get a list of `(status, path)` entries (see `coral_core::gitdiff::run`).
6. Build the prompt via `coral_runner::PromptBuilder` from the embedded template at `template/prompts/ingest.md`. Inject `{{repo_path}}`, `{{last_commit}}`, `{{head_sha}}`, `{{diff_summary}}`.
7. Invoke the runner — `claude --print` with the bibliotecario subagent (see `template/agents/wiki-bibliotecario.md`) loaded as `--append-system-prompt`.
8. Receive a YAML plan: `[{slug, action, rationale}, ...]`.
9. **In v0.1, print the plan only.** v0.2 will apply changes.
10. The CI workflow (template at `template/workflows/wiki-maintenance.yml`) opens a PR `wiki/auto-ingest` for review.

## Key invariants

- The diff range is **always** `last_commit..HEAD` unless `--from` overrides. This guarantees no double-processing.
- The runner is invoked **once per ingest**, not per file. Cost grows with diff size, not file count.
- HEAD always wins: if a page contradicts the diff, the page is marked `status: stale`, not the diff revised.

## Failure modes

- `claude` not in `PATH` → `RunnerError::NotFound` → exit non-zero.
- Diff range invalid → `CoralError::Git` with stderr captured.
- LLM returns malformed YAML → printed verbatim; user reads it; no automatic fail (v0.1 deliberate choice).

## Related

- [[karpathy-wiki]] — the underlying pattern.
- [[lint-checks]] — runs after the ingest to catch broken-link aftermath.
