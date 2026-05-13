use coral_core::config::{CredentialProvider, resolve_provider_credentials};
use coral_runner::{ClaudeRunner, GeminiRunner, HttpRunner, LocalRunner, Runner};
use std::path::Path;

/// Env var holding the chat-completions endpoint URL for `--provider http`.
const HTTP_ENDPOINT_ENV: &str = "CORAL_HTTP_ENDPOINT";
/// Env var holding the optional bearer token for `--provider http`.
const HTTP_API_KEY_ENV: &str = "CORAL_HTTP_API_KEY";

/// Env var the `claude` CLI consults for direct-API auth (bypasses
/// `claude setup-token`). When `[provider.anthropic].api_key` is set
/// in `.coral/config.toml`, we forward it under this name.
const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
/// Env var the `gemini` CLI consults for Google AI Studio auth. When
/// `[provider.gemini].api_key` is set, we forward it under this name.
const GEMINI_API_KEY_ENV: &str = "GEMINI_API_KEY";

/// OpenAI-compatible chat-completions path Ollama serves at since the
/// 2024-02 compat-shim release. `[provider.ollama].endpoint` in the
/// wizard-written `.coral/config.toml` is just the server root, so we
/// append this when bridging to `HttpRunner`.
const OLLAMA_OPENAI_CHAT_PATH: &str = "/v1/chat/completions";

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
    /// any local server). v0.34.x resolution order:
    ///   1. `[provider.ollama]` in `<cwd>/.coral/config.toml`
    ///      (written by `coral doctor --wizard`). The endpoint there
    ///      is the bare server root; the runner appends
    ///      `/v1/chat/completions` automatically.
    ///   2. `CORAL_HTTP_ENDPOINT` env var (legacy v0.33 path, kept
    ///      forever for BC).
    /// Optional bearer token: `CORAL_HTTP_API_KEY` (applies to either
    /// source — the schema has no `api_key` on Ollama today, but a
    /// user can still attach one via env for proxy auth).
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

/// Resolved settings for an HTTP-flavoured runner: the full
/// chat-completions URL and an optional bearer token.
///
/// This is the structured output of [`resolve_http_endpoint`], which is
/// the bridge between `.coral/config.toml`'s `[provider.ollama]`
/// section (written by `coral doctor --wizard`) and the legacy
/// `CORAL_HTTP_ENDPOINT` env-var path. v0.34.x: the wizard writes the
/// config and the runner reads it — closing the loop that v0.33 left
/// open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HttpResolution {
    /// Full URL passed to `HttpRunner::new`. For Ollama bridges this
    /// is `<endpoint>/v1/chat/completions` (the OpenAI-compat path);
    /// for env-var bridges it is whatever the user exported verbatim.
    pub endpoint: String,
    /// Optional bearer token attached via `HttpRunner::with_api_key`.
    /// Ollama in default install has no auth — `None` is normal.
    pub api_key: Option<String>,
}

/// Bridge `.coral/config.toml` → `HttpRunner` settings. Returns `None`
/// when neither a `[provider.ollama]` block nor `CORAL_HTTP_ENDPOINT`
/// is set; callers treat that as a hard error.
///
/// Resolution order (first match wins):
///
/// 1. **`[provider.ollama]`** in `<cwd>/.coral/config.toml`. The
///    wizard writes `endpoint = "http://localhost:11434"` (server
///    root). We append [`OLLAMA_OPENAI_CHAT_PATH`] so the runner
///    posts to Ollama's OpenAI-compat shim. If the user already typed
///    a path-suffix in the config, we leave it alone (idempotent).
/// 2. **`CORAL_HTTP_ENDPOINT`** env var (legacy v0.33 path). Kept
///    forever for BC: every v0.33 user pipeline that runs
///    `CORAL_HTTP_ENDPOINT=… coral bootstrap …` still works after
///    upgrade.
///
/// API key is layered the same way: `[provider.ollama]` has no
/// `api_key` field today, so token resolution falls through to
/// `CORAL_HTTP_API_KEY`. When the schema grows an `api_key` field
/// (e.g. Ollama Cloud), this function is the single place that needs
/// to consult it.
///
/// Note: a malformed `.coral/config.toml` does NOT crash here — we
/// surface a stderr warning and fall through to the env-var path so
/// a user whose config is half-written can still run with explicit
/// env vars.
pub(crate) fn resolve_http_endpoint(cwd: &Path) -> Option<HttpResolution> {
    // (1) Try the config file. `load_from_repo` returns `Default` for
    // a missing file, so a `None` ollama section is the common case.
    let cfg = match coral_core::config::load_from_repo(cwd) {
        Ok(c) => Some(c),
        Err(e) => {
            // Print a single-line warning so the user sees WHY their
            // config didn't take effect, but don't crash — they may
            // have valid env vars set as a fallback.
            eprintln!(
                "warning: could not parse {}/.coral/config.toml ({e}); \
                 falling back to {HTTP_ENDPOINT_ENV} env var",
                cwd.display()
            );
            None
        }
    };

    let env_endpoint = std::env::var(HTTP_ENDPOINT_ENV)
        .ok()
        .filter(|v| !v.is_empty());
    let env_api_key = std::env::var(HTTP_API_KEY_ENV)
        .ok()
        .filter(|v| !v.is_empty());

    if let Some(cfg) = cfg.as_ref()
        && let Some(ollama) = cfg.provider.ollama.as_ref()
    {
        let endpoint = ollama_endpoint_with_chat_path(&ollama.endpoint);
        // The schema currently has no `api_key` field for ollama,
        // but a user may have a custom Ollama proxy requiring auth.
        // Allow CORAL_HTTP_API_KEY to layer on top of a config
        // endpoint — explicit env overrides default-no-auth.
        return Some(HttpResolution {
            endpoint,
            api_key: env_api_key,
        });
    }

    // (2) Legacy env-var path. Preserves v0.33 behavior verbatim for
    // users who never ran the wizard.
    env_endpoint.map(|endpoint| HttpResolution {
        endpoint,
        api_key: env_api_key,
    })
}

/// Append the OpenAI chat-completions path to a bare Ollama server
/// root. If the user typed `http://host:11434/v1/chat/completions`
/// directly (or any path containing `/v1/`), we leave it alone — they
/// know what they're doing. Otherwise we append the canonical suffix.
fn ollama_endpoint_with_chat_path(raw: &str) -> String {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.contains("/v1/") || trimmed.ends_with("/v1") {
        // User typed an explicit `/v1/...` path. Respect it exactly —
        // including the no-trailing-slash form (`.../v1`) which some
        // OpenAI shims accept as a base.
        trimmed.to_string()
    } else {
        format!("{trimmed}{OLLAMA_OPENAI_CHAT_PATH}")
    }
}

/// Read the HTTP endpoint URL with the config-then-env-var precedence
/// described on [`resolve_http_endpoint`]. On failure prints an
/// actionable message to stderr and exits with code 2 — same
/// disposition as a clap usage error. Construction-time failure
/// (rather than failing inside [`Runner::run`]) is the right place
/// because the missing endpoint is purely a configuration / wiring
/// issue, not a per-prompt error.
fn resolve_http_or_die() -> HttpResolution {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    match resolve_http_endpoint(&cwd) {
        Some(r) => r,
        None => {
            eprintln!(
                "error: --provider http requires either a [provider.ollama] block in \
                 .coral/config.toml (run `coral doctor --wizard`) or \
                 {HTTP_ENDPOINT_ENV} to be set\n\
                 hint: export {HTTP_ENDPOINT_ENV}=http://localhost:8000/v1/chat/completions\n\
                 hint: optional auth via {HTTP_API_KEY_ENV}=<bearer-token>"
            );
            std::process::exit(2);
        }
    }
}

pub fn make_runner(provider: ProviderName) -> Box<dyn Runner> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    match provider {
        ProviderName::Claude => {
            // v0.34.x provider bridge: if `[provider.anthropic]` is set in
            // `.coral/config.toml`, forward `api_key` to the subprocess as
            // `ANTHROPIC_API_KEY`. Config wins over env — same precedence
            // the `resolve_http_endpoint` (Ollama) path adopted in v0.34.1.
            // When no config is present, the subprocess inherits the
            // parent process env unchanged, preserving v0.33 BC.
            let mut runner = ClaudeRunner::new();
            for (k, v) in resolve_runner_env_overrides(ProviderName::Claude, &cwd) {
                runner = runner.with_env_var(k, v);
            }
            Box::new(runner)
        }
        ProviderName::Gemini => {
            // Same bridge for the Gemini CLI — `[provider.gemini].api_key`
            // → `GEMINI_API_KEY` env var on the subprocess.
            let mut runner = GeminiRunner::new();
            for (k, v) in resolve_runner_env_overrides(ProviderName::Gemini, &cwd) {
                runner = runner.with_env_var(k, v);
            }
            Box::new(runner)
        }
        ProviderName::Local => Box::new(LocalRunner::new()),
        ProviderName::Http => {
            let resolved = resolve_http_or_die();
            let mut runner = HttpRunner::new(resolved.endpoint);
            if let Some(key) = resolved.api_key {
                runner = runner.with_api_key(key);
            }
            Box::new(runner)
        }
    }
}

/// v0.34.x bridge: resolve the env overrides that
/// [`make_runner`] would apply for `provider` given the repo cwd.
///
/// This is the test seam for the bridge — callers that want to
/// inspect what `make_runner` would inject (without actually
/// constructing a `Box<dyn Runner>`) can call this directly. The
/// output is a `Vec<(String, String)>` of env var assignments; the
/// usual shape for Claude / Gemini is zero or one entry.
///
/// Precedence: config wins. If a `[provider.anthropic]` /
/// `[provider.gemini]` block exists in `<cwd>/.coral/config.toml`,
/// the returned override **replaces** what the subprocess would have
/// inherited from the parent process env. When no config is present,
/// the returned vec is empty and the subprocess inherits the parent
/// env unchanged (the v0.33 BC path — every legacy
/// `ANTHROPIC_API_KEY=…` / `GEMINI_API_KEY=…` shell export keeps
/// working).
///
/// Providers that don't take config-resolved env vars (Local, Http)
/// always return an empty vec.
pub(crate) fn resolve_runner_env_overrides(
    provider: ProviderName,
    cwd: &Path,
) -> Vec<(String, String)> {
    match provider {
        ProviderName::Claude => {
            match resolve_provider_credentials(CredentialProvider::Anthropic, cwd) {
                Some(creds) => vec![(ANTHROPIC_API_KEY_ENV.to_string(), creds.api_key)],
                None => Vec::new(),
            }
        }
        ProviderName::Gemini => match resolve_provider_credentials(CredentialProvider::Gemini, cwd)
        {
            Some(creds) => vec![(GEMINI_API_KEY_ENV.to_string(), creds.api_key)],
            None => Vec::new(),
        },
        // Local + Http don't have a config-resolved subprocess env path.
        // (HttpRunner takes the api_key directly via its constructor,
        // not via env injection — see `resolve_http_or_die`.)
        ProviderName::Local | ProviderName::Http => Vec::new(),
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
        /// Explicitly unset the var within the guard's scope, restoring
        /// the prior value (or absence) on drop. Used by the
        /// `resolve_http_endpoint_*` tests that need to assert behavior
        /// when neither config nor env var is present.
        fn unset(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            // SAFETY: caller serializes env mutation via `ENV_LOCK`.
            unsafe {
                std::env::remove_var(key);
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

    /// Helper: write a `.coral/config.toml` with the given body inside
    /// `dir`. Mirrors what `coral doctor --wizard` writes (no schema
    /// gymnastics — the config crate's loader is what we want exercised).
    fn write_config(dir: &Path, body: &str) {
        let coral = dir.join(".coral");
        std::fs::create_dir_all(&coral).expect("create .coral");
        std::fs::write(coral.join("config.toml"), body).expect("write config.toml");
    }

    /// v0.34.x Item 3: `[provider.ollama]` in the config beats env vars.
    /// This is the bug the patch fixes — the wizard wrote the section
    /// but the runner ignored it.
    #[test]
    fn resolve_http_endpoint_reads_provider_ollama_block() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g1 = EnvVarGuard::unset("CORAL_HTTP_ENDPOINT");
        let _g2 = EnvVarGuard::unset("CORAL_HTTP_API_KEY");

        let tmp = tempfile::TempDir::new().unwrap();
        write_config(
            tmp.path(),
            r#"schema_version = 1
[provider.ollama]
endpoint = "http://localhost:11434"
model = "llama3.1:8b"
"#,
        );

        let resolved = resolve_http_endpoint(tmp.path()).expect("ollama block must resolve");
        // The wizard writes a bare server root; the runner needs the
        // OpenAI-compat path appended so the POST lands on the right
        // endpoint. This was the silent failure pre-fix.
        assert_eq!(
            resolved.endpoint,
            "http://localhost:11434/v1/chat/completions"
        );
        // Ollama default install has no auth; api_key must be None.
        assert!(resolved.api_key.is_none());
    }

    /// BC for v0.33 users: a repo without `.coral/config.toml` plus a
    /// pre-existing `CORAL_HTTP_ENDPOINT` env var must still resolve.
    /// This is the path every v0.33 user upgrading to v0.34.x will hit.
    #[test]
    fn resolve_http_endpoint_falls_back_to_env_var() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g1 = EnvVarGuard::set(
            "CORAL_HTTP_ENDPOINT",
            "http://example.com/v1/chat/completions",
        );
        let _g2 = EnvVarGuard::unset("CORAL_HTTP_API_KEY");

        let tmp = tempfile::TempDir::new().unwrap();
        // No `.coral/config.toml` written — load_from_repo returns
        // CoralConfig::default() with no provider sections.

        let resolved = resolve_http_endpoint(tmp.path()).expect("env var must resolve");
        assert_eq!(resolved.endpoint, "http://example.com/v1/chat/completions");
        assert!(resolved.api_key.is_none());
    }

    /// `CORAL_HTTP_API_KEY` layers on top of either source. The schema
    /// has no `[provider.ollama].api_key` field today, so the env var
    /// is the only way to attach auth to a config-based Ollama proxy
    /// (e.g. behind a reverse proxy requiring a bearer token).
    #[test]
    fn resolve_http_endpoint_api_key_from_env_overlays_ollama_block() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g1 = EnvVarGuard::unset("CORAL_HTTP_ENDPOINT");
        let _g2 = EnvVarGuard::set("CORAL_HTTP_API_KEY", "proxy-token-abc");

        let tmp = tempfile::TempDir::new().unwrap();
        write_config(
            tmp.path(),
            r#"schema_version = 1
[provider.ollama]
endpoint = "http://localhost:11434"
"#,
        );

        let resolved = resolve_http_endpoint(tmp.path()).expect("must resolve");
        assert_eq!(
            resolved.endpoint,
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(resolved.api_key.as_deref(), Some("proxy-token-abc"));
    }

    /// Neither config nor env: returns `None`. The caller
    /// (`resolve_http_or_die`) turns this into a clap-style usage error
    /// — we don't want the runner to construct an invalid URL.
    #[test]
    fn resolve_http_endpoint_returns_none_when_nothing_configured() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g1 = EnvVarGuard::unset("CORAL_HTTP_ENDPOINT");
        let _g2 = EnvVarGuard::unset("CORAL_HTTP_API_KEY");

        let tmp = tempfile::TempDir::new().unwrap();
        // No config, no env vars.
        assert!(resolve_http_endpoint(tmp.path()).is_none());
    }

    /// If the user typed an explicit `/v1/...` path in the config
    /// (perhaps because they're pointing at a non-default OpenAI shim),
    /// we must not double-append the suffix.
    #[test]
    fn resolve_http_endpoint_respects_explicit_v1_path_in_config() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g1 = EnvVarGuard::unset("CORAL_HTTP_ENDPOINT");
        let _g2 = EnvVarGuard::unset("CORAL_HTTP_API_KEY");

        let tmp = tempfile::TempDir::new().unwrap();
        write_config(
            tmp.path(),
            r#"schema_version = 1
[provider.ollama]
endpoint = "http://my-proxy.local/v1/chat/completions"
"#,
        );

        let resolved = resolve_http_endpoint(tmp.path()).expect("must resolve");
        assert_eq!(
            resolved.endpoint, "http://my-proxy.local/v1/chat/completions",
            "must not double-append /v1/chat/completions"
        );
    }

    /// Helper unit tests for `ollama_endpoint_with_chat_path`. Edge
    /// cases: trailing slash, already-suffixed path, bare `/v1`.
    #[test]
    fn ollama_endpoint_with_chat_path_handles_edge_cases() {
        // Bare server root → append suffix.
        assert_eq!(
            ollama_endpoint_with_chat_path("http://localhost:11434"),
            "http://localhost:11434/v1/chat/completions"
        );
        // Trailing slash → strip then append.
        assert_eq!(
            ollama_endpoint_with_chat_path("http://localhost:11434/"),
            "http://localhost:11434/v1/chat/completions"
        );
        // Already explicit → leave alone.
        assert_eq!(
            ollama_endpoint_with_chat_path("http://localhost:11434/v1/chat/completions"),
            "http://localhost:11434/v1/chat/completions"
        );
        // Bare `/v1` base → leave alone (some shims accept this shape).
        assert_eq!(
            ollama_endpoint_with_chat_path("http://localhost:11434/v1"),
            "http://localhost:11434/v1"
        );
    }

    // ── resolve_runner_env_overrides tests (v0.34.x A2 bridge) ───────
    //
    // The Claude + Gemini config → subprocess-env bridge. Same
    // precedence model as the Ollama → HttpRunner path above:
    // - config in `.coral/config.toml` wins over the legacy env var.
    // - absence of config returns an empty override list, leaving the
    //   subprocess to inherit the parent process env (the v0.33 BC
    //   path — every `ANTHROPIC_API_KEY=…` / `GEMINI_API_KEY=…`
    //   pipeline keeps working).
    //
    // The bridge is exercised at the env-override layer (rather than
    // through a spawned subprocess) because the inheritance is the
    // job of `coral-runner::ClaudeRunner::with_env_var`, which has
    // its own end-to-end tests in the runner crate. Here we pin the
    // CLI-side resolution logic.

    /// Anthropic config wins over env: even if `ANTHROPIC_API_KEY` is
    /// set in the parent process, the override list includes the
    /// config-derived value so the subprocess sees that one. This
    /// matches what users expect after `coral doctor --wizard`: the
    /// key they just typed wins, not the stale shell export they
    /// forgot about.
    #[test]
    fn resolve_runner_env_overrides_anthropic_config_wins_over_env() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Stale shell-level env var that should be eclipsed by config.
        let _g = EnvVarGuard::set("ANTHROPIC_API_KEY", "stale-shell-value");

        let tmp = tempfile::TempDir::new().unwrap();
        write_config(
            tmp.path(),
            r#"schema_version = 1
[provider.anthropic]
api_key = "sk-ant-from-config"
model = "claude-haiku-4-5"
"#,
        );

        let overrides = resolve_runner_env_overrides(ProviderName::Claude, tmp.path());
        assert_eq!(
            overrides,
            vec![(
                "ANTHROPIC_API_KEY".to_string(),
                "sk-ant-from-config".to_string()
            )],
            "config-derived ANTHROPIC_API_KEY must beat the stale shell env"
        );
    }

    /// BC for v0.33 users: no `.coral/config.toml` → empty override
    /// list → subprocess inherits whatever was in the parent env.
    /// This is the path every v0.33 user upgrading to v0.34.x hits.
    #[test]
    fn resolve_runner_env_overrides_anthropic_falls_back_to_env_when_no_config() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Env present, no config — overrides empty so the subprocess
        // inherits ANTHROPIC_API_KEY=legacy-env unchanged. The
        // override-list-is-empty assertion is the load-bearing one;
        // we don't assert on the env-var value because the bridge
        // explicitly does NOT copy from env to overrides (the parent
        // env is the fallback, not a source the override layer needs
        // to know about).
        let _g = EnvVarGuard::set("ANTHROPIC_API_KEY", "legacy-env-value");

        let tmp = tempfile::TempDir::new().unwrap();
        // No `.coral/config.toml` — `load_from_repo` returns Default.

        let overrides = resolve_runner_env_overrides(ProviderName::Claude, tmp.path());
        assert!(
            overrides.is_empty(),
            "no config → no overrides; subprocess inherits parent env: got {overrides:?}"
        );
    }

    /// Gemini config wins over env: same precedence as Anthropic,
    /// pinned independently because the two providers have separate
    /// config blocks and separate env vars (`GEMINI_API_KEY` vs
    /// `ANTHROPIC_API_KEY`) — a copy-paste regression in one wouldn't
    /// be caught by the other's test.
    #[test]
    fn resolve_runner_env_overrides_gemini_config_wins_over_env() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g = EnvVarGuard::set("GEMINI_API_KEY", "stale-shell-value");

        let tmp = tempfile::TempDir::new().unwrap();
        write_config(
            tmp.path(),
            r#"schema_version = 1
[provider.gemini]
api_key = "AIza-from-config"
"#,
        );

        let overrides = resolve_runner_env_overrides(ProviderName::Gemini, tmp.path());
        assert_eq!(
            overrides,
            vec![("GEMINI_API_KEY".to_string(), "AIza-from-config".to_string())],
            "config-derived GEMINI_API_KEY must beat the stale shell env"
        );
    }

    /// BC for v0.33 Gemini users: no config → empty override list.
    #[test]
    fn resolve_runner_env_overrides_gemini_falls_back_to_env_when_no_config() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g = EnvVarGuard::set("GEMINI_API_KEY", "legacy-env-value");

        let tmp = tempfile::TempDir::new().unwrap();
        let overrides = resolve_runner_env_overrides(ProviderName::Gemini, tmp.path());
        assert!(
            overrides.is_empty(),
            "no config → no overrides for Gemini: got {overrides:?}"
        );
    }

    /// Both providers in a single config.toml resolve independently:
    /// asking for Claude reads `[provider.anthropic]`, asking for
    /// Gemini reads `[provider.gemini]`, neither leaks into the
    /// other. This is the multi-provider user shape — pick Anthropic
    /// today, add Gemini next week.
    #[test]
    fn resolve_runner_env_overrides_reads_both_providers_independently() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g1 = EnvVarGuard::unset("ANTHROPIC_API_KEY");
        let _g2 = EnvVarGuard::unset("GEMINI_API_KEY");

        let tmp = tempfile::TempDir::new().unwrap();
        write_config(
            tmp.path(),
            r#"schema_version = 1
[provider.anthropic]
api_key = "sk-ant-A"

[provider.gemini]
api_key = "gem-B"
"#,
        );

        let claude_o = resolve_runner_env_overrides(ProviderName::Claude, tmp.path());
        assert_eq!(
            claude_o,
            vec![("ANTHROPIC_API_KEY".to_string(), "sk-ant-A".to_string())]
        );
        let gemini_o = resolve_runner_env_overrides(ProviderName::Gemini, tmp.path());
        assert_eq!(
            gemini_o,
            vec![("GEMINI_API_KEY".to_string(), "gem-B".to_string())]
        );
    }

    /// Local + Http providers never inject config-derived env vars,
    /// regardless of what other blocks the config has. (HttpRunner
    /// takes `api_key` via its constructor in `resolve_http_or_die`;
    /// LocalRunner has no auth surface.)
    #[test]
    fn resolve_runner_env_overrides_local_and_http_always_empty() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let tmp = tempfile::TempDir::new().unwrap();
        write_config(
            tmp.path(),
            r#"schema_version = 1
[provider.anthropic]
api_key = "sk-ant-noise"

[provider.gemini]
api_key = "gem-noise"
"#,
        );

        assert!(
            resolve_runner_env_overrides(ProviderName::Local, tmp.path()).is_empty(),
            "Local must never inject env vars — auth flows through llama-cli's own state"
        );
        assert!(
            resolve_runner_env_overrides(ProviderName::Http, tmp.path()).is_empty(),
            "Http auth lives in HttpRunner constructor (see resolve_http_or_die), \
             not in the env-override list"
        );
    }

    /// Malformed `.coral/config.toml` must NOT crash the resolver —
    /// the env-var fallback path must still work. Operationally
    /// critical: a half-written file (e.g. user killed the wizard
    /// mid-write) shouldn't prevent the user from running with
    /// `CORAL_HTTP_ENDPOINT=… coral bootstrap …`.
    #[test]
    fn resolve_http_endpoint_falls_through_on_malformed_config() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _g1 = EnvVarGuard::set(
            "CORAL_HTTP_ENDPOINT",
            "http://envfallback/v1/chat/completions",
        );
        let _g2 = EnvVarGuard::unset("CORAL_HTTP_API_KEY");

        let tmp = tempfile::TempDir::new().unwrap();
        // Deliberately broken TOML — bare bracket, no closing.
        write_config(tmp.path(), "schema_version = 1\n[provider.ollama\n");

        let resolved = resolve_http_endpoint(tmp.path()).expect("env fallback must kick in");
        assert_eq!(resolved.endpoint, "http://envfallback/v1/chat/completions");
    }
}
