//! Render an `EnvPlan` to a Docker Compose YAML string.
//!
//! v0.17 wave 2 covers the schema fields the wave-1 `EnvironmentSpec`
//! exposes: `image`, `build { context, dockerfile, target, args,
//! cache_from, cache_to }`, `ports`, `env`, `depends_on`,
//! `healthcheck`. `develop.watch` (compose 2.22+) was wired in v0.21.2:
//! `WatchSpec` flows from `[services.*.watch]` straight into the
//! emitted `develop.watch` sequence, with `sync` rules first, then
//! `rebuild`, then `restart` — see `render_watch` below.
//!
//! v0.23.0 adds an OPTIONAL chaos sidecar (Toxiproxy) when
//! `[environments.<env>.chaos]` is present. `chaos_proxies()` walks
//! `depends_on` edges, allocates a deterministic proxy port per edge,
//! and the renderer:
//!
//!   1. Emits a `toxiproxy` service with a generated init-config JSON
//!      mounted as `--config`.
//!   2. Rewrites each consumer's environment with `<DEP>_URL=http://toxiproxy:<port>`
//!      so the consumer hits the proxy instead of the real dep.
//!
//! When `chaos.is_none()`, none of this runs and the rendered YAML is
//! byte-identical to v0.22.6 — pinned by
//! `compose_yaml_no_chaos_byte_identical_to_v0226`.

use crate::plan::{EnvPlan, ServiceSpecPlan};
use crate::spec::{Healthcheck, HealthcheckKind, HealthcheckTiming, RealService, ServiceKind};
use serde_yaml_ng::Value;

/// One toxiproxy proxy declaration: the listen port the consumer
/// hits, plus the upstream `<dep>:<port>` it forwards to. Generated
/// for every `(consumer, dep)` edge found in
/// `services.<consumer>.depends_on` when chaos is on.
///
/// `name` is `<consumer>_to_<dep>` — that's the proxy ID the chaos
/// CLI uses on `POST /proxies/<name>/toxics`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChaosProxy {
    pub name: String,
    pub consumer: String,
    pub dep: String,
    pub listen_port: u16,
    pub upstream_port: u16,
}

/// Walk every consumer's `depends_on`, allocate a deterministic
/// listen port per edge, and return the proxy list. Output is sorted
/// by `name` so the render is byte-stable across reruns. Skips edges
/// where the dep declares no `ports` (nothing to proxy).
///
/// Public so the chaos CLI can compute the proxy name (`api_to_db`)
/// without re-deriving the depends_on traversal.
pub fn chaos_proxies(plan: &EnvPlan) -> Vec<ChaosProxy> {
    let mut out: Vec<ChaosProxy> = Vec::new();
    for (consumer_name, consumer) in &plan.services {
        let ServiceKind::Real(real) = &consumer.kind else {
            continue;
        };
        for dep in &real.depends_on {
            let dep_port = match plan.services.get(dep) {
                Some(s) => match &s.kind {
                    ServiceKind::Real(r) => r.ports.first().copied(),
                    ServiceKind::Mock(_) => None,
                },
                None => None,
            };
            let Some(upstream_port) = dep_port else {
                // No port to proxy — depending on a sidecar with no
                // exposed port (rare, but possible). Skip silently.
                continue;
            };
            let listen_port = chaos_proxy_port(consumer_name, dep);
            out.push(ChaosProxy {
                name: format!("{consumer_name}_to_{dep}"),
                consumer: consumer_name.clone(),
                dep: dep.clone(),
                listen_port,
                upstream_port,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Deterministic listen-port allocation: `30000 + fnv1a("<consumer>:<dep>") % 10000`,
/// so every edge falls in the 30000-39999 range. FNV-1a is plenty for
/// this — 10k slots, ~10 services means birthday-paradox collisions
/// after ~150 edges. We accept the worst-case collision (two edges
/// that hash to the same port) because:
///
///  1. The user controls the input — service names rarely collide.
///  2. A collision surfaces immediately on `coral up` (compose
///     refuses to bind two containers to the same host port).
///  3. There's no hidden corruption: toxiproxy fails its admin
///     `/version` health check and `coral chaos inject` returns a
///     friendly "sidecar not running" error.
///
/// If two real users hit a collision in the field we'll switch to a
/// content-hash that includes the env name. For v0.23.0 the simple
/// FNV is the right tradeoff.
fn chaos_proxy_port(consumer: &str, dep: &str) -> u16 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let key = format!("{consumer}:{dep}");
    let mut hash = FNV_OFFSET;
    for byte in key.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    30_000 + ((hash % 10_000) as u16)
}

/// JSON list of toxiproxy proxy declarations to write into
/// `.coral/env/compose/<hash>.toxiproxy.json` and bind-mount into the
/// sidecar via `--config`. Public so the backend can write it
/// alongside the YAML artifact.
pub fn chaos_init_json(proxies: &[ChaosProxy]) -> String {
    let value: Vec<serde_json::Value> = proxies
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "listen": format!("0.0.0.0:{}", p.listen_port),
                "upstream": format!("{}:{}", p.dep, p.upstream_port),
                "enabled": true,
            })
        })
        .collect();
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "[]".to_string())
}

/// Render the plan as a YAML document compatible with `docker compose
/// up -f <out>`. Stable ordering: services map is `BTreeMap` so the
/// output is byte-stable per plan, which makes the artifact-hash
/// comparison in `EnvBackend::up()` reliable.
pub fn render(plan: &EnvPlan) -> String {
    let proxies = if plan.chaos.is_some() {
        chaos_proxies(plan)
    } else {
        Vec::new()
    };
    // Index proxies by consumer name → list of (dep, listen_port).
    // Cheap because `proxies.len()` is bounded by total edge count.
    let mut by_consumer: std::collections::BTreeMap<&str, Vec<&ChaosProxy>> = Default::default();
    for p in &proxies {
        by_consumer.entry(p.consumer.as_str()).or_default().push(p);
    }

    let mut services_yaml = serde_yaml_ng::Mapping::new();
    for (name, service) in &plan.services {
        let consumer_proxies = by_consumer.get(name.as_str()).cloned().unwrap_or_default();
        let body = render_service(name, service, plan, &consumer_proxies);
        services_yaml.insert(Value::String(name.clone()), body);
    }

    // v0.23.0: when chaos is on, append the toxiproxy sidecar AFTER
    // user services. The append-at-end ordering keeps the chaos-off
    // YAML byte-identical to v0.22.6 (since the BTreeMap iteration
    // hasn't changed).
    if let Some(chaos) = &plan.chaos {
        services_yaml.insert(
            Value::String("toxiproxy".into()),
            render_toxiproxy_sidecar(chaos, plan, &proxies),
        );
    }

    let mut top = serde_yaml_ng::Mapping::new();
    top.insert(
        Value::String("name".into()),
        Value::String(plan.project_name.clone()),
    );
    top.insert(
        Value::String("services".into()),
        Value::Mapping(services_yaml),
    );

    let document = Value::Mapping(top);
    serde_yaml_ng::to_string(&document).unwrap_or_else(|_| String::new())
}

/// Render the toxiproxy sidecar service. The init-config JSON is
/// bind-mounted at `/etc/toxiproxy.json` and passed via `--config`,
/// so the proxies are pre-wired the moment toxiproxy starts.
fn render_toxiproxy_sidecar(
    chaos: &crate::spec::ChaosConfig,
    plan: &EnvPlan,
    _proxies: &[ChaosProxy],
) -> Value {
    let mut out = serde_yaml_ng::Mapping::new();
    out.insert(
        Value::String("container_name".into()),
        Value::String(format!("{}-toxiproxy", plan.project_name)),
    );
    out.insert(
        Value::String("image".into()),
        Value::String(chaos.image.clone()),
    );
    // Random host port — `EnvHandle.published_ports["toxiproxy"]`
    // surfaces the actual binding for the chaos CLI to discover.
    out.insert(
        Value::String("ports".into()),
        Value::Sequence(vec![Value::String(format!("{}", chaos.listen_port))]),
    );
    out.insert(
        Value::String("volumes".into()),
        Value::Sequence(vec![Value::String(
            "./toxiproxy-init.json:/etc/toxiproxy.json:ro".into(),
        )]),
    );
    out.insert(
        Value::String("command".into()),
        Value::Sequence(vec![
            Value::String("-host".into()),
            Value::String("0.0.0.0".into()),
            Value::String("-config".into()),
            Value::String("/etc/toxiproxy.json".into()),
        ]),
    );
    let mut healthcheck = serde_yaml_ng::Mapping::new();
    healthcheck.insert(
        Value::String("test".into()),
        Value::Sequence(vec![
            Value::String("CMD".into()),
            Value::String("wget".into()),
            Value::String("-qO-".into()),
            Value::String(format!("http://localhost:{}/version", chaos.listen_port)),
        ]),
    );
    healthcheck.insert(Value::String("interval".into()), Value::String("2s".into()));
    healthcheck.insert(Value::String("timeout".into()), Value::String("1s".into()));
    healthcheck.insert(
        Value::String("retries".into()),
        Value::Number(serde_yaml_ng::Number::from(10i64)),
    );
    out.insert(
        Value::String("healthcheck".into()),
        Value::Mapping(healthcheck),
    );
    Value::Mapping(out)
}

fn render_service(
    name: &str,
    service: &ServiceSpecPlan,
    plan: &EnvPlan,
    chaos_proxies: &[&ChaosProxy],
) -> Value {
    let mut out = serde_yaml_ng::Mapping::new();
    out.insert(
        Value::String("container_name".into()),
        Value::String(format!("{}-{}", plan.project_name, name)),
    );

    // Apply the environment-level `env_file` to every service so the
    // declared envs reach all containers (compose's idiomatic pattern).
    // Per-service `env: { K = V }` overrides still take precedence
    // because compose merges service-level `environment` after
    // `env_file`.
    if let Some(env_file) = &plan.env_file {
        out.insert(
            Value::String("env_file".into()),
            Value::Sequence(vec![Value::String(env_file.to_string_lossy().into_owned())]),
        );
    }

    match &service.kind {
        ServiceKind::Real(real) => render_real(&mut out, real, service, chaos_proxies),
        ServiceKind::Mock(_) => {
            // v0.18 will wire mock servers (Mockoon/WireMock/Hoverfly) by
            // launching their official containers; v0.17 leaves the entry
            // empty so the YAML still parses.
            out.insert(
                Value::String("image".into()),
                Value::String("ghcr.io/agustincbajo/coral-mock-placeholder:latest".into()),
            );
        }
    }

    Value::Mapping(out)
}

fn render_real(
    out: &mut serde_yaml_ng::Mapping,
    real: &RealService,
    plan: &ServiceSpecPlan,
    chaos_proxies: &[&ChaosProxy],
) {
    if let Some(image) = &real.image {
        out.insert(Value::String("image".into()), Value::String(image.clone()));
    }
    if let Some(build) = &real.build {
        let mut bmap = serde_yaml_ng::Mapping::new();
        let context = plan
            .resolved_context
            .clone()
            .map(|p| p.join(&build.context))
            .unwrap_or_else(|| build.context.clone());
        bmap.insert(
            Value::String("context".into()),
            Value::String(context.to_string_lossy().into_owned()),
        );
        if let Some(dockerfile) = &build.dockerfile {
            bmap.insert(
                Value::String("dockerfile".into()),
                Value::String(dockerfile.to_string_lossy().into_owned()),
            );
        }
        if let Some(target) = &build.target {
            bmap.insert(
                Value::String("target".into()),
                Value::String(target.clone()),
            );
        }
        if !build.args.is_empty() {
            let mut args = serde_yaml_ng::Mapping::new();
            for (k, v) in &build.args {
                args.insert(Value::String(k.clone()), Value::String(v.clone()));
            }
            out.insert(Value::String("args".into()), Value::Mapping(args));
        }
        if !build.cache_from.is_empty() {
            bmap.insert(
                Value::String("cache_from".into()),
                Value::Sequence(
                    build
                        .cache_from
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if let Some(cache_to) = &build.cache_to {
            bmap.insert(
                Value::String("cache_to".into()),
                Value::Sequence(vec![Value::String(cache_to.clone())]),
            );
        }
        out.insert(Value::String("build".into()), Value::Mapping(bmap));
    }
    if !real.ports.is_empty() {
        out.insert(
            Value::String("ports".into()),
            Value::Sequence(
                real.ports
                    .iter()
                    .map(|p| Value::String(format!("{p}:{p}")))
                    .collect(),
            ),
        );
    }
    // v0.23.0: when chaos is on, every (consumer, dep) edge that
    // belongs to this service generates a `<DEP>_URL=http://toxiproxy:<port>`
    // entry. These are added to the user's `env: {}` map (user values
    // win on collision — the user explicitly set the URL, so respect
    // the override). When `chaos_proxies` is empty (chaos off) the
    // existing branch runs unchanged → byte-identical to v0.22.6.
    let mut effective_env: std::collections::BTreeMap<String, String> = real.env.clone();
    for proxy in chaos_proxies {
        // <DEP> is uppercased: `db` → `DB_URL`. Compose env-var
        // names are case-sensitive on Linux, so this matches the
        // 12-factor convention every consumer expects.
        let key = format!("{}_URL", proxy.dep.to_ascii_uppercase());
        let value = format!("http://toxiproxy:{}", proxy.listen_port);
        // User-defined value wins — entry::or_insert is a no-op if
        // the key already exists. This lets a user pin a different
        // upstream when they don't want chaos to apply to a
        // specific dep.
        effective_env.entry(key).or_insert(value);
    }
    if !effective_env.is_empty() {
        out.insert(
            Value::String("environment".into()),
            Value::Sequence(
                effective_env
                    .iter()
                    .map(|(k, v)| Value::String(format!("{k}={v}")))
                    .collect(),
            ),
        );
    }
    if !real.depends_on.is_empty() {
        // depends_on supports the long-form `condition: service_healthy`
        // dict in compose 2.x; we always emit it because the EnvBackend
        // wait-for-healthy loop depends on healthchecks anyway.
        let mut deps = serde_yaml_ng::Mapping::new();
        for dep in &real.depends_on {
            let mut cond = serde_yaml_ng::Mapping::new();
            cond.insert(
                Value::String("condition".into()),
                Value::String("service_healthy".into()),
            );
            deps.insert(Value::String(dep.clone()), Value::Mapping(cond));
        }
        out.insert(Value::String("depends_on".into()), Value::Mapping(deps));
    }
    if let Some(hc) = &real.healthcheck {
        out.insert(Value::String("healthcheck".into()), render_healthcheck(hc));
    }
    if let Some(ws) = &real.watch
        && let Some(develop) = render_watch(ws, plan)
    {
        out.insert(Value::String("develop".into()), develop);
    }
}

/// Render the `develop.watch` sub-table for a service whose spec has
/// `[services.*.watch]`. Returns `None` if the watch block is empty
/// (zero `sync` / `rebuild` / `restart` rules) — emitting an empty
/// `develop.watch` sequence is useless YAML noise. The CLI surface
/// catches this case with a friendly error before we reach the
/// renderer; this branch is the defense-in-depth so the renderer never
/// produces an invalid Compose document for a malformed plan.
///
/// Order of emission: `sync` first, then `rebuild`, then `restart`.
/// Pinned for byte-stable output across rebuilds (the artifact hash
/// drives `.coral/env/compose/<hash>.yml`).
fn render_watch(ws: &crate::spec::WatchSpec, plan: &ServiceSpecPlan) -> Option<Value> {
    if ws.sync.is_empty() && ws.rebuild.is_empty() && ws.restart.is_empty() {
        return None;
    }
    let mut entries: Vec<Value> =
        Vec::with_capacity(ws.sync.len() + ws.rebuild.len() + ws.restart.len());

    // Resolve a host-side path against `plan.resolved_context` the
    // same way `build.context` is resolved — so a relative path under
    // a `repo = "..."` service hits the actual checkout root.
    //
    // v0.37 prep: normalize backslashes to forward slashes on the
    // resolved string. PathBuf::join uses OS-native separators, which
    // on Windows produces mixed-separator strings like
    // `/work/repos/api\./src`. Docker compose (and Docker Desktop on
    // Windows) accepts forward-slash paths universally, so emitting
    // them avoids the mixed-separator artifact and keeps the YAML
    // identical across host OSes.
    let resolve = |path: &std::path::Path| -> String {
        let resolved = plan
            .resolved_context
            .clone()
            .map(|root| root.join(path))
            .unwrap_or_else(|| path.to_path_buf());
        resolved.to_string_lossy().replace('\\', "/")
    };

    for rule in &ws.sync {
        let mut m = serde_yaml_ng::Mapping::new();
        m.insert(Value::String("action".into()), Value::String("sync".into()));
        m.insert(
            Value::String("path".into()),
            Value::String(resolve(&rule.path)),
        );
        m.insert(
            Value::String("target".into()),
            Value::String(rule.target.to_string_lossy().into_owned()),
        );
        if ws.initial_sync {
            // `initial_sync` requires compose ≥ 2.27. Compose silently
            // ignores unknown keys on older versions, so we don't probe
            // the binary's version — emit unconditionally and let the
            // older runtimes drop it.
            m.insert(Value::String("initial_sync".into()), Value::Bool(true));
        }
        entries.push(Value::Mapping(m));
    }
    for path_str in &ws.rebuild {
        let mut m = serde_yaml_ng::Mapping::new();
        m.insert(
            Value::String("action".into()),
            Value::String("rebuild".into()),
        );
        m.insert(
            Value::String("path".into()),
            Value::String(resolve(std::path::Path::new(path_str))),
        );
        entries.push(Value::Mapping(m));
    }
    for path_str in &ws.restart {
        let mut m = serde_yaml_ng::Mapping::new();
        m.insert(
            Value::String("action".into()),
            Value::String("restart".into()),
        );
        m.insert(
            Value::String("path".into()),
            Value::String(resolve(std::path::Path::new(path_str))),
        );
        entries.push(Value::Mapping(m));
    }

    let mut develop = serde_yaml_ng::Mapping::new();
    develop.insert(Value::String("watch".into()), Value::Sequence(entries));
    Some(Value::Mapping(develop))
}

fn render_healthcheck(hc: &Healthcheck) -> Value {
    let mut out = serde_yaml_ng::Mapping::new();
    let test = match &hc.kind {
        HealthcheckKind::Http {
            path,
            expect_status,
            headers,
        } => {
            // curl-based probe: portable across alpine/debian containers
            // that ship curl. Users with images that don't have curl can
            // override with `kind = "exec"`. We bake the expected status
            // into the test rather than parse `--write-out` output.
            //
            // Render any declared headers as `-H 'k: v'` flags so probes
            // against authenticated endpoints succeed. Without this the
            // probe would silently get 401/403 and the service would
            // report unhealthy forever.
            let mut header_args = String::new();
            for (k, v) in headers {
                // Single-quote the value because the whole CMD-SHELL is
                // already double-quoted by YAML scalar rules; embedded
                // single quotes in a header value are vanishingly rare.
                header_args.push_str(&format!(" -H '{k}: {v}'"));
            }
            vec![
                Value::String("CMD-SHELL".into()),
                Value::String(format!(
                    "curl -fsS -o /dev/null -w '%{{http_code}}'{header_args} http://localhost:8080{path} | grep -q '^{expect_status}$' || exit 1"
                )),
            ]
        }
        HealthcheckKind::Tcp { port } => vec![
            Value::String("CMD-SHELL".into()),
            Value::String(format!(
                "nc -z localhost {port} || (echo > /dev/tcp/localhost/{port}) >/dev/null 2>&1"
            )),
        ],
        HealthcheckKind::Exec { cmd } => {
            let mut tokens = vec![Value::String("CMD".into())];
            for arg in cmd {
                tokens.push(Value::String(arg.clone()));
            }
            tokens
        }
        HealthcheckKind::Grpc { port, service } => {
            let svc_arg = service
                .as_ref()
                .map(|s| format!(" -service {s}"))
                .unwrap_or_default();
            vec![
                Value::String("CMD-SHELL".into()),
                Value::String(format!(
                    "grpc_health_probe -addr=:{port}{svc_arg} || exit 1"
                )),
            ]
        }
    };
    out.insert(Value::String("test".into()), Value::Sequence(test));
    let HealthcheckTiming {
        interval_s,
        timeout_s,
        retries,
        start_period_s,
        start_interval_s,
        ..
    } = hc.timing;
    out.insert(
        Value::String("interval".into()),
        Value::String(format!("{interval_s}s")),
    );
    out.insert(
        Value::String("timeout".into()),
        Value::String(format!("{timeout_s}s")),
    );
    out.insert(
        Value::String("retries".into()),
        Value::Number(serde_yaml_ng::Number::from(retries as i64)),
    );
    out.insert(
        Value::String("start_period".into()),
        Value::String(format!("{start_period_s}s")),
    );
    if let Some(si) = start_interval_s {
        out.insert(
            Value::String("start_interval".into()),
            Value::String(format!("{si}s")),
        );
    }
    Value::Mapping(out)
}

/// Convenience: 8-char content hash of the rendered YAML, used to
/// detect drift in `EnvBackend::up()` and to name the rendered file
/// (`<hash>.yml`).
pub fn content_hash(yaml: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for byte in yaml.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{BuildSpec, HealthcheckKind, RealService};
    use std::collections::BTreeMap;

    fn empty_plan(name: &str) -> EnvPlan {
        EnvPlan {
            name: name.into(),
            project_name: format!("coral-{name}-deadbeef"),
            mode: crate::spec::EnvMode::Managed,
            services: BTreeMap::new(),
            env_file: None,
            project_root: std::path::PathBuf::from("/tmp"),
            chaos: None,
        }
    }

    fn real_service(name: &str) -> ServiceSpecPlan {
        ServiceSpecPlan {
            name: name.into(),
            kind: ServiceKind::Real(Box::new(RealService {
                repo: None,
                image: Some("postgres:16".into()),
                build: None,
                ports: vec![5432],
                env: BTreeMap::from([("POSTGRES_PASSWORD".into(), "dev".into())]),
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

    #[test]
    fn render_emits_project_name_and_services_section() {
        let mut plan = empty_plan("dev");
        plan.services.insert("db".into(), real_service("db"));
        let yaml = render(&plan);
        assert!(yaml.contains("name: coral-dev-deadbeef"));
        assert!(yaml.contains("services:"));
        assert!(yaml.contains("db:"));
        assert!(yaml.contains("image: postgres:16"));
    }

    #[test]
    fn render_includes_healthcheck_with_timing() {
        let mut plan = empty_plan("dev");
        plan.services.insert("db".into(), real_service("db"));
        let yaml = render(&plan);
        assert!(yaml.contains("healthcheck:"));
        assert!(yaml.contains("interval:"));
        assert!(yaml.contains("retries: 5"));
    }

    #[test]
    fn render_emits_depends_on_with_service_healthy() {
        let mut plan = empty_plan("dev");
        let mut api = real_service("api");
        if let ServiceKind::Real(real) = &mut api.kind {
            real.depends_on = vec!["db".into()];
        }
        plan.services.insert("api".into(), api);
        let yaml = render(&plan);
        assert!(yaml.contains("depends_on:"));
        assert!(yaml.contains("condition: service_healthy"));
    }

    #[test]
    fn render_with_build_emits_context_and_target() {
        let mut plan = empty_plan("dev");
        let svc = ServiceSpecPlan {
            name: "api".into(),
            kind: ServiceKind::Real(Box::new(RealService {
                repo: Some("api".into()),
                image: None,
                build: Some(BuildSpec {
                    context: std::path::PathBuf::from("."),
                    dockerfile: Some(std::path::PathBuf::from("Dockerfile")),
                    target: Some("dev".into()),
                    cache_from: vec![],
                    cache_to: None,
                    args: BTreeMap::new(),
                }),
                ports: vec![3000],
                env: BTreeMap::new(),
                depends_on: vec![],
                healthcheck: None,
                watch: None,
            })),
            resolved_context: Some(std::path::PathBuf::from("/work/repos/api")),
        };
        plan.services.insert("api".into(), svc);
        let yaml = render(&plan);
        assert!(yaml.contains("build:"));
        assert!(yaml.contains("/work/repos/api"));
        assert!(yaml.contains("target: dev"));
    }

    #[test]
    fn content_hash_is_stable() {
        let h1 = content_hash("hello");
        let h2 = content_hash("hello");
        assert_eq!(h1, h2);
        assert_ne!(content_hash("hello"), content_hash("world"));
        assert_eq!(h1.len(), 8);
    }

    #[test]
    fn render_propagates_env_file_to_every_service() {
        let mut plan = empty_plan("dev");
        plan.env_file = Some(std::path::PathBuf::from("env/dev.env"));
        plan.services.insert("db".into(), real_service("db"));
        plan.services.insert("api".into(), real_service("api"));
        let yaml = render(&plan);
        // Every service inherits env_file: [env/dev.env]. Compose merges
        // env_file before per-service `environment:` so per-service env
        // overrides still win.
        let env_file_count = yaml.matches("env/dev.env").count();
        assert!(
            env_file_count >= 2,
            "expected env_file to fan out across all services, got: {yaml}"
        );
    }

    #[test]
    fn render_omits_env_file_when_unset() {
        let mut plan = empty_plan("dev");
        plan.services.insert("db".into(), real_service("db"));
        let yaml = render(&plan);
        assert!(!yaml.contains("env_file"));
    }

    #[test]
    fn render_http_healthcheck_emits_header_flags() {
        let mut plan = empty_plan("dev");
        let mut svc = real_service("api");
        if let ServiceKind::Real(real) = &mut svc.kind {
            real.healthcheck = Some(Healthcheck {
                kind: HealthcheckKind::Http {
                    path: "/health".into(),
                    expect_status: 200,
                    headers: BTreeMap::from([(
                        "X-Internal-Auth".into(),
                        "${HEALTHCHECK_TOKEN}".into(),
                    )]),
                },
                timing: HealthcheckTiming::default(),
            });
        }
        plan.services.insert("api".into(), svc);
        let yaml = render(&plan);
        // YAML single-quote escaping turns `'foo'` into `''foo''` inside
        // the surrounding single-quoted scalar; assert the header field
        // name and value travel through, regardless of escape form.
        assert!(
            yaml.contains("-H ") && yaml.contains("X-Internal-Auth: ${HEALTHCHECK_TOKEN}"),
            "expected curl probe to render auth header flag, got:\n{yaml}"
        );
    }

    #[test]
    fn render_http_healthcheck_without_headers_is_clean() {
        let mut plan = empty_plan("dev");
        let mut svc = real_service("api");
        if let ServiceKind::Real(real) = &mut svc.kind {
            real.healthcheck = Some(Healthcheck {
                kind: HealthcheckKind::Http {
                    path: "/health".into(),
                    expect_status: 200,
                    headers: BTreeMap::new(),
                },
                timing: HealthcheckTiming::default(),
            });
        }
        plan.services.insert("api".into(), svc);
        let yaml = render(&plan);
        assert!(yaml.contains("curl -fsS"));
        // No stray `-H ` flag when no headers were declared.
        assert!(!yaml.contains(" -H "));
    }

    #[test]
    fn render_grpc_healthcheck_emits_grpc_health_probe() {
        let mut plan = empty_plan("dev");
        let mut svc = real_service("api");
        if let ServiceKind::Real(real) = &mut svc.kind {
            real.healthcheck = Some(Healthcheck {
                kind: HealthcheckKind::Grpc {
                    port: 50051,
                    service: Some("health.v1".into()),
                },
                timing: HealthcheckTiming::default(),
            });
        }
        plan.services.insert("api".into(), svc);
        let yaml = render(&plan);
        assert!(yaml.contains("grpc_health_probe -addr=:50051 -service health.v1"));
    }

    #[test]
    fn render_is_deterministic_for_identical_plans() {
        // The content hash drives `.coral/env/compose/<hash>.yml` and
        // drift detection; if rendering ever became non-deterministic
        // (e.g. iterating a HashMap), `coral up` would re-render every
        // invocation. Pin the property explicitly.
        let mut plan = empty_plan("dev");
        plan.services.insert("api".into(), real_service("api"));
        plan.services.insert("db".into(), real_service("db"));
        let a = render(&plan);
        let b = render(&plan);
        assert_eq!(a, b);
        assert_eq!(content_hash(&a), content_hash(&b));
    }

    // ---- v0.21.2: `develop.watch` rendering ----

    use crate::spec::{SyncRule, WatchSpec};

    fn watch_service(name: &str, watch: WatchSpec) -> ServiceSpecPlan {
        ServiceSpecPlan {
            name: name.into(),
            kind: ServiceKind::Real(Box::new(RealService {
                repo: None,
                image: Some("api:dev".into()),
                build: None,
                ports: vec![],
                env: BTreeMap::new(),
                depends_on: vec![],
                healthcheck: None,
                watch: Some(watch),
            })),
            resolved_context: None,
        }
    }

    #[test]
    fn watch_block_empty_emits_nothing() {
        // A `WatchSpec` with no rules is a user mistake — the CLI
        // catches it before we render — but the renderer must still
        // emit no `develop` block so a half-baked plan never produces
        // invalid YAML.
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            watch_service(
                "api",
                WatchSpec {
                    sync: vec![],
                    rebuild: vec![],
                    restart: vec![],
                    initial_sync: false,
                },
            ),
        );
        let yaml = render(&plan);
        assert!(
            !yaml.contains("develop:"),
            "empty WatchSpec must not emit a `develop` block, got:\n{yaml}"
        );
    }

    #[test]
    fn watch_block_sync_only() {
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            watch_service(
                "api",
                WatchSpec {
                    sync: vec![SyncRule {
                        path: std::path::PathBuf::from("./src"),
                        target: std::path::PathBuf::from("/app/src"),
                    }],
                    rebuild: vec![],
                    restart: vec![],
                    initial_sync: false,
                },
            ),
        );
        let yaml = render(&plan);
        assert!(yaml.contains("develop:"), "missing develop block:\n{yaml}");
        assert!(yaml.contains("watch:"), "missing watch sequence:\n{yaml}");
        assert!(
            yaml.contains("action: sync"),
            "missing sync action:\n{yaml}"
        );
        assert!(
            yaml.contains("./src") && yaml.contains("/app/src"),
            "missing path/target:\n{yaml}"
        );
        // No rebuild/restart should appear when only sync is declared.
        assert!(!yaml.contains("action: rebuild"));
        assert!(!yaml.contains("action: restart"));
        // initial_sync defaulted false — must NOT appear.
        assert!(!yaml.contains("initial_sync"));
    }

    #[test]
    fn watch_block_all_three_actions() {
        // Pin the rule order: sync first, then rebuild, then restart.
        // Compose treats the watch list as ordered for first-match
        // semantics in some edge cases, so a stable order is part of
        // the contract.
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            watch_service(
                "api",
                WatchSpec {
                    sync: vec![SyncRule {
                        path: std::path::PathBuf::from("./src"),
                        target: std::path::PathBuf::from("/app/src"),
                    }],
                    rebuild: vec!["./Dockerfile".into()],
                    restart: vec!["./config.yaml".into()],
                    initial_sync: false,
                },
            ),
        );
        let yaml = render(&plan);
        let sync_idx = yaml.find("action: sync").expect("missing sync action");
        let rebuild_idx = yaml
            .find("action: rebuild")
            .expect("missing rebuild action");
        let restart_idx = yaml
            .find("action: restart")
            .expect("missing restart action");
        assert!(
            sync_idx < rebuild_idx && rebuild_idx < restart_idx,
            "expected sync < rebuild < restart in YAML output, got:\n{yaml}"
        );
    }

    #[test]
    fn watch_initial_sync_propagates_to_sync_entries() {
        // `initial_sync = true` flips an `initial_sync: true` flag on
        // every sync entry (compose ≥ 2.27). It MUST NOT appear on
        // rebuild / restart entries — they're not sync ops.
        let mut plan = empty_plan("dev");
        plan.services.insert(
            "api".into(),
            watch_service(
                "api",
                WatchSpec {
                    sync: vec![
                        SyncRule {
                            path: std::path::PathBuf::from("./src"),
                            target: std::path::PathBuf::from("/app/src"),
                        },
                        SyncRule {
                            path: std::path::PathBuf::from("./templates"),
                            target: std::path::PathBuf::from("/app/templates"),
                        },
                    ],
                    rebuild: vec!["./Dockerfile".into()],
                    restart: vec![],
                    initial_sync: true,
                },
            ),
        );
        let yaml = render(&plan);
        // Two sync entries, two `initial_sync: true` flags.
        let count = yaml.matches("initial_sync: true").count();
        assert_eq!(
            count, 2,
            "expected two initial_sync flags, got {count} in:\n{yaml}"
        );
    }

    #[test]
    fn watch_path_resolves_against_resolved_context() {
        // For a service with `repo = "..."`, the renderer resolves
        // relative `path` values against `resolved_context` (the
        // checkout root) so `./src` lives at `/work/repos/api/src`,
        // NOT at the cwd of `coral up`. Mirrors how `build.context`
        // is resolved in `render_real`.
        let svc = ServiceSpecPlan {
            name: "api".into(),
            kind: ServiceKind::Real(Box::new(RealService {
                repo: Some("api".into()),
                image: None,
                build: None,
                ports: vec![],
                env: BTreeMap::new(),
                depends_on: vec![],
                healthcheck: None,
                watch: Some(WatchSpec {
                    sync: vec![SyncRule {
                        path: std::path::PathBuf::from("./src"),
                        target: std::path::PathBuf::from("/app/src"),
                    }],
                    rebuild: vec!["./Dockerfile".into()],
                    restart: vec![],
                    initial_sync: false,
                }),
            })),
            resolved_context: Some(std::path::PathBuf::from("/work/repos/api")),
        };
        let mut plan = empty_plan("dev");
        plan.services.insert("api".into(), svc);
        let yaml = render(&plan);
        // The sync path must be the joined absolute path; the target
        // (container-side) must pass through verbatim.
        assert!(
            yaml.contains("/work/repos/api/./src") || yaml.contains("/work/repos/api/src"),
            "sync path not resolved against resolved_context, got:\n{yaml}"
        );
        assert!(
            yaml.contains("/work/repos/api/./Dockerfile")
                || yaml.contains("/work/repos/api/Dockerfile"),
            "rebuild path not resolved against resolved_context, got:\n{yaml}"
        );
        // Target stays container-side, untouched.
        assert!(
            yaml.contains("target: /app/src"),
            "target should pass through, got:\n{yaml}"
        );
    }

    #[test]
    fn watch_absent_yields_yaml_identical_to_pre_watch() {
        // BC contract — the centerpiece of v0.21.2: a service with
        // `watch: None` (i.e. `[services.*.watch]` absent in TOML)
        // produces YAML byte-identical to v0.21.1 output. We pin this
        // by rendering the same plan twice (once with `watch: None`,
        // the default) and asserting no `develop:` keyword appears.
        let mut plan = empty_plan("dev");
        plan.services.insert("db".into(), real_service("db"));
        let yaml = render(&plan);
        assert!(
            !yaml.contains("develop:"),
            "services without `watch` must NOT emit a develop block, got:\n{yaml}"
        );
        assert!(
            !yaml.contains(" watch:"),
            "services without `watch` must NOT emit a watch key, got:\n{yaml}"
        );
    }

    // ---- v0.23.0: chaos sidecar + reroute ----

    /// **BC golden** — pre-v0.23.0 plans (chaos = None) MUST produce
    /// the SAME YAML on a v0.23.0 binary as on v0.22.6. We pin this
    /// by rendering an api+db plan with chaos = None and asserting
    /// the keyword `toxiproxy` appears nowhere in the output.
    ///
    /// Pair this with `compose_yaml_emits_toxiproxy_sidecar_when_chaos_enabled`
    /// — together they cover both arms of the gate.
    #[test]
    fn compose_yaml_no_chaos_byte_identical_to_v0226() {
        let mut plan = empty_plan("dev");
        plan.services.insert("db".into(), real_service("db"));
        let mut api = real_service("api");
        if let ServiceKind::Real(real) = &mut api.kind {
            real.image = Some("api:dev".into());
            real.ports = vec![3000];
            real.depends_on = vec!["db".into()];
        }
        plan.services.insert("api".into(), api);

        let yaml = render(&plan);
        // Critical BC pin: zero "toxiproxy" mentions when chaos is off.
        assert!(
            !yaml.contains("toxiproxy"),
            "chaos = None must NOT emit any toxiproxy artifact, got:\n{yaml}"
        );
        // No DB_URL injection either — the env block is purely user-driven.
        assert!(
            !yaml.contains("DB_URL"),
            "chaos = None must NOT inject DB_URL, got:\n{yaml}"
        );
        // Render twice, check determinism.
        assert_eq!(yaml, render(&plan));
    }

    #[test]
    fn compose_yaml_emits_toxiproxy_sidecar_when_chaos_enabled() {
        use crate::spec::{ChaosBackend, ChaosConfig};
        let mut plan = empty_plan("dev");
        plan.services.insert("db".into(), real_service("db"));
        let mut api = real_service("api");
        if let ServiceKind::Real(real) = &mut api.kind {
            real.image = Some("api:dev".into());
            real.ports = vec![3000];
            real.depends_on = vec!["db".into()];
        }
        plan.services.insert("api".into(), api);
        plan.chaos = Some(ChaosConfig {
            backend: ChaosBackend::Toxiproxy,
            image: "ghcr.io/shopify/toxiproxy:2.7.0".to_string(),
            listen_port: 8474,
        });

        let yaml = render(&plan);
        // Sidecar service emitted.
        assert!(yaml.contains("toxiproxy:"), "missing sidecar:\n{yaml}");
        assert!(
            yaml.contains("ghcr.io/shopify/toxiproxy:2.7.0"),
            "missing image:\n{yaml}"
        );
        assert!(yaml.contains("/version"), "missing healthcheck:\n{yaml}");
        // The init-config bind mount.
        assert!(
            yaml.contains("toxiproxy-init.json"),
            "missing init bind:\n{yaml}"
        );
        // Consumer's env reroute. The dep `db` has port 5432 in
        // `real_service`, so the proxy port is deterministic via FNV.
        let port = chaos_proxy_port("api", "db");
        let expected = format!("DB_URL=http://toxiproxy:{port}");
        assert!(
            yaml.contains(&expected),
            "missing reroute env, expected `{expected}`, got:\n{yaml}"
        );
        // Sanity: rendering twice is deterministic.
        assert_eq!(yaml, render(&plan));
    }

    #[test]
    fn chaos_proxy_port_is_deterministic_and_in_range() {
        let p = chaos_proxy_port("api", "db");
        assert_eq!(p, chaos_proxy_port("api", "db"), "must be stable");
        assert!(p >= 30_000, "port must be in 30000-39999 range: {p}");
        assert!(p < 40_000, "port must be in 30000-39999 range: {p}");
        // Different inputs → different (likely) ports.
        assert_ne!(
            chaos_proxy_port("api", "db"),
            chaos_proxy_port("worker", "queue")
        );
    }

    #[test]
    fn chaos_proxies_walks_depends_on_edges_and_skips_portless_deps() {
        use crate::spec::{ChaosBackend, ChaosConfig};
        let mut plan = empty_plan("dev");
        // db has port 5432 (from real_service); portless service
        // declared inline so the renderer must skip the edge.
        plan.services.insert("db".into(), real_service("db"));
        let portless = ServiceSpecPlan {
            name: "queue".into(),
            kind: ServiceKind::Real(Box::new(RealService {
                repo: None,
                image: Some("redis:7".into()),
                build: None,
                ports: vec![], // intentionally empty
                env: BTreeMap::new(),
                depends_on: vec![],
                healthcheck: None,
                watch: None,
            })),
            resolved_context: None,
        };
        plan.services.insert("queue".into(), portless);
        let mut api = real_service("api");
        if let ServiceKind::Real(real) = &mut api.kind {
            real.depends_on = vec!["db".into(), "queue".into()];
        }
        plan.services.insert("api".into(), api);
        plan.chaos = Some(ChaosConfig {
            backend: ChaosBackend::Toxiproxy,
            image: "ghcr.io/shopify/toxiproxy:2.7.0".to_string(),
            listen_port: 8474,
        });

        let proxies = chaos_proxies(&plan);
        // Only one proxy: the portful (api → db) edge.
        assert_eq!(
            proxies.len(),
            1,
            "expected 1 proxy (api→db); got: {proxies:?}"
        );
        assert_eq!(proxies[0].name, "api_to_db");
        assert_eq!(proxies[0].consumer, "api");
        assert_eq!(proxies[0].dep, "db");
        assert_eq!(proxies[0].upstream_port, 5432);
    }

    #[test]
    fn chaos_init_json_has_pre_wired_proxies() {
        let proxies = vec![ChaosProxy {
            name: "api_to_db".into(),
            consumer: "api".into(),
            dep: "db".into(),
            listen_port: 30421,
            upstream_port: 5432,
        }];
        let json = chaos_init_json(&proxies);
        // Must be parseable JSON.
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let arr = parsed.as_array().expect("top-level array");
        assert_eq!(arr.len(), 1);
        let p = &arr[0];
        assert_eq!(p["name"], "api_to_db");
        assert_eq!(p["listen"], "0.0.0.0:30421");
        assert_eq!(p["upstream"], "db:5432");
        assert_eq!(p["enabled"], true);
    }

    #[test]
    fn chaos_user_env_value_wins_over_inferred_url() {
        // If the user explicitly sets DB_URL in `env: {}`, the
        // chaos reroute does NOT clobber it. Treat the user value
        // as the authoritative override (e.g. they want chaos OFF
        // for that specific edge).
        use crate::spec::{ChaosBackend, ChaosConfig};
        let mut plan = empty_plan("dev");
        plan.services.insert("db".into(), real_service("db"));
        let mut api = real_service("api");
        if let ServiceKind::Real(real) = &mut api.kind {
            real.depends_on = vec!["db".into()];
            real.env
                .insert("DB_URL".into(), "postgres://direct/db".into());
        }
        plan.services.insert("api".into(), api);
        plan.chaos = Some(ChaosConfig {
            backend: ChaosBackend::Toxiproxy,
            image: "ghcr.io/shopify/toxiproxy:2.7.0".to_string(),
            listen_port: 8474,
        });
        let yaml = render(&plan);
        // The user-defined DB_URL must remain untouched.
        assert!(
            yaml.contains("DB_URL=postgres://direct/db"),
            "user-defined DB_URL must win, got:\n{yaml}"
        );
        // No second `DB_URL=http://toxiproxy:...` should appear.
        let count = yaml.matches("DB_URL=").count();
        assert_eq!(count, 1, "DB_URL must appear exactly once, got:\n{yaml}");
    }
}
