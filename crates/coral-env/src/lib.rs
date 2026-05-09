//! Coral environment layer (v0.17+).
//!
//! Pluggable backend trait — `EnvBackend` — for bringing up the
//! multi-service dev environment described in a `coral.toml`. The MVP
//! ships `ComposeBackend` (subprocess `docker compose` / `docker-compose`
//! / `podman compose`); future backends (`KindBackend`, `TiltBackend`,
//! `K3dBackend`) live behind Cargo features so the default dep tree
//! stays slim.
//!
//! Mirrors the shape of `coral_runner::Runner` deliberately — same
//! `Send + Sync`, `thiserror` errors, `MockBackend` for tests, factory
//! function on the manifest's `[environment].backend` string.

pub mod compose;
pub mod compose_yaml;
pub mod devcontainer;
pub mod error;
pub mod healthcheck;
pub mod import;
pub mod mock;
pub mod plan;
pub mod spec;

pub use devcontainer::{DevcontainerArtifact, DevcontainerOpts, render_devcontainer};
pub use error::{EnvError, EnvResult};
pub use mock::MockBackend;
pub use plan::{
    EnvHandle, EnvPlan, EnvStatus, HealthState, LogLine, LogStream, PublishedPort, ServiceSpecPlan,
    ServiceState, ServiceStatus,
};
pub use spec::{
    ChaosBackend, ChaosConfig, ChaosScenario, EnvironmentSpec, Healthcheck, HealthcheckKind,
    HealthcheckTiming, MonitorSpec, OnFailure, RealService, ServiceKind, ToxicKind,
};

use std::path::PathBuf;

/// Pluggable backend trait. The MVP only requires the lifecycle methods
/// (`up`/`down`/`status`) plus `logs`/`exec`; live-reload (`watch`),
/// devcontainer/k8s emit (`emit`), explicit port-forwarding, and
/// `attach`/`reset`/`prune` follow in v0.17.x as the testing layer
/// (v0.18) needs them.
pub trait EnvBackend: Send + Sync {
    fn name(&self) -> &'static str;

    fn up(&self, plan: &EnvPlan, opts: &UpOptions) -> EnvResult<EnvHandle>;
    fn down(&self, plan: &EnvPlan, opts: &DownOptions) -> EnvResult<()>;
    fn status(&self, plan: &EnvPlan) -> EnvResult<EnvStatus>;
    fn logs(&self, plan: &EnvPlan, service: &str, opts: &LogsOptions) -> EnvResult<Vec<LogLine>>;
    fn exec(
        &self,
        plan: &EnvPlan,
        service: &str,
        cmd: &[String],
        opts: &ExecOptions,
    ) -> EnvResult<ExecOutput>;

    fn capabilities(&self) -> EnvCapabilities {
        EnvCapabilities::default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct UpOptions {
    pub services: Vec<String>,
    pub detach: bool,
    pub build: bool,
    pub watch: bool,
}

#[derive(Debug, Clone, Default)]
pub struct DownOptions {
    pub volumes: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LogsOptions {
    pub follow: bool,
    pub tail: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct ExecOptions {
    pub working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EnvCapabilities {
    pub watch: bool,
    pub exec: bool,
    pub logs_follow: bool,
    pub port_forward_explicit: bool,
    pub emit_devcontainer: bool,
}
