//! Hurl format support — a small subset of [hurl](https://hurl.dev) as
//! a second input format for `.coral/tests/*.hurl` files.
//!
//! Why a hand-rolled parser rather than the official `hurl` crate:
//! - the official crate transitively pulls libcurl FFI, which adds a C
//!   linker requirement that breaks `cargo install --locked` on some
//!   platforms (especially macOS Apple Silicon without curl headers);
//! - we already have a working HTTP executor (`UserDefinedRunner`'s
//!   `run_http`) that subprocesses curl, so all we need is a parser
//!   that translates Hurl syntax into our existing `YamlSuite` model.
//!
//! Supported subset (v0.18 wave 3b):
//! - `<METHOD> <URL>` request line (METHOD = GET/POST/PUT/DELETE/PATCH/HEAD/OPTIONS)
//! - `<Header>: <value>` after the request line
//! - `HTTP <status>` response assertion
//! - `[Asserts]` block with `jsonpath "$.x" exists` (translated to a
//!   `body_contains` check with the JSON path string).
//!
//! Anything else (captures, options, files) is reserved for v0.19 wave 1.

use crate::error::{TestError, TestResult};
use crate::spec::{TestCase, TestKind, TestSource, TestSpec};
use crate::user_defined_runner::{HttpExpect, HttpStep, YamlStep, YamlSuite};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Top-level entry: walk `.coral/tests/**` recursively for `*.hurl`
/// files, parse each, and return paired (TestCase, YamlSuite). The
/// case is `kind = UserDefined` so `UserDefinedRunner` runs it — no
/// extra runner needed.
///
/// **Recursive walk is critical** — see `user_defined_runner` for
/// the same reasoning (committed `coral test-discover --commit` files
/// land in `.coral/tests/discovered/`).
pub fn discover_hurl_tests(project_root: &Path) -> TestResult<Vec<(TestCase, YamlSuite)>> {
    let dir = project_root.join(".coral/tests");
    let paths =
        crate::walk_tests::walk_tests_recursive(project_root, &["hurl"]).map_err(|source| {
            TestError::Io {
                path: dir.clone(),
                source,
            }
        })?;
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let raw = std::fs::read_to_string(&path).map_err(|source| TestError::Io {
            path: path.clone(),
            source,
        })?;
        let suite = parse_hurl(&raw, &path)?;
        let case = TestCase {
            id: format!("hurl:{}", suite.name),
            name: suite.name.clone(),
            kind: TestKind::UserDefined,
            service: suite.service.clone(),
            tags: suite.tags.clone(),
            source: TestSource::File { path: path.clone() },
            spec: TestSpec(serde_json::to_value(&suite).unwrap_or(serde_json::Value::Null)),
        };
        out.push((case, suite));
    }
    Ok(out)
}

/// Hand-rolled minimal Hurl parser. Splits the document on blank
/// lines into request-blocks; each block is one HTTP step.
pub fn parse_hurl(raw: &str, source_path: &Path) -> TestResult<YamlSuite> {
    let blocks = split_blocks(raw);
    let mut steps = Vec::new();
    let mut suite_name = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("hurl")
        .to_string();
    let mut suite_service: Option<String> = None;
    let mut suite_tags = Vec::new();

    // Recognize a leading `# coral: name=foo service=bar tags=smoke,api`
    // directive to override defaults — mirrors the YAML suite's metadata
    // without forcing users to learn a second config syntax.
    for line in raw.lines().take(5) {
        if let Some(rest) = line.strip_prefix("# coral:") {
            for kv in rest.split_whitespace() {
                let mut parts = kv.splitn(2, '=');
                if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                    match k {
                        "name" => suite_name = v.to_string(),
                        "service" => suite_service = Some(v.to_string()),
                        "tags" => {
                            suite_tags = v.split(',').map(str::to_string).collect();
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    for (idx, block) in blocks.iter().enumerate() {
        if block.iter().all(|l| {
            let t = l.trim();
            t.is_empty() || t.starts_with('#')
        }) {
            continue;
        }
        let step = parse_block(block, idx, source_path)?;
        steps.push(YamlStep::Http(step));
    }

    if steps.is_empty() {
        return Err(TestError::InvalidSpec {
            path: source_path.to_path_buf(),
            reason: "no request blocks found in hurl file".into(),
        });
    }

    Ok(YamlSuite {
        name: suite_name,
        service: suite_service,
        tags: suite_tags,
        steps,
        retry: None,
    })
}

fn split_blocks(raw: &str) -> Vec<Vec<&str>> {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks
}

fn parse_block(lines: &[&str], idx: usize, source_path: &Path) -> TestResult<HttpStep> {
    // Strip leading comment-only lines.
    let useful: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    if useful.is_empty() {
        return Err(TestError::InvalidSpec {
            path: source_path.to_path_buf(),
            reason: format!("block {idx} is empty"),
        });
    }

    let request_line = useful[0].trim();
    let (method, path) =
        parse_request_line(request_line).ok_or_else(|| TestError::InvalidSpec {
            path: source_path.to_path_buf(),
            reason: format!("block {idx}: invalid request line: {request_line}"),
        })?;

    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    let mut expect_status: Option<u16> = None;
    let mut body_contains: Option<String> = None;

    let mut i = 1;
    while i < useful.len() {
        let raw_line = useful[i];
        let line = raw_line.trim();
        if line.is_empty() {
            i += 1;
            continue;
        }
        // HTTP <status> response line.
        if let Some(status_str) = line.strip_prefix("HTTP ") {
            if let Ok(s) = status_str.trim().parse::<u16>() {
                expect_status = Some(s);
            }
            i += 1;
            continue;
        }
        // [Asserts] section.
        if line.eq_ignore_ascii_case("[Asserts]") {
            i += 1;
            while i < useful.len() {
                let assert_line = useful[i].trim();
                if assert_line.is_empty() || assert_line.starts_with('[') {
                    break;
                }
                if let Some(needle) = parse_jsonpath_exists(assert_line) {
                    body_contains = Some(needle);
                }
                i += 1;
            }
            continue;
        }
        // Header: any line with `:` outside the assertion section
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_string();
            let value = line[colon + 1..].trim().to_string();
            if !key.is_empty() && !key.contains(' ') {
                headers.insert(key, value);
                i += 1;
                continue;
            }
        }
        // Anything else we don't recognize yet (body literal, options
        // section, captures) is ignored in this minimal parser.
        i += 1;
    }

    Ok(HttpStep {
        http: format!("{method} {path}"),
        headers,
        body: None,
        expect: HttpExpect {
            status: expect_status,
            body_contains,
            snapshot: None,
        },
        retry: None,
        capture: BTreeMap::new(),
        id: None,
        depends_on: Vec::new(),
    })
}

fn parse_request_line(line: &str) -> Option<(&'static str, String)> {
    let mut parts = line.splitn(2, ' ');
    let method = match parts.next()?.trim() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "DELETE" => "DELETE",
        "PATCH" => "PATCH",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => return None,
    };
    let raw_url = parts.next()?.trim();
    if raw_url.is_empty() {
        return None;
    }
    // Hurl URLs are full `http://host/path`; we only keep the path
    // segment because our HTTP executor concatenates `127.0.0.1:<port>`
    // at run time. If the URL has no `://`, treat the whole thing as
    // the path.
    let path = if let Some(after_scheme) = raw_url.split_once("://") {
        match after_scheme.1.find('/') {
            Some(slash) => after_scheme.1[slash..].to_string(),
            None => "/".to_string(),
        }
    } else {
        raw_url.to_string()
    };
    Some((method, path))
}

fn parse_jsonpath_exists(line: &str) -> Option<String> {
    // `jsonpath "$.x" exists` → produce a body_contains hint that
    // matches the leaf field name. Conservative but better than
    // nothing for v0.18 wave 3b.
    let line = line.trim();
    let prefix = line.strip_prefix("jsonpath")?.trim();
    let path_part = prefix.split('"').nth(1)?.trim();
    let leaf = path_part.rsplit('.').next()?.trim();
    if leaf.is_empty() {
        return None;
    }
    Some(format!("\"{leaf}\""))
}

/// `HurlRunner` is a thin wrapper over `UserDefinedRunner` — Hurl
/// files are parsed into the same `YamlSuite` representation, so they
/// share the executor. Exists as a separate type so the test
/// orchestration can `parallelism_hint()` differently if needed
/// later.
pub struct HurlRunner;

impl HurlRunner {
    pub fn discover(project_root: &Path) -> TestResult<Vec<(TestCase, YamlSuite)>> {
        discover_hurl_tests(project_root)
    }
}

#[allow(dead_code)]
fn _ensure_pathbuf_used(_p: PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parses_minimal_get() {
        let raw = r#"GET http://api.example.com/users
HTTP 200
"#;
        let suite = parse_hurl(raw, Path::new("/tmp/a.hurl")).unwrap();
        assert_eq!(suite.steps.len(), 1);
        match &suite.steps[0] {
            YamlStep::Http(s) => {
                assert_eq!(s.http, "GET /users");
                assert_eq!(s.expect.status, Some(200));
            }
            _ => panic!("expected Http step"),
        }
    }

    #[test]
    fn parses_request_with_headers() {
        let raw = r#"GET /users
Authorization: Bearer xyz
Accept: application/json
HTTP 200
"#;
        let suite = parse_hurl(raw, Path::new("/tmp/a.hurl")).unwrap();
        match &suite.steps[0] {
            YamlStep::Http(s) => {
                assert_eq!(s.headers.get("Authorization"), Some(&"Bearer xyz".into()));
                assert_eq!(s.headers.get("Accept"), Some(&"application/json".into()));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parses_jsonpath_assert_into_body_contains() {
        let raw = r#"GET /users
HTTP 200
[Asserts]
jsonpath "$.users" exists
"#;
        let suite = parse_hurl(raw, Path::new("/tmp/a.hurl")).unwrap();
        match &suite.steps[0] {
            YamlStep::Http(s) => {
                assert_eq!(s.expect.body_contains.as_deref(), Some("\"users\""));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parses_multi_block_suite() {
        let raw = r#"GET /users
HTTP 200

POST /users
Authorization: Bearer xyz
HTTP 201
"#;
        let suite = parse_hurl(raw, Path::new("/tmp/a.hurl")).unwrap();
        assert_eq!(suite.steps.len(), 2);
    }

    #[test]
    fn coral_directive_overrides_metadata() {
        let raw = r#"# coral: name=api-smoke service=api tags=smoke,api
GET /health
HTTP 200
"#;
        let suite = parse_hurl(raw, Path::new("/tmp/a.hurl")).unwrap();
        assert_eq!(suite.name, "api-smoke");
        assert_eq!(suite.service.as_deref(), Some("api"));
        assert!(suite.tags.contains(&"smoke".to_string()));
        assert!(suite.tags.contains(&"api".to_string()));
    }

    #[test]
    fn rejects_empty_hurl_file() {
        let err = parse_hurl("# just comments\n", Path::new("/tmp/a.hurl"));
        assert!(err.is_err());
    }

    #[test]
    fn discover_returns_empty_when_dir_missing() {
        let dir = TempDir::new().unwrap();
        let pairs = discover_hurl_tests(dir.path()).unwrap();
        assert!(pairs.is_empty());
    }

    #[test]
    fn discover_finds_hurl_files() {
        let dir = TempDir::new().unwrap();
        let tests_dir = dir.path().join(".coral/tests");
        std::fs::create_dir_all(&tests_dir).unwrap();
        std::fs::write(
            tests_dir.join("smoke.hurl"),
            r#"GET /users
HTTP 200
"#,
        )
        .unwrap();
        // Non-hurl file should be ignored.
        std::fs::write(tests_dir.join("README.md"), "ignore me\n").unwrap();
        let pairs = discover_hurl_tests(dir.path()).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0.kind, TestKind::UserDefined);
    }
}
