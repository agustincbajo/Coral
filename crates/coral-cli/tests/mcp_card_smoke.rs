//! CLI smoke test for `coral mcp card` (v0.22.5).
//!
//! Spawns the binary with `mcp card`, captures stdout, and asserts the
//! body is pretty-printed JSON matching the spec D1 schema. Pins
//! acceptance criterion #5 (exit 0 + valid JSON of the same schema as
//! the HTTP body) at the binary boundary — not just at the library
//! layer the unit tests cover.
//!
//! AC #6 (stdout byte-equal to HTTP body modulo trailing newline) is
//! covered by the e2e suite for the HTTP side; this test pins the CLI
//! side so a future refactor can't drift the two surfaces apart.

use std::process::Command;

#[test]
fn cli_mcp_card_emits_json_to_stdout() {
    let coral = match cargo_bin_path() {
        Some(p) => p,
        None => {
            eprintln!("coral binary not built; skipping CLI smoke");
            return;
        }
    };

    // `coral mcp card` is a one-shot — no flags, prints to stdout, exits 0.
    let output = Command::new(&coral)
        .args(["mcp", "card"])
        .output()
        .expect("spawn coral mcp card");

    assert!(
        output.status.success(),
        "`coral mcp card` must exit 0; got status {:?}, stderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    // `println!` adds exactly one trailing `\n` after the pretty body.
    assert!(
        stdout.ends_with('\n'),
        "stdout must end with the trailing newline `println!` adds"
    );
    // Pretty output is multi-line, 2-space indented.
    assert!(
        stdout.contains('\n'),
        "card output must be multi-line (pretty JSON); got:\n{stdout}"
    );

    // Trim the trailing newline and confirm the body parses + matches
    // the spec D1 schema.
    let body = stdout.trim_end_matches('\n');
    let json: serde_json::Value = serde_json::from_str(body).expect("stdout must be valid JSON");

    // AC #3: top-level fields.
    assert_eq!(json["name"], "coral", "card.name must be 'coral'");
    let version = json["version"].as_str().expect("version is string");
    assert!(
        !version.is_empty(),
        "version must be non-empty (CARGO_PKG_VERSION)"
    );
    assert_eq!(
        json["protocolVersion"], "2025-11-25",
        "card.protocolVersion must be the spec freeze"
    );

    // Transports advertised: stdio + http (both compiled in since v0.21.1).
    let transports: Vec<&str> = json["transports"]
        .as_array()
        .expect("transports array")
        .iter()
        .map(|v| v.as_str().unwrap_or(""))
        .collect();
    assert!(transports.contains(&"stdio"));
    assert!(transports.contains(&"http"));

    // Capability counts are integers.
    let _ = json["capabilities"]["resources"]["count"]
        .as_u64()
        .expect("resources.count is integer");
    let _ = json["capabilities"]["tools"]["count"]
        .as_u64()
        .expect("tools.count is integer");
    let _ = json["capabilities"]["prompts"]["count"]
        .as_u64()
        .expect("prompts.count is integer");

    // x-coral namespace populated.
    assert_eq!(
        json["x-coral"]["ciStatus"], "green",
        "x-coral.ciStatus must be the literal 'green'"
    );
    let ts = json["x-coral"]["buildTimestamp"]
        .as_str()
        .expect("x-coral.buildTimestamp is string");
    assert!(
        !ts.is_empty(),
        "x-coral.buildTimestamp must be non-empty (was {ts:?})"
    );
}

/// Locate the `coral` binary in the cargo test harness's runtime
/// directory. Returns None if the binary wasn't built (matches the
/// idiom in `mcp_http_smoke.rs`).
fn cargo_bin_path() -> Option<std::path::PathBuf> {
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_coral") {
        let path = std::path::PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }
    None
}
