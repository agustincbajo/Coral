//! In-memory `EnvBackend` for tests.
//!
//! Mirrors the `MockRunner` pattern in `coral-runner` — scripted FIFO
//! responses + a `calls()` recorder. Lets the CLI tests assert that
//! `coral up` invokes `EnvBackend::up()` with the right plan, without
//! needing Docker or any subprocess.

use crate::{
    DownOptions, EnvBackend, EnvCapabilities, EnvHandle, EnvPlan, EnvResult, EnvStatus,
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
}
