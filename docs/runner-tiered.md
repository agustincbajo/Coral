# Tiered runner (`MultiStepRunner`) — v0.21.4+

`coral consolidate` (and, in the future, other commands that drive a single
LLM call) can route through a **planner → executor → reviewer** pipeline
instead of a single end-to-end call. Each tier picks its own provider and
model, so a workflow can use a fast cheap planner (`haiku`), a stronger
executor for each sub-task (`sonnet`), and a final reviewer (`opus`) without
overpaying for the easy parts.

## When to enable

Tiered routing helps when the prompt is genuinely a *multi-step* task —
"summarize 60 wiki pages and propose consolidations" is the textbook case
because the planner can carve out independent groups, the executor can
operate on each group in isolation, and the reviewer can stitch the result.
For short, single-step prompts the overhead of three calls (and three
network round-trips) outweighs the gain. **Default = off.** Opt in with
`--tiered` (per-invocation) or with `[runner.tiered.consolidate] enabled = true`
in `coral.toml` (per-project).

## Manifest schema

```toml
# coral.toml — minimal tiered config for `coral consolidate`
apiVersion = "coral.dev/v1"

[project]
name = "demo"

[[repos]]
name = "self"
url = "git@github.com:acme/self.git"

# `[runner]` is optional. Omit the whole block for v0.21.3-shape behavior.
# `[runner.tiered]` is also optional. When present, all THREE tier
# subtables (`planner`, `executor`, `reviewer`) are required.
[runner.tiered.planner]
provider = "claude"      # claude | gemini | local | http
model    = "haiku"       # optional per-tier model override

[runner.tiered.executor]
provider = "claude"
model    = "sonnet"

[runner.tiered.reviewer]
provider = "claude"
model    = "opus"

# Optional. Default: `max_tokens_per_run = 200_000` (mirrors a Claude
# Sonnet 200K-context window). The cap applies to the cumulative
# token estimate across all calls in the run.
[runner.tiered.budget]
max_tokens_per_run = 50000

# Optional. Default: `enabled = false` (i.e. only the CLI `--tiered`
# flag enables tiered routing).
[runner.tiered.consolidate]
enabled = true
```

## CLI flag

```bash
# Force tiered routing for one run (CLI flag wins over manifest)
coral consolidate --tiered

# Tiered + verbose summary line at end of run
coral consolidate --tiered --verbose
```

`--tiered` requires a `[runner.tiered]` block in `coral.toml`. Passing
`--tiered` against a manifest with no tiered section is a hard error
(actionable message points the user at this doc).

## How it works

1. **Pre-flight budget gate.** Estimate the planner prompt's token cost via
   `len/4`, multiply by `1.5×`, and abort if the projection exceeds
   `max_tokens_per_run` before any network call lands. Returns
   `RunnerError::BudgetExceeded { actual, budget }`.
2. **Plan call.** The planner runs with `system = PLANNER_SYSTEM` and a
   user prompt that wraps your original task in a "decompose into 1-5
   sub-tasks" instruction. The planner emits YAML:
   ```yaml
   subtasks:
     - id: t1
       description: "scan for duplicate slugs"
     - id: t2
       description: "find pages with stale frontmatter"
   ```
3. **Fallback if the plan is unparseable.** If the planner's stdout can't
   be parsed as `subtasks:` YAML, Coral logs a `tracing::warn!` and falls
   back to a single-subtask pipeline whose description is the original
   user prompt.
4. **Executor calls (sequential).** One executor call per sub-task, with
   `system = EXECUTOR_SYSTEM` and the sub-task description as user prompt.
   Each call's pre-flight budget check runs against the cumulative
   `tokens_used` so far. Any executor error aborts the run.
5. **Reviewer call.** Final call with `system = REVIEWER_SYSTEM` and a user
   prompt of `format!("Original task:\n{original}\n\nSub-task results:\n{joined}", …)`.
   The reviewer's `RunOutput.stdout` becomes `TieredOutput::final_output.stdout`
   — i.e. it's the bytes the consolidate plan parser sees.

## Budget semantics

`runner.tiered.budget.max_tokens_per_run` is a **cumulative** cap: the
sum of `(system + user + stdout).len() / 4` across every call in the
run. The token estimator is pure-Rust `len/4` (ceiling-divided) — it is
not billing-accurate, just order-of-magnitude correct. The cap exists
to catch "I accidentally pasted a 5MB log file into the prompt" and
similar foot-guns, not to be a usage meter.

When a budget breach is projected, the run aborts with
`RunnerError::BudgetExceeded { actual, budget }`. `actual` is the
projected cumulative token count if the next call ran; `budget` is
the configured cap.

## Per-step timeouts

`Prompt::timeout` is a **per-call** wall-clock cap. A tiered run with
three calls each carrying `timeout = 60s` can take up to **180s
end-to-end**. v0.21.4 does not introduce a tiered-level timeout —
configure per-tier timeouts on the underlying runners if you need a
tighter envelope.

## Streaming

`MultiStepRunner` is non-streaming. v0.21.4 does not have a streaming
variant; if the underlying command supports `--stream` (e.g. a future
`coral consolidate --stream`), passing `--tiered` will fall back to a
non-streaming run.

## Errors

| Variant                            | When                                           |
| ---------------------------------- | ---------------------------------------------- |
| `RunnerError::BudgetExceeded`      | Cumulative token estimate exceeds the cap      |
| `RunnerError::NotFound`            | A tier's underlying runner binary is missing   |
| `RunnerError::AuthFailed`          | A tier's provider returned a 401-shaped error  |
| `RunnerError::Timeout`             | A single tier's call exceeded `Prompt::timeout` |
| `RunnerError::NonZeroExit`         | A tier's underlying runner exited non-zero    |
| `CoralError::Walk` (parse-time)    | `[runner.tiered]` is missing one of `planner|executor|reviewer`, or `budget.max_tokens_per_run = 0` |
| Provider-name parse error (build)  | A tier's `provider` field is not in `claude|gemini|local|http` |

## Compatibility

- A `coral.toml` with no `[runner]` section round-trips byte-identically
  to its v0.21.3 form. Default `RunnerSection { tiered: None }` emits
  zero bytes from `render_toml`.
- Every existing `Runner` impl (`ClaudeRunner`, `GeminiRunner`,
  `LocalRunner`, `HttpRunner`, `MockRunner`) compiles unchanged —
  `MultiStepRunner` is an additive trait, not a method on `Runner`.
- `RunnerError::BudgetExceeded` is an additive variant; existing
  `matches!` patterns in the codebase keep working.
