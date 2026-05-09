//! Resolve `[[environments]]` from `coral.toml` into typed
//! `EnvironmentSpec` values.
//!
//! `coral-core` keeps `environments_raw: Vec<toml::Value>` opaque so
//! the wiki layer doesn't drag in `coral-env`. The CLI re-types those
//! values via `serde` here, on demand.

use anyhow::{Context, Result};
use coral_core::project::Project;
use coral_env::EnvironmentSpec;

/// Find the `[[environments]]` entry whose `name` matches `wanted`.
/// Returns the typed spec or an error pointing at the user's manifest.
pub fn resolve_env(project: &Project, wanted: &str) -> Result<EnvironmentSpec> {
    let envs = parse_all(project).context("parsing [[environments]]")?;
    envs.into_iter().find(|e| e.name == wanted).ok_or_else(|| {
        anyhow::anyhow!(
            "environment '{}' is not declared in coral.toml; available: {}",
            wanted,
            available_names(project).join(", ")
        )
    })
}

/// Parse every `[[environments]]` block. Used by `coral env list` and
/// by `coral status` to enumerate which environments exist.
///
/// v0.23.0: each spec is validated against
/// `EnvironmentSpec::validate` so chaos invariants (scenario without
/// `[chaos]` block, unknown service, unknown toxic attribute,
/// `toxiproxy` name collision) surface here rather than at chaos-CLI
/// invocation time.
pub fn parse_all(project: &Project) -> Result<Vec<EnvironmentSpec>> {
    let mut out = Vec::with_capacity(project.environments_raw.len());
    for (idx, raw) in project.environments_raw.iter().enumerate() {
        let spec: EnvironmentSpec = raw
            .clone()
            .try_into()
            .with_context(|| format!("parsing [[environments]][{idx}]"))?;
        spec.validate()
            .map_err(|msg| anyhow::anyhow!("[[environments]][{idx}]: {msg}"))?;
        out.push(spec);
    }
    Ok(out)
}

fn available_names(project: &Project) -> Vec<String> {
    project
        .environments_raw
        .iter()
        .filter_map(|raw| {
            raw.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect()
}

/// Default environment name to use when `--env` is not given. Returns
/// the first declared environment, or `dev` as a fallback so commands
/// fail with an actionable "environment 'dev' is not declared" error
/// rather than a generic "no environment selected".
pub fn default_env_name(project: &Project) -> String {
    available_names(project)
        .into_iter()
        .next()
        .unwrap_or_else(|| "dev".to_string())
}
