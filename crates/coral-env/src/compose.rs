//! `ComposeBackend` — `docker compose` v2 / `docker-compose` v1 / `podman compose` wrapper.
//!
//! v0.17 wave 2 wires the real subprocess lifecycle: `up -d`, `down`,
//! `ps --format json` for status, `logs`, `exec`. `develop.watch`
//! follows in v0.17.x once the rebuild/healthcheck flapping
//! interaction is pinned by the integration test (PRD risk #6).

use crate::compose_yaml;
use crate::plan::{
    EnvHandle, EnvPlan, EnvStatus, HealthState, LogLine, LogStream, PublishedPort, ServiceState,
    ServiceStatus,
};
use crate::{
    DownOptions, EnvBackend, EnvCapabilities, EnvError, EnvResult, ExecOptions, ExecOutput,
    LogsOptions, UpOptions,
};
use chrono::Utc;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Auto / docker / podman selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeRuntime {
    Auto,
    Docker,
    Podman,
}

impl ComposeRuntime {
    /// Parse from a `coral.toml` `compose_command` string. Unknown
    /// values fall back to `Auto`. Named `parse` (not `from_str`) to
    /// avoid colliding with the `std::str::FromStr` trait.
    pub fn parse(s: &str) -> Self {
        match s {
            "docker" => Self::Docker,
            "podman" => Self::Podman,
            _ => Self::Auto,
        }
    }
}

pub struct ComposeBackend {
    runtime: ComposeRuntime,
}

impl ComposeBackend {
    pub fn new(runtime: ComposeRuntime) -> Self {
        Self { runtime }
    }

    /// Probe the runtime: returns `(binary, args_prefix)` so callers
    /// can run e.g. `docker compose up` or `podman compose up` via
    /// the same `Command::new(binary).args(prefix).args(...)` pattern.
    pub fn detect_invocation(&self) -> EnvResult<(String, Vec<String>)> {
        match self.runtime {
            ComposeRuntime::Auto => {
                if try_invocation("docker", &["compose", "version"]) {
                    return Ok(("docker".into(), vec!["compose".into()]));
                }
                if try_invocation("docker-compose", &["version"]) {
                    return Ok(("docker-compose".into(), vec![]));
                }
                if try_invocation("podman", &["compose", "version"]) {
                    return Ok(("podman".into(), vec!["compose".into()]));
                }
                Err(EnvError::BackendNotFound {
                    backend: "compose".into(),
                    hint: "no `docker`, `docker-compose`, or `podman compose` on PATH".into(),
                })
            }
            ComposeRuntime::Docker => {
                if try_invocation("docker", &["compose", "version"]) {
                    Ok(("docker".into(), vec!["compose".into()]))
                } else if try_invocation("docker-compose", &["version"]) {
                    Ok(("docker-compose".into(), vec![]))
                } else {
                    Err(EnvError::BackendNotFound {
                        backend: "compose".into(),
                        hint: "neither `docker compose` nor `docker-compose` is on PATH".into(),
                    })
                }
            }
            ComposeRuntime::Podman => {
                if try_invocation("podman", &["compose", "version"]) {
                    Ok(("podman".into(), vec!["compose".into()]))
                } else {
                    Err(EnvError::BackendNotFound {
                        backend: "compose".into(),
                        hint: "`podman compose` is not on PATH".into(),
                    })
                }
            }
        }
    }

    /// Render the plan to YAML, write it to `.coral/env/compose/<hash>.yml`,
    /// and return the path + hash. Idempotent: re-rendering an
    /// unchanged plan yields the same path so `down()` can find it.
    fn render_plan_artifact(&self, plan: &EnvPlan) -> EnvResult<(PathBuf, String)> {
        let yaml = compose_yaml::render(plan);
        let hash = compose_yaml::content_hash(&yaml);
        let dir = plan.project_root.join(".coral/env/compose");
        let path = dir.join(format!("{hash}.yml"));
        std::fs::create_dir_all(&dir).map_err(|source| EnvError::Io {
            path: dir.clone(),
            source,
        })?;
        // v0.19.5 audit H9: write atomically (temp + rename) so a
        // concurrent reader (e.g. `docker compose up` racing this
        // process) sees either the OLD or the NEW YAML, never a
        // half-written file. The bytes round-trip identically because
        // atomic_write_string just defers to fs::write under the hood.
        coral_core::atomic::atomic_write_string(&path, &yaml).map_err(|e| match e {
            coral_core::error::CoralError::Io { path: p, source } => {
                EnvError::Io { path: p, source }
            }
            other => EnvError::Io {
                path: path.clone(),
                source: std::io::Error::other(other.to_string()),
            },
        })?;
        Ok((path, hash))
    }

    fn run_compose(
        &self,
        plan: &EnvPlan,
        artifact: &Path,
        extra_args: &[&str],
    ) -> EnvResult<std::process::Output> {
        let (bin, prefix) = self.detect_invocation()?;
        let mut cmd = Command::new(&bin);
        for arg in &prefix {
            cmd.arg(arg);
        }
        cmd.arg("--file").arg(artifact);
        cmd.arg("--project-name").arg(&plan.project_name);
        for arg in extra_args {
            cmd.arg(arg);
        }
        if let Some(env_file) = &plan.env_file {
            cmd.env("COMPOSE_ENV_FILES", env_file);
        }
        cmd.output().map_err(|e| EnvError::BackendError {
            backend: "compose".into(),
            message: format!("failed to invoke {bin}: {e}"),
        })
    }
}

fn try_invocation(bin: &str, args: &[&str]) -> bool {
    Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

impl EnvBackend for ComposeBackend {
    fn name(&self) -> &'static str {
        "compose"
    }

    fn up(&self, plan: &EnvPlan, opts: &UpOptions) -> EnvResult<EnvHandle> {
        // `mode = "adopt"` requires invoking the user's existing
        // `docker-compose.yml` instead of generating one. Wave 3 of
        // v0.17 will wire this; until then, fail loudly so users
        // don't unknowingly run a managed YAML when they declared
        // `mode = "adopt"`.
        if matches!(plan.mode, crate::spec::EnvMode::Adopt) {
            return Err(EnvError::InvalidSpec(
                "environment `mode = \"adopt\"` is reserved for v0.17.x; \
                 set `mode = \"managed\"` (the default) for now"
                    .into(),
            ));
        }
        let (artifact_path, artifact_hash) = self.render_plan_artifact(plan)?;
        let mut args: Vec<&str> = vec!["up"];
        if opts.detach {
            args.push("-d");
        }
        if opts.build {
            args.push("--build");
        }
        // `--wait` makes compose poll healthchecks itself; we additionally run
        // `wait_for_healthy` from the higher layer for backend-portable behavior.
        if opts.detach {
            args.push("--wait");
        }
        let owned_services: Vec<String> = opts.services.clone();
        for s in &owned_services {
            args.push(s.as_str());
        }
        let output = self.run_compose(plan, &artifact_path, &args)?;
        if !output.status.success() {
            let tail = tail(&String::from_utf8_lossy(&output.stderr));
            return Err(EnvError::BackendError {
                backend: "compose".into(),
                message: format!("up failed: {tail}"),
            });
        }
        let mut state = BTreeMap::new();
        state.insert("project_name".into(), plan.project_name.clone());
        Ok(EnvHandle {
            backend: "compose".into(),
            artifact_hash,
            artifact_path,
            state,
        })
    }

    fn down(&self, plan: &EnvPlan, opts: &DownOptions) -> EnvResult<()> {
        let (artifact_path, _) = self.render_plan_artifact(plan)?;
        let mut args: Vec<&str> = vec!["down"];
        if opts.volumes {
            args.push("--volumes");
        }
        let output = self.run_compose(plan, &artifact_path, &args)?;
        if !output.status.success() {
            let tail = tail(&String::from_utf8_lossy(&output.stderr));
            return Err(EnvError::BackendError {
                backend: "compose".into(),
                message: format!("down failed: {tail}"),
            });
        }
        Ok(())
    }

    fn status(&self, plan: &EnvPlan) -> EnvResult<EnvStatus> {
        let (artifact_path, _) = self.render_plan_artifact(plan)?;
        let output =
            self.run_compose(plan, &artifact_path, &["ps", "--all", "--format", "json"])?;
        if !output.status.success() {
            // `ps` on an env that has never been `up`'d returns
            // success with empty stdout; treat any non-success as
            // empty rather than an error to keep `coral status`
            // resilient.
            return Ok(EnvStatus {
                services: plan
                    .services
                    .keys()
                    .map(|name| ServiceStatus {
                        name: name.clone(),
                        state: ServiceState::Pending,
                        health: HealthState::Unknown,
                        restarts: 0,
                        published_ports: vec![],
                    })
                    .collect(),
            });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut by_name: BTreeMap<String, ServiceStatus> = plan
            .services
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    ServiceStatus {
                        name: name.clone(),
                        state: ServiceState::Pending,
                        health: HealthState::Unknown,
                        restarts: 0,
                        published_ports: vec![],
                    },
                )
            })
            .collect();
        // compose `ps --format json` emits one JSON object per line in v2
        // and a JSON array in some older v2.x. Both shapes parse below
        // because we feed every non-empty line through `from_str` and
        // accept either Object or Array results.
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() || line == "[" || line == "]" {
                continue;
            }
            let value: Result<serde_json::Value, _> = serde_json::from_str(line);
            let entries = match value {
                Ok(serde_json::Value::Array(a)) => a,
                Ok(v) => vec![v],
                Err(_) => continue,
            };
            for entry in entries {
                let name = entry
                    .get("Service")
                    .or_else(|| entry.get("Name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                if let Some(status) = by_name.get_mut(&name) {
                    status.state =
                        parse_state(entry.get("State").and_then(|v| v.as_str()).unwrap_or(""));
                    status.health =
                        parse_health(entry.get("Health").and_then(|v| v.as_str()).unwrap_or(""));
                    status.restarts = entry
                        .get("RestartCount")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    if let Some(ports_str) = entry.get("Publishers") {
                        status.published_ports = parse_publishers(ports_str);
                    }
                }
            }
        }
        Ok(EnvStatus {
            services: by_name.into_values().collect(),
        })
    }

    fn logs(&self, plan: &EnvPlan, service: &str, opts: &LogsOptions) -> EnvResult<Vec<LogLine>> {
        if !plan.services.contains_key(service) {
            return Err(EnvError::ServiceNotFound(service.to_string()));
        }
        let (artifact_path, _) = self.render_plan_artifact(plan)?;
        let mut args: Vec<String> = vec![
            "logs".into(),
            "--no-color".into(),
            "--no-log-prefix".into(),
            "--timestamps".into(),
        ];
        if let Some(tail) = opts.tail {
            args.push("--tail".into());
            args.push(tail.to_string());
        }
        // `--follow` is intentionally not honored at this layer — the CLI
        // wraps log streaming directly via subprocess piping. Returning
        // accumulated lines is the right shape for the trait.
        args.push(service.to_string());
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self.run_compose(plan, &artifact_path, &argv)?;
        let raw = String::from_utf8_lossy(&output.stdout);
        let now = Utc::now();
        let mut lines = Vec::new();
        for line in raw.lines() {
            lines.push(LogLine {
                service: service.to_string(),
                timestamp: now,
                stream: LogStream::Stdout,
                line: line.to_string(),
            });
        }
        Ok(lines)
    }

    fn exec(
        &self,
        plan: &EnvPlan,
        service: &str,
        cmd: &[String],
        _opts: &ExecOptions,
    ) -> EnvResult<ExecOutput> {
        if !plan.services.contains_key(service) {
            return Err(EnvError::ServiceNotFound(service.to_string()));
        }
        let (artifact_path, _) = self.render_plan_artifact(plan)?;
        let mut args: Vec<String> = vec!["exec".into(), "-T".into(), service.to_string()];
        for arg in cmd {
            args.push(arg.clone());
        }
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self.run_compose(plan, &artifact_path, &argv)?;
        Ok(ExecOutput {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    fn capabilities(&self) -> EnvCapabilities {
        EnvCapabilities {
            watch: false, // wave-3
            exec: true,
            logs_follow: false, // CLI handles --follow via direct streaming
            port_forward_explicit: false,
            emit_devcontainer: false,
        }
    }
}

fn parse_state(s: &str) -> ServiceState {
    // `docker compose ps --format json` emits state strings like
    // "running", "exited", "created", "restarting", "paused".
    match s {
        "running" => ServiceState::Running,
        "starting" | "created" | "restarting" => ServiceState::Starting,
        "exited" | "dead" => ServiceState::Crashed,
        "stopped" => ServiceState::Stopped,
        "paused" => ServiceState::Stopped,
        "" => ServiceState::Unknown,
        _ => ServiceState::Unknown,
    }
}

fn parse_health(s: &str) -> HealthState {
    match s {
        "healthy" => HealthState::Pass,
        "unhealthy" => HealthState::Fail,
        _ => HealthState::Unknown,
    }
}

fn parse_publishers(value: &serde_json::Value) -> Vec<PublishedPort> {
    let mut out = Vec::new();
    if let Some(arr) = value.as_array() {
        for item in arr {
            let host_port = item
                .get("PublishedPort")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u16;
            let container_port =
                item.get("TargetPort").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
            if host_port > 0 || container_port > 0 {
                out.push(PublishedPort {
                    container_port,
                    host_port,
                });
            }
        }
    }
    out
}

fn tail(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= 400 {
        trimmed.to_string()
    } else {
        format!("…{}", &trimmed[trimmed.len() - 400..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_from_str_falls_back_to_auto() {
        assert_eq!(ComposeRuntime::parse("docker"), ComposeRuntime::Docker);
        assert_eq!(ComposeRuntime::parse("podman"), ComposeRuntime::Podman);
        assert_eq!(ComposeRuntime::parse("auto"), ComposeRuntime::Auto);
        assert_eq!(ComposeRuntime::parse("garbage"), ComposeRuntime::Auto);
    }

    #[test]
    fn name_is_compose() {
        let b = ComposeBackend::new(ComposeRuntime::Auto);
        assert_eq!(b.name(), "compose");
    }

    #[test]
    fn parse_state_maps_compose_strings() {
        assert!(matches!(parse_state("running"), ServiceState::Running));
        assert!(matches!(parse_state("created"), ServiceState::Starting));
        assert!(matches!(parse_state("exited"), ServiceState::Crashed));
        assert!(matches!(parse_state("stopped"), ServiceState::Stopped));
        assert!(matches!(parse_state("unknown"), ServiceState::Unknown));
    }

    #[test]
    fn parse_health_recognizes_healthy_and_unhealthy() {
        assert!(matches!(parse_health("healthy"), HealthState::Pass));
        assert!(matches!(parse_health("unhealthy"), HealthState::Fail));
        assert!(matches!(parse_health("starting"), HealthState::Unknown));
    }

    #[test]
    fn up_rejects_adopt_mode_with_invalid_spec_error() {
        // `mode = "adopt"` is reserved for v0.17.x and must not silently
        // fall through to managed-mode rendering — the user explicitly
        // declared a different intent.
        use crate::EnvBackend;
        use crate::plan::EnvPlan;
        use crate::spec::EnvMode;
        let plan = EnvPlan {
            name: "dev".into(),
            project_name: "coral-dev-deadbeef".into(),
            mode: EnvMode::Adopt,
            services: Default::default(),
            env_file: None,
            project_root: std::path::PathBuf::from("/tmp"),
        };
        let backend = ComposeBackend::new(ComposeRuntime::Auto);
        let err = backend
            .up(&plan, &Default::default())
            .expect_err("adopt mode must be rejected");
        match err {
            EnvError::InvalidSpec(msg) => {
                assert!(
                    msg.contains("adopt") && msg.contains("managed"),
                    "expected helpful error message, got: {msg}"
                );
            }
            other => panic!("expected InvalidSpec, got: {other:?}"),
        }
    }

    #[test]
    fn up_managed_mode_does_not_short_circuit_on_mode() {
        // Sanity-check the converse: a Managed plan must NOT be
        // rejected at the mode check (it'll fail later trying to invoke
        // docker, which is the correct error path for tests w/o docker).
        use crate::EnvBackend;
        use crate::plan::EnvPlan;
        use crate::spec::EnvMode;
        let plan = EnvPlan {
            name: "dev".into(),
            project_name: "coral-dev-deadbeef".into(),
            mode: EnvMode::Managed,
            services: Default::default(),
            env_file: None,
            project_root: std::path::PathBuf::from("/tmp"),
        };
        let backend = ComposeBackend::new(ComposeRuntime::Auto);
        let err = backend
            .up(&plan, &Default::default())
            .expect_err("docker is unavailable in unit tests");
        // The error must NOT be the InvalidSpec we'd get from adopt; it
        // should be a binary-not-found / backend error.
        if matches!(err, EnvError::InvalidSpec(_)) {
            panic!("managed mode should not be rejected as InvalidSpec");
        }
    }
}
