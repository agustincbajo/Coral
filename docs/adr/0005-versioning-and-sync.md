# ADR 0005 — Versioning and sync semantics

**Date:** 2026-04-30  
**Status:** accepted

## Context

Coral has two distinct things to version:

- **The `coral` binary** (CLI + crates).
- **The `template/` skill bundle** (subagents, prompts, SCHEMA, workflow).

Consumer repos pin Coral to lay subagents/workflows in their `.claude/` and `.github/`. Open questions:

- Are these two things versioned together or independently?
- How does a consumer know it has consistent components?
- What happens to a consumer's customized `SCHEMA.md` when they bump versions?

## Decision

**Single SemVer line: the binary version IS the template version.**

- Coral releases follow [SemVer](https://semver.org/): `vMAJOR.MINOR.PATCH`.
- Major bumps may break the consumer interface (clap flags, action.yml inputs, SCHEMA contract).
- Minor bumps add features (new subcommand, new lint rule, new prompt template).
- Patch bumps fix bugs without changing public surface.
- The `template/` content shipped with `vX.Y.Z` is byte-identical for every install of `vX.Y.Z` (via `include_dir!` — see [ADR 0003](0003-template-distribution-via-include-dir.md)).

**Pin file: `.coral-template-version`** at the consumer's repo root. Single line: `v0.1.0`. Written automatically by `coral sync`.

**SCHEMA divergence is expected and protected.**

- `coral init` copies `template/schema/SCHEMA.base.md` → `<wiki>/SCHEMA.md` exactly **once** (only if the destination doesn't exist).
- Subsequent `coral sync` calls **never overwrite** `<wiki>/SCHEMA.md`. The consumer is expected to extend the SCHEMA locally with project-specific page types and rules.
- Subagents (`.claude/agents/wiki-*.md`), prompts (`prompts/*.md`), and the workflow template **are** updated by `coral sync` (with `--force` if needed). These are infrastructure; the SCHEMA is the consumer's contract.

## Consequences

**Positive:**
- **Atomic upgrade story.** "Run `cargo install --tag v0.2.0` and `coral sync --version v0.2.0`. Done."
- **No skew.** Subagents shipped with v0.2.0 always match the prompts shipped with v0.2.0.
- **Consumer SCHEMA stays.** Years of project-specific extensions don't get clobbered on upgrade.
- **Deterministic CI**: `coral sync --version v0.1.0` in repo A and repo B yields byte-identical files (verified by `e2e_multi_repo::sync_reproducible_across_repos`).

**Negative:**
- **No partial upgrades.** A consumer can't upgrade the workflow but keep the old subagents. (Mitigation: edit the synced files in-place; `coral sync --force` re-applies, plain `coral sync` skips existing.)
- **Major-bump churn**: a v1.0 → v2.0 SCHEMA change requires consumers to manually merge their local SCHEMA with the new base. Documented in CHANGELOG migration sections.

## Alternatives considered

- **Separate template versioning (e.g., `coral-template@v0.5.0`)**: rejected as unnecessary complexity. The template is shipped *in* the binary; decoupling adds zero value for v0.1.
- **Always-overwrite sync**: rejected because it would clobber consumer SCHEMA changes — a deal-breaker for adoption.
- **Symlink-based sync** (template files are symlinks into the Coral install): rejected because symlinks are platform-fragile and break the "Markdown in Git" promise.

## Future evolution

- **v0.2**: add `coral sync --remote v0.X.Y` that fetches a release tarball from GitHub (for users who want bleeding-edge before they reinstall the binary).
- **v0.3**: per-file pinning. A consumer might want `wiki-bibliotecario.md` from v0.5.0 but `wiki-linter.md` from v0.4.0. Not a v0.1 problem.
