//! Loads prompt templates with priority: local override > embedded template > hardcoded fallback.
//!
//! The CLI subcommands ship hardcoded `const XXX_SYSTEM_FALLBACK` strings as a
//! last-resort safety net. The richer `template/prompts/*.md` files are
//! embedded into the binary via `include_dir` and used by default. Power users
//! can override any prompt by writing `<cwd>/prompts/<name>.md`.

use include_dir::{Dir, include_dir};
use std::path::{Path, PathBuf};

static EMBEDDED: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../template/prompts");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptSource {
    Local(PathBuf),
    Embedded(String), // path inside template/prompts/
    Fallback,         // none of the above; caller used the hardcoded const
}

#[derive(Debug, Clone)]
pub struct LoadedPrompt {
    pub name: String,
    pub source: PromptSource,
    pub content: String,
}

/// Try to load `<cwd>/prompts/<name>.md` first. If absent, fall back to the embedded
/// template at `template/prompts/<name>.md`. If that's also missing, return the
/// caller's `fallback` string and mark `PromptSource::Fallback`.
///
/// This is a thin wrapper over [`load_or_fallback_in`] that uses the process'
/// current working directory (`Path::new(".")`).
pub fn load_or_fallback(name: &str, fallback: &str) -> LoadedPrompt {
    load_or_fallback_in(Path::new("."), name, fallback)
}

/// Like [`load_or_fallback`], but resolves the local-override path relative to
/// `cwd` instead of the process' current working directory. Useful for tests
/// (no `set_current_dir` race) and for programmatic callers that want to
/// inspect prompt resolution against a specific repo root.
pub fn load_or_fallback_in(cwd: &Path, name: &str, fallback: &str) -> LoadedPrompt {
    // 1. Local override
    let local_path = cwd.join("prompts").join(format!("{name}.md"));
    if local_path.exists()
        && let Ok(content) = std::fs::read_to_string(&local_path)
    {
        return LoadedPrompt {
            name: name.to_string(),
            source: PromptSource::Local(local_path),
            content,
        };
    }

    // 2. Embedded
    let key = format!("{name}.md");
    if let Some(file) = EMBEDDED.get_file(&key)
        && let Some(content) = file.contents_utf8()
    {
        return LoadedPrompt {
            name: name.to_string(),
            source: PromptSource::Embedded(key),
            content: content.to_string(),
        };
    }

    // 3. Fallback
    LoadedPrompt {
        name: name.to_string(),
        source: PromptSource::Fallback,
        content: fallback.to_string(),
    }
}

/// Names of prompts known to v0.2. Kept in sync with the LLM commands that
/// call [`load_or_fallback`].
pub const KNOWN_PROMPTS: &[&str] = &[
    "bootstrap",
    "ingest",
    "query",
    "lint-semantic",
    "consolidate",
    "onboard",
];

/// Lists all known prompt names with their resolved source. The `content`
/// field reflects the actual loaded text (or empty if no fallback was supplied).
pub fn list_prompts() -> Vec<LoadedPrompt> {
    list_prompts_in(Path::new("."))
}

/// Like [`list_prompts`], but resolves overrides relative to `cwd`.
pub fn list_prompts_in(cwd: &Path) -> Vec<LoadedPrompt> {
    KNOWN_PROMPTS
        .iter()
        .map(|n| load_or_fallback_in(cwd, n, ""))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // Tests use `*_in` variants exclusively — no `current_dir` mutation, no
    // cross-binary race. Safe to run in parallel with any other test.

    #[test]
    fn load_uses_local_when_present() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join("prompts")).unwrap();
        std::fs::write(tmp.path().join("prompts/query.md"), "LOCAL CONTENT").unwrap();

        let p = load_or_fallback_in(tmp.path(), "query", "fallback");
        assert_eq!(p.content, "LOCAL CONTENT");
        assert!(matches!(p.source, PromptSource::Local(_)));
    }

    #[test]
    fn load_uses_embedded_when_local_missing() {
        let tmp = TempDir::new().unwrap();
        let p = load_or_fallback_in(tmp.path(), "ingest", "fallback");
        // template/prompts/ingest.md exists embedded
        assert!(matches!(p.source, PromptSource::Embedded(_)));
        assert!(
            !p.content.is_empty(),
            "embedded ingest.md must not be empty"
        );
    }

    #[test]
    fn load_falls_back_when_neither_present() {
        let tmp = TempDir::new().unwrap();
        let p = load_or_fallback_in(tmp.path(), "nonexistent-prompt-xyz", "FALLBACK");
        assert!(matches!(p.source, PromptSource::Fallback));
        assert_eq!(p.content, "FALLBACK");
    }

    #[test]
    fn list_prompts_returns_all_known() {
        let tmp = TempDir::new().unwrap();
        let prompts = list_prompts_in(tmp.path());
        let names: Vec<_> = prompts.iter().map(|p| p.name.clone()).collect();
        for expected in KNOWN_PROMPTS {
            assert!(
                names.contains(&expected.to_string()),
                "missing known prompt: {expected}"
            );
        }
        assert_eq!(prompts.len(), KNOWN_PROMPTS.len());
    }

    #[test]
    fn load_default_uses_process_cwd() {
        // Sanity: the `load_or_fallback` wrapper delegates to `_in(".")`.
        // For an embedded-only name with no local override at process CWD,
        // it must return Embedded.
        let p = load_or_fallback("query", "fallback");
        // Either Embedded (template/prompts/query.md exists) or Local (if the
        // test runner happens to have a prompts/query.md at the workspace root,
        // which we don't, but be flexible). Fallback would be a regression.
        assert!(
            matches!(p.source, PromptSource::Embedded(_) | PromptSource::Local(_)),
            "process-cwd loader returned Fallback unexpectedly"
        );
    }
}
