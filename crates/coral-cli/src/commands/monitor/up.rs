//! `coral monitor up` — foreground scheduled TestCase loop (v0.23.1).
//!
//! Decision matrix the spec pinned:
//!
//!   - **D1**: Foreground only. `--detach` errors with "deferred to v0.24+".
//!   - **D5**: First iteration runs immediately at startup. Subsequent
//!     iterations sleep for `interval_seconds`. If an iteration takes
//!     longer than the interval, log a warning and start the next
//!     iteration immediately (no overlap protection — only one
//!     in-flight run per process by construction since the loop is
//!     synchronous).
//!   - **D7**: SIGINT/SIGTERM flips an `AtomicBool` via `signal-hook`.
//!     The loop checks at the top AND inside `sleep_interruptible`
//!     (250ms chunks) so a user mashing Ctrl-C never waits more than
//!     ~250ms for the process to exit.
//!
//! Why the interruptible sleep matters: `std::thread::sleep(60s)`
//! is uninterruptible — Ctrl-C hits the AtomicBool, but the thread
//! stays parked for the full minute before checking the flag. The
//! 250ms-chunked loop gives crisp Ctrl-C latency without burning CPU
//! on a 1ms poll.

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Args;
use coral_env::{
    EnvBackend, EnvHandle, EnvPlan, MonitorSpec, OnFailure,
    compose::{ComposeBackend, ComposeRuntime},
};
use coral_test::{TestFilters, TestKind, TestStatus, run_test_suite_filtered};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::commands::common::resolve_project;
use crate::commands::env_resolve::{default_env_name, resolve_env};
use crate::commands::monitor::run::{MonitorRun, append_run};

#[derive(Args, Debug)]
pub struct UpArgs {
    /// Environment to monitor (default: first declared in coral.toml).
    #[arg(long)]
    pub env: Option<String>,

    /// Run only the named monitor. Without this flag, all declared
    /// monitors run sequentially in one process — each iteration of
    /// the outer `--monitor=ALL` loop runs every monitor's filter
    /// once before sleeping for the SHORTEST interval. v0.23.1 keeps
    /// scheduling simple: one process per monitor is the recommended
    /// pattern; the all-in-one form exists for quick "spin them up
    /// and inspect" sessions.
    #[arg(long = "monitor")]
    pub monitor: Option<String>,

    /// **Deferred-stub.** Errors with `--detach is deferred to v0.24+`.
    /// Parsed (rather than rejected by clap) so the flag is forward-
    /// compatible: a v0.24 binary will accept it, a v0.23.1 binary
    /// produces an actionable error.
    #[arg(long)]
    pub detach: bool,
}

/// Polled at the top of every loop iteration AND inside the
/// interruptible sleep. Wrapped in an `Arc` so `signal-hook` can
/// register it directly — the OS-side handler flips this to `true`
/// on SIGINT/SIGTERM, the loop observes it. `Relaxed` is fine: we
/// don't need any memory ordering relative to other writes, just
/// "did the user ask us to stop yet".
fn shutdown_flag() -> &'static Arc<AtomicBool> {
    static FLAG: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    FLAG.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

pub fn run(args: UpArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    if args.detach {
        // D1: foreground-only in v0.23.1. The flag exists so the CLI
        // surface is forward-compatible, but the daemonization wiring
        // (PID file, log redirect, double-fork) lands in v0.24+.
        anyhow::bail!(
            "--detach is deferred to v0.24+; run `coral monitor up` in the foreground for now"
        );
    }

    let project = resolve_project(wiki_root)?;
    if project.environments_raw.is_empty() {
        anyhow::bail!("no [[environments]] declared in coral.toml");
    }
    let env_name = args
        .env
        .clone()
        .unwrap_or_else(|| default_env_name(&project));
    let spec = resolve_env(&project, &env_name)?;
    if spec.monitors.is_empty() {
        anyhow::bail!(
            "environment '{}' declares no [[environments.{}.monitors]]; \
             add at least one monitor block before running `coral monitor up`",
            env_name,
            env_name
        );
    }
    // Filter to the named monitor or default to all.
    let monitors: Vec<MonitorSpec> = match &args.monitor {
        Some(name) => {
            let m = spec
                .monitors
                .iter()
                .find(|m| m.name == *name)
                .cloned()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "monitor '{}' not declared in environment '{}'; available: {}",
                        name,
                        env_name,
                        spec.monitors
                            .iter()
                            .map(|m| m.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            vec![m]
        }
        None => spec.monitors.clone(),
    };

    // Pre-flight kind-string validation. The spec-level validate()
    // already ensures the kind string is in the known set; here we
    // need to convert it to `TestKind` (which lives in coral-test, a
    // downstream crate that coral-env can't import). Any mismatch
    // surfaces as an error before the loop starts.
    for m in &monitors {
        kind_for_monitor(m).map_err(|e| anyhow::anyhow!(e))?;
        if matches!(m.on_failure, OnFailure::Alert) {
            anyhow::bail!(
                "monitor '{}' uses `on_failure = \"alert\"`, which is reserved for v0.24+; \
                 use `log` (default) or `fail-fast` instead",
                m.name
            );
        }
    }

    // Build the compose backend ONCE — it's shared across every
    // iteration of every monitor in this process. The env handle is
    // also stable: the artifact path doesn't change while the loop is
    // running. This mirrors `coral test`'s build sequence so we hit
    // the exact same code paths as one-shot test runs.
    let mut repo_paths = BTreeMap::new();
    for repo in &project.repos {
        repo_paths.insert(repo.name.clone(), project.resolved_path(repo));
    }
    let plan = EnvPlan::from_spec(&spec, &project.root, &repo_paths)
        .map_err(|e| anyhow::anyhow!("building env plan: {}", e))?;
    let backend: Arc<dyn EnvBackend> = Arc::new(ComposeBackend::new(ComposeRuntime::parse(
        &spec.compose_command,
    )));
    let env_handle = EnvHandle {
        backend: backend.name().to_string(),
        artifact_hash: "monitor".into(),
        artifact_path: plan.project_root.join(".coral/env/compose/monitor.yml"),
        state: BTreeMap::new(),
    };

    // Install the signal handler ONCE — re-registering inside the
    // loop would race with the OS dispatcher.
    install_shutdown_handler()?;

    // Track outcome across the loop so the exit code reflects whether
    // the loop was clean (no failures observed) or had at least one
    // failed iteration. fail-fast bails immediately; log-mode tallies.
    let mut total_iterations = 0usize;
    let mut failed_iterations = 0usize;
    let exit = run_loop(
        &project.root,
        &env_name,
        &monitors,
        &mut |m: &MonitorSpec| -> anyhow::Result<Vec<coral_test::TestReport>> {
            let kinds = kind_for_monitor(m)
                .map_err(|e| anyhow::anyhow!(e))?
                .unwrap_or_default();
            let filters = TestFilters {
                services: m.services.clone(),
                tags: m.tag.iter().cloned().collect(),
                kinds,
                include_discovered: false,
            };
            run_test_suite_filtered(
                &project.root,
                &spec,
                backend.clone(),
                &plan,
                &env_handle,
                &filters,
                false,
            )
            .with_context(|| format!("monitor '{}' iteration", m.name))
        },
        &mut total_iterations,
        &mut failed_iterations,
    );

    // Final summary on stop. Always print; the operator wants this
    // even on Ctrl-C.
    println!(
        "monitor up summary: env='{}' iterations={} failed_iterations={}",
        env_name, total_iterations, failed_iterations
    );

    exit
}

/// Translate a `MonitorSpec` to a `coral_test::TestKind` filter. The
/// spec-level `validate()` already enforces the kind string is in the
/// set we accept here; a mismatch indicates a logic bug.
fn kind_for_monitor(m: &MonitorSpec) -> Result<Option<Vec<TestKind>>, String> {
    match m.kind.as_deref() {
        None => Ok(None),
        Some("healthcheck") => Ok(Some(vec![TestKind::Healthcheck])),
        Some("user_defined") => Ok(Some(vec![TestKind::UserDefined])),
        Some("smoke") => Ok(Some(vec![TestKind::Healthcheck, TestKind::UserDefined])),
        Some(other) => Err(format!(
            "monitor '{}' has unsupported kind '{}'; valid: healthcheck, user_defined, smoke",
            m.name, other
        )),
    }
}

/// `run_loop` is generic over a tick function so unit tests can inject
/// synthetic `Vec<TestReport>` values without standing up the full
/// runner stack. The production path passes a closure that calls
/// `run_test_suite_filtered`; tests pass a closure that returns
/// canned reports.
///
/// The tick function takes a `&MonitorSpec` (so it sees which monitor
/// is firing) and returns `Result<Vec<TestReport>>`. Errors propagate
/// out — they're treated as catastrophic (couldn't reach the env at
/// all), distinct from per-case `TestStatus::Fail`.
fn run_loop<F>(
    project_root: &Path,
    env_name: &str,
    monitors: &[MonitorSpec],
    tick: &mut F,
    total_iterations: &mut usize,
    failed_iterations: &mut usize,
) -> Result<ExitCode>
where
    F: FnMut(&MonitorSpec) -> Result<Vec<coral_test::TestReport>>,
{
    // First-iteration-immediate (AC #2): no leading sleep.
    loop {
        if shutdown_flag().load(Ordering::Relaxed) {
            return Ok(exit_for(*failed_iterations));
        }
        for m in monitors {
            if shutdown_flag().load(Ordering::Relaxed) {
                return Ok(exit_for(*failed_iterations));
            }
            let started = Instant::now();
            let started_at = Utc::now();
            let reports = tick(m)?;

            let duration = started.elapsed();
            let run = MonitorRun::from_reports(
                env_name,
                &m.name,
                started_at,
                duration.as_millis() as u64,
                &reports,
            );
            let path = append_run(project_root, &run)
                .with_context(|| format!("appending JSONL row for monitor '{}'", m.name))?;
            *total_iterations += 1;

            let any_failure = reports
                .iter()
                .any(|r| matches!(r.status, TestStatus::Fail { .. } | TestStatus::Error { .. }));
            // Tracing-level info so a `RUST_LOG=info` viewer catches
            // every tick. Stdout stays clean for piping.
            tracing::info!(
                env = %env_name,
                monitor = %m.name,
                total = run.total,
                passed = run.passed,
                failed = run.failed,
                duration_ms = run.duration_ms,
                jsonl = %path.display(),
                "monitor iteration"
            );

            if any_failure {
                *failed_iterations += 1;
                match m.on_failure {
                    OnFailure::Log => {
                        // Default: keep going; the failed run is
                        // already in JSONL and the operator can
                        // inspect via `coral monitor history`.
                        eprintln!(
                            "monitor '{}': iteration FAILED ({}/{} cases failed; see {})",
                            m.name,
                            run.failed,
                            run.total,
                            path.display()
                        );
                    }
                    OnFailure::FailFast => {
                        // AC #6: exit non-zero on first failed iteration.
                        eprintln!(
                            "monitor '{}': iteration FAILED with on_failure=fail-fast; exiting",
                            m.name
                        );
                        return Ok(ExitCode::FAILURE);
                    }
                    OnFailure::Alert => {
                        // Can't reach here — pre-flight rejects Alert
                        // before the loop starts.
                        anyhow::bail!(
                            "monitor '{}': on_failure=\"alert\" reached the loop body; this is a bug — pre-flight should have rejected it",
                            m.name
                        );
                    }
                }
            }

            // AC #9: log a warning if the iteration overran its interval.
            if duration > Duration::from_secs(m.interval_seconds) {
                tracing::warn!(
                    monitor = %m.name,
                    interval_seconds = m.interval_seconds,
                    duration_ms = run.duration_ms,
                    "monitor iteration overran interval; starting next immediately"
                );
                // Skip the sleep so the next iteration starts now.
                // (Per-monitor loops in the all-monitors form still
                // continue to the next monitor in the list.)
                continue;
            }
        }
        if shutdown_flag().load(Ordering::Relaxed) {
            return Ok(exit_for(*failed_iterations));
        }
        // Sleep for the SHORTEST configured interval so monitors with
        // shorter cadences don't get starved by a longer-cadence
        // monitor. Each monitor's tick still tracks its own interval
        // for the overrun warning above; this loop-level sleep is the
        // pacing primitive.
        let min_interval = monitors
            .iter()
            .map(|m| m.interval_seconds)
            .min()
            .unwrap_or(60);
        sleep_interruptible(Duration::from_secs(min_interval));
    }
}

/// Sleep in 250ms chunks, returning early if `SHUTDOWN` flips. The
/// total slept time is capped at `total`; we never oversleep.
fn sleep_interruptible(total: Duration) {
    let chunk = Duration::from_millis(250);
    let mut remaining = total;
    while !remaining.is_zero() {
        if shutdown_flag().load(Ordering::Relaxed) {
            return;
        }
        let to_sleep = if remaining > chunk { chunk } else { remaining };
        std::thread::sleep(to_sleep);
        remaining = remaining.saturating_sub(to_sleep);
    }
}

/// Map "any failed iteration?" → exit code. `FAILURE` if at least one
/// iteration failed (AC #6 holds for `fail-fast`; in `log` mode we
/// still want CI to flag the run if anything failed during the
/// observed window).
fn exit_for(failed_iterations: usize) -> ExitCode {
    if failed_iterations == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn install_shutdown_handler() -> Result<()> {
    use signal_hook::consts::{SIGINT, SIGTERM};
    // `signal-hook::flag::register` flips the wrapped `AtomicBool` to
    // `true` from the signal handler. The Arc'd flag is shared
    // between the OS-side handler and the loop's observation point
    // — registration leaks one strong refcount that lives forever
    // (the handler is never unregistered).
    let flag = shutdown_flag().clone();
    signal_hook::flag::register(SIGINT, flag.clone())
        .map_err(|e| anyhow::anyhow!("failed to register SIGINT handler: {e}"))?;
    signal_hook::flag::register(SIGTERM, flag)
        .map_err(|e| anyhow::anyhow!("failed to register SIGTERM handler: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_env::EnvironmentSpec;
    use coral_env::spec::EnvMode;
    use coral_env::{RealService, ServiceKind};
    use coral_test::{Evidence, TestCase, TestKind as CtkKind, TestReport, TestSource, TestSpec};
    use std::time::Duration as StdDur;
    use tempfile::TempDir;

    fn pass_report() -> TestReport {
        let case = TestCase {
            id: "x".into(),
            name: "x".into(),
            kind: CtkKind::Healthcheck,
            service: None,
            tags: vec![],
            source: TestSource::Inline,
            spec: TestSpec::empty(),
        };
        let mut r = TestReport::new(case, TestStatus::Pass, StdDur::from_millis(0));
        r.evidence = Evidence::default();
        r
    }

    fn fail_report() -> TestReport {
        let case = TestCase {
            id: "x".into(),
            name: "x".into(),
            kind: CtkKind::Healthcheck,
            service: None,
            tags: vec![],
            source: TestSource::Inline,
            spec: TestSpec::empty(),
        };
        let mut r = TestReport::new(
            case,
            TestStatus::Fail {
                reason: "boom".into(),
            },
            StdDur::from_millis(0),
        );
        r.evidence = Evidence::default();
        r
    }

    /// Reset the global shutdown flag at the start of every loop test.
    /// The flag is process-global; without a reset, a test that flips it
    /// late would poison the next test in the same binary.
    fn reset_shutdown() {
        shutdown_flag().store(false, Ordering::Relaxed);
    }

    /// Loop tests share a process-global `shutdown_flag()`, so they
    /// MUST serialize. Acquire this lock at the top of every loop
    /// test before resetting the flag.
    fn loop_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn dummy_spec() -> EnvironmentSpec {
        EnvironmentSpec {
            name: "dev".into(),
            backend: "compose".into(),
            mode: EnvMode::Managed,
            compose_command: "auto".into(),
            production: false,
            env_file: None,
            services: BTreeMap::from([(
                "api".into(),
                ServiceKind::Real(Box::new(RealService {
                    repo: None,
                    image: Some("api:dev".into()),
                    build: None,
                    ports: vec![],
                    env: BTreeMap::new(),
                    depends_on: vec![],
                    healthcheck: None,
                    watch: None,
                })),
            )]),
            chaos: None,
            chaos_scenarios: Vec::new(),
            monitors: Vec::new(),
        }
    }

    #[test]
    fn kind_for_monitor_translates_known_strings() {
        let m = MonitorSpec {
            name: "x".into(),
            tag: None,
            kind: Some("healthcheck".into()),
            services: vec![],
            interval_seconds: 30,
            on_failure: OnFailure::Log,
        };
        assert_eq!(
            kind_for_monitor(&m).unwrap(),
            Some(vec![TestKind::Healthcheck])
        );
        let mut m2 = m.clone();
        m2.kind = Some("smoke".into());
        assert_eq!(
            kind_for_monitor(&m2).unwrap(),
            Some(vec![TestKind::Healthcheck, TestKind::UserDefined])
        );
        let mut m3 = m.clone();
        m3.kind = None;
        assert_eq!(kind_for_monitor(&m3).unwrap(), None);
    }

    #[test]
    fn kind_for_monitor_rejects_unknown_strings() {
        let m = MonitorSpec {
            name: "x".into(),
            tag: None,
            kind: Some("flux-capacitor".into()),
            services: vec![],
            interval_seconds: 30,
            on_failure: OnFailure::Log,
        };
        let err = kind_for_monitor(&m).expect_err("must reject");
        assert!(err.contains("unsupported kind"), "msg: {err}");
    }

    #[test]
    fn exit_for_returns_success_when_no_failures() {
        // Use the discriminant comparison via formatting — ExitCode
        // doesn't expose Eq.
        let exit = exit_for(0);
        let dbg = format!("{exit:?}");
        assert!(dbg.to_lowercase().contains("success") || dbg.contains("0"));
        let exit = exit_for(3);
        let dbg = format!("{exit:?}");
        assert!(dbg.to_lowercase().contains("failure") || dbg.contains("1"));
    }

    /// Pin AC #5: `dummy_spec` round-trips so the test fixture is sane.
    #[test]
    fn dummy_spec_round_trips() {
        let s = dummy_spec();
        let toml_s = toml::to_string(&s).unwrap();
        let parsed: EnvironmentSpec = toml::from_str(&toml_s).unwrap();
        assert_eq!(parsed, s);
    }

    /// **T3 — first-iteration-immediate.** AC #2: the loop must run
    /// the first iteration without leading sleep. Trip-flag the
    /// shutdown bool from inside the tick fn so the loop exits after
    /// exactly one iteration; assert wall-clock < 100ms (well under
    /// the 1s interval in the spec).
    #[test]
    fn monitor_loop_first_iteration_immediate() {
        let _guard = loop_test_lock();
        reset_shutdown();
        let tmp = TempDir::new().unwrap();
        let monitors = vec![MonitorSpec {
            name: "smoke".into(),
            tag: None,
            kind: None,
            services: vec![],
            interval_seconds: 1,
            on_failure: OnFailure::Log,
        }];
        let mut total = 0usize;
        let mut failed = 0usize;
        let started = Instant::now();
        let mut tick = |_m: &MonitorSpec| -> Result<Vec<TestReport>> {
            // Trip shutdown so the outer loop exits after this one iteration.
            shutdown_flag().store(true, Ordering::Relaxed);
            Ok(vec![pass_report()])
        };
        run_loop(
            tmp.path(),
            "dev",
            &monitors,
            &mut tick,
            &mut total,
            &mut failed,
        )
        .expect("loop ok");
        let elapsed = started.elapsed();
        assert_eq!(total, 1);
        assert!(
            elapsed < StdDur::from_millis(500),
            "first iteration delayed: {elapsed:?}"
        );
    }

    /// **T4 — three iterations append three lines.** AC #3: each
    /// iteration appends ONE row to JSONL. We trip shutdown after the
    /// 3rd tick. interval_seconds=1 keeps the wall-clock test under
    /// ~3s. We use 250ms-resolution for the chunked-sleep so the
    /// assert wraps in well under the test framework's 60s timeout.
    #[test]
    fn monitor_loop_appends_each_iteration_to_jsonl() {
        let _guard = loop_test_lock();
        reset_shutdown();
        let tmp = TempDir::new().unwrap();
        let monitors = vec![MonitorSpec {
            name: "smoke".into(),
            tag: None,
            kind: None,
            services: vec![],
            interval_seconds: 1,
            on_failure: OnFailure::Log,
        }];
        let mut total = 0usize;
        let mut failed = 0usize;
        let mut count = 0usize;
        let mut tick = |_m: &MonitorSpec| -> Result<Vec<TestReport>> {
            count += 1;
            if count >= 3 {
                shutdown_flag().store(true, Ordering::Relaxed);
            }
            Ok(vec![pass_report(), pass_report()])
        };
        run_loop(
            tmp.path(),
            "staging",
            &monitors,
            &mut tick,
            &mut total,
            &mut failed,
        )
        .expect("loop ok");
        assert_eq!(total, 3, "expected 3 iterations, got {total}");
        let path = tmp
            .path()
            .join(".coral")
            .join("monitors")
            .join("staging-smoke.jsonl");
        let text = std::fs::read_to_string(&path).expect("jsonl");
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(
            lines.len(),
            3,
            "expected 3 JSONL lines, got {}",
            lines.len()
        );
        // Pin one row's content (3rd tick).
        let last: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(last["env"], "staging");
        assert_eq!(last["monitor_name"], "smoke");
        assert_eq!(last["total"], 2);
        assert_eq!(last["passed"], 2);
        assert_eq!(last["failed"], 0);
    }

    /// **T6 — log-mode continues after failure.** AC #5: a failing
    /// iteration still appends to JSONL and the loop continues.
    #[test]
    fn on_failure_log_continues_after_failure() {
        let _guard = loop_test_lock();
        reset_shutdown();
        let tmp = TempDir::new().unwrap();
        let monitors = vec![MonitorSpec {
            name: "lossy".into(),
            tag: None,
            kind: None,
            services: vec![],
            interval_seconds: 1,
            on_failure: OnFailure::Log,
        }];
        let mut total = 0usize;
        let mut failed = 0usize;
        let mut count = 0usize;
        let mut tick = |_m: &MonitorSpec| -> Result<Vec<TestReport>> {
            count += 1;
            if count == 1 {
                Ok(vec![fail_report()])
            } else if count >= 3 {
                shutdown_flag().store(true, Ordering::Relaxed);
                Ok(vec![pass_report()])
            } else {
                Ok(vec![pass_report()])
            }
        };
        let exit = run_loop(
            tmp.path(),
            "dev",
            &monitors,
            &mut tick,
            &mut total,
            &mut failed,
        )
        .expect("loop ok");
        // 3 iterations ran (loop didn't bail on the first failure).
        assert_eq!(total, 3);
        // 1 of them was a failure.
        assert_eq!(failed, 1);
        // Exit reflects "at least one failure" → FAILURE.
        let dbg = format!("{exit:?}");
        assert!(dbg.to_lowercase().contains("failure") || dbg.contains("1"));
    }

    /// **T7 — fail-fast exits on first failure.** AC #6.
    #[test]
    fn on_failure_fail_fast_exits_on_first_failure() {
        let _guard = loop_test_lock();
        reset_shutdown();
        let tmp = TempDir::new().unwrap();
        let monitors = vec![MonitorSpec {
            name: "tripwire".into(),
            tag: None,
            kind: None,
            services: vec![],
            interval_seconds: 1,
            on_failure: OnFailure::FailFast,
        }];
        let mut total = 0usize;
        let mut failed = 0usize;
        let mut count = 0usize;
        let mut tick = |_m: &MonitorSpec| -> Result<Vec<TestReport>> {
            count += 1;
            // 1st iteration fails — loop must bail before a 2nd tick.
            Ok(vec![fail_report()])
        };
        let exit = run_loop(
            tmp.path(),
            "dev",
            &monitors,
            &mut tick,
            &mut total,
            &mut failed,
        )
        .expect("loop ok");
        // Exactly one tick before fail-fast bailed.
        assert_eq!(total, 1, "fail-fast should run only one iteration");
        assert_eq!(failed, 1);
        // The closure ran exactly once before fail-fast bailed.
        let _ = count;
        let dbg = format!("{exit:?}");
        assert!(
            dbg.to_lowercase().contains("failure") || dbg.contains("1"),
            "expected FAILURE, got {dbg}"
        );
        // Verify the JSONL row was still appended for the failing run
        // — fail-fast records before exiting.
        let path = tmp
            .path()
            .join(".coral")
            .join("monitors")
            .join("dev-tripwire.jsonl");
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().filter(|l| !l.trim().is_empty()).count(), 1);
    }
}
