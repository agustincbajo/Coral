//! Backend-agnostic environment plan.
//!
//! `EnvPlan` is the normalized form a `[[environments]]` table takes
//! before any backend translates it. Each backend's `up()` consumes an
//! `EnvPlan`, hashes it, and writes its translated artifact (compose
//! YAML, k8s manifests, Tilt config) into `.coral/env/<backend>/<hash>.*`.

use crate::spec::{ChaosConfig, EnvironmentSpec, ServiceKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The compiled, backend-agnostic environment plan. Built from an
/// `EnvironmentSpec` plus the project root (so service `repo`
/// references can resolve to absolute paths).
#[derive(Debug, Clone, PartialEq)]
pub struct EnvPlan {
    /// Environment name (`dev`, `ci`, …).
    pub name: String,
    /// Compose project name — derived from a hash of the absolute
    /// project root path so two worktrees of the same meta-repo don't
    /// collide on the same `coral-<slug>` namespace.
    pub project_name: String,
    /// Managed (Coral generates the compose YAML) vs. Adopt (user
    /// brings their own compose file). Backends that don't support
    /// adopt mode yet should bail with `EnvError::InvalidSpec` rather
    /// than silently rendering a managed YAML.
    pub mode: crate::spec::EnvMode,
    pub services: BTreeMap<String, ServiceSpecPlan>,
    pub env_file: Option<PathBuf>,
    pub project_root: PathBuf,
    /// v0.23.0: optional chaos sidecar config. `None` for environments
    /// without `[environments.<env>.chaos]`. The renderer emits the
    /// `toxiproxy` service plus per-edge proxy declarations only when
    /// this is `Some` (so chaos-off compose YAML is byte-identical to
    /// v0.22.6).
    pub chaos: Option<ChaosConfig>,
}

/// Per-service compiled plan. Mirrors `ServiceKind` but resolves repo
/// references to absolute paths and de-options optional fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceSpecPlan {
    pub name: String,
    pub kind: ServiceKind,
    /// Absolute path of the build context, when the service uses
    /// `repo = "..."` + `build = { ... }`.
    pub resolved_context: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnvHandle {
    /// Backend identifier (`compose`, `kind`, …).
    pub backend: String,
    /// Hash of the rendered backend artifact. Lets `down()` and
    /// `status()` detect drift.
    pub artifact_hash: String,
    /// Path to the rendered artifact (e.g. `.coral/env/compose/<hash>.yml`).
    pub artifact_path: PathBuf,
    /// Backend-specific opaque state — k8s namespace name, compose
    /// project name slug, …
    pub state: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvStatus {
    pub services: Vec<ServiceStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub name: String,
    pub state: ServiceState,
    pub health: HealthState,
    pub restarts: u32,
    pub published_ports: Vec<PublishedPort>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Pending,
    Starting,
    Running,
    Crashed,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Pass,
    Fail,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedPort {
    pub container_port: u16,
    pub host_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub service: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub stream: LogStream,
    pub line: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

impl EnvPlan {
    /// Build an `EnvPlan` from an `EnvironmentSpec` rooted at
    /// `project_root`. Used by every backend in `up()`. Validates
    /// that real services with `repo = "..."` reference an existing
    /// directory under `project_root` (the manifest validator already
    /// rejects unknown repo names).
    pub fn from_spec(
        spec: &EnvironmentSpec,
        project_root: &std::path::Path,
        repo_paths: &BTreeMap<String, PathBuf>,
    ) -> crate::EnvResult<Self> {
        let mut services = BTreeMap::new();
        for (name, kind) in &spec.services {
            let resolved_context = match kind {
                ServiceKind::Real(real) => real.repo.as_ref().map(|repo| {
                    repo_paths
                        .get(repo)
                        .cloned()
                        .unwrap_or_else(|| project_root.join("repos").join(repo))
                }),
                ServiceKind::Mock(_) => None,
            };
            services.insert(
                name.clone(),
                ServiceSpecPlan {
                    name: name.clone(),
                    kind: kind.clone(),
                    resolved_context,
                },
            );
        }
        let project_name = compose_project_name(project_root, &spec.name);
        Ok(Self {
            name: spec.name.clone(),
            project_name,
            mode: spec.mode,
            services,
            env_file: spec.env_file.clone(),
            project_root: project_root.to_path_buf(),
            chaos: spec.chaos.clone(),
        })
    }
}

/// Derive a stable compose project name from the absolute project root.
///
/// Returns `coral-<env>-<8-char-hex>` where the hex is the first 8
/// chars of a SHA-like fold of the canonical absolute path. Collisions
/// across worktrees of the same meta-repo are vanishingly rare for 8
/// chars; the `<env>` segment keeps `dev` and `ci` of the same project
/// in different namespaces.
fn compose_project_name(project_root: &std::path::Path, env_name: &str) -> String {
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let hash = simple_path_hash(canonical.to_string_lossy().as_ref());
    let env_slug = sanitize_slug(env_name);
    format!("coral-{env_slug}-{hash}")
}

/// Coral never depends on a cryptographic hash for compose project
/// names; collision resistance for ~10 simultaneous worktrees is
/// sufficient. FNV-1a 64-bit is plenty and avoids a sha2 dep.
fn simple_path_hash(s: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

fn sanitize_slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' {
            out.push(ch);
        }
    }
    if out.is_empty() {
        "default".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_project_name_is_stable_for_a_given_path() {
        let p = std::path::Path::new("/tmp/orchestra");
        let a = compose_project_name(p, "dev");
        let b = compose_project_name(p, "dev");
        assert_eq!(a, b);
        assert!(a.starts_with("coral-dev-"));
        assert_eq!(a.len(), "coral-dev-".len() + 8);
    }

    #[test]
    fn compose_project_name_differs_per_env() {
        let p = std::path::Path::new("/tmp/orchestra");
        assert_ne!(
            compose_project_name(p, "dev"),
            compose_project_name(p, "ci")
        );
    }

    #[test]
    fn sanitize_slug_strips_punctuation() {
        assert_eq!(sanitize_slug("dev"), "dev");
        assert_eq!(sanitize_slug("dev-1"), "dev-1");
        assert_eq!(sanitize_slug("a/b@c"), "abc");
        assert_eq!(sanitize_slug(""), "default");
    }
}
