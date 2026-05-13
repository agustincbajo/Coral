//! `GET /api/v1/affected?since=<git-ref>` — list repos touched since a ref.
//!
//! Implementation strategy: shell out to `git -C <repo_root> log
//! --pretty=format: --name-only <since>..HEAD`, collect the unique
//! top-level directory of each changed path, then (if a `coral.toml`
//! lives at `repo_root`) intersect against the repos it declares. When
//! the git invocation fails or no `coral.toml` is present we fall back
//! to `["default"]` so the M1 single-repo case stays useful without
//! configuration.
//!
//! The `since` query parameter is **sanitized** before being passed to
//! `git` — we only accept `[A-Za-z0-9._/-]`, which covers branch
//! names, SHAs, tags, and `origin/main` style refs without exposing
//! us to shell-quoting bugs or option-injection (the leading `-` of
//! anything like `--all` would be rejected).

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::error::ApiError;
use crate::state::AppState;

pub fn handle(state: &Arc<AppState>, query_string: &str) -> Result<Vec<u8>, ApiError> {
    let since = query_string
        .split('&')
        .find_map(|p| p.strip_prefix("since="))
        .ok_or_else(|| ApiError::InvalidFilter("missing required query param: since".into()))?;
    if since.is_empty() {
        return Err(ApiError::InvalidFilter(
            "missing required query param: since".into(),
        ));
    }
    // Reject anything that looks like an option flag (leading `-`),
    // shell metacharacters, or whitespace. Allowed charset covers
    // SHAs, branch names, tags, `origin/main`, and the common
    // `HEAD~1` / `HEAD^` / `HEAD@{1}` revparse syntax.
    if since.starts_with('-') {
        return Err(ApiError::InvalidFilter(format!(
            "invalid git ref: {since:?} (must not start with '-')"
        )));
    }
    if !since.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(c, '.' | '_' | '/' | '-' | '~' | '^' | '@' | '{' | '}')
    }) {
        return Err(ApiError::InvalidFilter(format!(
            "invalid git ref: {since:?} (allowed: [A-Za-z0-9._/~^@{{}}-])"
        )));
    }

    let repo_root: std::path::PathBuf = state
        .wiki_root
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| state.wiki_root.clone());

    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .arg("log")
        .arg("--pretty=format:")
        .arg("--name-only")
        .arg(format!("{since}..HEAD"))
        .output();

    let affected: Vec<String> = match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let mut dirs: BTreeSet<String> = BTreeSet::new();
            for line in stdout.lines().filter(|l| !l.is_empty()) {
                if let Some(top) = line.split('/').next()
                    && !top.is_empty()
                {
                    dirs.insert(top.to_string());
                }
            }
            // Intersect with declared repos in coral.toml when present.
            let declared = declared_repos(&repo_root);
            if let Some(declared) = declared {
                dirs.into_iter()
                    .filter(|d| declared.contains(d))
                    .collect::<Vec<_>>()
            } else {
                dirs.into_iter().collect()
            }
        }
        _ => vec!["default".to_string()],
    };

    let total = affected.len();
    let body = serde_json::json!({
        "data": affected,
        "meta": {"total": total, "since": since}
    });
    serde_json::to_vec(&body).map_err(|e| anyhow::anyhow!(e).into())
}

/// Best-effort parse of `coral.toml` to extract repo names from
/// `[[repos]]` entries. Returns `None` if the file is missing or
/// unparseable — callers should fall back to "no filter". Intentionally
/// avoids depending on `coral_core::project::manifest::parse_toml`:
/// that parser is strict (will error on unknown api_version, missing
/// fields, etc.), but for the `affected` route we just need the names.
fn declared_repos(repo_root: &std::path::Path) -> Option<BTreeSet<String>> {
    let toml_path = repo_root.join("coral.toml");
    let text = std::fs::read_to_string(&toml_path).ok()?;
    let value: toml::Value = toml::from_str(&text).ok()?;
    let repos = value.get("repos")?.as_array()?;
    let mut out = BTreeSet::new();
    for entry in repos {
        if let Some(name) = entry.get("name").and_then(|v| v.as_str()) {
            out.insert(name.to_string());
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn state(root: PathBuf) -> Arc<AppState> {
        Arc::new(AppState {
            bind: "127.0.0.1".into(),
            port: 3838,
            wiki_root: root,
            token: None,
            allow_write_tools: false,
            runner: None,
        })
    }

    #[test]
    fn missing_since_param_rejected() {
        let s = state(PathBuf::from("/tmp/nope"));
        let err = handle(&s, "").unwrap_err();
        assert_eq!(err.code(), "INVALID_FILTER");
    }

    #[test]
    fn invalid_chars_in_ref_rejected() {
        let s = state(PathBuf::from("/tmp/nope"));
        // Single quote / semicolon would be classic shell-injection vectors.
        let err = handle(&s, "since=main;rm").unwrap_err();
        assert_eq!(err.code(), "INVALID_FILTER");
        let err = handle(&s, "since=$(whoami)").unwrap_err();
        assert_eq!(err.code(), "INVALID_FILTER");
        // Leading dash would be parsed as a `git log` option.
        let err = handle(&s, "since=--all").unwrap_err();
        assert_eq!(err.code(), "INVALID_FILTER");
    }

    #[test]
    fn non_repo_dir_falls_back_to_default() {
        // Point at a tmpdir that is not a git repo so git fails.
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        let s = state(wiki);
        let body = handle(&s, "since=HEAD~1").unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = v["data"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0], "default");
        assert_eq!(v["meta"]["since"], "HEAD~1");
    }

    #[test]
    fn declared_repos_reads_coral_toml() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("coral.toml"),
            r#"
api_version = "0.1"
name = "demo"

[[repos]]
name = "alpha"
remote = "github"

[[repos]]
name = "beta"
remote = "github"
"#,
        )
        .unwrap();
        let names = declared_repos(tmp.path()).expect("should parse");
        assert!(names.contains("alpha"));
        assert!(names.contains("beta"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn declared_repos_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        assert!(declared_repos(tmp.path()).is_none());
    }
}
