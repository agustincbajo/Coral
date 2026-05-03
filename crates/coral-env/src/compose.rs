//! `ComposeBackend` — `docker compose` v2 / `docker-compose` v1 / `podman compose` wrapper.
//!
//! v0.17 ships only `up` / `down` / `status` / `logs` / `exec`. Watch
//! (compose 2.22 `develop.watch`), devcontainer emit, port-forward, and
//! attach/reset/prune follow in v0.17.x.
//!
//! For the v0.17 wave-1 release this module is intentionally a
//! **placeholder** with the trait wiring + runtime detection in place
//! but actual subprocess invocation (compose YAML rendering, `up -d`,
//! container introspection) deferred to wave 2 to keep the diff
//! reviewable. `MockBackend` covers the upstream tests.

use crate::{
    DownOptions, EnvBackend, EnvCapabilities, EnvError, EnvHandle, EnvPlan, EnvResult, EnvStatus,
    ExecOptions, ExecOutput, LogLine, LogsOptions, UpOptions,
};
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

    fn up(&self, _plan: &EnvPlan, _opts: &UpOptions) -> EnvResult<EnvHandle> {
        Err(EnvError::BackendError {
            backend: "compose".into(),
            message:
                "ComposeBackend::up() is wave-2; v0.17.0 ships only the trait + runtime detection"
                    .into(),
        })
    }

    fn down(&self, _plan: &EnvPlan, _opts: &DownOptions) -> EnvResult<()> {
        Err(EnvError::BackendError {
            backend: "compose".into(),
            message: "ComposeBackend::down() is wave-2".into(),
        })
    }

    fn status(&self, _plan: &EnvPlan) -> EnvResult<EnvStatus> {
        Err(EnvError::BackendError {
            backend: "compose".into(),
            message: "ComposeBackend::status() is wave-2".into(),
        })
    }

    fn logs(
        &self,
        _plan: &EnvPlan,
        _service: &str,
        _opts: &LogsOptions,
    ) -> EnvResult<Vec<LogLine>> {
        Err(EnvError::BackendError {
            backend: "compose".into(),
            message: "ComposeBackend::logs() is wave-2".into(),
        })
    }

    fn exec(
        &self,
        _plan: &EnvPlan,
        _service: &str,
        _cmd: &[String],
        _opts: &ExecOptions,
    ) -> EnvResult<ExecOutput> {
        Err(EnvError::BackendError {
            backend: "compose".into(),
            message: "ComposeBackend::exec() is wave-2".into(),
        })
    }

    fn capabilities(&self) -> EnvCapabilities {
        EnvCapabilities {
            watch: false, // wave-2
            exec: false,
            logs_follow: false,
            port_forward_explicit: false,
            emit_devcontainer: false,
        }
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
}
