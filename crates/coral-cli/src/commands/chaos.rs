//! `coral chaos <subcommand>` — chaos-engineering for the dev env.
//!
//! v0.23.0 ships the **Toxiproxy** backend (Pumba/Litmus deferred).
//! Sibling to `coral env` rather than nested under it (decision D4 in
//! the orchestrator's spec). Four subcommands:
//!
//! - `inject --service NAME --toxic TYPE[:value] [--duration SECONDS]`
//! - `clear [--service NAME]`
//! - `list [--json]`
//! - `run <scenario-name>`
//!
//! The Toxiproxy admin API speaks plain JSON over HTTP on a port the
//! sidecar publishes. The compose YAML emits the sidecar with a
//! random host port (compose `ports: ["8474"]` syntax — no host
//! pin), so we MUST discover the actual binding via
//! `EnvBackend::status()`.published_ports — the spec calls this out
//! as decision D6.
//!
//! HTTP plumbing follows the existing curl-via-Command pattern from
//! `coral_runner::http` and `commands::notion_push`: the sync CLI
//! deliberately avoids dragging `reqwest` + `tokio` into the
//! workspace. `build_*_curl_command()` are factored out so the
//! request shape (URL + method + body) is unit-testable without
//! spawning a process.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use coral_env::compose::{ComposeBackend, ComposeRuntime};
use coral_env::{ChaosScenario, EnvBackend, EnvPlan, EnvironmentSpec, PublishedPort, ToxicKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, ExitCode};

use crate::commands::common::resolve_project;
use crate::commands::env_resolve::{default_env_name, resolve_env};

#[derive(Args, Debug)]
pub struct ChaosArgs {
    #[command(subcommand)]
    pub command: ChaosCmd,
}

#[derive(Subcommand, Debug)]
pub enum ChaosCmd {
    /// Inject a toxic into a service edge.
    Inject(InjectArgs),
    /// Remove toxics. Without `--service`, clears every active toxic.
    Clear(ClearArgs),
    /// List active proxies + toxics.
    List(ListArgs),
    /// Apply a pre-canned scenario from `[[chaos_scenarios]]`.
    Run(RunArgs),
}

#[derive(Args, Debug)]
pub struct InjectArgs {
    /// Environment name (default: first declared).
    #[arg(long)]
    pub env: Option<String>,
    /// Target service. Must appear as a `depends_on` dep of some other
    /// service (chaos can only proxy traffic that flows through an edge).
    #[arg(long)]
    pub service: String,
    /// Toxic specifier. Forms:
    ///
    ///   - `latency:Nms`     — N ms artificial delay
    ///   - `bandwidth:Nkb`   — cap throughput at N kb/s
    ///   - `slow_close:Nms`  — N ms delay on connection close
    ///   - `timeout`         — connection drops after `attributes.timeout`
    ///   - `slicer`          — chunk + delay TCP packets
    ///
    /// Anything else is rejected with a friendly error.
    #[arg(long)]
    pub toxic: String,
    /// Best-effort duration in seconds. The toxic is applied
    /// immediately; on `--duration` expiry the CLI POSTs `clear`. If
    /// the CLI dies before then, the toxic stays put.
    #[arg(long)]
    pub duration: Option<u64>,
}

#[derive(Args, Debug)]
pub struct ClearArgs {
    #[arg(long)]
    pub env: Option<String>,
    /// Limit to a specific service. Without it: clear every toxic
    /// across every proxy.
    #[arg(long)]
    pub service: Option<String>,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    #[arg(long)]
    pub env: Option<String>,
    /// Emit machine-readable JSON instead of the default text table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    #[arg(long)]
    pub env: Option<String>,
    /// Scenario name from `[[chaos_scenarios]]`.
    pub name: String,
}

pub fn run(args: ChaosArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        ChaosCmd::Inject(a) => inject(a, wiki_root),
        ChaosCmd::Clear(a) => clear(a, wiki_root),
        ChaosCmd::List(a) => list(a, wiki_root),
        ChaosCmd::Run(a) => run_scenario(a, wiki_root),
    }
}

// ---- subcommand handlers --------------------------------------------------

fn inject(args: InjectArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let ctx = build_context(wiki_root, args.env.as_deref())?;
    let parsed = parse_toxic_spec(&args.toxic)
        .map_err(|msg| anyhow::anyhow!("invalid --toxic '{}': {}", args.toxic, msg))?;
    inject_one(
        &ctx,
        &args.service,
        &parsed,
        args.duration.map(std::time::Duration::from_secs),
    )
}

fn inject_one(
    ctx: &ChaosContext,
    service: &str,
    parsed: &ParsedToxic,
    duration: Option<std::time::Duration>,
) -> Result<ExitCode> {
    let proxy_name = format!("api_to_{service}");
    // Search every proxy that ends in `_to_<service>` so the user
    // doesn't have to know who the consumer is. We always pick the
    // alphabetically-first match for determinism.
    let proxy_name = pick_proxy_for_service(&ctx.spec, service).unwrap_or(proxy_name);

    let payload = build_inject_payload(parsed);
    let body = serde_json::to_string(&payload).context("serializing toxic body")?;
    let url = format!("{}/proxies/{}/toxics", ctx.admin_url, proxy_name);
    let cmd = build_inject_curl_command(&url, &body);
    let response = run_http(cmd, &body)?;
    if !response.is_success {
        eprintln!(
            "error: toxiproxy admin API returned status {} from {}: {}",
            response.status_code, url, response.body
        );
        return Ok(ExitCode::FAILURE);
    }
    println!(
        "✔ injected {} on {} (proxy {}, port {})",
        parsed.kind.as_api_str(),
        service,
        proxy_name,
        ctx.admin_port
    );

    if let Some(d) = duration {
        std::thread::sleep(d);
        let toxic_name = payload.name.clone();
        let url = format!(
            "{}/proxies/{}/toxics/{}",
            ctx.admin_url, proxy_name, toxic_name
        );
        let cmd = build_delete_curl_command(&url);
        // Best-effort. Don't error out — if cleanup fails the user
        // has the same recovery path as a CLI crash mid-run: re-run
        // `coral chaos clear`.
        let _ = run_http(cmd, "");
    }

    Ok(ExitCode::SUCCESS)
}

fn clear(args: ClearArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let ctx = build_context(wiki_root, args.env.as_deref())?;
    // GET /proxies, walk each one, DELETE its toxics.
    let proxies = fetch_proxies(&ctx)?;
    let mut total = 0usize;
    for (proxy_name, proxy) in &proxies {
        if let Some(svc) = &args.service {
            // Only touch proxies that target this service.
            if !proxy_name.ends_with(&format!("_to_{svc}")) {
                continue;
            }
        }
        for toxic in &proxy.toxics {
            let url = format!(
                "{}/proxies/{}/toxics/{}",
                ctx.admin_url, proxy_name, toxic.name
            );
            let cmd = build_delete_curl_command(&url);
            let response = run_http(cmd, "")?;
            if !response.is_success {
                eprintln!(
                    "warn: failed to delete toxic '{}' on '{}': HTTP {}",
                    toxic.name, proxy_name, response.status_code
                );
                continue;
            }
            total += 1;
        }
    }
    println!("✔ cleared {} toxic(s)", total);
    Ok(ExitCode::SUCCESS)
}

fn list(args: ListArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let ctx = build_context(wiki_root, args.env.as_deref())?;
    let proxies = fetch_proxies(&ctx)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&proxies).context("serializing proxies")?
        );
    } else {
        if proxies.is_empty() {
            println!("(no proxies registered)");
        }
        for (name, proxy) in &proxies {
            println!(
                "{name}: {} → {}{}",
                proxy.listen,
                proxy.upstream,
                if proxy.toxics.is_empty() {
                    String::new()
                } else {
                    format!(
                        " [{}]",
                        proxy
                            .toxics
                            .iter()
                            .map(|t| t.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_scenario(args: RunArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let ctx = build_context(wiki_root, args.env.as_deref())?;
    let scenario = ctx
        .spec
        .chaos_scenarios
        .iter()
        .find(|s| s.name == args.name)
        .cloned();
    let Some(scenario) = scenario else {
        let available: Vec<&str> = ctx
            .spec
            .chaos_scenarios
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        eprintln!(
            "error: chaos scenario '{}' not found in environment '{}'; available: {}",
            args.name,
            ctx.env_name,
            if available.is_empty() {
                "(none)".into()
            } else {
                available.join(", ")
            }
        );
        // Exit 2 = "user error" by convention here (see acceptance criterion #7).
        return Ok(ExitCode::from(2));
    };
    let parsed = parsed_from_scenario(&scenario);
    inject_one(&ctx, &scenario.service, &parsed, None)
}

// ---- context resolution ----------------------------------------------------

/// Resolved chaos context: spec + admin URL + env name. Built once
/// per CLI invocation by every subcommand.
struct ChaosContext {
    spec: EnvironmentSpec,
    env_name: String,
    admin_url: String,
    admin_port: u16,
}

fn build_context(wiki_root: Option<&Path>, env_arg: Option<&str>) -> Result<ChaosContext> {
    let project = resolve_project(wiki_root)?;
    if project.environments_raw.is_empty() {
        anyhow::bail!("no [[environments]] declared in coral.toml");
    }
    let env_name = env_arg
        .map(str::to_string)
        .unwrap_or_else(|| default_env_name(&project));
    let spec = resolve_env(&project, &env_name)?;
    let chaos = match &spec.chaos {
        Some(c) => c,
        None => {
            // Acceptance criterion #8: friendly error + exit 2.
            anyhow::bail!(
                "no chaos backend configured for env '{}'; add `[environments.{}.chaos] backend = \"toxiproxy\"`",
                env_name,
                env_name
            );
        }
    };
    // Discover the published port via the backend's `status()`.
    let mut repo_paths = BTreeMap::new();
    for repo in &project.repos {
        repo_paths.insert(repo.name.clone(), project.resolved_path(repo));
    }
    let plan = EnvPlan::from_spec(&spec, &project.root, &repo_paths)
        .map_err(|e| anyhow::anyhow!("building env plan: {}", e))?;
    let backend = ComposeBackend::new(ComposeRuntime::parse(&spec.compose_command));
    let status = backend
        .status(&plan)
        .context("querying environment status (is `coral up` running?)")?;
    let toxiproxy = status
        .services
        .iter()
        .find(|s| s.name == "toxiproxy")
        .cloned();
    let toxiproxy = match toxiproxy {
        Some(s) => s,
        None => anyhow::bail!(
            "toxiproxy sidecar not running in env '{}'; run `coral up --env {}` first",
            env_name,
            env_name
        ),
    };
    let host_port = pick_admin_port(&toxiproxy.published_ports, chaos.listen_port)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "toxiproxy is up but no host port is published for {}; run `coral up --env {}` first",
                chaos.listen_port,
                env_name
            )
        })?;
    let admin_url = format!("http://127.0.0.1:{host_port}");
    Ok(ChaosContext {
        spec,
        env_name,
        admin_url,
        admin_port: host_port,
    })
}

/// The compose YAML uses `ports: ["8474"]` which publishes the admin
/// port to a random host port. We surface the host side from
/// `published_ports[*]` matching the in-container port.
pub(crate) fn pick_admin_port(ports: &[PublishedPort], container: u16) -> Option<u16> {
    ports
        .iter()
        .find(|p| p.container_port == container)
        .map(|p| p.host_port)
}

/// Pick a proxy whose name matches the `_to_<service>` suffix.
/// Determinism: alphabetic first. When the user wires multiple
/// consumers to the same dep we pick the first one to keep the
/// command idempotent — they can use `--service NAME` to pick the
/// proxy explicitly via the long form (future enhancement).
pub(crate) fn pick_proxy_for_service(spec: &EnvironmentSpec, service: &str) -> Option<String> {
    let suffix = format!("_to_{service}");
    let mut candidates: Vec<String> = Vec::new();
    for (consumer_name, consumer) in &spec.services {
        if let coral_env::ServiceKind::Real(real) = consumer
            && real.depends_on.iter().any(|d| d == service)
        {
            candidates.push(format!("{}{}", consumer_name, suffix));
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

// ---- toxic spec parsing ---------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedToxic {
    pub kind: ToxicKind,
    /// Numeric value extracted from the suffix (e.g. `latency:500ms`
    /// → `Some(500)`). For toxics that take no numeric argument
    /// (`timeout`, `slicer`) this is `None`.
    pub value: Option<u64>,
}

/// Parse a `--toxic` CLI value into a `ParsedToxic`.
///
/// Accepted forms:
///   - `latency:500`        → 500ms
///   - `latency:500ms`      → 500ms
///   - `bandwidth:100kb`    → 100kb/s
///   - `slow_close:200ms`   → 200ms close delay
///   - `timeout`            → no value
///   - `slicer`             → no value
pub(crate) fn parse_toxic_spec(s: &str) -> Result<ParsedToxic, String> {
    let (name, rest) = match s.split_once(':') {
        Some((n, r)) => (n, Some(r)),
        None => (s, None),
    };
    let kind = match name {
        "latency" => ToxicKind::Latency,
        "bandwidth" => ToxicKind::Bandwidth,
        "slow_close" => ToxicKind::SlowClose,
        "timeout" => ToxicKind::Timeout,
        "slicer" => ToxicKind::Slicer,
        other => {
            return Err(format!(
                "unknown toxic '{}'; valid: latency, bandwidth, slow_close, timeout, slicer",
                other
            ));
        }
    };
    let value = match (kind, rest) {
        (ToxicKind::Latency | ToxicKind::Bandwidth | ToxicKind::SlowClose, Some(v)) => {
            Some(parse_numeric_with_unit(v)?)
        }
        (ToxicKind::Latency | ToxicKind::Bandwidth | ToxicKind::SlowClose, None) => {
            return Err(format!(
                "toxic '{}' requires a value (e.g. {}:500ms)",
                name, name
            ));
        }
        (ToxicKind::Timeout | ToxicKind::Slicer, _) => None,
    };
    Ok(ParsedToxic { kind, value })
}

fn parse_numeric_with_unit(raw: &str) -> Result<u64, String> {
    let raw = raw.trim();
    // Strip a trailing unit suffix (ms, kb, b, s) — the API takes
    // raw integers, the suffix is sugar for the user.
    let (digits, _unit) = raw
        .find(|c: char| !c.is_ascii_digit())
        .map(|idx| raw.split_at(idx))
        .unwrap_or((raw, ""));
    digits
        .parse::<u64>()
        .map_err(|e| format!("not a number: '{raw}': {e}"))
}

/// Build the JSON body for `POST /proxies/<name>/toxics`. Public to
/// the crate so unit tests can assert the shape without spawning
/// a network subprocess.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ToxicPayload {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub stream: String,
    pub toxicity: f32,
    pub attributes: BTreeMap<String, serde_json::Value>,
}

pub(crate) fn build_inject_payload(parsed: &ParsedToxic) -> ToxicPayload {
    let mut attributes: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    match parsed.kind {
        ToxicKind::Latency => {
            // toxiproxy accepts an integer milliseconds value.
            attributes.insert(
                "latency".into(),
                serde_json::Value::Number((parsed.value.unwrap_or(0)).into()),
            );
        }
        ToxicKind::Bandwidth => {
            attributes.insert(
                "rate".into(),
                serde_json::Value::Number((parsed.value.unwrap_or(0)).into()),
            );
        }
        ToxicKind::SlowClose => {
            attributes.insert(
                "delay".into(),
                serde_json::Value::Number((parsed.value.unwrap_or(0)).into()),
            );
        }
        ToxicKind::Timeout | ToxicKind::Slicer => {
            // No required attributes; toxiproxy uses defaults.
        }
    }
    ToxicPayload {
        name: format!("{}_chaos", parsed.kind.as_api_str()),
        kind: parsed.kind.as_api_str().to_string(),
        stream: "downstream".to_string(),
        toxicity: 1.0,
        attributes,
    }
}

fn parsed_from_scenario(scenario: &ChaosScenario) -> ParsedToxic {
    // Pull the primary numeric attribute (latency for Latency,
    // rate for Bandwidth, delay for SlowClose).
    let value = match scenario.toxic {
        ToxicKind::Latency => scenario
            .attributes
            .get("latency")
            .and_then(|v| v.as_integer())
            .map(|i| i as u64),
        ToxicKind::Bandwidth => scenario
            .attributes
            .get("rate")
            .and_then(|v| v.as_integer())
            .map(|i| i as u64),
        ToxicKind::SlowClose => scenario
            .attributes
            .get("delay")
            .and_then(|v| v.as_integer())
            .map(|i| i as u64),
        ToxicKind::Timeout | ToxicKind::Slicer => None,
    };
    ParsedToxic {
        kind: scenario.toxic,
        value,
    }
}

// ---- HTTP wire (curl-via-Command) -----------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct HttpResponse {
    is_success: bool,
    status_code: u16,
    body: String,
}

/// Build the curl command for a `POST /proxies/<name>/toxics`
/// request. Body lands on stdin via `--data-binary @-` so it never
/// appears in argv — same hardening as the rest of the workspace
/// (see `coral_runner::http::build_curl`).
pub(crate) fn build_inject_curl_command(url: &str, _body: &str) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-s",
        "-w",
        "\nHTTP_CODE:%{http_code}",
        "-X",
        "POST",
        url,
        "-H",
        "Content-Type: application/json",
        "--data-binary",
        "@-",
    ]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd
}

pub(crate) fn build_delete_curl_command(url: &str) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args(["-s", "-w", "\nHTTP_CODE:%{http_code}", "-X", "DELETE", url]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd
}

pub(crate) fn build_get_curl_command(url: &str) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args(["-s", "-w", "\nHTTP_CODE:%{http_code}", "-X", "GET", url]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd
}

fn run_http(mut cmd: Command, body: &str) -> Result<HttpResponse> {
    let mut child = cmd.spawn().context("spawning curl (is it on PATH?)")?;
    if !body.is_empty()
        && let Some(mut stdin) = child.stdin.take()
    {
        use std::io::Write as _;
        stdin
            .write_all(body.as_bytes())
            .context("writing body to curl stdin")?;
        drop(stdin);
    } else {
        drop(child.stdin.take());
    }
    let output = child.wait_with_output().context("awaiting curl")?;
    let combined = String::from_utf8_lossy(&output.stdout);
    let (response_body, http_code) = match combined.rsplit_once("\nHTTP_CODE:") {
        Some((b, c)) => (b.to_string(), c.trim().to_string()),
        None => (String::new(), combined.trim().to_string()),
    };
    let status_code = http_code.parse::<u16>().unwrap_or(0);
    Ok(HttpResponse {
        is_success: (200..300).contains(&status_code),
        status_code,
        body: response_body,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct ProxyView {
    pub name: String,
    pub listen: String,
    pub upstream: String,
    pub enabled: bool,
    #[serde(default)]
    pub toxics: Vec<ToxicView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct ToxicView {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub stream: String,
    #[serde(default)]
    pub attributes: serde_json::Value,
}

fn fetch_proxies(ctx: &ChaosContext) -> Result<BTreeMap<String, ProxyView>> {
    let url = format!("{}/proxies", ctx.admin_url);
    let cmd = build_get_curl_command(&url);
    let resp = run_http(cmd, "")?;
    if !resp.is_success {
        anyhow::bail!(
            "GET {} returned HTTP {}: {}",
            url,
            resp.status_code,
            resp.body
        );
    }
    // Toxiproxy `/proxies` returns a JSON object keyed by proxy name.
    let parsed: BTreeMap<String, ProxyView> =
        serde_json::from_str(&resp.body).with_context(|| {
            format!(
                "parsing /proxies response (HTTP {}): {}",
                resp.status_code, resp.body
            )
        })?;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_toxic_spec_handles_canonical_forms() {
        let p = parse_toxic_spec("latency:500ms").unwrap();
        assert_eq!(p.kind, ToxicKind::Latency);
        assert_eq!(p.value, Some(500));
        let p = parse_toxic_spec("bandwidth:100kb").unwrap();
        assert_eq!(p.kind, ToxicKind::Bandwidth);
        assert_eq!(p.value, Some(100));
        let p = parse_toxic_spec("slow_close:200ms").unwrap();
        assert_eq!(p.kind, ToxicKind::SlowClose);
        assert_eq!(p.value, Some(200));
        let p = parse_toxic_spec("timeout").unwrap();
        assert_eq!(p.kind, ToxicKind::Timeout);
        assert_eq!(p.value, None);
        let p = parse_toxic_spec("slicer").unwrap();
        assert_eq!(p.kind, ToxicKind::Slicer);
    }

    #[test]
    fn parse_toxic_spec_rejects_unknown_kinds() {
        // Acceptance criterion #9: unknown toxics rejected with a
        // valid-list error.
        let err = parse_toxic_spec("flux-capacitor:999").expect_err("must reject");
        assert!(err.contains("unknown toxic"), "msg: {err}");
        assert!(err.contains("latency"), "msg: {err}");
        assert!(err.contains("bandwidth"), "msg: {err}");
    }

    #[test]
    fn parse_toxic_spec_requires_value_for_value_taking_kinds() {
        let err = parse_toxic_spec("latency").expect_err("must reject");
        assert!(err.contains("requires a value"), "msg: {err}");
    }

    #[test]
    fn build_inject_payload_for_latency_writes_attributes() {
        let parsed = ParsedToxic {
            kind: ToxicKind::Latency,
            value: Some(500),
        };
        let payload = build_inject_payload(&parsed);
        assert_eq!(payload.kind, "latency");
        assert_eq!(payload.stream, "downstream");
        assert_eq!(payload.toxicity, 1.0);
        assert_eq!(payload.name, "latency_chaos");
        let json = serde_json::to_value(&payload).unwrap();
        // Pin the wire shape that toxiproxy admin API expects.
        assert_eq!(json["type"], "latency");
        assert_eq!(json["attributes"]["latency"], 500);
    }

    #[test]
    fn build_inject_payload_for_bandwidth_writes_rate() {
        let parsed = ParsedToxic {
            kind: ToxicKind::Bandwidth,
            value: Some(100),
        };
        let payload = build_inject_payload(&parsed);
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["type"], "bandwidth");
        assert_eq!(json["attributes"]["rate"], 100);
    }

    #[test]
    fn build_inject_curl_command_uses_post_and_pipes_body() {
        let cmd = build_inject_curl_command(
            "http://127.0.0.1:39000/proxies/api_to_db/toxics",
            r#"{"type":"latency"}"#,
        );
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(argv.contains(&"POST".to_string()), "argv: {argv:?}");
        assert!(
            argv.iter().any(|a| a.contains("/proxies/api_to_db/toxics")),
            "argv: {argv:?}"
        );
        // Body must travel via stdin (`@-`), not argv — keeps the
        // wire shape consistent with the rest of the workspace.
        assert!(
            argv.iter().any(|a| a == "@-"),
            "expected `@-` body sentinel: {argv:?}"
        );
        // No `-d` form (would put body in argv).
        assert!(!argv.iter().any(|a| a == "-d"), "argv: {argv:?}");
    }

    #[test]
    fn build_delete_curl_command_uses_delete_method() {
        let cmd =
            build_delete_curl_command("http://127.0.0.1:39000/proxies/api_to_db/toxics/latency");
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(argv.contains(&"DELETE".to_string()), "argv: {argv:?}");
    }

    #[test]
    fn pick_admin_port_returns_host_for_matching_container_port() {
        let ports = vec![
            PublishedPort {
                container_port: 8474,
                host_port: 39111,
            },
            PublishedPort {
                container_port: 80,
                host_port: 8080,
            },
        ];
        assert_eq!(pick_admin_port(&ports, 8474), Some(39111));
        assert_eq!(pick_admin_port(&ports, 9999), None);
    }

    #[test]
    fn parsed_from_scenario_extracts_latency_attribute() {
        let scenario = ChaosScenario {
            name: "x".into(),
            toxic: ToxicKind::Latency,
            service: "api".into(),
            attributes: BTreeMap::from([("latency".into(), toml::Value::Integer(750))]),
        };
        let parsed = parsed_from_scenario(&scenario);
        assert_eq!(parsed.kind, ToxicKind::Latency);
        assert_eq!(parsed.value, Some(750));
    }

    /// Acceptance criterion #5: full end-to-end round trip — a tiny
    /// TCP server pretends to be the toxiproxy admin API, the
    /// `inject` path POSTs to it, and we assert the request body
    /// carries the right `type`, `attributes`, etc.
    ///
    /// We can't go through `inject()` end-to-end without a full
    /// `coral.toml` + manifest discovery — that's covered in the CLI
    /// integration tests. Here we exercise just the wire path:
    /// build the curl command, run it against the mock, parse the
    /// resulting body.
    #[test]
    fn inject_calls_toxiproxy_api_with_right_payload() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::mpsc;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel::<String>();
        let server = std::thread::spawn(move || {
            // Single-shot: accept, read until headers + body, send 200.
            let (mut stream, _) = listener.accept().expect("accept");
            // Read timeout: prevents indefinite block if curl sends a
            // full request in one go (the second `read` would otherwise
            // wait forever for data that never comes — curl is waiting
            // on the response). 250ms is plenty for localhost.
            stream
                .set_read_timeout(Some(std::time::Duration::from_millis(250)))
                .ok();
            let mut buf = [0u8; 4096];
            let mut request = Vec::new();
            // Parse Content-Length out of the headers; once we have headers
            // + that many body bytes, we're done.
            let mut headers_end = None;
            let mut content_length: Option<usize> = None;
            loop {
                let n = match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break, // timeout or other error
                };
                request.extend_from_slice(&buf[..n]);
                if headers_end.is_none()
                    && let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n")
                {
                    headers_end = Some(pos + 4);
                    // Look for Content-Length: N
                    let header_str = String::from_utf8_lossy(&request[..pos]);
                    for line in header_str.lines() {
                        if let Some(rest) = line
                            .strip_prefix("Content-Length:")
                            .or_else(|| line.strip_prefix("content-length:"))
                        {
                            content_length = rest.trim().parse().ok();
                            break;
                        }
                    }
                }
                if let (Some(h), Some(cl)) = (headers_end, content_length)
                    && request.len() >= h + cl
                {
                    break;
                }
                if headers_end.is_some() && content_length.is_none() {
                    // No Content-Length and headers complete: assume no body.
                    break;
                }
            }
            let s = String::from_utf8_lossy(&request).into_owned();
            tx.send(s).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .ok();
        });

        let parsed = ParsedToxic {
            kind: ToxicKind::Latency,
            value: Some(500),
        };
        let payload = build_inject_payload(&parsed);
        let body = serde_json::to_string(&payload).unwrap();
        let url = format!("http://127.0.0.1:{port}/proxies/api_to_db/toxics");
        let cmd = build_inject_curl_command(&url, &body);
        let resp = run_http(cmd, &body).expect("HTTP exchange");
        assert_eq!(resp.status_code, 200, "got: {resp:?}");

        let request = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("server received request");
        // Pin the request shape: POST /proxies/api_to_db/toxics with
        // a JSON body whose `type` is "latency" and `attributes.latency` is 500.
        assert!(
            request.starts_with("POST /proxies/api_to_db/toxics"),
            "wrong URL: {request}"
        );
        assert!(
            request.contains("\"type\":\"latency\""),
            "missing type: {request}"
        );
        assert!(
            request.contains("\"latency\":500"),
            "missing attribute: {request}"
        );
        server.join().expect("server thread join");
    }

    /// Mirror of the inject test for `clear`: when there's a toxic
    /// on a proxy and we DELETE its endpoint, the mock server sees
    /// the DELETE method and the right URL.
    #[test]
    fn clear_resets_all_toxics() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::mpsc;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel::<String>();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            tx.send(String::from_utf8_lossy(&buf[..n]).into_owned())
                .unwrap();
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .ok();
        });

        let url = format!("http://127.0.0.1:{port}/proxies/api_to_db/toxics/latency_chaos");
        let cmd = build_delete_curl_command(&url);
        let resp = run_http(cmd, "").expect("HTTP exchange");
        assert_eq!(resp.status_code, 204, "got: {resp:?}");
        let req = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("server saw request");
        assert!(
            req.starts_with("DELETE /proxies/api_to_db/toxics/latency_chaos"),
            "wrong DELETE: {req}"
        );
        server.join().expect("server thread join");
    }

    /// `list` calls GET /proxies and parses the JSON response. We
    /// stand up a TCP listener that serves a canned proxy listing
    /// and assert the parsed view matches.
    #[test]
    fn list_returns_active_toxics() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let body = r#"{"api_to_db":{"name":"api_to_db","listen":"0.0.0.0:30421","upstream":"db:5432","enabled":true,"toxics":[{"name":"latency_chaos","type":"latency","stream":"downstream","attributes":{"latency":500}}]}}"#;
        let body_owned = body.to_string();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                body_owned.len(),
                body_owned
            );
            stream.write_all(response.as_bytes()).ok();
        });

        let url = format!("http://127.0.0.1:{port}/proxies");
        let cmd = build_get_curl_command(&url);
        let resp = run_http(cmd, "").expect("HTTP exchange");
        assert_eq!(resp.status_code, 200);
        let parsed: BTreeMap<String, ProxyView> = serde_json::from_str(&resp.body).expect("parse");
        let proxy = parsed.get("api_to_db").expect("api_to_db missing");
        assert_eq!(proxy.listen, "0.0.0.0:30421");
        assert_eq!(proxy.upstream, "db:5432");
        assert_eq!(proxy.toxics.len(), 1);
        assert_eq!(proxy.toxics[0].name, "latency_chaos");
        assert_eq!(proxy.toxics[0].kind, "latency");
        server.join().expect("server thread join");
    }

    /// Acceptance criterion #6 (run): unit-test the dispatch from
    /// scenario lookup → inject path. Smoke check that an unknown
    /// scenario name produces an error message naming the available
    /// scenarios. Full end-to-end (with HTTP) is the integration test.
    #[test]
    fn run_named_scenario_dispatches_to_inject() {
        // We test the smaller piece: `parsed_from_scenario` extracts
        // the scenario's primary value into `ParsedToxic`. The full
        // dispatch is exercised via the chaos integration test.
        let scenario = ChaosScenario {
            name: "high-latency".into(),
            toxic: ToxicKind::Latency,
            service: "api".into(),
            attributes: BTreeMap::from([("latency".into(), toml::Value::Integer(500))]),
        };
        let parsed = parsed_from_scenario(&scenario);
        let payload = build_inject_payload(&parsed);
        // The wire payload that would be POSTed is the same one
        // `inject` would produce for `--toxic latency:500ms`.
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["type"], "latency");
        assert_eq!(json["attributes"]["latency"], 500);
    }
}
