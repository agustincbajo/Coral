//! Probe implementations for `Healthcheck`. Used by `HealthcheckRunner`
//! (this crate) and reusable by `UserDefinedRunner` for `step.healthcheck`.
//!
//! HTTP and gRPC use subprocess (`curl` / `grpc_health_probe`) so we
//! avoid pulling a heavy HTTP client into the default dep tree. The
//! same reasoning as `coral_core::git_remote` (subprocess `git`).
//! TCP uses std `std::net::TcpStream::connect_timeout`. Exec wraps
//! `std::process::Command`.

use coral_env::healthcheck::ProbeResult;
use coral_env::{HealthcheckKind, ServiceStatus};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::process::Command;
use std::time::Duration;

/// Build a probe closure suitable for `wait_for_healthy`. The probe
/// resolves the service's published port from the live `EnvStatus`
/// snapshot at call time, so port-allocation changes between runs
/// don't stale-cache the target address.
pub fn make_probe<'a>(
    status: &'a ServiceStatus,
    timeout: Duration,
) -> impl FnMut(&HealthcheckKind) -> ProbeResult + 'a {
    move |kind| probe_once(status, kind, timeout)
}

/// One-shot probe — exposed for the `HealthcheckRunner` so a single
/// pass produces a single `TestStatus::{Pass,Fail}` instead of looping.
pub fn probe_once(
    status: &ServiceStatus,
    kind: &HealthcheckKind,
    timeout: Duration,
) -> ProbeResult {
    match kind {
        HealthcheckKind::Http {
            path,
            expect_status,
            headers,
        } => probe_http(status, path, *expect_status, headers, timeout),
        HealthcheckKind::Tcp { port } => probe_tcp(status, *port, timeout),
        HealthcheckKind::Exec { cmd } => probe_exec(cmd, timeout),
        HealthcheckKind::Grpc { port, service } => {
            probe_grpc(status, *port, service.as_deref(), timeout)
        }
    }
}

fn probe_http(
    status: &ServiceStatus,
    path: &str,
    expect: u16,
    headers: &std::collections::BTreeMap<String, String>,
    timeout: Duration,
) -> ProbeResult {
    let host_port = match status.published_ports.first() {
        Some(p) if p.host_port > 0 => p.host_port,
        _ => return ProbeResult::Fail,
    };
    let url = format!("http://127.0.0.1:{host_port}{path}");
    let mut cmd = Command::new("curl");
    cmd.args([
        "-fsS",
        "-o",
        "/dev/null",
        "-w",
        "%{http_code}",
        "--max-time",
        &timeout.as_secs().max(1).to_string(),
    ]);
    for (k, v) in headers {
        cmd.arg("-H").arg(format!("{k}: {v}"));
    }
    cmd.arg(&url);
    let output = match cmd.output() {
        Ok(o) => o,
        Err(_) => return ProbeResult::Fail,
    };
    if !output.status.success() {
        return ProbeResult::Fail;
    }
    let body = String::from_utf8_lossy(&output.stdout);
    let code: u16 = body.trim().parse().unwrap_or(0);
    if code == expect {
        ProbeResult::Pass
    } else {
        ProbeResult::Fail
    }
}

fn probe_tcp(status: &ServiceStatus, container_port: u16, timeout: Duration) -> ProbeResult {
    let host_port = status
        .published_ports
        .iter()
        .find(|p| p.container_port == container_port)
        .map(|p| p.host_port)
        .or_else(|| status.published_ports.first().map(|p| p.host_port))
        .unwrap_or(0);
    if host_port == 0 {
        return ProbeResult::Fail;
    }
    let addr_str = format!("127.0.0.1:{host_port}");
    let addr: SocketAddr = match addr_str.to_socket_addrs().ok().and_then(|mut it| it.next()) {
        Some(a) => a,
        None => return ProbeResult::Fail,
    };
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(_) => ProbeResult::Pass,
        Err(_) => ProbeResult::Fail,
    }
}

fn probe_exec(cmd: &[String], _timeout: Duration) -> ProbeResult {
    if cmd.is_empty() {
        return ProbeResult::Fail;
    }
    let mut command = Command::new(&cmd[0]);
    for arg in &cmd[1..] {
        command.arg(arg);
    }
    match command.status() {
        Ok(status) if status.success() => ProbeResult::Pass,
        _ => ProbeResult::Fail,
    }
}

fn probe_grpc(
    status: &ServiceStatus,
    container_port: u16,
    service: Option<&str>,
    timeout: Duration,
) -> ProbeResult {
    // Best-effort grpc_health_probe (a separately-installed binary).
    // If it's not on PATH, fall back to a TCP connect — covers the
    // most common case where the user just wants "is the gRPC server
    // listening".
    let host_port = status
        .published_ports
        .iter()
        .find(|p| p.container_port == container_port)
        .map(|p| p.host_port)
        .unwrap_or(0);
    if host_port == 0 {
        return ProbeResult::Fail;
    }
    let mut cmd = Command::new("grpc_health_probe");
    cmd.args(["-addr", &format!("127.0.0.1:{host_port}")]);
    if let Some(svc) = service {
        cmd.args(["-service", svc]);
    }
    match cmd.status() {
        Ok(status) if status.success() => return ProbeResult::Pass,
        Ok(_) => return ProbeResult::Fail,
        Err(_) => {}
    }
    // Fallback: TCP connect.
    probe_tcp(status, container_port, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_env::PublishedPort;

    fn status_with_port(host_port: u16, container_port: u16) -> ServiceStatus {
        ServiceStatus {
            name: "api".into(),
            state: coral_env::ServiceState::Running,
            health: coral_env::HealthState::Unknown,
            restarts: 0,
            published_ports: vec![PublishedPort {
                container_port,
                host_port,
            }],
        }
    }

    #[test]
    fn probe_tcp_fails_against_unreachable_port() {
        // Use a port the OS gives us, then close it before probing —
        // guaranteed-no-listener semantics.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let status = status_with_port(port, port);
        assert_eq!(
            probe_tcp(&status, port, Duration::from_millis(100)),
            ProbeResult::Fail
        );
    }

    #[test]
    fn probe_tcp_passes_against_open_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let status = status_with_port(port, port);
        assert_eq!(
            probe_tcp(&status, port, Duration::from_millis(500)),
            ProbeResult::Pass
        );
    }

    #[test]
    fn probe_exec_passes_when_cmd_returns_zero() {
        let cmd = vec!["true".to_string()];
        assert_eq!(
            probe_exec(&cmd, Duration::from_millis(500)),
            ProbeResult::Pass
        );
    }

    #[test]
    fn probe_exec_fails_when_cmd_returns_nonzero() {
        let cmd = vec!["false".to_string()];
        assert_eq!(
            probe_exec(&cmd, Duration::from_millis(500)),
            ProbeResult::Fail
        );
    }
}
