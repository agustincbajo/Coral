//! `HealthcheckRunner` — auto-derives `TestCase`s from each service's
//! `service.healthcheck` declaration in `coral.toml` and executes one
//! probe per case. Powers `coral verify`.

use crate::error::{TestError, TestResult};
use crate::probe::probe_once;
use crate::report::{Evidence, TestReport, TestStatus};
use crate::spec::{TestCase, TestKind, TestSource, TestSpec};
use crate::{ParallelismHint, TestRunner};
use coral_env::healthcheck::ProbeResult;
use coral_env::{
    EnvBackend, EnvHandle, EnvPlan, EnvironmentSpec, Healthcheck, RealService, ServiceKind,
    ServiceStatus,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct HealthcheckRunner {
    backend: Arc<dyn EnvBackend>,
    plan: EnvPlan,
    spec: EnvironmentSpec,
}

impl HealthcheckRunner {
    pub fn new(backend: Arc<dyn EnvBackend>, plan: EnvPlan, spec: EnvironmentSpec) -> Self {
        Self {
            backend,
            plan,
            spec,
        }
    }

    /// Build the list of `TestCase`s from declared healthchecks. One
    /// case per service that declares `healthcheck = { ... }`.
    pub fn cases_from_spec(spec: &EnvironmentSpec) -> Vec<TestCase> {
        let mut cases = Vec::new();
        for (name, kind) in &spec.services {
            if let ServiceKind::Real(real) = kind {
                if real.healthcheck.is_some() {
                    cases.push(TestCase {
                        id: format!("healthcheck:{name}"),
                        name: format!("{name} healthcheck"),
                        kind: TestKind::Healthcheck,
                        service: Some(name.clone()),
                        tags: vec!["healthcheck".into(), "smoke".into()],
                        source: TestSource::Discovered {
                            from: format!(
                                "[environments.{}.services.{name}.healthcheck]",
                                spec.name
                            ),
                        },
                        spec: TestSpec::empty(),
                    });
                }
            }
        }
        cases
    }
}

impl TestRunner for HealthcheckRunner {
    fn name(&self) -> &'static str {
        "healthcheck"
    }

    fn supports(&self, kind: TestKind) -> bool {
        matches!(kind, TestKind::Healthcheck)
    }

    fn run(&self, case: &TestCase, _env: &EnvHandle) -> TestResult<TestReport> {
        let started = Instant::now();
        let service_name = case.service.as_deref().unwrap_or_default();
        let real = lookup_real_service(&self.spec, service_name)
            .ok_or_else(|| TestError::ServiceNotExposed(service_name.to_string()))?;
        let hc: &Healthcheck = match real.healthcheck.as_ref() {
            Some(h) => h,
            None => {
                return Ok(TestReport::new(
                    case.clone(),
                    TestStatus::Skip {
                        reason: format!("service '{service_name}' has no healthcheck declared"),
                    },
                    Duration::from_millis(0),
                ));
            }
        };

        let env_status = self.backend.status(&self.plan)?;
        let svc_status = match env_status
            .services
            .into_iter()
            .find(|s| s.name == service_name)
        {
            Some(s) => s,
            None => {
                return Ok(report(
                    case.clone(),
                    TestStatus::Fail {
                        reason: format!(
                            "service '{service_name}' is not running in this environment"
                        ),
                    },
                    started,
                ));
            }
        };

        if matches!(svc_status.state, coral_env::ServiceState::Crashed) {
            return Ok(report(
                case.clone(),
                TestStatus::Fail {
                    reason: format!("service '{service_name}' is in `crashed` state"),
                },
                started,
            ));
        }

        let timeout = Duration::from_secs(hc.timing.timeout_s.max(1) as u64);
        let result = probe_once(&svc_status, &hc.kind, timeout);
        let status = match result {
            ProbeResult::Pass => TestStatus::Pass,
            ProbeResult::Fail => TestStatus::Fail {
                reason: format!(
                    "healthcheck probe for '{service_name}' returned Fail (state={:?}, health={:?})",
                    svc_status.state, svc_status.health
                ),
            },
        };
        let mut report = TestReport::new(case.clone(), status, started.elapsed());
        report.evidence = evidence_from_status(&svc_status);
        Ok(report)
    }

    fn discover(&self, _project_root: &Path) -> TestResult<Vec<TestCase>> {
        Ok(Self::cases_from_spec(&self.spec))
    }

    fn parallelism_hint(&self) -> ParallelismHint {
        ParallelismHint::Isolated
    }
}

fn lookup_real_service<'s>(spec: &'s EnvironmentSpec, name: &str) -> Option<&'s RealService> {
    match spec.services.get(name) {
        Some(ServiceKind::Real(real)) => Some(real),
        _ => None,
    }
}

fn report(case: TestCase, status: TestStatus, started: Instant) -> TestReport {
    TestReport::new(case, status, started.elapsed())
}

fn evidence_from_status(status: &ServiceStatus) -> Evidence {
    let mut e = Evidence::default();
    let pretty = format!(
        "state={:?} health={:?} restarts={} ports={:?}",
        status.state, status.health, status.restarts, status.published_ports
    );
    e.stdout_tail = Some(pretty);
    e
}

/// Convenience for the CLI: build cases + run them all sequentially
/// against a live env. Returns one TestReport per case.
pub fn run_all(runner: &HealthcheckRunner, env: &EnvHandle) -> TestResult<Vec<TestReport>> {
    let cases = HealthcheckRunner::cases_from_spec(&runner.spec);
    let mut reports = Vec::with_capacity(cases.len());
    for case in cases {
        reports.push(runner.run(&case, env)?);
    }
    Ok(reports)
}

// Brings BTreeMap into scope for the test fixture below.
#[allow(unused_imports)]
use BTreeMap as _BTreeMap;

#[cfg(test)]
mod tests {
    use super::*;
    use coral_env::spec::{EnvMode, HealthcheckKind, HealthcheckTiming};
    use std::collections::BTreeMap as Map;

    fn spec_with_healthcheck() -> EnvironmentSpec {
        let mut services = Map::new();
        services.insert(
            "api".to_string(),
            ServiceKind::Real(Box::new(RealService {
                repo: None,
                image: Some("nginx:latest".into()),
                build: None,
                ports: vec![80],
                env: Map::new(),
                depends_on: vec![],
                healthcheck: Some(Healthcheck {
                    kind: HealthcheckKind::Tcp { port: 80 },
                    timing: HealthcheckTiming::default(),
                }),
                watch: None,
            })),
        );
        EnvironmentSpec {
            name: "dev".into(),
            backend: "compose".into(),
            mode: EnvMode::Managed,
            compose_command: "auto".into(),
            production: false,
            env_file: None,
            services,
        }
    }

    #[test]
    fn cases_from_spec_emits_one_case_per_healthcheck() {
        let spec = spec_with_healthcheck();
        let cases = HealthcheckRunner::cases_from_spec(&spec);
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].id, "healthcheck:api");
        assert!(cases[0].tags.contains(&"smoke".to_string()));
    }
}
