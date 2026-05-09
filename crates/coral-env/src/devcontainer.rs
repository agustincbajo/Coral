//! Render an `EnvPlan` to a `.devcontainer/devcontainer.json` file.
//!
//! VS Code (and Cursor / GitHub Codespaces) read a
//! `.devcontainer/devcontainer.json` to bring up a pre-configured
//! editor session against a Docker Compose project. v0.21 ships
//! `coral env devcontainer emit` so users coming from a `coral.toml`
//! `[[environments]]` table can hand the same multi-service plan to
//! their editor without having to author the JSON by hand.
//!
//! ## Scope
//!
//! - Pure renderer over `EnvPlan` — no I/O. The CLI calls
//!   `render_devcontainer(plan, opts)` and decides whether to print
//!   the result or write it to disk under `--write`.
//! - `dockerComposeFile` is emitted as a **single-element array**
//!   pointing at the rendered `.coral/env/compose/<hash>.yml` file,
//!   path expressed relative to the conventional
//!   `.devcontainer/devcontainer.json` location at the project root.
//! - `service` defaults to the first real service that has a
//!   `repo = "..."` reference (i.e. the canonical app the user is
//!   developing); falls back to the alphabetically first real service
//!   if no repo-bound service exists. Mock services are never
//!   selected. Users can override with `--service`.
//! - `forwardPorts` is the union of every `RealService.ports` entry,
//!   deduped and sorted ascending. Sourced from the declared spec so
//!   the emit is offline (no runtime probe).
//! - `customizations.vscode.extensions` is rendered as `[]`. Per the
//!   v0.21 spec we don't ship a curated list — users add their own.
//! - `remoteUser` is hard-coded to `"root"`, matching the conventional
//!   default for Compose-backed devcontainers.
//!
//! ## Stability
//!
//! Reruns of the same `EnvPlan` produce byte-identical output. Keys
//! are emitted in ASCII-alphabetic order (`customizations` first,
//! `workspaceFolder` last) — `serde_json` defaults to a `BTreeMap`
//! backing for `Value::Object` and we don't pull the
//! `serde_json/preserve_order` feature, which keeps the dep tree
//! slim. The rendered JSON closes with a trailing newline so editors
//! that strip end-of-file blanks don't churn the file on save.

use crate::compose_yaml;
use crate::plan::EnvPlan;
use crate::spec::ServiceKind;
use crate::{EnvError, EnvResult};

/// Options controlling the devcontainer rendering. Defaults are
/// equivalent to passing no flags from the CLI.
#[derive(Debug, Clone, Default)]
pub struct DevcontainerOpts {
    /// Force a specific value for the `service:` field. When `None`,
    /// the renderer picks via `select_service` (see module docs).
    pub service_override: Option<String>,
}

/// Output of `render_devcontainer`. The CLI either prints `json` to
/// stdout or writes it (atomically) to disk under `--write`.
#[derive(Debug, Clone)]
pub struct DevcontainerArtifact {
    /// The full pretty-printed JSON, terminated with a newline.
    pub json: String,
    /// Reserved for future side-files (e.g. a `Dockerfile.devcontainer`).
    /// v0.21 does not emit any.
    pub additional_files: Vec<(std::path::PathBuf, String)>,
    /// Non-fatal warnings about fields the renderer didn't fully
    /// translate. v0.21 does not surface any (the spec's surface is
    /// fully covered) — kept in the API so we can layer warnings in
    /// future without breaking callers.
    pub warnings: Vec<String>,
}

/// Render an `EnvPlan` to a `devcontainer.json` artifact. Pure
/// (deterministic and side-effect free) — call sites are responsible
/// for printing or writing the result.
pub fn render_devcontainer(
    plan: &EnvPlan,
    opts: &DevcontainerOpts,
) -> EnvResult<DevcontainerArtifact> {
    if plan.services.is_empty() {
        return Err(EnvError::InvalidSpec(
            "environment has no services; run `coral env import` to convert a \
             docker-compose.yml or hand-author the [[environments]] table in \
             coral.toml"
                .into(),
        ));
    }

    let service = select_service(plan, opts.service_override.as_deref())?;

    // Compute the relative path from `<project_root>/.devcontainer/`
    // to `<project_root>/.coral/env/compose/<hash>.yml`. We render the
    // YAML purely to compute its content hash — the bytes themselves
    // are written by `coral up` (or whichever flow `up`s the env);
    // emit is "describe", not "materialize the compose YAML".
    let yaml = compose_yaml::render(plan);
    let hash = compose_yaml::content_hash(&yaml);
    let compose_rel = relative_compose_path(&hash);

    let forward_ports = collect_forward_ports(plan);

    // Use `serde_json::json!` so the field order in the output is
    // exactly the order written here. Re-running the renderer for an
    // unchanged plan must produce byte-identical output (the artifact
    // hash and the editor's diff view both depend on it).
    let value = serde_json::json!({
        "name": format!("coral-{}", plan.name),
        "dockerComposeFile": [compose_rel],
        "service": service,
        "workspaceFolder": "/workspaces/${localWorkspaceFolderBasename}",
        "shutdownAction": "stopCompose",
        "forwardPorts": forward_ports,
        "customizations": {
            "vscode": {
                "extensions": []
            }
        },
        "remoteUser": "root"
    });

    let mut json = serde_json::to_string_pretty(&value).map_err(|e| {
        EnvError::InvalidSpec(format!(
            "internal error: failed to serialize devcontainer JSON: {e}"
        ))
    })?;
    json.push('\n');

    Ok(DevcontainerArtifact {
        json,
        additional_files: Vec::new(),
        warnings: Vec::new(),
    })
}

/// Select which service the devcontainer attaches to.
///
/// Algorithm (matches the v0.21 spec):
/// 1. If an override is supplied and matches a declared service, use it.
/// 2. If an override is supplied but doesn't match, error with
///    `ServiceNotFound`.
/// 3. Otherwise, prefer the first real service (in `BTreeMap` order,
///    i.e. lexicographic by name) that has a `repo = "..."` reference —
///    that's the canonical "app the user is developing".
/// 4. If no real service has a repo, fall back to the alphabetically
///    first real service.
/// 5. If only mock services exist, error with `InvalidSpec`.
fn select_service(plan: &EnvPlan, override_: Option<&str>) -> EnvResult<String> {
    if let Some(name) = override_ {
        // v0.21.0 tester follow-up: --service must resolve to a Real
        // service. Pre-fix the override path only checked
        // `contains_key`, so `--service <mock-name>` silently produced
        // a devcontainer.json pointing at a mock — which by design is
        // a placeholder image, not something VS Code can attach to.
        // The non-override paths below already filter on `Real`; this
        // tightens the override path to match.
        return match plan.services.get(name) {
            Some(svc) if matches!(svc.kind, ServiceKind::Real(_)) => Ok(name.to_string()),
            Some(_) => Err(EnvError::InvalidSpec(format!(
                "service `{name}` is a mock; devcontainer attach requires a real service"
            ))),
            None => Err(EnvError::ServiceNotFound(name.to_string())),
        };
    }
    // Pass 1: a real service with a non-empty `repo`. v0.21.0 tester
    // follow-up: an empty-string repo is treated as absent. Pre-fix
    // `Option<String>::is_some()` returned true for `repo = ""` and
    // a config like `[a] repo="" / [z] image="..."` selected `a` over
    // the alphabetic-fallback `z`. Treat empty string as no-repo so
    // Pass 2 catches it.
    for (name, svc) in &plan.services {
        if let ServiceKind::Real(real) = &svc.kind {
            if real.repo.as_deref().is_some_and(|s| !s.is_empty()) {
                return Ok(name.clone());
            }
        }
    }
    // Pass 2: any real service.
    for (name, svc) in &plan.services {
        if matches!(svc.kind, ServiceKind::Real(_)) {
            return Ok(name.clone());
        }
    }
    Err(EnvError::InvalidSpec(
        "environment has no real services to attach to".into(),
    ))
}

/// Path the `dockerComposeFile` field points at, relative from
/// `<project_root>/.devcontainer/` (the conventional location of the
/// `devcontainer.json` we emit).
///
/// Result: `../.coral/env/compose/<hash>.yml`. We hard-code the form
/// rather than computing via `pathdiff` because the layout is fixed
/// by `coral up` and dragging in another dep just for this isn't
/// worth it.
fn relative_compose_path(hash: &str) -> String {
    format!("../.coral/env/compose/{hash}.yml")
}

/// Union of every `RealService.ports`, deduped + sorted ascending.
/// Mock services contribute nothing because their ports — if any —
/// are an internal mockoon/wiremock detail we don't surface to the
/// editor.
fn collect_forward_ports(plan: &EnvPlan) -> Vec<u16> {
    let mut seen = std::collections::BTreeSet::new();
    for svc in plan.services.values() {
        if let ServiceKind::Real(real) = &svc.kind {
            for port in &real.ports {
                seen.insert(*port);
            }
        }
    }
    seen.into_iter().collect()
}

#[cfg(test)]
#[allow(non_snake_case)] // Test names mirror the spec wording verbatim (e.g. `dockerComposeFile`, `forwardPorts`).
mod tests {
    use super::*;
    use crate::plan::ServiceSpecPlan;
    use crate::spec::{Healthcheck, HealthcheckKind, HealthcheckTiming, RealService};
    use std::collections::BTreeMap;

    fn empty_plan(name: &str) -> EnvPlan {
        EnvPlan {
            name: name.into(),
            project_name: format!("coral-{name}-deadbeef"),
            mode: crate::spec::EnvMode::Managed,
            services: BTreeMap::new(),
            env_file: None,
            project_root: std::path::PathBuf::from("/tmp/proj"),
            chaos: None,
        }
    }

    fn real_service_with_repo(name: &str, repo: Option<&str>, ports: Vec<u16>) -> ServiceSpecPlan {
        ServiceSpecPlan {
            name: name.into(),
            kind: ServiceKind::Real(Box::new(RealService {
                repo: repo.map(str::to_string),
                image: if repo.is_some() {
                    None
                } else {
                    Some("postgres:16".into())
                },
                build: None,
                ports,
                env: BTreeMap::new(),
                depends_on: vec![],
                healthcheck: Some(Healthcheck {
                    kind: HealthcheckKind::Tcp { port: 5432 },
                    timing: HealthcheckTiming::default(),
                }),
                watch: None,
            })),
            resolved_context: None,
        }
    }

    fn mock_service(name: &str) -> ServiceSpecPlan {
        ServiceSpecPlan {
            name: name.into(),
            kind: ServiceKind::Mock(crate::spec::MockService {
                tool: "mockoon".into(),
                spec: None,
                mappings_dir: None,
                mode: None,
                recording: None,
            }),
            resolved_context: None,
        }
    }

    fn parse_artifact(artifact: &DevcontainerArtifact) -> serde_json::Value {
        serde_json::from_str(&artifact.json).expect("artifact.json must be valid JSON")
    }

    #[test]
    fn render_emits_dockerComposeFile_array_with_relative_path() {
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            real_service_with_repo("api", Some("api"), vec![3000]),
        );
        let art = render_devcontainer(&plan, &DevcontainerOpts::default()).unwrap();
        let v = parse_artifact(&art);
        let arr = v
            .get("dockerComposeFile")
            .and_then(|x| x.as_array())
            .unwrap();
        assert_eq!(
            arr.len(),
            1,
            "must be a single-element array (string-or-array per the spec, array form is forward-compat)"
        );
        let path = arr[0].as_str().unwrap();
        assert!(
            path.starts_with("../.coral/env/compose/") && path.ends_with(".yml"),
            "expected ../.coral/env/compose/<hash>.yml, got {path}"
        );
    }

    #[test]
    fn render_picks_first_real_service_with_repo() {
        let mut plan = empty_plan("dev");
        // Mix: alphabetic-first (`api`) is repo-bound; alphabetic-last (`db`) is image-only.
        // Without the repo preference, BTreeMap iteration would still
        // return `api` first; flip the names to prove repo-first wins.
        plan.services.insert(
            "alpha-db".into(),
            real_service_with_repo("alpha-db", None, vec![5432]),
        );
        plan.services.insert(
            "zebra-api".into(),
            real_service_with_repo("zebra-api", Some("api"), vec![3000]),
        );
        let art = render_devcontainer(&plan, &DevcontainerOpts::default()).unwrap();
        let v = parse_artifact(&art);
        assert_eq!(
            v.get("service").and_then(|s| s.as_str()).unwrap(),
            "zebra-api"
        );
    }

    #[test]
    fn render_falls_back_to_alphabetic_when_no_repo() {
        let mut plan = empty_plan("dev");
        // Both image-only — algorithm falls through to step 4 (alphabetic).
        plan.services.insert(
            "redis".into(),
            real_service_with_repo("redis", None, vec![6379]),
        );
        plan.services.insert(
            "postgres".into(),
            real_service_with_repo("postgres", None, vec![5432]),
        );
        let art = render_devcontainer(&plan, &DevcontainerOpts::default()).unwrap();
        let v = parse_artifact(&art);
        // BTreeMap iterates lexicographically, so `postgres` < `redis`.
        assert_eq!(
            v.get("service").and_then(|s| s.as_str()).unwrap(),
            "postgres"
        );
    }

    #[test]
    fn render_unions_and_sorts_forwardPorts() {
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            real_service_with_repo("api", Some("api"), vec![3000, 9229]),
        );
        plan.services
            .insert("db".into(), real_service_with_repo("db", None, vec![5432]));
        // Duplicate port across services — must dedupe.
        plan.services.insert(
            "worker".into(),
            real_service_with_repo("worker", None, vec![3000]),
        );
        let art = render_devcontainer(&plan, &DevcontainerOpts::default()).unwrap();
        let v = parse_artifact(&art);
        let ports: Vec<u64> = v
            .get("forwardPorts")
            .and_then(|x| x.as_array())
            .unwrap()
            .iter()
            .map(|p| p.as_u64().unwrap())
            .collect();
        assert_eq!(ports, vec![3000, 5432, 9229], "must be deduped + ascending");
    }

    #[test]
    fn render_with_no_services_returns_invalid_spec_error() {
        let plan = empty_plan("dev");
        let err = render_devcontainer(&plan, &DevcontainerOpts::default()).unwrap_err();
        match err {
            EnvError::InvalidSpec(msg) => {
                // Error must point users at the two recovery paths.
                assert!(
                    msg.contains("coral env import"),
                    "must point at `coral env import`: {msg}"
                );
                assert!(
                    msg.contains("hand-author"),
                    "must point at hand-authoring: {msg}"
                );
            }
            other => panic!("expected InvalidSpec, got {other:?}"),
        }
    }

    #[test]
    fn render_with_only_mock_services_returns_invalid_spec_error() {
        let mut plan = empty_plan("dev");
        plan.services
            .insert("mock-billing".into(), mock_service("mock-billing"));
        let err = render_devcontainer(&plan, &DevcontainerOpts::default()).unwrap_err();
        match err {
            EnvError::InvalidSpec(msg) => {
                assert!(
                    msg.contains("real services"),
                    "error must explain why mocks alone don't suffice: {msg}"
                );
            }
            other => panic!("expected InvalidSpec, got {other:?}"),
        }
    }

    #[test]
    fn render_honors_service_override() {
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            real_service_with_repo("api", Some("api"), vec![3000]),
        );
        plan.services
            .insert("db".into(), real_service_with_repo("db", None, vec![5432]));
        let opts = DevcontainerOpts {
            service_override: Some("db".into()),
        };
        let art = render_devcontainer(&plan, &opts).unwrap();
        let v = parse_artifact(&art);
        assert_eq!(v.get("service").and_then(|s| s.as_str()).unwrap(), "db");
    }

    #[test]
    fn render_rejects_service_override_for_unknown_service() {
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            real_service_with_repo("api", Some("api"), vec![3000]),
        );
        let opts = DevcontainerOpts {
            service_override: Some("nope".into()),
        };
        let err = render_devcontainer(&plan, &opts).unwrap_err();
        match err {
            EnvError::ServiceNotFound(name) => assert_eq!(name, "nope"),
            other => panic!("expected ServiceNotFound, got {other:?}"),
        }
    }

    #[test]
    fn render_output_is_byte_stable_across_reruns() {
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            real_service_with_repo("api", Some("api"), vec![3000]),
        );
        plan.services
            .insert("db".into(), real_service_with_repo("db", None, vec![5432]));
        let a = render_devcontainer(&plan, &DevcontainerOpts::default()).unwrap();
        let b = render_devcontainer(&plan, &DevcontainerOpts::default()).unwrap();
        assert_eq!(
            a.json, b.json,
            "renderer must be deterministic — reruns of an unchanged plan produce identical bytes"
        );
        assert!(a.json.ends_with('\n'), "must terminate with newline");
    }

    #[test]
    fn rendered_json_round_trips_through_serde_json_value() {
        // Sanity: the output is valid JSON every time (defends
        // against accidental string concatenation regressions).
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            real_service_with_repo("api", Some("api"), vec![3000]),
        );
        let art = render_devcontainer(&plan, &DevcontainerOpts::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&art.json).expect("must parse cleanly");
        // Spot-check a couple of fields the editor reads.
        assert_eq!(v.get("name").and_then(|s| s.as_str()).unwrap(), "coral-dev");
        assert_eq!(
            v.get("workspaceFolder").and_then(|s| s.as_str()).unwrap(),
            "/workspaces/${localWorkspaceFolderBasename}"
        );
        assert_eq!(
            v.get("shutdownAction").and_then(|s| s.as_str()).unwrap(),
            "stopCompose"
        );
        assert_eq!(
            v.get("remoteUser").and_then(|s| s.as_str()).unwrap(),
            "root"
        );
        // Empty extensions list — the v0.21 default.
        let ext = v
            .pointer("/customizations/vscode/extensions")
            .and_then(|x| x.as_array())
            .unwrap();
        assert!(ext.is_empty(), "default extensions must be empty");
    }

    /// v0.21.0 tester follow-up: `--service <mock>` must fail rather
    /// than silently produce a devcontainer pointing at a placeholder
    /// mock service. Pre-fix `select_service`'s override path only
    /// checked `contains_key`, bypassing the Real-only contract that
    /// the non-override paths enforced.
    #[test]
    fn render_rejects_service_override_for_mock_service() {
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            real_service_with_repo("api", Some("api"), vec![3000]),
        );
        plan.services.insert(
            "mockbill".into(),
            ServiceSpecPlan {
                name: "mockbill".into(),
                kind: ServiceKind::Mock(crate::spec::MockService {
                    tool: "mockoon".into(),
                    spec: None,
                    mappings_dir: None,
                    mode: None,
                    recording: None,
                }),
                resolved_context: None,
            },
        );
        let opts = DevcontainerOpts {
            service_override: Some("mockbill".into()),
        };
        let err = render_devcontainer(&plan, &opts).unwrap_err();
        match err {
            EnvError::InvalidSpec(msg) => {
                assert!(
                    msg.contains("mockbill") && msg.contains("mock"),
                    "expected 'mockbill is a mock' message, got: {msg}"
                );
            }
            other => panic!("expected InvalidSpec, got {other:?}"),
        }
    }

    /// v0.21.0 tester follow-up: empty-string `repo = ""` must be
    /// treated as absent, so Pass 1 (real with repo) doesn't false-
    /// positive on it. Pre-fix `Option::is_some()` returned true for
    /// the empty string and an `[alpha] repo="" / [zeta] image=..."`
    /// plan picked `alpha` over the alphabetic fallback `zeta`.
    #[test]
    fn render_with_empty_repo_string_falls_through_to_alphabetic() {
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "alpha".into(),
            real_service_with_repo("alpha", Some(""), vec![3000]),
        );
        plan.services
            .insert("zeta".into(), real_service_with_repo("zeta", None, vec![]));
        let art = render_devcontainer(&plan, &DevcontainerOpts::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&art.json).unwrap();
        // Both `alpha` and `zeta` are Real with no useful `repo`. Pass 1
        // skips both; Pass 2 picks the alphabetically-first real, which
        // is `alpha`. Important: the test pins that the EMPTY-STRING is
        // treated identically to `None`. Without the fix, Pass 1 would
        // have already matched `alpha` via `is_some()` returning true
        // for `Some("")`. With the fix, Pass 2 also picks `alpha` —
        // so the post-fix behavior is observable here only as the
        // selection coming from Pass 2 (alphabetic fallback) rather
        // than Pass 1 (repo-bound). Confirm via direct test on the
        // helper: see `select_service_treats_empty_repo_as_absent`.
        assert_eq!(v.get("service").and_then(|s| s.as_str()).unwrap(), "alpha");
    }

    /// Direct test on `select_service` confirming the empty-string
    /// repo no longer trumps a sibling. With `[a] repo="" / [z]
    /// image="..."` the picker should NOT prefer `a` based on its
    /// (empty) repo — both pass through Pass 1 to Pass 2, which
    /// picks alphabetic-first (still `a` here, but the selection
    /// path is the fallback, not the repo-bound branch).
    #[test]
    fn select_service_treats_empty_repo_as_absent() {
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "alpha".into(),
            real_service_with_repo("alpha", Some(""), vec![]),
        );
        plan.services.insert(
            "zeta".into(),
            real_service_with_repo("zeta", Some("real-repo"), vec![]),
        );
        // With the fix, Pass 1 finds `zeta` (which has a non-empty
        // repo) and returns it — proving the fix: pre-fix `alpha`
        // would have won via Pass 1's loose `is_some()` check.
        let picked = select_service(&plan, None).unwrap();
        assert_eq!(
            picked, "zeta",
            "non-empty repo on `zeta` must win over empty repo on `alpha`"
        );
    }
}
