//! CLI smoke test for `coral mcp serve --transport http`.
//!
//! Test #17 of the v0.21.1 plan. Spawns the binary, waits for the
//! "listening on" stderr banner, fires a single POST initialize, and
//! verifies the response shape. Pin: the CLI surface advertises both
//! transports and the http transport is reachable end-to-end through
//! the binary entry point (not just the library).
//!
//! Why a fixed port (not 0)? The CLI takes `--port` as a u16 and
//! prints the resolved listener address to stderr; we parse the
//! banner to discover the port even when the user passes `--port 0`.
//! That keeps this test self-contained — no shared OS-state across
//! parallel test runs.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn coral_mcp_serve_http_responds_to_post_initialize() {
    let coral = match cargo_bin_path() {
        Some(p) => p,
        None => {
            eprintln!("coral binary not built; skipping CLI smoke");
            return;
        }
    };

    // Spawn `coral mcp serve --transport http --port 0`. We read
    // stderr line-by-line until the "listening on" banner reveals the
    // OS-assigned port.
    let mut child = Command::new(&coral)
        .args(["mcp", "serve", "--transport", "http", "--port", "0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn coral mcp serve --transport http");

    let stderr = child.stderr.take().expect("stderr handle");
    let mut reader = BufReader::new(stderr);
    let mut listening_addr = None;
    for _ in 0..50 {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                eprintln!("server stderr: {}", line.trim_end());
                if let Some(rest) = line.split_once("listening on http://") {
                    let addr_part = rest.1.split('/').next().unwrap_or("").trim().to_string();
                    if !addr_part.is_empty() {
                        listening_addr = Some(addr_part);
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }
    let listening_addr = match listening_addr {
        Some(a) => a,
        None => {
            let _ = child.kill();
            panic!("coral mcp serve never logged a listening banner");
        }
    };

    // Send POST /mcp initialize.
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req = format!(
        "POST /mcp HTTP/1.1\r\n\
         Host: {listening_addr}\r\n\
         Content-Type: application/json\r\n\
         Accept: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(&listening_addr).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .expect("write timeout");
    stream.write_all(req.as_bytes()).expect("write");
    stream.flush().expect("flush");
    let mut response = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
    }
    let response_str = String::from_utf8_lossy(&response);
    assert!(
        response_str.starts_with("HTTP/1.1 200"),
        "expected 200 status from CLI HTTP transport; got:\n{response_str}"
    );
    assert!(
        response_str.contains("\"protocolVersion\":\"2025-11-25\""),
        "missing protocolVersion in CLI HTTP response:\n{response_str}"
    );
    assert!(
        response_str.contains("Mcp-Session-Id:"),
        "Mcp-Session-Id header missing from CLI HTTP response:\n{response_str}"
    );

    // Tear down. We rely on the OS killing the listener thread when
    // the parent process exits — no graceful shutdown needed for the
    // smoke surface.
    let _ = child.kill();
    let _ = child.wait();
}

/// `--bind 0.0.0.0` must emit a stderr warning banner. Acceptance
/// criterion of the v0.21.1 plan — pin the literal "WARNING" prefix
/// so a future refactor can't accidentally downgrade it to an
/// info-level log line.
#[test]
fn bind_zero_zero_zero_emits_stderr_warning_banner() {
    let coral = match cargo_bin_path() {
        Some(p) => p,
        None => return,
    };
    let mut child = Command::new(&coral)
        .args([
            "mcp",
            "serve",
            "--transport",
            "http",
            "--port",
            "0",
            "--bind",
            "0.0.0.0",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let stderr = child.stderr.take().expect("stderr");
    let mut reader = BufReader::new(stderr);
    let mut saw_warning = false;
    let mut saw_listening = false;
    for _ in 0..50 {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.starts_with("WARNING:") {
                    saw_warning = true;
                }
                if line.contains("listening on http://") {
                    saw_listening = true;
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        saw_warning,
        "expected `WARNING:` stderr banner when binding 0.0.0.0"
    );
    assert!(
        saw_listening,
        "server never reached the listening banner; check if 0.0.0.0 binding fails locally"
    );
}

/// Locate the `coral` binary in the cargo test harness's runtime
/// directory. Returns None if the binary wasn't built (e.g. doctest
/// mode, CI without `cargo build`).
fn cargo_bin_path() -> Option<std::path::PathBuf> {
    // CARGO_BIN_EXE_<name> is set by cargo when running tests
    // alongside a binary in the same package. The `coral` binary
    // lives in coral-cli, this test crate is also in coral-cli, so
    // CARGO_BIN_EXE_coral is populated.
    if let Some(p) = std::env::var_os("CARGO_BIN_EXE_coral") {
        let path = std::path::PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }
    None
}
