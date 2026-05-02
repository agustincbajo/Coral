use coral_runner::{ClaudeRunner, GeminiRunner, LocalRunner, Runner};

/// Names known by `--provider` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProviderName {
    #[default]
    Claude,
    Gemini,
    /// Local llama.cpp via the `llama-cli` binary. Set the model path with
    /// `--model /path/to/model.gguf` (or `prompt.model` programmatically).
    Local,
}

impl std::str::FromStr for ProviderName {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "gemini" => Ok(Self::Gemini),
            "local" | "llama" | "llama.cpp" => Ok(Self::Local),
            other => Err(format!(
                "unknown provider: {other} (valid: claude, gemini, local)"
            )),
        }
    }
}

pub fn make_runner(provider: ProviderName) -> Box<dyn Runner> {
    match provider {
        ProviderName::Claude => Box::new(ClaudeRunner::new()),
        ProviderName::Gemini => Box::new(GeminiRunner::new()),
        ProviderName::Local => Box::new(LocalRunner::new()),
    }
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
    fn provider_name_rejects_unknown() {
        let err = "openai".parse::<ProviderName>().unwrap_err();
        assert!(err.contains("unknown provider"));
        assert!(err.contains("openai"));
    }

    #[test]
    fn resolve_provider_prefers_cli_over_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // SAFETY: serialized via ENV_LOCK; only this test mutates CORAL_PROVIDER.
        unsafe {
            std::env::set_var("CORAL_PROVIDER", "gemini");
        }
        let p = resolve_provider(Some("claude")).unwrap();
        assert_eq!(p, ProviderName::Claude);
        unsafe {
            std::env::remove_var("CORAL_PROVIDER");
        }
    }
}
