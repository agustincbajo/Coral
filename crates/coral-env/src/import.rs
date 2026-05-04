//! `coral env import` — convert an existing `docker-compose.yml` into a
//! `coral.toml` `[[environments]]` block.
//!
//! v0.19.7 ships this as an onboarding accelerator: users coming from a
//! plain Docker Compose workflow can run `coral env import compose.yml`
//! and get a starter `[[environments]]` table they can paste into
//! `coral.toml`, instead of authoring it by hand. The output is
//! deliberately conservative — only the fields that round-trip cleanly
//! through `EnvironmentSpec` are emitted; anything the converter
//! doesn't fully understand surfaces as a `# TODO` comment so the user
//! sees the gap.
//!
//! ## Scope
//!
//! Supported subset of compose v2/v3:
//! - `services.<name>.image` → `[environments.services.<name>] kind = "real", image = ...`.
//! - `services.<name>.build` → `[environments.services.<name>.build]` (context, dockerfile, target, args).
//! - `services.<name>.ports` (the short `"3000:3000"` and `"3000"` forms) → `ports = [...]`.
//! - `services.<name>.environment` (map form) → `env = { ... }`.
//! - `services.<name>.depends_on` (list form) → `depends_on = [...]`.
//! - `services.<name>.healthcheck.test` → `[environments.services.<name>.healthcheck]` with
//!   `kind = "exec"` (CMD form) or `kind = "http"` heuristically inferred from a `curl` test.
//!
//! ## Out of scope (emitted as TODO)
//!
//! - Compose extends, profiles, secrets, configs, volumes, networks.
//! - Long-form `depends_on: { service_x: { condition: ... } }`.
//! - `environment` as a list (`["FOO=bar"]`); we accept the map form
//!   only to keep the parser simple — the list form is rare in modern
//!   compose files.
//! - Anything in `services.<name>` we don't recognize.
//!
//! Importer is **purely advisory**. The user is expected to review the
//! output and tweak it. The CLI prints a banner reminding them.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level compose-yaml shape we read.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeFile {
    /// Compose v2 dropped the top-level `version`; v3 still has it.
    /// We accept either by ignoring the field.
    #[serde(default)]
    pub version: Option<serde_yaml_ng::Value>,
    #[serde(default)]
    pub services: BTreeMap<String, ComposeService>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposeService {
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub build: Option<ComposeBuild>,
    #[serde(default)]
    pub ports: Vec<serde_yaml_ng::Value>,
    #[serde(default)]
    pub environment: Option<serde_yaml_ng::Value>,
    #[serde(default)]
    pub depends_on: Option<serde_yaml_ng::Value>,
    #[serde(default)]
    pub healthcheck: Option<ComposeHealthcheck>,
    /// Catch-all so we can surface unknown fields as TODOs without
    /// rejecting the whole file.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml_ng::Value>,
}

/// Compose `build:` accepts either a string (= context path) or a map.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ComposeBuild {
    Shorthand(String),
    Long {
        #[serde(default)]
        context: Option<String>,
        #[serde(default)]
        dockerfile: Option<String>,
        #[serde(default)]
        target: Option<String>,
        #[serde(default)]
        args: BTreeMap<String, serde_yaml_ng::Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeHealthcheck {
    /// `test:` is `["CMD", "curl", "-f", ...]` or `["CMD-SHELL", "..."]`
    /// or a bare string. Capture as untyped Value.
    #[serde(default)]
    pub test: Option<serde_yaml_ng::Value>,
    #[serde(default)]
    pub interval: Option<String>,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub retries: Option<u32>,
    #[serde(default)]
    pub start_period: Option<String>,
}

/// The converter's output. The caller writes `toml` to disk (or
/// stdout). `warnings` lists fields we couldn't translate; the CLI
/// prints them so the user knows what to review.
#[derive(Debug)]
pub struct ImportResult {
    pub toml: String,
    pub warnings: Vec<String>,
}

/// Parse `compose_yaml` and emit a `coral.toml` `[[environments]]` block
/// (env name + service tables) the user can paste into their manifest.
///
/// `env_name` is the value for `name = ...` in the emitted block —
/// typically `"dev"`. Validated against `coral_core::slug` rules so
/// the output round-trips through `Project::validate()`.
pub fn import_compose_to_toml(compose_yaml: &str, env_name: &str) -> Result<ImportResult, String> {
    if !is_safe_env_name(env_name) {
        return Err(format!(
            "invalid env name '{env_name}': must match [a-zA-Z0-9_-], no leading dot/dash"
        ));
    }
    let compose: ComposeFile = serde_yaml_ng::from_str(compose_yaml)
        .map_err(|e| format!("failed to parse compose YAML: {e}"))?;
    let mut toml = String::new();
    let mut warnings: Vec<String> = Vec::new();

    toml.push_str("# Generated by `coral env import`. Review before committing.\n");
    toml.push_str(
        "# Anything Coral couldn't translate cleanly appears as a `# TODO:` comment.\n\n",
    );
    toml.push_str("[[environments]]\n");
    toml.push_str(&format!("name            = \"{env_name}\"\n"));
    toml.push_str("backend         = \"compose\"\n");
    toml.push_str("mode            = \"managed\"\n");
    toml.push_str("compose_command = \"auto\"\n");
    toml.push_str("production      = false\n\n");

    if compose.services.is_empty() {
        warnings.push("compose file declares no services; emitted env block is empty".into());
    }

    for (svc_name, svc) in &compose.services {
        if !is_safe_service_name(svc_name) {
            warnings.push(format!(
                "service `{svc_name}`: name has characters Coral doesn't allow ([a-zA-Z0-9_-]); skipped"
            ));
            continue;
        }
        emit_service(&mut toml, &mut warnings, svc_name, svc);
    }

    Ok(ImportResult { toml, warnings })
}

fn emit_service(out: &mut String, warnings: &mut Vec<String>, name: &str, svc: &ComposeService) {
    out.push_str(&format!("[environments.services.{name}]\n"));
    out.push_str("kind = \"real\"\n");

    if let Some(image) = &svc.image {
        out.push_str(&format!("image = \"{}\"\n", escape_toml_string(image)));
    }

    let ports = parse_ports(&svc.ports, name, warnings);
    if !ports.is_empty() {
        let list: Vec<String> = ports.iter().map(|p| p.to_string()).collect();
        out.push_str(&format!("ports = [{}]\n", list.join(", ")));
    }

    let env_map = parse_environment(&svc.environment, name, warnings);
    if !env_map.is_empty() {
        out.push_str("env = { ");
        let items: Vec<String> = env_map
            .iter()
            .map(|(k, v)| format!("{k} = \"{}\"", escape_toml_string(v)))
            .collect();
        out.push_str(&items.join(", "));
        out.push_str(" }\n");
    }

    let deps = parse_depends_on(&svc.depends_on, name, warnings);
    if !deps.is_empty() {
        let list: Vec<String> = deps.iter().map(|d| format!("\"{d}\"")).collect();
        out.push_str(&format!("depends_on = [{}]\n", list.join(", ")));
    }

    // Surface unknown / unsupported fields as TODOs in the output.
    for k in svc.extra.keys() {
        if matches!(
            k.as_str(),
            "container_name" | "restart" | "command" | "entrypoint" | "user" | "working_dir"
        ) {
            warnings.push(format!(
                "service `{name}`: field `{k}` is not yet wired in coral env spec; left as TODO"
            ));
            out.push_str(&format!(
                "# TODO: compose field `{k}` is not yet supported by coral env\n"
            ));
        } else {
            warnings.push(format!(
                "service `{name}`: unknown field `{k}`; left as TODO"
            ));
            out.push_str(&format!("# TODO: unknown compose field `{k}`\n"));
        }
    }
    out.push('\n');

    if let Some(build) = &svc.build {
        emit_build(out, name, build);
    }

    if let Some(hc) = &svc.healthcheck {
        emit_healthcheck(out, warnings, name, hc);
    }
}

fn emit_build(out: &mut String, svc_name: &str, build: &ComposeBuild) {
    out.push_str(&format!("[environments.services.{svc_name}.build]\n"));
    match build {
        ComposeBuild::Shorthand(ctx) => {
            out.push_str(&format!("context = \"{}\"\n", escape_toml_string(ctx)));
        }
        ComposeBuild::Long {
            context,
            dockerfile,
            target,
            args,
        } => {
            if let Some(ctx) = context {
                out.push_str(&format!("context = \"{}\"\n", escape_toml_string(ctx)));
            }
            if let Some(df) = dockerfile {
                out.push_str(&format!("dockerfile = \"{}\"\n", escape_toml_string(df)));
            }
            if let Some(tg) = target {
                out.push_str(&format!("target = \"{}\"\n", escape_toml_string(tg)));
            }
            if !args.is_empty() {
                out.push_str("args = { ");
                let items: Vec<String> = args
                    .iter()
                    .filter_map(|(k, v)| {
                        v.as_str()
                            .map(|s| format!("{k} = \"{}\"", escape_toml_string(s)))
                    })
                    .collect();
                out.push_str(&items.join(", "));
                out.push_str(" }\n");
            }
        }
    }
    out.push('\n');
}

fn emit_healthcheck(
    out: &mut String,
    warnings: &mut Vec<String>,
    svc_name: &str,
    hc: &ComposeHealthcheck,
) {
    out.push_str(&format!("[environments.services.{svc_name}.healthcheck]\n"));
    match parse_healthcheck_test(&hc.test) {
        ParsedHealthcheckTest::Http {
            path,
            expect_status,
        } => {
            out.push_str("kind = \"http\"\n");
            out.push_str(&format!("path = \"{}\"\n", escape_toml_string(&path)));
            out.push_str(&format!("expect_status = {expect_status}\n"));
        }
        ParsedHealthcheckTest::Exec { cmd } => {
            out.push_str("kind = \"exec\"\n");
            let items: Vec<String> = cmd
                .iter()
                .map(|c| format!("\"{}\"", escape_toml_string(c)))
                .collect();
            out.push_str(&format!("cmd = [{}]\n", items.join(", ")));
        }
        ParsedHealthcheckTest::Unknown => {
            warnings.push(format!(
                "service `{svc_name}`: healthcheck test couldn't be parsed; emitted as TODO"
            ));
            out.push_str("# TODO: original `test:` was not in CMD / CMD-SHELL form\n");
            out.push_str("kind = \"exec\"\n");
            out.push_str("cmd = [\"true\"]\n");
        }
    }

    // Timing — compose strings like "5s", "30s" → seconds integers.
    let interval_s = parse_compose_duration(&hc.interval).unwrap_or(5);
    let timeout_s = parse_compose_duration(&hc.timeout).unwrap_or(3);
    let start_period_s = parse_compose_duration(&hc.start_period).unwrap_or(0);
    let retries = hc.retries.unwrap_or(3);

    out.push_str(&format!(
        "[environments.services.{svc_name}.healthcheck.timing]\n"
    ));
    out.push_str(&format!("interval_s     = {interval_s}\n"));
    out.push_str(&format!("timeout_s      = {timeout_s}\n"));
    out.push_str(&format!("retries        = {retries}\n"));
    out.push_str(&format!("start_period_s = {start_period_s}\n"));
    out.push('\n');
}

enum ParsedHealthcheckTest {
    Http { path: String, expect_status: u16 },
    Exec { cmd: Vec<String> },
    Unknown,
}

/// Best-effort heuristic. Compose `test:` is a sequence; the first
/// element is `"CMD"` (literal exec) or `"CMD-SHELL"` (one-string
/// shell). For `CMD ["curl", "-f", "http://localhost:8080/health"]`
/// we infer `kind = "http"` and pull the path. Anything else lands
/// as `kind = "exec"`.
fn parse_healthcheck_test(test: &Option<serde_yaml_ng::Value>) -> ParsedHealthcheckTest {
    let seq = match test {
        Some(serde_yaml_ng::Value::Sequence(s)) => s,
        _ => return ParsedHealthcheckTest::Unknown,
    };
    if seq.is_empty() {
        return ParsedHealthcheckTest::Unknown;
    }
    let first = match seq[0].as_str() {
        Some(s) => s,
        None => return ParsedHealthcheckTest::Unknown,
    };
    let rest: Vec<String> = seq[1..]
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    match first {
        "CMD" => {
            // Look for curl-with-URL or wget-with-URL → infer HTTP.
            if let Some(http) = infer_http_from_cmd(&rest) {
                return http;
            }
            ParsedHealthcheckTest::Exec { cmd: rest }
        }
        "CMD-SHELL" => {
            // Single shell string. We don't pull a path out of arbitrary
            // shell — emit as exec with `sh -c <line>`.
            if let Some(line) = rest.first() {
                ParsedHealthcheckTest::Exec {
                    cmd: vec!["sh".into(), "-c".into(), line.clone()],
                }
            } else {
                ParsedHealthcheckTest::Unknown
            }
        }
        _ => ParsedHealthcheckTest::Unknown,
    }
}

fn infer_http_from_cmd(args: &[String]) -> Option<ParsedHealthcheckTest> {
    let exe = args.first()?.as_str();
    if !matches!(exe, "curl" | "wget") {
        return None;
    }
    // Find the first argv entry that looks like an HTTP URL.
    for arg in args.iter().skip(1) {
        if let Some(rest) = arg.strip_prefix("http://") {
            // "host:port/path" → strip host:port to get the path.
            let path = rest.split_once('/').map(|(_, p)| format!("/{p}"));
            if let Some(p) = path {
                return Some(ParsedHealthcheckTest::Http {
                    path: p,
                    expect_status: 200,
                });
            }
        }
        if let Some(rest) = arg.strip_prefix("https://") {
            let path = rest.split_once('/').map(|(_, p)| format!("/{p}"));
            if let Some(p) = path {
                return Some(ParsedHealthcheckTest::Http {
                    path: p,
                    expect_status: 200,
                });
            }
        }
    }
    None
}

/// Compose duration strings: `5s`, `30s`, `1m`, `1m30s`. Returns
/// seconds. Unknown → None. A bare number (`"90"`) is accepted as
/// seconds for forwards compat with users hand-typing the value.
fn parse_compose_duration(s: &Option<String>) -> Option<u32> {
    let s = s.as_deref()?.trim();
    if s.is_empty() {
        return None;
    }
    let mut total: u32 = 0;
    let mut buf = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            buf.push(ch);
        } else if !buf.is_empty() {
            let n: u32 = buf.parse().ok()?;
            buf.clear();
            match ch {
                's' => total = total.checked_add(n)?,
                'm' => total = total.checked_add(n.checked_mul(60)?)?,
                'h' => total = total.checked_add(n.checked_mul(3600)?)?,
                _ => return None,
            }
        } else {
            // Non-digit before any digits accumulated → garbage.
            return None;
        }
    }
    if !buf.is_empty() {
        // Trailing digits without a unit — assume seconds.
        let n: u32 = buf.parse().ok()?;
        total = total.checked_add(n)?;
    }
    Some(total)
}

fn parse_ports(
    ports: &[serde_yaml_ng::Value],
    svc_name: &str,
    warnings: &mut Vec<String>,
) -> Vec<u16> {
    let mut out = Vec::new();
    for p in ports {
        match p {
            serde_yaml_ng::Value::Number(n) => {
                if let Some(u) = n.as_u64().and_then(|u| u16::try_from(u).ok()) {
                    out.push(u);
                }
            }
            serde_yaml_ng::Value::String(s) => {
                // Forms: "3000", "3000:3000", "127.0.0.1:3000:3000", "3000-3010:3000-3010".
                // Coral spec only takes a u16 (the container port). For a "host:container" form
                // we want the container port (the right-hand half).
                if s.contains('-') {
                    warnings.push(format!(
                        "service `{svc_name}`: port range '{s}' not supported in coral env spec; skipped"
                    ));
                    continue;
                }
                let last = s.rsplit_once(':').map(|(_, c)| c).unwrap_or(s.as_str());
                if let Ok(u) = last.parse::<u16>() {
                    out.push(u);
                } else {
                    warnings.push(format!(
                        "service `{svc_name}`: port '{s}' couldn't be parsed; skipped"
                    ));
                }
            }
            _ => {
                warnings.push(format!(
                    "service `{svc_name}`: skipped unrecognized port entry"
                ));
            }
        }
    }
    out
}

fn parse_environment(
    env: &Option<serde_yaml_ng::Value>,
    svc_name: &str,
    warnings: &mut Vec<String>,
) -> BTreeMap<String, String> {
    match env {
        None => BTreeMap::new(),
        Some(serde_yaml_ng::Value::Mapping(m)) => {
            let mut out = BTreeMap::new();
            for (k, v) in m {
                let key = match k.as_str() {
                    Some(s) => s.to_string(),
                    None => continue,
                };
                let val = match v {
                    serde_yaml_ng::Value::String(s) => s.clone(),
                    serde_yaml_ng::Value::Number(n) => n.to_string(),
                    serde_yaml_ng::Value::Bool(b) => b.to_string(),
                    _ => {
                        warnings.push(format!(
                            "service `{svc_name}`: env var `{key}` has non-scalar value; skipped"
                        ));
                        continue;
                    }
                };
                out.insert(key, val);
            }
            out
        }
        Some(_) => {
            warnings.push(format!(
                "service `{svc_name}`: `environment` is in list form (`[\"FOO=bar\"]`); only map form is supported, skipped"
            ));
            BTreeMap::new()
        }
    }
}

fn parse_depends_on(
    deps: &Option<serde_yaml_ng::Value>,
    svc_name: &str,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    match deps {
        None => Vec::new(),
        Some(serde_yaml_ng::Value::Sequence(s)) => s
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        Some(serde_yaml_ng::Value::Mapping(_)) => {
            warnings.push(format!(
                "service `{svc_name}`: long-form `depends_on: {{ … }}` mapping not supported; left empty (use list form for now)"
            ));
            Vec::new()
        }
        Some(_) => {
            warnings.push(format!(
                "service `{svc_name}`: `depends_on` has unrecognized shape; left empty"
            ));
            Vec::new()
        }
    }
}

/// Allowlist for env / service names — same shape as
/// `coral_core::slug::is_safe_repo_name` but exposed locally so we
/// don't have to add a coral-core dep just for the check (the env
/// crate already depends on coral-core, so we DO use it).
fn is_safe_env_name(s: &str) -> bool {
    coral_core::slug::is_safe_repo_name(s)
}

fn is_safe_service_name(s: &str) -> bool {
    coral_core::slug::is_safe_repo_name(s)
}

/// Conservative TOML string escape — covers `\\` and `"` which is
/// enough for the values we emit (image tags, paths, env values). Not
/// a general TOML emitter; we only need to escape strings inside
/// already-double-quoted scalars.
fn escape_toml_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_compose_emits_skeleton() {
        let result = import_compose_to_toml("services: {}\n", "dev").unwrap();
        assert!(result.toml.contains("[[environments]]"));
        assert!(result.toml.contains("name            = \"dev\""));
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("declares no services"))
        );
    }

    #[test]
    fn imports_image_only_service() {
        let yaml = r#"
services:
  db:
    image: postgres:16
    ports: ["5432:5432"]
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(r.toml.contains("[environments.services.db]"));
        assert!(r.toml.contains("image = \"postgres:16\""));
        assert!(r.toml.contains("ports = [5432]"));
        assert!(r.warnings.is_empty(), "got warnings: {:?}", r.warnings);
    }

    #[test]
    fn imports_build_long_form_with_target() {
        let yaml = r#"
services:
  api:
    build:
      context: ./api
      dockerfile: Dockerfile
      target: dev
      args:
        GOFLAGS: -mod=vendor
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(r.toml.contains("[environments.services.api.build]"));
        assert!(r.toml.contains("context = \"./api\""));
        assert!(r.toml.contains("dockerfile = \"Dockerfile\""));
        assert!(r.toml.contains("target = \"dev\""));
        assert!(r.toml.contains("GOFLAGS = \"-mod=vendor\""));
    }

    #[test]
    fn imports_build_shorthand() {
        let yaml = r#"
services:
  api:
    build: .
    image: api:dev
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(r.toml.contains("[environments.services.api.build]"));
        assert!(r.toml.contains("context = \".\""));
    }

    #[test]
    fn imports_environment_map_form() {
        let yaml = r#"
services:
  api:
    image: api:1
    environment:
      DATABASE_URL: postgres://db:5432/app
      DEBUG: "true"
      PORT: 3000
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(r.toml.contains("DATABASE_URL = \"postgres://db:5432/app\""));
        assert!(r.toml.contains("DEBUG = \"true\""));
        assert!(r.toml.contains("PORT = \"3000\""));
    }

    #[test]
    fn imports_depends_on_list_form() {
        let yaml = r#"
services:
  api:
    image: api:1
    depends_on:
      - db
      - cache
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(r.toml.contains("depends_on = [\"db\", \"cache\"]"));
    }

    #[test]
    fn warns_on_depends_on_long_form() {
        let yaml = r#"
services:
  api:
    image: api:1
    depends_on:
      db:
        condition: service_healthy
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(
            r.warnings
                .iter()
                .any(|w| w.contains("long-form `depends_on")),
            "warnings: {:?}",
            r.warnings
        );
        // We don't emit a depends_on field in this case.
        assert!(!r.toml.contains("depends_on ="));
    }

    #[test]
    fn warns_on_environment_list_form() {
        let yaml = r#"
services:
  api:
    image: api:1
    environment:
      - DEBUG=true
      - PORT=3000
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(
            r.warnings.iter().any(|w| w.contains("list form")),
            "warnings: {:?}",
            r.warnings
        );
    }

    #[test]
    fn imports_http_healthcheck_via_curl() {
        let yaml = r#"
services:
  api:
    image: api:1
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 5s
      timeout: 3s
      retries: 5
      start_period: 30s
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(r.toml.contains("[environments.services.api.healthcheck]"));
        assert!(r.toml.contains("kind = \"http\""));
        assert!(r.toml.contains("path = \"/health\""));
        assert!(r.toml.contains("expect_status = 200"));
        assert!(r.toml.contains("interval_s     = 5"));
        assert!(r.toml.contains("retries        = 5"));
        assert!(r.toml.contains("start_period_s = 30"));
    }

    #[test]
    fn imports_exec_healthcheck_for_pg_isready() {
        let yaml = r#"
services:
  db:
    image: postgres:16
    healthcheck:
      test: ["CMD", "pg_isready", "-U", "postgres"]
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(r.toml.contains("kind = \"exec\""));
        assert!(
            r.toml
                .contains("cmd = [\"pg_isready\", \"-U\", \"postgres\"]")
        );
    }

    #[test]
    fn imports_cmd_shell_healthcheck_via_sh_c() {
        let yaml = r#"
services:
  api:
    image: api:1
    healthcheck:
      test: ["CMD-SHELL", "test -f /tmp/ready"]
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(r.toml.contains("kind = \"exec\""));
        assert!(
            r.toml
                .contains("cmd = [\"sh\", \"-c\", \"test -f /tmp/ready\"]")
        );
    }

    #[test]
    fn parse_compose_duration_handles_seconds_minutes_hours() {
        assert_eq!(parse_compose_duration(&Some("5s".into())), Some(5));
        assert_eq!(parse_compose_duration(&Some("1m".into())), Some(60));
        assert_eq!(parse_compose_duration(&Some("1m30s".into())), Some(90));
        assert_eq!(parse_compose_duration(&Some("2h".into())), Some(7200));
        assert_eq!(parse_compose_duration(&Some("90".into())), Some(90));
        assert_eq!(parse_compose_duration(&Some("garbage".into())), None);
        assert_eq!(parse_compose_duration(&None), None);
    }

    #[test]
    fn unrecognized_compose_field_surfaces_as_todo() {
        let yaml = r#"
services:
  api:
    image: api:1
    restart: always
    user: "1000:1000"
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(r.toml.contains("# TODO: compose field `restart`"));
        assert!(r.toml.contains("# TODO: compose field `user`"));
    }

    #[test]
    fn rejects_unsafe_env_name() {
        let err = import_compose_to_toml("services: {}\n", "../escape").unwrap_err();
        assert!(err.contains("invalid env name"));
    }

    #[test]
    fn skips_service_with_unsafe_name() {
        let yaml = r#"
services:
  "../escape":
    image: evil:1
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        assert!(!r.toml.contains("../escape"));
        assert!(
            r.warnings
                .iter()
                .any(|w| w.contains("characters Coral doesn't allow"))
        );
    }

    #[test]
    fn output_round_trips_through_environment_spec() {
        // Pin: the emitted TOML, when wrapped in a minimal `coral.toml`,
        // deserializes through `EnvironmentSpec` without errors. This is
        // the same shape `coral up` would consume at runtime.
        let yaml = r#"
services:
  api:
    image: api:1
    ports: ["3000:3000"]
    environment:
      DEBUG: "true"
    depends_on: ["db"]
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 5s
      retries: 5
      start_period: 30s
  db:
    image: postgres:16
    ports: [5432]
    healthcheck:
      test: ["CMD", "pg_isready"]
"#;
        let r = import_compose_to_toml(yaml, "dev").unwrap();
        let manifest = format!(
            "apiVersion = \"coral.dev/v1\"\n\
             [project]\n\
             name = \"test\"\n\
             [[repos]]\n\
             name = \"api\"\n\
             url = \"git@example.com/api.git\"\n\
             [[repos]]\n\
             name = \"db\"\n\
             url = \"git@example.com/db.git\"\n\n{}",
            r.toml
        );
        let parsed: toml::Value =
            toml::from_str(&manifest).expect("emitted TOML must parse standalone");
        let envs = parsed
            .get("environments")
            .and_then(|v| v.as_array())
            .expect("environments array");
        assert_eq!(envs.len(), 1);
        // Deserialize the array entry through to EnvironmentSpec.
        let raw = envs[0].clone();
        let spec: crate::EnvironmentSpec = raw
            .try_into()
            .expect("emitted env block must round-trip through EnvironmentSpec");
        assert_eq!(spec.name, "dev");
        assert_eq!(spec.services.len(), 2);
        assert!(spec.services.contains_key("api"));
        assert!(spec.services.contains_key("db"));
    }
}
