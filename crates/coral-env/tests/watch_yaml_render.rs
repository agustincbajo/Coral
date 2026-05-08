//! v0.21.2: end-to-end watch YAML render test.
//!
//! Round-trips parse → plan → render → re-parse YAML, then asserts the
//! `develop.watch` shape matches what `docker compose watch` (compose
//! 2.22+) accepts. The unit tests in `compose_yaml::tests` pin the
//! string-level emission; this test pins the structural integrity of
//! the resulting YAML document end-to-end so a future serializer
//! change can't silently break the wire shape.

use coral_core::project::manifest::parse_toml;
use coral_env::{EnvPlan, EnvironmentSpec, compose_yaml};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MANIFEST_WITH_WATCH: &str = r#"
apiVersion = "coral.dev/v1"

[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@example.com/api.git"

[[environments]]
name    = "dev"
backend = "compose"

[environments.services.api]
kind = "real"
repo = "api"
ports = [3000]

[environments.services.api.watch]
rebuild      = ["./Dockerfile"]
restart      = ["./config.yaml"]
initial_sync = true

[[environments.services.api.watch.sync]]
path   = "./src"
target = "/app/src"

[[environments.services.api.watch.sync]]
path   = "./templates"
target = "/app/templates"
"#;

fn build_plan() -> EnvPlan {
    let manifest = parse_toml(MANIFEST_WITH_WATCH, Path::new("/tmp/coral.toml"))
        .expect("manifest with watch must parse");
    manifest
        .validate()
        .expect("manifest with watch must validate");
    let raw = manifest.environments_raw[0].clone();
    let spec: EnvironmentSpec = raw.try_into().expect("EnvironmentSpec deserialization");
    let mut repo_paths = BTreeMap::new();
    repo_paths.insert("api".into(), PathBuf::from("/work/repos/api"));
    EnvPlan::from_spec(&spec, Path::new("/work"), &repo_paths)
        .expect("EnvPlan::from_spec must succeed for a watch manifest")
}

#[test]
fn parse_then_render_emits_develop_watch_block() {
    let plan = build_plan();
    let yaml = compose_yaml::render(&plan);
    // Re-parse — the rendered document must be valid YAML.
    let value: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&yaml).expect("rendered YAML must round-trip");
    let svc = value
        .get("services")
        .and_then(|v| v.get("api"))
        .expect("api service must be present in rendered YAML");
    let develop = svc
        .get("develop")
        .expect("api service must have a develop block");
    let watch = develop
        .get("watch")
        .and_then(|v| v.as_sequence())
        .expect("develop.watch must be a sequence");
    // 2 sync + 1 rebuild + 1 restart = 4 entries.
    assert_eq!(
        watch.len(),
        4,
        "expected 4 watch entries (2 sync + rebuild + restart), got {}: {yaml}",
        watch.len()
    );
}

#[test]
fn watch_actions_emit_in_canonical_order() {
    let plan = build_plan();
    let yaml = compose_yaml::render(&plan);
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    let watch = value
        .get("services")
        .and_then(|v| v.get("api"))
        .and_then(|v| v.get("develop"))
        .and_then(|v| v.get("watch"))
        .and_then(|v| v.as_sequence())
        .unwrap();
    let actions: Vec<&str> = watch
        .iter()
        .map(|e| e.get("action").and_then(|v| v.as_str()).unwrap_or(""))
        .collect();
    assert_eq!(
        actions,
        vec!["sync", "sync", "rebuild", "restart"],
        "actions should land sync-first, rebuild, then restart"
    );
}

#[test]
fn sync_paths_resolve_against_repo_checkout() {
    let plan = build_plan();
    let yaml = compose_yaml::render(&plan);
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    let watch = value
        .get("services")
        .and_then(|v| v.get("api"))
        .and_then(|v| v.get("develop"))
        .and_then(|v| v.get("watch"))
        .and_then(|v| v.as_sequence())
        .unwrap();
    // First entry = sync of ./src — must be resolved against
    // /work/repos/api (the checkout root for repo "api").
    let first = &watch[0];
    let path = first
        .get("path")
        .and_then(|v| v.as_str())
        .expect("sync path must be a string");
    assert!(
        path.starts_with("/work/repos/api"),
        "sync path should be rooted at the repo checkout, got: {path}"
    );
    // Container target stays absolute and unchanged.
    let target = first.get("target").and_then(|v| v.as_str()).unwrap();
    assert_eq!(target, "/app/src");
}

#[test]
fn initial_sync_propagates_to_every_sync_entry() {
    let plan = build_plan();
    let yaml = compose_yaml::render(&plan);
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    let watch = value
        .get("services")
        .and_then(|v| v.get("api"))
        .and_then(|v| v.get("develop"))
        .and_then(|v| v.get("watch"))
        .and_then(|v| v.as_sequence())
        .unwrap();
    // Both sync entries should carry initial_sync: true; rebuild/
    // restart entries should NOT have it.
    let sync_count = watch
        .iter()
        .filter(|e| {
            e.get("action").and_then(|v| v.as_str()) == Some("sync")
                && e.get("initial_sync").and_then(|v| v.as_bool()) == Some(true)
        })
        .count();
    assert_eq!(sync_count, 2, "both sync entries must carry initial_sync");
    let rebuild_has_initial = watch
        .iter()
        .filter(|e| e.get("action").and_then(|v| v.as_str()) == Some("rebuild"))
        .any(|e| e.get("initial_sync").is_some());
    assert!(
        !rebuild_has_initial,
        "rebuild entries must NOT carry initial_sync"
    );
}

/// BC pin: a service with the SAME shape sans the `[..watch]` block
/// must produce a YAML whose `services.api` value has no `develop`
/// key. This is the Acceptance Criterion #3 from the spec — services
/// without watch are byte-identical to v0.21.1.
#[test]
fn service_without_watch_emits_no_develop_block() {
    const NO_WATCH_MANIFEST: &str = r#"
apiVersion = "coral.dev/v1"

[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@example.com/api.git"

[[environments]]
name    = "dev"
backend = "compose"

[environments.services.api]
kind = "real"
repo = "api"
ports = [3000]
"#;
    let manifest = parse_toml(NO_WATCH_MANIFEST, Path::new("/tmp/coral.toml")).unwrap();
    manifest.validate().unwrap();
    let raw = manifest.environments_raw[0].clone();
    let spec: EnvironmentSpec = raw.try_into().unwrap();
    let mut repo_paths = BTreeMap::new();
    repo_paths.insert("api".into(), PathBuf::from("/work/repos/api"));
    let plan = EnvPlan::from_spec(&spec, Path::new("/work"), &repo_paths).unwrap();
    let yaml = compose_yaml::render(&plan);
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&yaml).unwrap();
    let svc = value.get("services").and_then(|v| v.get("api")).unwrap();
    assert!(
        svc.get("develop").is_none(),
        "service without [services.*.watch] must emit no develop block, got:\n{yaml}"
    );
}

/// Adding `[services.*.watch]` to an otherwise-identical plan must
/// change the artifact hash so `coral up` re-renders. Pins Acceptance
/// Criterion #9 from the spec.
#[test]
fn adding_watch_changes_artifact_hash() {
    const NO_WATCH: &str = r#"
apiVersion = "coral.dev/v1"
[project]
name = "demo"
[[repos]]
name = "api"
url = "git@example.com/api.git"
[[environments]]
name = "dev"
backend = "compose"
[environments.services.api]
kind = "real"
repo = "api"
ports = [3000]
"#;
    const WITH_WATCH: &str = r#"
apiVersion = "coral.dev/v1"
[project]
name = "demo"
[[repos]]
name = "api"
url = "git@example.com/api.git"
[[environments]]
name = "dev"
backend = "compose"
[environments.services.api]
kind = "real"
repo = "api"
ports = [3000]
[environments.services.api.watch]
rebuild = ["./Dockerfile"]
"#;
    let mut repo_paths = BTreeMap::new();
    repo_paths.insert("api".into(), PathBuf::from("/work/repos/api"));

    let m_a = parse_toml(NO_WATCH, Path::new("/tmp/coral.toml")).unwrap();
    let spec_a: EnvironmentSpec = m_a.environments_raw[0].clone().try_into().unwrap();
    let plan_a = EnvPlan::from_spec(&spec_a, Path::new("/work"), &repo_paths).unwrap();
    let hash_a = compose_yaml::content_hash(&compose_yaml::render(&plan_a));

    let m_b = parse_toml(WITH_WATCH, Path::new("/tmp/coral.toml")).unwrap();
    let spec_b: EnvironmentSpec = m_b.environments_raw[0].clone().try_into().unwrap();
    let plan_b = EnvPlan::from_spec(&spec_b, Path::new("/work"), &repo_paths).unwrap();
    let hash_b = compose_yaml::content_hash(&compose_yaml::render(&plan_b));

    assert_ne!(
        hash_a, hash_b,
        "adding [services.*.watch] must change the artifact hash"
    );
}
