//! Pin the `coral.toml` examples in README.md as parseable + valid.
//!
//! v0.19's first README rewrite shipped with multi-line inline-tables
//! (TOML syntax error). The example block looked sensible to a reader
//! but threw at parse time. This regression suite guards against that
//! class of doc rot — every example block under a `### …coral.toml…`
//! heading must round-trip through `parse_toml` without error and pass
//! `Project::validate`.

use coral_core::project::manifest::parse_toml;
use std::path::Path;

/// Minimal `coral.toml` from README "A `coral.toml` looks like this".
/// Snapshot pinned here in code so CI can fail fast on syntactic drift
/// even before the docs job runs.
const README_PROJECT_EXAMPLE: &str = r#"
apiVersion = "coral.dev/v1"

[project]
name = "orchestra"

[project.toolchain]
coral = "0.19.0"

[project.defaults]
ref           = "main"
remote        = "github"
path_template = "repos/{name}"

[remotes.github]
fetch = "git@github.com:acme/{name}.git"

[[repos]]
name = "api"
ref  = "release/v3"
tags = ["service", "team:platform"]

[[repos]]
name       = "worker"
remote     = "github"
tags       = ["service", "team:data"]
depends_on = ["api"]
"#;

#[test]
fn readme_project_example_parses_and_validates() {
    let manifest = parse_toml(README_PROJECT_EXAMPLE, Path::new("/tmp/coral.toml"))
        .expect("README project example must parse");
    manifest
        .validate()
        .expect("README project example must validate");
    assert_eq!(manifest.repos.len(), 2);
    assert_eq!(manifest.repos[0].name, "api");
    assert_eq!(manifest.repos[1].depends_on, vec!["api".to_string()]);
}

/// Healthcheck timing-as-subtable shape from the v0.19 README. The
/// previous shape (`timing = { … }` inline) was a known foot-gun in
/// TOML; the `[…]` subtable form is the one we want users to copy.
const README_ENVIRONMENT_HEALTHCHECK_SUBTABLE: &str = r#"
apiVersion = "coral.dev/v1"

[project]
name = "demo"

[[repos]]
name = "api"
url  = "git@example.com/api.git"

[[environments]]
name            = "dev"
backend         = "compose"
mode            = "managed"
compose_command = "auto"
production      = false

[environments.dev.services.api]
kind  = "real"
repo  = "api"
ports = [3000]

[environments.dev.services.api.healthcheck]
kind          = "http"
path          = "/health"
expect_status = 200

[environments.dev.services.api.healthcheck.timing]
interval_s     = 2
timeout_s      = 5
retries        = 5
start_period_s = 30
"#;

#[test]
fn readme_environment_healthcheck_subtable_example_parses() {
    let manifest = parse_toml(
        README_ENVIRONMENT_HEALTHCHECK_SUBTABLE,
        Path::new("/tmp/coral.toml"),
    )
    .expect("README environment example must parse");
    manifest
        .validate()
        .expect("README environment example must validate");
}

/// Pin the contract-check shape from README "Multi-repo interface
/// change detection": both repos commit `coral.toml` and
/// `[[repos]] depends_on` is what drives the cross-repo edge.
const README_CONTRACT_CHECK_TOPOLOGY: &str = r#"
apiVersion = "coral.dev/v1"

[project]
name = "orchestra"

[[repos]]
name = "api"
url  = "git@example.com/api.git"

[[repos]]
name       = "worker"
url        = "git@example.com/worker.git"
depends_on = ["api"]
"#;

#[test]
fn readme_contract_check_topology_parses() {
    let manifest = parse_toml(README_CONTRACT_CHECK_TOPOLOGY, Path::new("/tmp/coral.toml"))
        .expect("README contract topology must parse");
    manifest.validate().expect("topology must validate");
    let worker = manifest
        .repos
        .iter()
        .find(|r| r.name == "worker")
        .expect("worker repo declared");
    assert_eq!(worker.depends_on, vec!["api".to_string()]);
}
