use coral_runner::{ClaudeRunner, GeminiRunner, HttpRunner, LocalRunner, Runner};

/// Env var holding the chat-completions endpoint URL for `--provider http`.
const HTTP_ENDPOINT_ENV: &str = "CORAL_HTTP_ENDPOINT";
/// Env var holding the optional bearer token for `--provider http`.
const HTTP_API_KEY_ENV: &str = "CORAL_HTTP_API_KEY";

/// Names known by `--provider` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProviderName {
    #[default]
    Claude,
    Gemini,
    /// Local llama.cpp via the `llama-cli` binary. Set the model path with
    /// `--model /path/to/model.gguf` (or `prompt.model` programmatically).
    Local,
    /// Generic OpenAI-compatible HTTP endpoint (vLLM, Ollama, OpenAI,
    /// any local server). Reads endpoint URL from `CORAL_HTTP_ENDPOINT`
    /// and optional bearer token from `CORAL_HTTP_API_KEY`.
    Http,
}

impl std::str::FromStr for ProviderName {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "gemini" => Ok(Self::Gemini),
            "local" | "llama" | "llama.cpp" => Ok(Self::Local),
            "http" => Ok(Self::Http),
            other => Err(format!(
                "unknown provider: {other} (valid: claude, gemini, local, http)"
            )),
        }
    }
}

/// Read the HTTP endpoint URL from `CORAL_HTTP_ENDPOINT`. On failure
/// prints an actionable message to stderr and exits with code 2 — same
/// disposition as a clap usage error. Construction-time failure (rather
/// than failing inside [`Runner::run`]) is the right place because the
/// missing env var is purely a configuration / wiring issue, not a
/// per-prompt error.
fn endpoint_from_env() -> String {
    match std::env::var(HTTP_ENDPOINT_ENV) {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!(
                "error: --provider http requires {HTTP_ENDPOINT_ENV} to be set\n\
                 hint: export {HTTP_ENDPOINT_ENV}=http://localhost:8000/v1/chat/completions\n\
                 hint: optional auth via {HTTP_API_KEY_ENV}=<bearer-token>"
            );
            std::process::exit(2);
        }
    }
}

pub fn make_runner(provider: ProviderName) -> Box<dyn Runner> {
    match provider {
        ProviderName::Claude => Box::new(ClaudeRunner::new()),
        ProviderName::Gemini => Box::new(GeminiRunner::new()),
        ProviderName::Local => Box::new(LocalRunner::new()),
        ProviderName::Http => {
            let endpoint = endpoint_from_env();
            let mut runner = HttpRunner::new(endpoint);
            if let Ok(key) = std::env::var(HTTP_API_KEY_ENV) {
                if !key.is_empty() {
                    runner = runner.with_api_key(key);
                }
            }
            Box::new(runner)
        }
    }
}

/// v0.21.4: build a `Box<dyn Runner>` from a `provider` name string.
/// Used by `build_tiered_runner` to assemble per-tier runners from
/// the manifest's `[runner.tiered.*]` blocks. Returns the parser
/// error verbatim so the caller can wrap it with the offending tier
/// name. Construction-time validation (no network call) — surfaces
/// "unknown provider" at build, not at first run.
pub fn make_runner_for_provider_str(s: &str) -> Result<Box<dyn Runner>, String> {
    let p: ProviderName = s.parse()?;
    Ok(make_runner(p))
}

/// Resolve the provider from CLI flag → env var → default(claude).
pub fn resolve_provider(cli_flag: Option<&str>) -> Result<ProviderName, String> {
    if let Some(s) = cli_flag {
        return s.parse();
    }
    if let Ok(env) = std::env::var("CORAL_PROVIDER") {
        return env.parse();
    }
    Ok(ProviderName::Claude)
}

/// Constructs the default runner. Subcommands that need an LLM use this
/// when the test harness hasn't injected its own runner.
pub fn default_runner() -> Box<dyn Runner> {
    let p = resolve_provider(None).unwrap_or(ProviderName::Claude);
    make_runner(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Tests in this module mutate process-global env; serialize them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// v0.30.0 audit cycle 5 B11: RAII guard for a single env var.
    /// Captures the previous value on construction and restores it on
    /// `Drop`, so a panic between `set_var` and `remove_var` (or
    /// between two `set_var`s) never leaks state into the next test.
    ///
    /// The bare `set_var` / `remove_var` pair this replaces was already
    /// inside an `ENV_LOCK` critical section, so it was safe against
    /// concurrent tests; the failure mode it didn't cover was an
    /// `unwrap()` panicking AFTER `set_var` ran but BEFORE the matching
    /// `remove_var`, which would then leave the env var set for any
    /// later test that ran without holding the lock.
    ///
    /// `unsafe` is required because `std::env::set_var` /
    /// `std::env::remove_var` are `unsafe` in Rust 1.85+ (the MSRV
    /// gate on `rust-version.workspace`). Both are safe here because
    /// the caller holds `ENV_LOCK` for the lifetime of the guard.
    struct EnvVarGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            // SAFETY: caller serializes env mutation via `ENV_LOCK`.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: caller still holds `ENV_LOCK` for the lifetime of
            // the guard (the guard's scope is nested inside the lock's
            // scope at every call site).
            unsafe {
                match &self.prev {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn provider_name_parses_claude_and_gemini() {
        assert_eq!(
            "claude".parse::<ProviderName>().unwrap(),
            ProviderName::Claude
        );
        assert_eq!(
            "CLAUDE".parse::<ProviderName>().unwrap(),
            ProviderName::Claude
        );
        assert_eq!(
            "gemini".parse::<ProviderName>().unwrap(),
            ProviderName::Gemini
        );
    }

    #[test]
    fn provider_name_parses_local_aliases() {
        for s in ["local", "llama", "llama.cpp", "Local", "LLAMA"] {
            assert_eq!(s.parse::<ProviderName>().unwrap(), ProviderName::Local);
        }
    }

    #[test]
    fn provider_name_parses_http() {
        assert_eq!("http".parse::<ProviderName>().unwrap(), ProviderName::Http);
        assert_eq!("HTTP".parse::<ProviderName>().unwrap(), ProviderName::Http);
    }

    #[test]
    fn provider_name_rejects_unknown() {
        let err = "openai".parse::<ProviderName>().unwrap_err();
        assert!(err.contains("unknown provider"));
        assert!(err.contains("openai"));
    }

    #[test]
    fn provider_name_unknown_lists_http_in_valid_set() {
        let err = "voyage".parse::<ProviderName>().unwrap_err();
        assert!(err.contains("http"), "valid-set should mention http: {err}");
    }

    #[test]
    fn resolve_provider_prefers_cli_over_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // v0.30.0 audit cycle 5 B11: RAII guard restores `CORAL_PROVIDER`
        // on drop, even if `resolve_provider` panics. Pre-fix this was a
        // bare `set_var` / `remove_var` pair with `.unwrap()` between
        // them — a panic at the unwrap would leak `CORAL_PROVIDER=gemini`
        // into every later test in the process.
        let _env = EnvVarGuard::set("CORAL_PROVIDER", "gemini");
        let p = resolve_provider(Some("claude")).unwrap();
        assert_eq!(p, ProviderName::Claude);
    }

    /// v0.30.0 audit cycle 5 B11: regression test for the RAII guard.
    /// Setting a var, then dropping the guard, must restore the
    /// previous value (or absence). Covered without a real panic by
    /// observing the env state before / after a guarded scope.
    #[test]
    fn env_var_guard_restores_previous_value_on_drop() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        const KEY: &str = "CORAL_TEST_B11_GUARD";

        // Case 1: var was unset before; guard must leave it unset after.
        // SAFETY: serialized via ENV_LOCK.
        unsafe {
            std::env::remove_var(KEY);
        }
        {
            let _g = EnvVarGuard::set(KEY, "scoped-value");
            assert_eq!(std::env::var(KEY).ok().as_deref(), Some("scoped-value"));
        }
        assert!(
            std::env::var(KEY).is_err(),
            "guard must remove the var if it was unset before"
        );

        // Case 2: var had a value before; guard must restore it.
        // SAFETY: serialized via ENV_LOCK.
        unsafe {
            std::env::set_var(KEY, "original");
        }
        {
            let _g = EnvVarGuard::set(KEY, "scoped");
            assert_eq!(std::env::var(KEY).ok().as_deref(), Some("scoped"));
        }
        assert_eq!(
            std::env::var(KEY).ok().as_deref(),
            Some("original"),
            "guard must restore the prior value"
        );
        // SAFETY: serialized via ENV_LOCK.
        unsafe {
            std::env::remove_var(KEY);
        }
    }
}
