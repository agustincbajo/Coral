# ADR 0008 — Multi-provider Runner and Embeddings traits

**Date:** 2026-05-01
**Status:** accepted (v0.4 + v0.5)

## Context

By v0.3, Coral had a single LLM execution path (`ClaudeRunner` shelling to
`claude --print`) and a single embeddings path (a private `voyage` module
in `coral-cli` shelling to `curl`). Both were hard-wired.

Three pressures drove the v0.4 + v0.5 work:

1. **`GeminiRunner` was a stub** (`ClaudeRunner::with_binary("gemini")`)
   that didn't actually work because gemini-cli's flags differ from
   claude's (`-p` vs `--print`, no `--append-system-prompt`).
2. **Voyage hard-wire** meant offline / OpenAI-only users had no
   embeddings option, even though the on-disk cache schema (`provider`
   field) was already designed to be provider-agnostic.
3. **Coral runs from inside Claude Code** (a common path for
   maintainers) where the parent's `ANTHROPIC_API_KEY` doesn't reach
   the subprocess. Without an alternative provider, those users were
   blocked outright.

## Decision

**Two traits, four runners, three embeddings providers.**

```
coral-runner/src/
├── runner.rs          ← Runner trait + ClaudeRunner + helpers
├── gemini.rs          ← GeminiRunner (real, not a stub)
├── local.rs           ← LocalRunner (llama.cpp / llama-cli)
├── mock.rs            ← MockRunner (tests)
└── embeddings.rs      ← EmbeddingsProvider trait
                          + VoyageProvider + OpenAIProvider + Mock
```

**Runner trait** (unchanged shape):

```rust
pub trait Runner: Send + Sync {
    fn run(&self, prompt: &Prompt) -> RunnerResult<RunOutput>;
    fn run_streaming(&self, prompt: &Prompt, on_chunk: &mut dyn FnMut(&str))
        -> RunnerResult<RunOutput>;
}
```

**EmbeddingsProvider trait** (new in v0.4):

```rust
pub trait EmbeddingsProvider: Send + Sync {
    fn name(&self) -> &str;       // cache-key identity
    fn dim(&self) -> usize;       // vector dimension
    fn embed_batch(&self, texts: &[String], input_type: Option<&str>)
        -> EmbedResult<Vec<Vec<f32>>>;
}
```

**Provider selection** lives in `coral-cli`:

```rust
// runner_helper.rs — `--provider <claude|gemini|local>`
match provider {
    ProviderName::Claude => Box::new(ClaudeRunner::new()),
    ProviderName::Gemini => Box::new(GeminiRunner::new()),
    ProviderName::Local  => Box::new(LocalRunner::new()),
}

// search.rs — `--embeddings-provider <voyage|openai>`
match args.embeddings_provider.as_str() {
    "voyage" => Box::new(VoyageProvider::new(api_key, model, dim)),
    "openai" => Box::new(OpenAIProvider::new(api_key, model, dim)),
    ...
}
```

## Why two traits, not one

Runners and embeddings providers are different kinds of services:

| Aspect | Runner | EmbeddingsProvider |
|---|---|---|
| Output shape | streaming text | dense `Vec<f32>` |
| Latency profile | seconds-to-minutes per call | tens of ms per batch |
| Caching | none (each call hits the network) | mtime-keyed JSON file |
| Auth | per-vendor OAuth or API key | per-vendor API key |
| Streaming | yes (`run_streaming`) | no — batches only |
| Failure modes | timeouts, auth, model-overload | rate limit, dim mismatch |

Forcing both into one trait would mean `embed_batch` returning a stream
of nothing (silly) and `run` returning vectors-as-bytes (wrong shape).

## Why each runner is its own struct, not a config-driven dispatcher

Initial v0.2 stub did that:

```rust
GeminiRunner { inner: ClaudeRunner::with_binary("gemini") }
```

But the gemini CLI takes `-p`, not `--print`; LocalRunner needs
`--no-display-prompt`; future runners may need entirely different
spawn shapes (e.g. an HTTP-based runner with no subprocess at all).
Each as its own type:

- Lets `build_args` be a pure, testable function per runner.
- Forces flag conventions to live next to the binary they target.
- Keeps `with_binary` as a single escape hatch (point at a wrapper
  script if your install diverges).

Code duplication is real (~80 lines of spawn/wait/timeout per runner)
but mitigated via the shared `run_streaming_command` helper extracted
in v0.5 — runners now build their `Command`, the helper handles I/O.

## Why `EmbeddingsError` is its own enum, not `RunnerError`

`RunnerError::AuthFailed` carries a hardcoded "Run `claude setup-token`"
hint. Reusing that variant for Voyage 401s would surface a wrong
suggestion. Two enums, two messages — both go through
`thiserror::Display` so the CLI just prints `{e}`.

## Consequences

**Positive:**
- Adding a fourth runner (OpenAI Responses, vLLM, etc.) is one new file
  in `coral-runner/src/` + 1 line in `make_runner`.
- Adding a third embeddings provider is one new struct in
  `embeddings.rs` + 1 match arm in `build_embeddings_provider`.
- Tests run against `MockRunner` / `MockEmbeddingsProvider` — no network
  in the unit-test layer.
- `coral` can run fully offline (`--provider local --engine tfidf`).
- Auth failures surface actionable messages tailored to each provider.

**Negative:**
- Code duplication across runners (~80 LOC each before the streaming
  helper was extracted; ~40 LOC after). Acceptable for a 4-runner system;
  worth revisiting at 8+.
- Each provider's flag convention must be hardcoded somewhere; the
  `with_binary` wrapper-script escape hatch helps but isn't a full fix.

## Alternatives considered

- **One `Runner` trait with embeddings as a method**: rejected, see
  "Why two traits" above.
- **`async_trait` + tokio-based runners**: rejected to keep Coral a
  sync, single-binary CLI. The streaming use case is solved by the
  `mpsc::channel` + reader-thread pattern in `run_streaming_command`.
- **Plug-in architecture (dynamic loading)**: massively overengineered
  for a system with 4 runners and 3 providers. Rust's static dispatch
  via `Box<dyn Runner>` is the right tool here.
- **External provider crates** (one crate per provider): would force
  consumers to opt in via Cargo features. Premature; the binary is still
  ~3 MB stripped.

## Future evolution

- **Anthropic embeddings provider** when Anthropic ships the API.
- **A third axis** if local LLMs grow a meaningful streaming protocol
  (vLLM, Ollama HTTP) — likely a `HttpRunner` that doesn't spawn
  subprocesses at all.
- **Provider-specific timeouts**: today `Prompt.timeout` is a single
  `Duration`. Network-bound providers may want different defaults than
  subprocess-spawning ones.
