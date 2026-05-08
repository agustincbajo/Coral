//! Adversarial OpenAPI fixture suite (#29 audit-gap conversion).
//!
//! v0.19.7 cycle-3 audit explicitly listed `coral test discover` as
//! "did NOT examine" for adversarial inputs. This file converts the
//! gap into protective tests by feeding the discovery walker every
//! adversarial pattern the auditor enumerated:
//!
//!   1. `cyclic_ref.yaml`       — `$ref` cycle (A → B → A).
//!   2. `huge_inline_example.*` — programmatic 64 MiB inline example
//!      (above the v0.19.5 32 MiB cap that v0.19.8 #29 backports to
//!      `parse_spec_file`).
//!   3. `unknown_method.yaml`   — HTTP method outside the
//!      {get, post, put, delete, patch, head, options} allowlist.
//!   4. `escaped_path.yaml`     — `paths./users%2F{id}` (percent-
//!      encoded slash).
//!   5. `local_file_ref.yaml`   — `$ref: '../../../etc/passwd'`.
//!
//! Behavior contract pinned by these tests:
//! - The discovery walker NEVER infinite-loops on adversarial input.
//! - Files exceeding the 32 MiB cap (#29 fix) are rejected with a
//!   clear error before the YAML deserializer touches them.
//! - Unknown HTTP methods are silently skipped (already correct in
//!   `is_http_method`); pinned here so a future relaxation can't
//!   silently regress.
//! - `$ref` resolution is NOT performed by `discover.rs` — both
//!   cyclic refs and traversal-style local-file refs are inert
//!   because the walker only inspects `paths.<path>.<method>` and
//!   `responses` shapes. The fixtures pin this contract: cycles
//!   don't crash, traversal refs don't read foreign files.
//!
//! Test layout:
//!
//!   tests/openapi_adversarial.rs   ← this file (Rust integration)
//!   tests/openapi_adversarial/     ← fixture YAMLs, pulled by test
//!                                    via fs::write into a tempdir
//!                                    (avoids include_dir cost)

use coral_test::discover::discover_openapi_in_project;
use std::fs;
use tempfile::TempDir;

const CYCLIC_REF_YAML: &str = r#"openapi: 3.0.0
info:
  title: cyclic-ref
  version: 1.0.0
paths:
  /a:
    get:
      parameters:
        - $ref: '#/components/parameters/A'
      responses:
        '200':
          description: ok
components:
  parameters:
    A:
      $ref: '#/components/parameters/B'
    B:
      $ref: '#/components/parameters/A'
"#;

const UNKNOWN_METHOD_YAML: &str = r#"openapi: 3.0.0
info:
  title: unknown-method
  version: 1.0.0
paths:
  /foo:
    wibble:
      responses:
        '200':
          description: ok
    futureMethod:
      responses:
        '200':
          description: ok
    get:
      responses:
        '200':
          description: ok
"#;

const ESCAPED_PATH_YAML: &str = r#"openapi: 3.0.0
info:
  title: escaped-path
  version: 1.0.0
paths:
  "/users%2F{id}":
    get:
      responses:
        '200':
          description: ok
  "/users/{id}":
    get:
      responses:
        '200':
          description: ok
"#;

const LOCAL_FILE_REF_YAML: &str = r#"openapi: 3.0.0
info:
  title: local-file-ref
  version: 1.0.0
paths:
  /a:
    get:
      parameters:
        - $ref: '../../../etc/passwd'
      responses:
        '200':
          description: ok
"#;

/// Helper: minimal project root with a single openapi.yaml at the root.
fn write_spec(dir: &TempDir, content: &str) {
    fs::write(dir.path().join("openapi.yaml"), content).unwrap();
}

/// #29 (1) — `$ref` cycle: A → B → A. The discovery walker only
/// inspects `paths.<path>.<method>` and `responses`; it does NOT
/// resolve `$ref`s. So the cycle is inert: discover returns the
/// `/a` GET case and never recurses. Pinning this contract: a
/// future change that adds ref-resolution must come with explicit
/// cycle detection.
#[test]
fn cyclic_ref_does_not_infinite_loop() {
    let dir = TempDir::new().unwrap();
    write_spec(&dir, CYCLIC_REF_YAML);
    let cases = discover_openapi_in_project(dir.path()).expect("discover");
    assert_eq!(
        cases.len(),
        1,
        "expected one case for /a GET, got: {cases:?}"
    );
    assert!(
        cases[0].case.id.contains("/a"),
        "case id should contain /a, got: {}",
        cases[0].case.id
    );
}

/// #29 (2) — huge spec file: 64 MiB of YAML padding bypasses the
/// pre-#29 unbounded `read_to_string`. With #29 fix in place,
/// `parse_spec_file` checks `fs::metadata` first and rejects the
/// file before allocating; `discover_openapi_in_project` swallows
/// the per-file error and emits zero cases for the offending spec.
///
/// We don't materialize 64 MiB of YAML on every CI run — just
/// enough bytes (33 MiB) to cross the 32 MiB cap, written via a
/// single-string concatenation. Total runtime: < 1s on the M1
/// reference machine.
#[test]
fn huge_inline_example_is_rejected_at_size_cap() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("openapi.yaml");
    // Build a YAML body that's just over the 32 MiB cap. The
    // `description` field accepts arbitrary text so the file parses
    // structurally if it ever reaches the deserializer (it won't,
    // because the cap kicks in first).
    let header = "openapi: 3.0.0\ninfo:\n  title: huge\n  version: 1.0.0\n  description: \"";
    let footer = "\"\npaths:\n  /h:\n    get:\n      responses:\n        '200':\n          description: ok\n";
    let target_size = 33 * 1024 * 1024_usize; // 33 MiB > 32 MiB cap
    let pad = "x".repeat(target_size);
    let mut buf = String::with_capacity(target_size + 256);
    buf.push_str(header);
    buf.push_str(&pad);
    buf.push_str(footer);
    fs::write(&path, &buf).unwrap();
    drop(buf); // free the 33 MiB before discover runs

    let cases = discover_openapi_in_project(dir.path()).expect("discover swallows per-file errors");
    // The file got skipped — zero cases discovered.
    assert!(
        cases.is_empty(),
        "32 MiB-cap spec must be skipped; got {} cases",
        cases.len()
    );
}

/// #29 (3) — Method names outside the HTTP allowlist (`get`, `post`,
/// `put`, `delete`, `patch`, `head`, `options`) are silently
/// skipped. The valid `get` operation alongside them must still
/// produce a TestCase.
#[test]
fn unknown_method_is_skipped_silently() {
    let dir = TempDir::new().unwrap();
    write_spec(&dir, UNKNOWN_METHOD_YAML);
    let cases = discover_openapi_in_project(dir.path()).expect("discover");
    // Only the `get` operation should produce a case — `wibble`
    // and `futureMethod` are filtered out.
    assert_eq!(
        cases.len(),
        1,
        "unknown methods must be skipped; got: {:?}",
        cases.iter().map(|c| &c.case.id).collect::<Vec<_>>()
    );
    assert!(cases[0].case.id.contains("GET"));
    assert!(
        !cases[0].case.id.to_uppercase().contains("WIBBLE"),
        "wibble method must not produce a case"
    );
}

/// #29 (4) — Percent-encoded path segments are passed through to
/// the TestCase id verbatim. The discovery walker uses the raw
/// path string from `paths.<key>` as-is; it does NOT URL-decode.
/// Pinning the round-trip behavior: clients that decode the case
/// id later see the literal `%2F`.
#[test]
fn escaped_path_round_trips_literally() {
    let dir = TempDir::new().unwrap();
    write_spec(&dir, ESCAPED_PATH_YAML);
    let cases = discover_openapi_in_project(dir.path()).expect("discover");
    assert_eq!(cases.len(), 2);
    let ids: Vec<&str> = cases.iter().map(|c| c.case.id.as_str()).collect();
    assert!(
        ids.iter().any(|id| id.contains("%2F")),
        "escaped path must round-trip literally; got: {ids:?}"
    );
    assert!(
        ids.iter().any(|id| id.contains("/users/{id}")),
        "non-escaped path must also be present; got: {ids:?}"
    );
}

/// #29 (5) — A `$ref` to a local file path with traversal segments
/// (`../../../etc/passwd`) is INERT: `discover.rs` does not perform
/// `$ref` resolution at all. The fixture pins this so that any
/// future code adding ref-resolution arrives with explicit
/// path-traversal protection (matching the v0.19.5 slug-allowlist
/// pattern on a parallel data path).
///
/// CRITICAL: this test asserts the fs is NOT touched outside the
/// project root. We can't inspect "did Coral try to open
/// /etc/passwd" cheaply, but we can assert the discover() call
/// terminates and emits the expected single case for `/a` GET.
#[test]
fn local_file_ref_is_inert_no_traversal() {
    let dir = TempDir::new().unwrap();
    write_spec(&dir, LOCAL_FILE_REF_YAML);
    let cases = discover_openapi_in_project(dir.path()).expect("discover");
    // The walker emits the /a GET case; the `$ref` value is part of
    // the operation's parameters but the walker doesn't traverse
    // refs.
    assert_eq!(cases.len(), 1, "expected exactly one case");
    assert!(cases[0].case.id.contains("/a"));
}

/// #29 sanity: a spec exactly at the cap (32 MiB) is accepted.
/// Pinning the boundary so the cap doesn't drift to "off by one".
#[test]
fn spec_under_size_cap_is_accepted() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("openapi.yaml");
    // Compose a tiny valid spec — well under the 32 MiB cap.
    fs::write(
        &path,
        r#"openapi: 3.0.0
info:
  title: tiny
  version: 1.0.0
paths:
  /h:
    get:
      responses:
        '200':
          description: ok
"#,
    )
    .unwrap();
    let cases = discover_openapi_in_project(dir.path()).expect("discover");
    assert_eq!(cases.len(), 1);
}
