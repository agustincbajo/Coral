# Sessions

`coral session` (shipped in **v0.20.0**, issue [#16](https://github.com/agustincbajo/Coral/issues/16)) captures agent transcripts (Claude Code today; Cursor and ChatGPT tracked) and distills them into wiki pages. Every step is opt-in and gated by the same trust-by-curation contract that governs the rest of Coral.

This document covers:

- The five subcommands and what each does.
- Where captured data lives and what is / isn't gitignored by default.
- The privacy posture (what's scrubbed, how to opt out, how the escape hatch works).
- The trust-by-curation gate (`reviewed: false` lint + pre-commit hook).
- Why each design question was answered the way it was.

## Quickstart

```bash
# 1. Capture the most-recent Claude Code session for this project.
coral session capture --from claude-code
# → captured <session-id> (<n> messages, <m> redactions)
#    → .coral/sessions/2026-05-08_claude-code_<sha8>.jsonl

# 2. List what's been captured.
coral session list

# 3. Inspect before distilling.
coral session show <short-id>

# 4. Distill into wiki-shaped synthesis pages (LLM call).
coral session distill <short-id>
# → .coral/sessions/distilled/<slug>.md      (always, reviewed: false)
# → .wiki/synthesis/<slug>.md                (with --apply, also reviewed: false)

# 5. Review + flip `reviewed: true` in the page; commit.
#    `coral lint` + the pre-commit hook block any reviewed: false page.

# 6. (Optional) Forget the session once distilled output is curated.
coral session forget <short-id> --yes
```

## Subcommands

### `coral session capture --from claude-code [PATH]`

Copies a Claude Code transcript (`~/.claude/projects/<project>/<uuid>.jsonl`) into `<project_root>/.coral/sessions/<date>_claude-code_<sha8>.jsonl`. Default behaviour:

- **Auto-discovery**: when `PATH` is omitted, walks `~/.claude/projects/`, parses each transcript's first record, and picks the most-recently-modified one whose `cwd` matches the current project. If nothing matches, exits with a clear "no captured sessions yet for this project" message.
- **Privacy scrubbing on by default**: the captured bytes are run through the regex scrubber (`crates/coral-session/src/scrub.rs`) before they hit disk. Each match is replaced by `[REDACTED:<kind>]`. The summary line includes the redaction count.
- **Atomic write**: temp-file + rename via `coral_core::atomic::atomic_write_string`. Concurrent `capture` and `forget` invocations never race (`with_exclusive_lock` on the index file).

#### Opt-out: `--no-scrub --yes-i-really-mean-it`

The scrubber is intentionally hard to disable. Both flags are required. Without `--yes-i-really-mean-it`, `--no-scrub` fails fast with a hint:

```
error: refusing to skip the privacy scrubber: pass --yes-i-really-mean-it
       alongside --no-scrub. Captured transcripts may contain API keys, JWTs,
       AWS creds, etc. — see docs/SESSIONS.md.
```

Use the escape hatch when you have a manual reason to keep raw bytes (e.g., debugging the scrubber). Don't routinely bypass it. If the scrubber misses a token shape that matters, file an issue with the shape and a fixture — every shape we ship has a regression test.

#### Cross-format support order

`--from cursor` and `--from chatgpt` exist as CLI args but currently emit:

```
error: invalid input: source 'cursor' is not yet implemented; track issue #16.
       Only --from claude-code ships in v0.20.
```

Claude Code first because its JSONL schema is the most stable and best-documented. Cursor (IndexedDB; lower ROI) and ChatGPT (Markdown export; partial lift already done by the format) follow once we have evidence of demand.

### `coral session list [--format markdown|json]`

Renders `<project_root>/.coral/sessions/index.json` as a Markdown table (default) or JSON array. Sorted by `captured_at` descending. Empty state prints `_No captured sessions yet._` rather than a header-only table.

### `coral session show <SESSION_ID> [--n N]`

Prints session metadata (id, source, captured_at, message count, redaction count, distilled flag) plus the first `N` (default: 5) extracted messages with each preview truncated to 200 chars.

`SESSION_ID` accepts either the full UUID or any unique prefix of ≥4 chars. An ambiguous prefix returns `InvalidInput` with the match count so you can disambiguate.

### `coral session distill <SESSION_ID> [--apply] [--provider …] [--model …]`

Single-pass LLM call that extracts 1–3 *surprising* / *non-obvious* findings from the transcript and emits each as a synthesis page. The page always lands as `reviewed: false`.

Without `--apply`: writes `.coral/sessions/distilled/<slug>.md`. The page is gitignored by default — curated distillations are intended to live in `.wiki/synthesis/`, not next to the raw transcripts.

With `--apply`: also writes `.wiki/synthesis/<slug>.md`, where `coral search` and `coral lint` will see it. The `reviewed: false` flag means a human MUST review and flip it before committing — see the trust-by-curation section below.

Provider resolution is the same as every other LLM-driven Coral command (claude / gemini / local / http; falls back to `CORAL_PROVIDER` env or `claude`).

### `coral session forget <SESSION_ID> [--yes]`

Deletes the raw `.jsonl`, the distilled `.md` if present, and the index entry under an exclusive lock so concurrent operations don't race. Without `--yes` an interactive `[y/N]` prompt is shown.

`SESSION_ID` matches the same way as `show`/`distill`: full UUID or unique 4+-char prefix.

## On-disk layout

```
<project_root>/
├── .coral/
│   └── sessions/
│       ├── 2026-05-08_claude-code_a1b2c3d4.jsonl     # captured (gitignored)
│       ├── index.json                                 # metadata (gitignored)
│       ├── index.json.lock                            # flock sentinel (gitignored)
│       └── distilled/
│           └── <slug>.md                              # distilled (gitignored)
└── .wiki/
    └── synthesis/
        └── <slug>.md                                  # apply target (NOT gitignored)
```

`coral init` writes the gitignore entries into the **project-root** `.gitignore` (idempotent on re-run; preserves existing user-managed lines):

```
.coral/sessions/*.jsonl
.coral/sessions/*.lock
.coral/sessions/index.json
!.coral/sessions/distilled/
```

The negation `!.coral/sessions/distilled/` lets curated distillations ship in git while keeping raw JSONL captures local-only. (The PRD's "design Q1: storage default — gitignored or committed?" answer was *gitignored, with the distill flow as the path to curated commits*.)

## Privacy posture

The scrubber runs by default and redacts these token shapes:

| Pattern | Marker | Source |
|---|---|---|
| `sk-ant-…` | `anthropic_key` | Anthropic API keys |
| `sk-…` (incl. `sk-proj-…`) | `openai_key` | OpenAI keys |
| `gh[pousr]_…` | `github_token` | GitHub PATs (classic, fine-grained, OAuth) |
| `AKIA[A-Z0-9]{16}` | `aws_access_key` | AWS access key IDs |
| `aws_secret_access_key=…` | `aws_secret_key` | AWS secret keys (assignment shape) |
| `xox[bporsa]-…` | `slack_token` | Slack bot/user/admin/refresh tokens |
| `glpat-…` | `gitlab_token` | GitLab PATs |
| 3-segment base64url `eyJ…` | `jwt` | JWTs |
| `Authorization: Bearer …` | `authorization` | Header form |
| `x-api-key: …` | `x_api_key` | Header form |
| Bare `Bearer <token>` (≥20 chars) | `bearer` | Bare bearer tokens |
| `ANTHROPIC_API_KEY=…` (and friends) | `env_assignment` | Env-export inline |

Each match is replaced with `[REDACTED:<marker>]`. The marker is part of the redaction itself so you can grep the captured file for redaction sites. The original token bytes are never stored — `Redaction` records carry only `kind`, `byte_offset`, and `original_len`.

False-positive rate is intentionally low: the regex requires meaningful payload length (e.g. `sk-…{20,}`, `eyJ…` JWT segments ≥10 chars each), so prose mentions like "OpenAI keys start with `sk-`" don't trigger.

If a real-world token shape escapes the scrubber, file an issue with a redacted reproduction. Adding a new pattern is a one-line append to `crates/coral-session/src/scrub.rs::PATTERNS` plus a regression test. The principle is "false positives are recoverable; false negatives leak credentials."

## Trust-by-curation gate

Distilled output **never** auto-merges into the wiki. Every emitted page carries `reviewed: false` in its frontmatter:

```yaml
---
slug: my-finding
type: synthesis
last_updated_commit: unknown
confidence: 0.4
status: draft
sources:
  - "src/lib.rs"
backlinks: []
reviewed: false
source:
  runner: "claude"
  prompt_version: 1
  session_id: "abc123-…"
  captured_at: "2026-05-08T10:00:00+00:00"
---
```

Two layers enforce review:

1. **`coral lint --rule unreviewed-distilled`** — surfaces a Critical issue for any page with `reviewed: false`. Critical issues flip `coral lint` to a non-zero exit, so any CI pipeline that runs lint will fail until the page is reviewed.

2. **Pre-commit hook** — the bundled `template/hooks/pre-commit.sh` runs `coral lint` against staged pages and refuses the commit if a `reviewed: false` page is staged. If you bypass with `git commit --no-verify` (please don't), CI will catch it.

Reviewers flip the flag manually:

```yaml
reviewed: true
```

Coral never flips it for you. That's the point.

## Why the answers we picked

The v0.20 PRD ([#16](https://github.com/agustincbajo/Coral/issues/16)) left six design questions open. Each was answered explicitly during implementation:

| # | Question | Answer | Reasoning |
|---|---|---|---|
| Q1 | Storage default | **Gitignored** raw + non-gitignored `distilled/` via `!` negation | Raw transcripts are PII-rich; curated distillations are project knowledge |
| Q2 | Privacy scrubbing | **On by default; opt-out needs `--yes-i-really-mean-it`** | "False positives recoverable; false negatives leak credentials" |
| Q3 | Distill output format | **Distill-as-page (option a) for MVP** | Distill-as-patch (option b) needs diff/merge UX out of MVP scope |
| Q4 | Trust gating | **`reviewed: false` + lint Critical + pre-commit hook** | Reuse the same v0.19.x machinery; don't reinvent |
| Q5 | Cross-format order | **Claude Code first; Cursor/ChatGPT stubs return clear errors** | Best-documented schema, highest user overlap |
| Q6 | MultiStepRunner usage | **Single-tier `Runner::run` for MVP** | Tiering is a v0.20.x optimization once we have data |

## Storage growth

A heavy Claude Code user generates 10s of MB / month. Captures are local, so the impact is on your dev machine, not git. If `du -sh .coral/sessions/` becomes uncomfortable:

- `coral session forget <id>` deletes a single session.
- Plain `rm -rf .coral/sessions/*.jsonl` drops every raw transcript while keeping the index intact (next `coral session list` will show "captured_path missing" but the rows survive — a future v0.20.x release will add `coral session prune` for this).

Distilled output is small (one Markdown page per finding) and is the durable artifact; the raw JSONL is the disposable evidence trail.

## See also

- `crates/coral-session/src/lib.rs` — module structure.
- `crates/coral-session/src/scrub.rs` — the scrubber + 25 regression tests.
- `crates/coral-cli/src/commands/session.rs` — CLI wiring.
- `crates/coral-cli/tests/session_e2e.rs` — end-to-end test of capture + list + show + forget + lint integration.
- [Issue #16](https://github.com/agustincbajo/Coral/issues/16) — original PRD-stub.
