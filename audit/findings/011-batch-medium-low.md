---
title: "audit batch: medium/low-severity findings from v0.30.0 audit (one issue tracking 11 items)"
severity: Medium
labels: bug, audit-followup
confidence: varies
---

This is an umbrella issue grouping the medium/low-severity findings
from the v0.30.0 multi-agent audit so the tracker isn't spammed.
Each item below has its severity, confidence, and concrete location.

---

### B1 — `coral mcp serve --transport http` lacks SIGINT/SIGTERM handler
- Severity: Medium · Confidence: 4
- `crates/coral-cli/src/commands/mcp.rs:302-347` — both transport
  paths skip `install_shutdown_handler()`. Stdio gracefully exits on
  EOF, so the gap mostly bites HTTP/SSE: Ctrl-C tears down mid-request,
  no `done` frame, no flush.
- `serve.rs:62-64`, `interface.rs:105`, `monitor/up.rs:168` already
  show the correct pattern using `signal-hook` (already a workspace dep).

### B2 — `lint`, `verify`, `contract check` collapse "found issues" and "tool crashed" to exit 1
- Severity: Medium · Confidence: 4
- `commands/lint.rs:280-284`, `commands/verify.rs:72-76`,
  `commands/contract.rs:93-101`. CI can't distinguish a backend-down
  crash from real findings. `commands/test.rs` already uses
  `ExitCode::from(2)` for usage errors — define the contract
  (0=clean, 1=findings, 2=usage, 3=internal) and apply it consistently.

### B3 — `coral interface watch` no own-write debounce, second-precision mtime
- Severity: Medium · Confidence: 4
- `commands/interface.rs:127-165, 196-203`. `mtime_secs` truncates to
  whole seconds → sub-second writes drop events. No protection
  against the watcher's downstream consumer writing back into `.wiki/`,
  causing unbounded re-emission.

### B4 — Audit log rotation not crash-safe
- Severity: Medium · Confidence: 5
- `crates/coral-cli/src/commands/mcp.rs:428-445`. `remove_file` +
  `rename` are two non-atomic syscalls without parent-dir fsync.
  Concurrent dispatcher threads under HTTP transport can race the
  rotation; appends between metadata-check and rename can be lost.
- The audit log lives in `coral-cli`, not in `coral-mcp` — tools
  dispatched through `NoOpDispatcher` or any library consumer get no
  audit trail.

### B5 — HTTP transport: `POST /mcp` does not validate `Content-Type`
- Severity: Medium · Confidence: 5
- `crates/coral-mcp/src/transport/http_sse.rs:285-378`. MCP
  Streamable HTTP spec requires `application/json` on POST. Should
  return `415 Unsupported Media Type`.

### B6 — HTTP transport: `initialize` detection uses substring match on body
- Severity: Medium · Confidence: 4
- `crates/coral-mcp/src/transport/http_sse.rs:348-358`.
  `body.contains("\"initialize\"")` false-positives on `tools/call`
  arguments containing that literal token (e.g., a prompt that
  mentions the word "initialize"). Pass the parsed method instead.

### B7 — `coral bootstrap` / `coral ingest` skip-on-error returns SUCCESS even if every entry failed
- Severity: Low · Confidence: 4
- `commands/bootstrap.rs:148-155, 288-296` (similar in `ingest.rs`).
  If `created == 0 && !skipped.is_empty()`, exit code should be
  non-zero. Or add `--strict`.

### B8 — `coral-cli/src/commands/ingest.rs:69, 241` reads `.wiki/index.md` without 32 MiB cap
- Severity: Low · Confidence: 3
- README claims "32 MiB cap on every `read_to_string` of user-supplied
  content." These two `read_to_string` calls are uncapped.
  `coral-core/src/walk.rs:117` and `coral-test/src/discover.rs:140`
  show the correct pattern.

### B9 — `ClaudeRunner` passes `prompt.user` as positional without `--` separator
- Severity: Low · Confidence: 2
- `crates/coral-runner/src/runner.rs:289-300, 368-379`. If
  `prompt.user` starts with `--`, Claude CLI may parse it as a flag.
  `GeminiRunner` / `LocalRunner` use `-p <value>` (immune).

### B10 — 9 sites of `assert!(result.is_ok())` discard `Err` context in tests
- Severity: Low (test DX) · Confidence: 5
- `coral-env/src/healthcheck.rs:126`,
  `coral-cli/src/commands/mcp.rs:691,697`,
  `coral-cli/src/commands/project/doctor.rs:233`,
  `coral-cli/src/commands/project/list.rs:129`,
  `coral-cli/src/commands/project/lock.rs:126,172`,
  `coral-cli/src/commands/project/new.rs:133,186`.
  Replace with `.expect("…")` so the actual error surfaces in CI logs.

### B11 — Test isolation: `CWD_LOCK` adoption asymmetric
- Severity: Low (test infra) · Confidence: 4
- `crates/coral-cli/src/commands/mcp.rs:684` does
  `std::env::set_current_dir` without acquiring `CWD_LOCK`. Other
  call sites (`project/new.rs:143,167`) do acquire it. Process-wide
  CWD is global; mixed adoption races under `cargo test` parallelism.
- Also: `runner_helper.rs:160,165` does `env::set_var` / `remove_var`
  pair without a guard struct → env var leaks if a panic/return
  occurs between them.

### B12 — `tantivy` / `pgvector` feature-gated code has no CI coverage
- Severity: Low · Confidence: 4
- `crates/coral-core/src/tantivy_backend.rs` and `pgvector.rs` live
  under `#[cfg(feature = "...")]`. The CI test job runs
  `cargo test --workspace --all-features` per `.github/workflows/ci.yml:54`
  — wait, double-check that. If not, those modules' inline tests
  never run. Add a CI matrix row with `--all-features` to cover them.

---

Open these as separate issues only as the team plans to work on each.
Or use this umbrella as a TODO checklist.
