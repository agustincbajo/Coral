//! Loads prompt templates with priority: local override > embedded template > hardcoded fallback.
//!
//! The CLI subcommands ship hardcoded `const XXX_SYSTEM_FALLBACK` strings as a
//! last-resort safety net. The richer `template/prompts/*.md` files are
//! embedded into the binary via `include_dir` and used by default. Power users
//! can override any prompt by writing `<cwd>/prompts/<name>.md`.

use include_dir::{Dir, include_dir};
use std::path::PathBuf;

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
pub fn load_or_fallback(name: &str, fallback: &str) -> LoadedPrompt {
    // 1. Local override
    let local_path = PathBuf::from("prompts").join(format!("{name}.md"));
    if local_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&local_path) {
            return LoadedPrompt {
                name: name.to_string(),
                source: PromptSource::Local(local_path),
                content,
            };
        }
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
/// call `load_or_fallback`.
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
    KNOWN_PROMPTS
        .iter()
        .map(|n| load_or_fallback(n, ""))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Tests in this module mutate `current_dir`; serialize them.
    static CWD_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn load_uses_local_when_present() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        std::fs::create_dir("prompts").unwrap();
        std::fs::write("prompts/query.md", "LOCAL CONTENT").unwrap();

        let p = load_or_fallback("query", "fallback");
        assert_eq!(p.content, "LOCAL CONTENT");
        assert!(matches!(p.source, PromptSource::Local(_)));

        std::env::set_current_dir(prev).unwrap();
    }

    #[test]
    fn load_uses_embedded_when_local_missing() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let p = load_or_fallback("ingest", "fallback");
        // template/prompts/ingest.md exists embedded
        assert!(matches!(p.source, PromptSource::Embedded(_)));
        assert!(
            !p.content.is_empty(),
            "embedded ingest.md must not be empty"
        );

        std::env::set_current_dir(prev).unwrap();
    }

    #[test]
    fn load_falls_back_when_neither_present() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let p = load_or_fallback("nonexistent-prompt-xyz", "FALLBACK");
        assert!(matches!(p.source, PromptSource::Fallback));
        assert_eq!(p.content, "FALLBACK");

        std::env::set_current_dir(prev).unwrap();
    }

    #[test]
    fn list_prompts_returns_all_known() {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = TempDir::new().unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let prompts = list_prompts();
        let names: Vec<_> = prompts.iter().map(|p| p.name.clone()).collect();
        assert!(names.contains(&"ingest".to_string()));
        assert!(names.contains(&"query".to_string()));
        assert!(names.contains(&"bootstrap".to_string()));
        assert!(names.contains(&"consolidate".to_string()));
        assert!(names.contains(&"onboard".to_string()));
        assert!(names.contains(&"lint-semantic".to_string()));
        assert!(prompts.len() >= 5);

        std::env::set_current_dir(prev).unwrap();
    }
}
