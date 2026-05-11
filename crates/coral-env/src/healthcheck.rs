//! Healthcheck loop: poll a service until `consecutive_failures` is
//! reached or the timing budget exhausts.
//!
//! Used by `EnvBackend::up()` to wait-for-healthy after starting a
//! service, and by `coral verify` (v0.18 will wire this into a
//! `TestRunner::Healthcheck` impl). Backend-agnostic: the probe
//! function is supplied by the caller — `ComposeBackend` does HTTP via
//! reqwest-blocking-equivalent (curl subprocess for v0.17 to avoid the
//! reqwest dep), TCP via `TcpStream::connect_timeout`, exec via the
//! container's exec endpoint.

use crate::error::EnvError;
use crate::spec::{Healthcheck, HealthcheckKind, HealthcheckTiming};
use std::time::{Duration, Instant};

/// Outcome of a single probe attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeResult {
    Pass,
    Fail,
}

/// `wait_for_healthy` polls `probe` according to the timing config in
/// `hc.timing`. Returns `Ok(())` on the first PASS; `Err(EnvError::Timeout)`
/// when the cumulative budget elapses; `Err(EnvError::Crashed)` when
/// `consecutive_failures` is reached.
///
/// Always probes at least once before honoring any timeouts — a
/// healthy service should never be reported as `Timeout` because of
/// arithmetic in the timing config.
pub fn wait_for_healthy<F>(service: &str, hc: &Healthcheck, mut probe: F) -> crate::EnvResult<()>
where
    F: FnMut(&HealthcheckKind) -> ProbeResult,
{
    let timing = &hc.timing;
    let started = Instant::now();
    let total_budget = Duration::from_secs(
        (timing.start_period_s as u64)
            + (timing.interval_s as u64) * (timing.retries as u64).max(1),
    );
    let mut consecutive_fail = 0u32;

    // Loop probes at least once before checking the budget so a healthy
    // service is never reported as `Timeout` because of arithmetic in
    // the timing config.
    loop {
        let in_startup = started.elapsed() < Duration::from_secs(timing.start_period_s as u64);
        let interval = if in_startup {
            Duration::from_secs(timing.start_interval_s.unwrap_or(timing.interval_s) as u64)
        } else {
            Duration::from_secs(timing.interval_s as u64)
        };

        match probe(&hc.kind) {
            ProbeResult::Pass => return Ok(()),
            ProbeResult::Fail => {
                consecutive_fail += 1;
                if !in_startup && consecutive_fail >= timing.consecutive_failures {
                    return Err(EnvError::Crashed {
                        service: service.to_string(),
                        code: -1,
                        stderr_tail: format!(
                            "healthcheck failed {} times in a row",
                            consecutive_fail
                        ),
                    });
                }
            }
        }
        if started.elapsed() >= total_budget {
            break;
        }
        std::thread::sleep(interval);
    }

    Err(EnvError::Timeout {
        what: format!("service '{service}' to become healthy"),
        seconds: total_budget.as_secs(),
    })
}

/// Convenience: derives a sensible total budget from the timing config
/// without polling. Useful for tests and for the `--strict` exit policy
/// of `coral up`.
pub fn budget(timing: &HealthcheckTiming) -> Duration {
    Duration::from_secs(
        (timing.start_period_s as u64) + (timing.interval_s as u64) * (timing.retries as u64),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn quick_timing() -> HealthcheckTiming {
        // total_budget = start_period + interval * retries = 0 + 1 * 5 = 5s.
        // 3 consecutive failures at 1s intervals → Crashed in ~3 seconds.
        // start_interval_s = Some(0) so when in_startup we don't sleep, but
        // start_period is 0 so we're never in_startup.
        HealthcheckTiming {
            interval_s: 1,
            timeout_s: 1,
            retries: 5,
            start_period_s: 0,
            start_interval_s: Some(0),
            consecutive_failures: 3,
        }
    }

    fn http_check() -> Healthcheck {
        Healthcheck {
            kind: HealthcheckKind::Http {
                path: "/health".into(),
                expect_status: 200,
                headers: Default::default(),
            },
            timing: quick_timing(),
        }
    }

    #[test]
    fn wait_for_healthy_returns_ok_on_first_pass() {
        let hc = http_check();
        let result = wait_for_healthy("api", &hc, |_| ProbeResult::Pass);
        result.expect("wait_for_healthy must succeed when the probe returns Pass on the first try");
    }

    #[test]
    fn wait_for_healthy_returns_crashed_after_consecutive_failures() {
        let hc = http_check();
        let counter = AtomicU32::new(0);
        let result = wait_for_healthy("api", &hc, |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            ProbeResult::Fail
        });
        match result {
            Err(EnvError::Crashed { service, .. }) => assert_eq!(service, "api"),
            other => panic!("expected Crashed, got {other:?}"),
        }
        assert!(counter.load(Ordering::SeqCst) >= 3);
    }

    #[test]
    fn budget_combines_start_period_and_interval_x_retries() {
        let t = HealthcheckTiming {
            interval_s: 2,
            timeout_s: 1,
            retries: 3,
            start_period_s: 5,
            start_interval_s: None,
            consecutive_failures: 3,
        };
        assert_eq!(budget(&t), Duration::from_secs(5 + 2 * 3));
    }
}
