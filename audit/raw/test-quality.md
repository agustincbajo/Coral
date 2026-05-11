# Raw test-quality audit (agent aa229f15ef5ae6fd9)

## F2 (medium): 9 sites of assert!(result.is_ok()) swallowing error context
- crates/coral-env/src/healthcheck.rs:126
- crates/coral-cli/src/commands/mcp.rs:691,697
- crates/coral-cli/src/commands/project/doctor.rs:233
- crates/coral-cli/src/commands/project/list.rs:129
- crates/coral-cli/src/commands/project/lock.rs:126,172
- crates/coral-cli/src/commands/project/new.rs:133,186

## F6/F12 (medium): CWD_LOCK adoption asymmetric → flaky parallel tests
- crates/coral-cli/src/commands/project/new.rs:143,167 acquires lock before set_current_dir
- crates/coral-cli/src/commands/mcp.rs:684 does NOT acquire lock before set_current_dir
- crates/coral-cli/src/commands/runner_helper.rs:160,165 set_var + remove_var without guard, leaks on early return/panic

## F8 (medium): tests gated `#[ignore]` that probably should run in CI
- crates/coral-core/src/git_remote.rs:462 sync_repo_clones_a_local_bare_repo
- crates/coral-core/src/gitdiff.rs:302,335 run_against_real_repo / head_sha_returns_40_char_hex

## F9 (medium): tantivy/pgvector features have inline #[cfg(test)] modules but no `cargo test --all-features` CI row
- crates/coral-core/src/tantivy_backend.rs and pgvector.rs entire modules under feature gates

## F4 (medium): proptest slug strategies happy-path only — no UTF-8, no `.`/`_`/leading/trailing-`-`/empty
- crates/coral-core/tests/proptest_index.rs:59,69-71
- crates/coral-core/tests/proptest_search.rs:40
- crates/coral-core/tests/proptest_lint.rs:33
- confidence_strategy uses n/100 → 2-decimal round-trip exact by construction; serializer mis-rounding would not be caught

## F10 (medium, conf 3): openapi_adversarial.rs:172 33-MiB cap test asserts only `cases.is_empty()`, doesn't verify rejection-stage (size cap vs deserialize)

## Strong (no bug)
- F1: tests assert behavior not just no-panic
- F5: concurrency tests use real cross-process subprocess matrices
- F7: bc_regression fixtures make exact-equals assertions
- F3: MockRunner well-engineered (closed audit gap #40)

## Observations (not bugs)
- F3 second half: no cross_test_runner_contract.rs analog
- F11: coral-stats has no tests/ dir, only inline unit tests (acceptable for pure-functional)
