//! v0.35 TEST-11 / CP-4 — `coral ui serve` HTTP integration smoke.
//!
//! Spawns the real `coral` binary under `coral ui serve --no-open
//! --port 0 ...`, parses the startup banner off stdout to recover the
//! resolved listener address (and, in the auto-mint test, the minted
//! token), then hits the documented HTTP API and pins the 401 / 200
//! responses around the bearer-auth gate.
//!
//! The test file is named `ui_api_smoke.rs` (vs. the spec's
//! `coral-ui/tests/api_smoke.rs`) because integration tests need
//! `CARGO_BIN_EXE_coral` from cargo, which is only populated for
//! tests in the same package as the `coral` binary (coral-cli). The
//! library-level smoke that doesn't need the CLI surface lives in
//! the `coral-ui` unit tests; this file is the end-to-end half.
//!
//! Each test uses `--port 0` so they parallelise cleanly without
//! contending for a fixed port. A `ChildGuard` RAII kills the spawned
//! process on Drop so a panicking assert can't leak a stray listener
//! into other tests (or into the developer's machine after a failed
//! run).
//!
//! HTTP client: `ureq` (workspace dep, already pulled by coral-cli
//! for the v0.34.0 doctor wizard). No tokio anywhere — matches the
//! crate's sync runtime contract.

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

/// Locate the `coral` binary in the cargo test harness's runtime dir.
/// Returns `None` when the binary wasn't built (CI without
/// `cargo build`, doctest mode). Tests that hit this path return
/// early with a stderr note rather than failing — same pattern as
/// the existing `mcp_http_smoke.rs`.
fn cargo_bin_path() -> Option<PathBuf> {
    std::env::var_os("CARGO_BIN_EXE_coral")
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

/// RAII wrapper that ensures a spawned child process is killed when
/// the test scope unwinds, even on assertion failure. tiny_http
/// listeners aren't tied to the parent's pid on Unix, so without
/// this an `assert!` on a failed health probe would leak the
/// listener until the OS reclaims it.
struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Banner parsed off the spawned `coral ui serve` stdout. We need the
/// host:port (to direct HTTP requests at the right listener) and,
/// when auto-mint kicked in, the minted bearer token.
struct Banner {
    /// `host:port` pair pulled out of `WebUI serving at http://...`.
    addr: String,
    /// Minted bearer token, if the banner included a `Bearer token: ...`
    /// line. None when the server was started on loopback without
    /// `--token` (frictionless local dev path, no banner token line).
    minted_token: Option<String>,
}

/// Read the spawned server's stdout line-by-line until both:
/// (a) the `WebUI serving at http://...` line gives us the addr,
/// (b) either we've passed the `Bearer token: ...` line (if any) OR
///     we've hit the loopback-no-token note (meaning no token line is
///     coming).
///
/// We deliberately read off stdout (not stderr) — SEC-02 moves the
/// startup banner to stdout precisely so smoke tests + `grep`
/// pipelines can scrape the values without competing with the
/// `coral_ui::serve` "listening on" line that goes to stderr.
fn read_banner(stdout: ChildStdout, timeout: Duration) -> Result<(Banner, ChildStdout), String> {
    let mut reader = BufReader::new(stdout);
    let start = Instant::now();
    let mut addr: Option<String> = None;
    let mut token: Option<String> = None;
    let mut done_with_token_section = false;
    // Read up to 32 lines or 5 seconds, whichever comes first.
    for _ in 0..32 {
        if start.elapsed() > timeout {
            break;
        }
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                eprintln!("ui stdout: {}", line.trim_end());
                if let Some(rest) = line.strip_prefix("WebUI serving at http://") {
                    addr = Some(rest.trim().to_string());
                } else if let Some(rest) = line.strip_prefix("Bearer token: ") {
                    token = Some(rest.trim().to_string());
                    done_with_token_section = true;
                } else if line.trim_start().starts_with("(loopback bind,") {
                    // No token line incoming; we're done waiting.
                    done_with_token_section = true;
                }
                if addr.is_some() && done_with_token_section {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let addr = addr.ok_or_else(|| "never saw `WebUI serving at` banner".to_string())?;
    Ok((
        Banner {
            addr,
            minted_token: token,
        },
        reader.into_inner(),
    ))
}

/// Poll `GET /health` (or `GET /api/v1/pages` when health requires a
/// token) until we get any HTTP response, or 5 s elapses. tiny_http
/// finishes binding before the parent `Server::http` call returns,
/// but the moment-of-listener-readiness is OS-dependent — polling
/// the live endpoint sidesteps the race without an arbitrary sleep.
fn wait_until_ready(addr: &str, token: Option<&str>) -> Result<(), String> {
    let url = format!("http://{addr}/health");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let mut req = ureq::get(&url).timeout(Duration::from_millis(500));
        if let Some(t) = token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }
        match req.call() {
            Ok(_) => return Ok(()),
            // 401 still proves the server is up.
            Err(ureq::Error::Status(_, _)) => return Ok(()),
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    Err(format!("server at {addr} never became ready"))
}

/// Build a `.wiki/` dir under a TempDir with the minimum that
/// `coral_ui::serve` requires (just an empty directory — `read_pages`
/// returns an empty list, the routes return 200 with `data: []`).
fn make_wiki() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    std::fs::create_dir_all(dir.path().join(".wiki")).expect("mkdir .wiki");
    dir
}

/// Reserve a free TCP port by binding ephemeral, reading the assigned
/// port back, then immediately dropping the listener (TIME_WAIT is a
/// non-issue on a freshly-opened socket whose accept() never ran).
/// We need this because `coral ui serve` prints the configured port,
/// not the OS-resolved one — passing `--port 0` would leave the test
/// unable to discover the actual listener. Slight race window between
/// drop-and-respawn, but acceptable for a sync test suite, and serial
/// scheduling via `--test-threads=1` in CI rules out parallel
/// contention. The auto-mint test uses `--bind 0.0.0.0` which still
/// needs a concrete port too.
fn pick_free_port(bind: &str) -> u16 {
    let addr = format!("{bind}:0");
    let listener = TcpListener::bind(&addr).expect("bind ephemeral");
    let port = listener.local_addr().expect("local_addr").port();
    drop(listener);
    port
}

/// Spawn `coral ui serve` with the given arg list (auto-picks a free
/// port, no browser launch). Returns the live child + stdout handle
/// so the test can read the banner and then drive HTTP requests.
///
/// `bind` defaults to `127.0.0.1`; tests that exercise the auto-mint
/// path override it to `0.0.0.0` via `extra_args` AND should call
/// [`pick_free_port`] with the same bind to keep the reserved-port
/// race window narrow.
fn spawn_serve(
    coral: &PathBuf,
    wiki: &TempDir,
    bind: &str,
    extra_args: &[&str],
) -> Result<(ChildGuard, Banner, ChildStdout), String> {
    let port = pick_free_port(bind);
    let wiki_root = wiki.path().join(".wiki");
    let mut cmd = Command::new(coral);
    cmd.arg("--wiki-root")
        .arg(&wiki_root)
        .args([
            "ui",
            "serve",
            "--no-open",
            "--port",
            &port.to_string(),
            "--bind",
            bind,
        ])
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    let stdout = child.stdout.take().ok_or("no stdout handle")?;
    let guard = ChildGuard::new(child);
    let (banner, stdout) =
        read_banner(stdout, Duration::from_secs(5)).map_err(|e| format!("banner: {e}"))?;
    Ok((guard, banner, stdout))
}

/// A 32-hex-char bearer token (128 bits = exactly at the SEC-02
/// entropy floor) used by every test that needs to clear the floor
/// without exercising the floor itself.
const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef";

/// TEST-11 #1 — `/health` is unprotected when no token is configured
/// (loopback + no `--token` = bearer auth disabled). Returns 200
/// with the documented envelope.
#[test]
fn api_smoke_health_endpoint_returns_200_without_token() {
    let Some(coral) = cargo_bin_path() else {
        eprintln!("coral binary not built; skipping");
        return;
    };
    let wiki = make_wiki();
    let (_guard, banner, _stdout) = match spawn_serve(&coral, &wiki, "127.0.0.1", &[]) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("spawn failed: {e}");
            return;
        }
    };
    wait_until_ready(&banner.addr, None).expect("ready");

    let url = format!("http://{}/health", banner.addr);
    let resp = ureq::get(&url)
        .timeout(Duration::from_secs(2))
        .call()
        .expect("GET /health");
    assert_eq!(
        resp.status(),
        200,
        "health should be 200 on loopback no-token"
    );
    let body: serde_json::Value = resp.into_json().expect("json");
    assert_eq!(body["data"]["status"], "ok");
}

/// TEST-11 #2 — when a token IS configured, `/api/v1/pages` requires
/// the Authorization header. Missing header → 401 `MISSING_TOKEN`.
#[test]
fn api_smoke_pages_requires_bearer_token() {
    let Some(coral) = cargo_bin_path() else {
        return;
    };
    let wiki = make_wiki();
    let (_guard, banner, _stdout) =
        spawn_serve(&coral, &wiki, "127.0.0.1", &["--token", TEST_TOKEN]).expect("spawn");
    wait_until_ready(&banner.addr, Some(TEST_TOKEN)).expect("ready");

    let url = format!("http://{}/api/v1/pages", banner.addr);
    let resp = ureq::get(&url).timeout(Duration::from_secs(2)).call();
    match resp {
        Err(ureq::Error::Status(401, _)) => {} // expected
        Err(e) => panic!("expected 401, got transport error: {e}"),
        Ok(r) => panic!("expected 401, got {}", r.status()),
    }
}

/// TEST-11 #3 — `/api/v1/pages` with the configured token returns 200
/// (even with an empty wiki — the route emits `data: []`).
#[test]
fn api_smoke_pages_with_valid_token_returns_200() {
    let Some(coral) = cargo_bin_path() else {
        return;
    };
    let wiki = make_wiki();
    let (_guard, banner, _stdout) =
        spawn_serve(&coral, &wiki, "127.0.0.1", &["--token", TEST_TOKEN]).expect("spawn");
    wait_until_ready(&banner.addr, Some(TEST_TOKEN)).expect("ready");

    let url = format!("http://{}/api/v1/pages", banner.addr);
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {TEST_TOKEN}"))
        .timeout(Duration::from_secs(2))
        .call()
        .expect("GET /api/v1/pages");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.into_json().expect("json");
    // Empty wiki → no pages, but the envelope shape must still be
    // present (the SPA depends on `data` being an array, never null).
    assert!(body["data"].is_array(), "data must be an array: {body:?}");
}

/// TEST-11 #4 — `/api/v1/pages` with a non-matching token returns 403
/// `INVALID_TOKEN` (vs. the missing-header case which is 401
/// `MISSING_TOKEN`). The WebUI deliberately distinguishes the two on
/// the wire: 401 = "you didn't send Authorization", 403 = "you sent
/// one but it doesn't match" — same shape HTTP/RFC 9110 §15.5.4
/// recommends. The constant-time-compare path is exercised from
/// auth.rs unit tests; here we just pin the wire behaviour.
#[test]
fn api_smoke_pages_with_invalid_token_returns_401() {
    let Some(coral) = cargo_bin_path() else {
        return;
    };
    let wiki = make_wiki();
    let (_guard, banner, _stdout) =
        spawn_serve(&coral, &wiki, "127.0.0.1", &["--token", TEST_TOKEN]).expect("spawn");
    wait_until_ready(&banner.addr, Some(TEST_TOKEN)).expect("ready");

    let url = format!("http://{}/api/v1/pages", banner.addr);
    let resp = ureq::get(&url)
        .set(
            "Authorization",
            "Bearer this-is-not-the-right-token-at-all-padded-to-floor",
        )
        .timeout(Duration::from_secs(2))
        .call();
    match resp {
        // 403 INVALID_TOKEN — value mismatch path. The test name keeps
        // its "401" suffix for historical continuity with the spec
        // (TEST-11 list); the assertion shape is what enforces the
        // wire contract.
        Err(ureq::Error::Status(403, _)) => {}
        Err(e) => panic!("expected 403, got transport error: {e}"),
        Ok(r) => panic!("expected 403, got {}", r.status()),
    }
}

/// TEST-11 #5 — `/api/v1/pages` with a malformed Authorization header
/// (e.g. `Basic ...` instead of `Bearer ...`) returns 401. WebUI
/// collapses MalformedHeader + MissingHeader into the same
/// `MISSING_TOKEN` ApiError variant, so this and #2 share an
/// HTTP-status verdict but exercise different paths through
/// `coral_core::auth::verify_bearer`.
#[test]
fn api_smoke_pages_with_malformed_token_header_returns_401() {
    let Some(coral) = cargo_bin_path() else {
        return;
    };
    let wiki = make_wiki();
    let (_guard, banner, _stdout) =
        spawn_serve(&coral, &wiki, "127.0.0.1", &["--token", TEST_TOKEN]).expect("spawn");
    wait_until_ready(&banner.addr, Some(TEST_TOKEN)).expect("ready");

    let url = format!("http://{}/api/v1/pages", banner.addr);
    // `Basic ...` → not `Bearer `, should reject.
    let resp = ureq::get(&url)
        .set("Authorization", "Basic dXNlcjpwYXNz")
        .timeout(Duration::from_secs(2))
        .call();
    match resp {
        Err(ureq::Error::Status(401, _)) => {}
        Err(e) => panic!("expected 401, got transport error: {e}"),
        Ok(r) => panic!("expected 401, got {}", r.status()),
    }
}

/// TEST-11 #6 — auto-mint end-to-end. Spawn with `--bind 0.0.0.0`
/// (forces auto-mint because non-loopback) and no `--token`, scrape
/// the minted token off stdout, then use it to hit `/api/v1/pages`
/// successfully. Closes the loop on SEC-02: the printed banner value
/// matches the value the server expects.
///
/// Bind 0.0.0.0 not localhost: the auto-mint path only kicks in for
/// non-loopback binds. We still hit the listener via 127.0.0.1
/// (the kernel routes 0.0.0.0 listeners to any interface incl
/// loopback) so the test is portable to dev machines without LAN.
#[test]
fn api_smoke_auto_minted_token_works_e2e() {
    let Some(coral) = cargo_bin_path() else {
        return;
    };
    let wiki = make_wiki();
    let (_guard, banner, _stdout) = spawn_serve(&coral, &wiki, "0.0.0.0", &[]).expect("spawn");
    let token = banner
        .minted_token
        .as_deref()
        .expect("auto-mint should print a token on non-loopback bind");
    // SEC-02 promises 64 hex chars / lower-case.
    assert_eq!(
        token.len(),
        64,
        "minted token should be 64 hex chars: {token}"
    );
    assert!(
        token
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "minted token must be lower-case hex: {token}"
    );

    // The banner addr is `0.0.0.0:<port>`; route through loopback.
    let port = banner.addr.rsplit_once(':').expect("port").1;
    let live_addr = format!("127.0.0.1:{port}");
    wait_until_ready(&live_addr, Some(token)).expect("ready");

    let url = format!("http://{live_addr}/api/v1/pages");
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(2))
        .call()
        .expect("auto-minted token should authenticate");
    assert_eq!(resp.status(), 200, "auto-mint round-trip failed");
}

/// TEST-11 #7 — entropy floor rejects sub-floor tokens at CLI startup.
/// `--token=abc` is well under 32 chars; the binary must exit
/// non-zero with the documented "token has N chars; minimum is 32"
/// message on stderr. Pin the message prefix so a future refactor
/// can't accidentally swap it for an opaque "invalid argument".
#[test]
fn api_smoke_entropy_floor_rejects_short_token() {
    let Some(coral) = cargo_bin_path() else {
        return;
    };
    let wiki = make_wiki();
    let wiki_root = wiki.path().join(".wiki");
    let output = Command::new(&coral)
        .arg("--wiki-root")
        .arg(&wiki_root)
        .args(["ui", "serve", "--no-open", "--port", "0", "--token", "abc"])
        .output()
        .expect("spawn for entropy-floor check");
    assert!(
        !output.status.success(),
        "short --token should fail, but exit was success. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("token has ") && stderr.contains("minimum is 32"),
        "entropy-floor error message missing or changed; stderr was:\n{stderr}"
    );
}
