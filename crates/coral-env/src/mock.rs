//! In-memory `EnvBackend` for tests.
//!
//! Mirrors the `MockRunner` pattern in `coral-runner` — scripted FIFO
//! responses + a `calls()` recorder. Lets the CLI tests assert that
//! `coral up` invokes `EnvBackend::up()` with the right plan, without
//! needing Docker or any subprocess.

use crate::{
    DownOptions, EnvBackend, EnvCapabilities, EnvError, EnvHandle, EnvPlan, EnvResult, EnvStatus,
    ExecOptions, ExecOutput, HealthState, LogLine, LogsOptions, ServiceState, ServiceStatus,
    UpOptions,
};
use std::sync::Mutex;

/// `MockBackend` records every call and returns scripted responses.
pub struct MockBackend {
    inner: Mutex<MockState>,
}

#[derive(Default)]
struct MockState {
    pub calls: Vec<MockCall>,
    /// Pre-scripted statuses returned by `status()`. Drains in order.
    pub statuses: std::collections::VecDeque<EnvStatus>,
}

#[derive(Debug, Clone)]
pub enum MockCall {
    Up {
        services: Vec<String>,
        watch: bool,
        build: bool,
    },
    Down {
        volumes: bool,
    },
    Status,
    Logs {
        service: String,
        follow: bool,
    },
    Exec {
        service: String,
        cmd: Vec<String>,
    },
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MockState::default()),
        }
    }

    pub fn push_status(&self, status: EnvStatus) {
        self.inner.lock().unwrap().statuses.push_back(status);
    }

    pub fn calls(&self) -> Vec<MockCall> {
        self.inner.lock().unwrap().calls.clone()
    }
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvBackend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn up(&self, plan: &EnvPlan, opts: &UpOptions) -> EnvResult<EnvHandle> {
        // v0.20.2 audit-followup #39: mirror `ComposeBackend::up`'s
        // `EnvMode::Adopt` rejection so the mock can't silently
        // succeed where the real backend errors. Pre-fix tests using
        // `MockBackend` could pass against an `Adopt` plan but the
        // production `ComposeBackend` would have refused it. Same
        // string shape so test assertions hold across both.
        if matches!(plan.mode, crate::spec::EnvMode::Adopt) {
            return Err(EnvError::InvalidSpec(
                "environment `mode = \"adopt\"` is reserved for v0.17.x; \
                 set `mode = \"managed\"` (the default) for now"
                    .into(),
            ));
        }
        self.inner.lock().unwrap().calls.push(MockCall::Up {
            services: opts.services.clone(),
            watch: opts.watch,
            build: opts.build,
        });
        Ok(EnvHandle {
            backend: "mock".to_string(),
            artifact_hash: "mock".to_string(),
            artifact_path: plan.project_root.join(".coral/env/mock/plan.json"),
            state: Default::default(),
        })
    }

    fn down(&self, _plan: &EnvPlan, opts: &DownOptions) -> EnvResult<()> {
        self.inner.lock().unwrap().calls.push(MockCall::Down {
            volumes: opts.volumes,
        });
        Ok(())
    }

    fn status(&self, plan: &EnvPlan) -> EnvResult<EnvStatus> {
        self.inner.lock().unwrap().calls.push(MockCall::Status);
        if let Some(status) = self.inner.lock().unwrap().statuses.pop_front() {
            return Ok(status);
        }
        // Default: every service Pending + Unknown.
        Ok(EnvStatus {
            services: plan
                .services
                .keys()
                .map(|name| ServiceStatus {
                    name: name.clone(),
                    state: ServiceState::Pending,
                    health: HealthState::Unknown,
                    restarts: 0,
                    published_ports: Vec::new(),
                })
                .collect(),
        })
    }

    fn logs(&self, _plan: &EnvPlan, service: &str, opts: &LogsOptions) -> EnvResult<Vec<LogLine>> {
        self.inner.lock().unwrap().calls.push(MockCall::Logs {
            service: service.to_string(),
            follow: opts.follow,
        });
        Ok(Vec::new())
    }

    fn exec(
        &self,
        _plan: &EnvPlan,
        service: &str,
        cmd: &[String],
        _opts: &ExecOptions,
    ) -> EnvResult<ExecOutput> {
        self.inner.lock().unwrap().calls.push(MockCall::Exec {
            service: service.to_string(),
            cmd: cmd.to_vec(),
        });
        Ok(ExecOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
        })
    }

    fn capabilities(&self) -> EnvCapabilities {
        EnvCapabilities {
            watch: true,
            exec: true,
            logs_follow: true,
            port_forward_explicit: false,
            emit_devcontainer: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::EnvPlan;

    fn empty_plan() -> EnvPlan {
        EnvPlan {
            name: "dev".into(),
            project_name: "coral-dev-deadbeef".into(),
            mode: crate::spec::EnvMode::Managed,
            services: Default::default(),
            env_file: None,
            project_root: std::path::PathBuf::from("/tmp"),
        }
    }

    #[test]
    fn mock_records_up_and_down() {
        let mb = MockBackend::new();
        let plan = empty_plan();
        mb.up(&plan, &UpOptions::default()).unwrap();
        mb.down(&plan, &DownOptions::default()).unwrap();
        let calls = mb.calls();
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0], MockCall::Up { .. }));
        assert!(matches!(calls[1], MockCall::Down { .. }));
    }

    #[test]
    fn mock_returns_pre_scripted_status() {
        let mb = MockBackend::new();
        mb.push_status(EnvStatus {
            services: vec![ServiceStatus {
                name: "api".into(),
                state: ServiceState::Running,
                health: HealthState::Pass,
                restarts: 0,
                published_ports: Vec::new(),
            }],
        });
        let status = mb.status(&empty_plan()).unwrap();
        assert_eq!(status.services.len(), 1);
        assert!(matches!(status.services[0].state, ServiceState::Running));
    }

    /// v0.20.2 audit-followup #39: regression — `MockBackend::up`
    /// must reject `EnvMode::Adopt` with the same `EnvError::InvalidSpec`
    /// that `ComposeBackend::up` raises. Pre-fix the mock returned
    /// `Ok` and the divergence was invisible to upstream tests.
    #[test]
    fn mock_up_rejects_adopt_mode_like_compose_does() {
        let mb = MockBackend::new();
        let mut plan = empty_plan();
        plan.mode = crate::spec::EnvMode::Adopt;
        let err = mb
            .up(&plan, &UpOptions::default())
            .expect_err("must reject adopt");
        match err {
            EnvError::InvalidSpec(msg) => {
                assert!(
                    msg.contains("adopt"),
                    "error must mention adopt mode: {msg}"
                );
            }
            other => panic!("expected InvalidSpec, got {other:?}"),
        }
        // No call should be recorded — the rejection happens before
        // we touch state.
        assert!(
            mb.calls().is_empty(),
            "rejected up() must not record a call"
        );
    }

    /// v0.20.2 audit-followup #39: managed mode (the default) keeps
    /// working unchanged. Pin so a future overzealous gate doesn't
    /// regress the happy path.
    #[test]
    fn mock_up_accepts_managed_mode() {
        let mb = MockBackend::new();
        let plan = empty_plan(); // default Managed
        let _ = mb
            .up(&plan, &UpOptions::default())
            .expect("managed mode must succeed");
        let calls = mb.calls();
        assert_eq!(calls.len(), 1);
        assert!(matches!(calls[0], MockCall::Up { .. }));
    }
}
