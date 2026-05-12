# tiny-greeter

A deliberately tiny Rust library used as a fixture for Coral's
end-to-end Ollama bootstrap test (FR-ONB-28).

## What it does

Exposes two functions:

- `greet(name: &str) -> String` — returns `"Hello, <name>!"`.
- `shout(name: &str) -> String` — same, but uppercased and with three exclamation marks.

The goal is ~50 LOC total across `src/lib.rs` and `src/main.rs` so a
local-LLM bootstrap (Ollama with `llama3.1:8b`) completes within the
test's 10-minute budget on a developer laptop without GPU acceleration.

## Layout

```
tiny-repo-50loc/
├── README.md      <- this file
├── Cargo.toml     <- crate manifest (NOT a workspace member)
├── src/
│   ├── lib.rs     <- the two functions + unit tests
│   └── main.rs    <- 5-line CLI wrapper around greet()
```

The `Cargo.toml` is intentionally NOT part of the Coral workspace —
`tests/fixtures/**` is in the workspace exclude list — so `cargo build`
of the outer workspace does NOT try to compile this fixture.
