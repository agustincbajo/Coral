# ADR 0009 — `coral lint --auto-fix` scope and YAML plan shape

**Date:** 2026-05-01
**Status:** accepted (v0.5)

## Context

Structural lint (v0.1) and semantic lint (v0.2) both surface issues
without fixing them. Users routinely have 5–20 issues in a wiki of
50+ pages — fixing each by hand is the friction that drives wikis
to drift.

Three forces shaped the v0.5 design:

1. **Scope creep risk.** "LLM auto-fix" can mean anything from "trim
   trailing whitespace" to "rewrite the whole page." The first is
   fine, the second is dangerous (the LLM can hallucinate sources,
   delete real claims, or reformat past human edits).
2. **Reviewability.** Auto-applied changes need to be reviewable as a
   structured diff, not buried in free-form prose.
3. **Reversibility.** Mistakes happen. Apply must be a separate gate
   from preview, matching the `bootstrap`/`ingest`/`consolidate`/
   `notion-push` `--apply` family.

## Decision

**Cap the LLM scope. Use a YAML plan. Default to dry-run.**

### Capped scope

The auto-fix LLM can do exactly four things per page:

| Action | Mutation | Why safe |
|---|---|---|
| `confidence: <f64>` | Bump confidence value in frontmatter | Numeric, validated `0.0..=1.0`. |
| `status: <enum>` | Set status to `draft`/`stale`/`reviewed`/etc. | Enum, validated against `Status`. |
| `body_append: <md>` | Append a Markdown chunk to the body | Append-only — never modifies prior content. |
| `action: retire` | Set status to `stale` | Same as `status: stale`, but explicit signal. |

The system prompt reinforces:

> Do NOT rewrite whole bodies. Do NOT invent sources.

Anything outside this set must be `action: skip` with a `rationale:`.

### YAML plan shape

```yaml
fixes:
  - slug: order
    action: update                  # update | retire | skip
    confidence: 0.5                 # optional
    status: draft                   # optional
    body_append: |                  # optional; appended with two leading newlines
      _Stale: needs sources._
    rationale: dropped below threshold
```

**Why this shape:**
- One entry per fix, keyed by slug → can be diffed structurally.
- Action enum prevents free-form intent ambiguity.
- `body_append` (not `body_replace`) is append-only by design.
- `confidence` and `status` are typed so `Confidence::try_new` and
  `Status::FromStr` reject garbage at parse time, not at write time.

### Default dry-run

`coral lint --auto-fix` prints the YAML plan and exits 0. `--apply`
mutates `.wiki/`. This matches the v0.2-onward family of subcommands
(`bootstrap`, `ingest`, `consolidate`, `notion-push`, `onboard`).

### Override path

The system prompt lives at:

```
<cwd>/prompts/lint-auto-fix.md   ← local override
template/prompts/lint-auto-fix.md ← embedded (none yet; uses fallback)
const AUTO_FIX_SYSTEM_FALLBACK   ← in coral_cli::commands::lint
```

Same priority chain as every other LLM subcommand
(`prompt_loader::load_or_fallback`).

## Why not free-form rewrites

The natural LLM mode is "here's the issue, here's the page, propose a
new page body." That's powerful and dangerous:

- The LLM can drop sentences a human wrote intentionally.
- It can invent `sources:` that don't exist on disk.
- It can change wikilinks in ways that break `coral lint --structural`.
- It can introduce its own style (em-dash everywhere, formal voice)
  that drifts from the rest of the wiki.

Capped scope means an auto-fix run can ALWAYS be reverted by
re-running with the inverse confidence/status — there's no body content
to recover.

## Why YAML, not a custom DSL or JSON Patch

- **YAML** matches `bootstrap`/`ingest`/`consolidate` output. The
  bibliotecario already knows the conventions.
- **JSON Patch (RFC 6902)** would need explicit pointers
  (`/fixes/0/confidence`); too verbose.
- **A custom DSL** would need its own parser and docs; YAML +
  `serde_yaml_ng::from_str` + a typed struct is ~30 LOC.

## Consequences

**Positive:**
- Auto-fix mutations are bounded, typed, and reviewable as a YAML
  diff in the dry-run output.
- `--apply` semantics match every other apply-family command.
- The 4 unit tests cover both the parser (with fences, missing-action
  default-to-skip) and the apply path (frontmatter+body changes,
  retire-marks-stale).
- A consumer that wants different semantics can override the prompt
  template without recompiling.

**Negative:**
- Real wiki rot needs more than these four actions
  (e.g. "this page should cite `src/x.rs:42` — please add it"). Out of
  scope for v0.5; tracked as future "source-suggestion" pass.
- The LLM can still produce a syntactically-valid plan with a bad
  `confidence` floor for a high-quality page. Caller must review the
  dry-run before `--apply`.
- Body-append is unconditional — a careless LLM could spam every page
  with the same italic note. The capped scope makes this annoying but
  not destructive.

## Alternatives considered

- **No-LLM "fix" mode** (e.g. `coral lint --fix` that trims whitespace,
  normalizes wikilink syntax): considered, rejected for v0.5 — almost
  every real lint issue needs judgment that pure rules can't make. May
  ship as a separate `coral fmt` subcommand.
- **Inline diffs in stdout** (no YAML, just patch hunks): harder to
  parse, harder to test, harder to override.
- **Git-style staging** (`coral lint --auto-fix --stage` writes to a
  side-tree, then `coral lint --apply-staged` commits): overkill for
  the v0.5 use case. Re-run with `--apply` is simple enough.

## Future evolution

- **`source-suggestion` pass**: a separate LLM call after auto-fix that
  proposes `sources:` paths from `git ls-files` output. Higher-risk
  than the v0.5 set; will need its own prompt + dedicated tests.
- **Per-rule fix policies**: today every issue feeds the same
  prompt. Future: route `BrokenWikilink` to a wikilink-specific
  prompt that has access to the full slug list.
- **Confidence-from-coverage**: if `sources:` cite files that no
  longer exist, auto-downgrade confidence by a fixed step. Pure rule,
  no LLM. Could ship as part of the no-LLM `coral fmt` above.
