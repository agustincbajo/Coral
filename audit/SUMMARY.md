# Coral v0.30.0 — Multi-agent audit summary (2026-05-11)

## Methodology

1. Baseline build attempted on Windows (could not complete locally — see
   finding #001). CI is Ubuntu/macOS only, so audit was code-only.
2. Five parallel domain agents (security, concurrency, MCP server,
   CLI UX, test quality) reviewed the codebase against the README's
   hardening claims.
3. Top-severity findings were counter-validated by direct code reads
   AND by the natural overlap between agents — finding #002 (WikiState
   stale) was independently identified by both the MCP agent and the
   concurrency agent, from different starting points.
4. Findings written to `audit/findings/NNN-slug.md` for batch upload
   via `gh issue create` once authentication is established.

## High-confidence findings (open as issues)

| ID  | Severity | Title                                                                                | Confirmed |
|-----|----------|--------------------------------------------------------------------------------------|-----------|
| 001 | Low      | Windows-GNU build fails without `dlltool.exe` (no docs, no CI)                       | 5/5       |
| 002 | High     | MCP `WikiState` dirty-flag wired to watcher but never consulted on `resources/read`  | 5/5 ×2    |
| 003 | High     | `distill_patch::diff_targets_slug` only checks first `---`/`+++` pair (path traversal) | 4/5       |
| 004 | High     | `coral guarantee` returns Verdict::Green when wiki is unreadable                     | 5/5       |
| 005 | High     | `coral project lock` writes `coral.lock` outside flock — lost-update vs `project sync`| 5/5       |
| 006 | Medium   | BM25 search-index non-atomic write + `compute_content_hash` doc/code mismatch        | 5/5       |
| 007 | Medium   | `coral stats --format json` HashMap-ordered keys → non-deterministic                 | 5/5       |
| 008 | Medium   | MCP all errors collapse to `-32601` regardless of cause (JSON-RPC §5.1)              | 5/5       |
| 009 | Low      | README MCP counts drift (says 6 res/5 tools; code has 8 res/7 tools)                 | 5/5       |
| 010 | Medium   | MCP HTTP/SSE never drains notifications nor honors `Last-Event-ID`                   | 5/5       |
| 011 | Medium  | Batch: 12 medium/low items (mcp serve SIGINT, exit codes, watcher debounce, audit-log rotation, Content-Type validation, initialize sniff, bootstrap exit, ingest read cap, ClaudeRunner --, test assertions, CWD_LOCK, feature CI) | varies |

## Strong points

Five hardening claims **held up** under audit:

- Slug allowlist (`is_safe_filename_slug` / `is_safe_repo_name`) is tight
  and applied at every interpolation site checked.
- `git` invocations correctly use `--` separator for user-controlled refs;
  flag-shaped refs pre-rejected with regression tests.
- Secret scrubbing covers `Authorization: Bearer`, `x-api-key:`, bare
  `Bearer <tok>`; every `RunnerError` Display routes through it.
- API key via stdin / body via tempfile mode 0600 with RAII guard is
  correctly implemented and regression-tested.
- MCP HTTP transport defaults to 127.0.0.1, Origin allowlist limited to
  localhost, body capped at 4 MiB, batch JSON-RPC rejected.

Test quality is far above the "tests just exercise code paths" baseline:
- `crates/coral-cli/tests/cross_process_lock.rs` runs real cross-process
  subprocess matrices.
- `bc_regression.rs` asserts byte-exact stdout fragments.
- Cross-runner contract suite drives all 5 runner impls against a
  shared matrix.
- `WikiLog::append_atomic` race-free claim is genuinely tested.

## What I could NOT verify locally

- `cargo build --workspace` — local Windows toolchain missing
  prerequisites (finding #001). The 5 agents read source directly.
- `cargo test --workspace` — same blocker.
- `cargo clippy -- -D warnings` — same blocker.
- End-to-end smoke of CLI flows (`init`, `ingest`, `query`, etc.) —
  no built binary on this host.

These should be run in CI as a follow-up; their passing on Ubuntu (per
`.github/workflows/ci.yml`) gives some assurance but does not substitute
for re-running after audit fixes land.

## Submitting the issues

```powershell
gh auth login
Get-ChildItem audit/findings/*.md | ForEach-Object {
    $body = Get-Content $_.FullName -Raw
    $title  = [regex]::Match($body, '(?m)^title:\s*"(.+)"$').Groups[1].Value
    $labels = [regex]::Match($body, '(?m)^labels:\s*(.+)$').Groups[1].Value -replace '\s', ''
    gh issue create --title $title --body $body --label $labels
}
```

Or open them one by one in the order shown above (002→011) so the
high-severity issues land first.
