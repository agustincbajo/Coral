//! Per-repo Coral configuration: `.coral/config.toml`.
//!
//! v0.34.0 (M1) introduces a frozen-contract TOML schema describing
//! the provider credentials (Anthropic / Gemini / Ollama / claude
//! CLI), the bootstrap cost thresholds, and the WebUI defaults the
//! provider mini-wizard writes during `coral-doctor` (FR-ONB-27,
//! FR-ONB-14, FR-ONB-26). Hard contract — see PRD v1.4 Appendix E.
//!
//! Design choices:
//!
//! * **Schema-versioned**: every file starts with `schema_version`.
//!   The binary refuses to read a file whose schema is newer than
//!   what it understands (forward-incompat) and continues silently
//!   on equal-or-older (we own backwards compat as the field set
//!   grows).
//! * **Optional file**: v0.33 users never wrote a `.coral/config.toml`;
//!   `load_from_repo` returns `Self::default()` when the file is
//!   absent. There is no migration path — the wizard writes a fresh
//!   file the first time the user picks a provider.
//! * **Lock-then-write**: secrets land in this file (API keys),
//!   so the write path acquires an exclusive flock on the file
//!   itself (via `coral_core::atomic::with_exclusive_lock`) before
//!   parse-merge-rewrite. Two concurrent `coral-doctor` runs cannot
//!   race the wizard.
//! * **Permissions**: on Unix, the post-write `chmod 600` ensures
//!   group/other cannot read a freshly-written API key. Windows has
//!   no equivalent (`std::os::unix::fs::PermissionsExt` only); we
//!   document that and rely on the user's ACL defaults.
//! * **Atomic write**: temp-then-rename via
//!   `coral_core::atomic::atomic_write_string` so a crash mid-write
//!   never strands the file in a torn state.

use crate::error::{CoralError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Highest `.coral/config.toml` schema this binary understands. Bump
/// whenever a field is *removed* or *re-typed*; additive fields are
/// non-breaking because `serde(default)` + `Option<T>` cover them.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// Top-level `.coral/config.toml` document.
///
/// Every section is optional from the user's perspective — a freshly-
/// scaffolded file may only contain `schema_version` plus one
/// `[provider.*]` block written by the wizard. The `Default` impl
/// returns a schema-version-1 document with all sections empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoralConfig {
    /// Document schema version. See [`CONFIG_SCHEMA_VERSION`].
    pub schema_version: u32,
    #[serde(default)]
    pub provider: ProviderConfigs,
    #[serde(default)]
    pub bootstrap: BootstrapConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

impl Default for CoralConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            provider: ProviderConfigs::default(),
            bootstrap: BootstrapConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

/// `[provider.*]` sections. Each variant is opt-in via the
/// `coral-doctor` mini-wizard. Presence (`Some(_)`) means "the user
/// configured this provider"; the actual usability is gated by
/// `coral self-check`'s `providers_available` probe.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderConfigs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<AnthropicProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini: Option<GeminiProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ollama: Option<OllamaProvider>,
    /// `claude CLI` provider — opt-in flag only; auth lives inside
    /// the `claude` binary's own state directory. We just record
    /// that the user picked this path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_cli: Option<ClaudeCliProvider>,
}

/// `[provider.anthropic]`. The `api_key` is a secret — wizard chmods
/// 600 on Unix. Model + token limit have safe defaults so a wizard
/// write can omit them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicProvider {
    pub api_key: String,
    #[serde(default = "default_anthropic_model")]
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens_per_page: u32,
}

fn default_anthropic_model() -> String {
    "claude-sonnet-4-5".to_string()
}

fn default_max_tokens() -> u32 {
    4096
}

/// `[provider.gemini]`. Secret + model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiProvider {
    pub api_key: String,
    #[serde(default = "default_gemini_model")]
    pub model: String,
}

fn default_gemini_model() -> String {
    "gemini-2.0-flash".to_string()
}

/// `[provider.ollama]`. No secret — local endpoint + model name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaProvider {
    #[serde(default = "default_ollama_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_ollama_model")]
    pub model: String,
}

fn default_ollama_endpoint() -> String {
    "http://localhost:11434".to_string()
}

fn default_ollama_model() -> String {
    "llama3.1:8b".to_string()
}

/// `[provider.claude_cli]`. Empty marker section — presence means
/// "use the `claude` binary on PATH". No secret here.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeCliProvider {}

/// `[bootstrap]` section: cost-confirmation thresholds (FR-ONB-14).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapConfig {
    /// Below this, `coral bootstrap --apply` skips the cost prompt.
    #[serde(default = "default_auto_confirm")]
    pub auto_confirm_under_usd: f64,
    /// Above this, the skill flashes a warning before confirm.
    #[serde(default = "default_warn_threshold")]
    pub warn_threshold_usd: f64,
    /// Above this, the skill suggests `--max-pages=50 --priority=high`.
    #[serde(default = "default_big_repo_threshold")]
    pub big_repo_threshold_usd: f64,
    #[serde(default)]
    pub defaults: BootstrapDefaults,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            auto_confirm_under_usd: default_auto_confirm(),
            warn_threshold_usd: default_warn_threshold(),
            big_repo_threshold_usd: default_big_repo_threshold(),
            defaults: BootstrapDefaults::default(),
        }
    }
}

fn default_auto_confirm() -> f64 {
    0.10
}

fn default_warn_threshold() -> f64 {
    1.00
}

fn default_big_repo_threshold() -> f64 {
    5.00
}

/// `[bootstrap.defaults]`. Flag values applied when the user did
/// NOT pass an explicit override on the CLI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BootstrapDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pages: Option<u32>,
}

/// `[ui]` section: WebUI defaults consumed by `coral ui serve`
/// (FR-ONB-11, FR-ONB-17).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_ui_port")]
    pub port: u16,
    #[serde(default = "default_auto_serve")]
    pub auto_serve_after_bootstrap: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            port: default_ui_port(),
            auto_serve_after_bootstrap: default_auto_serve(),
        }
    }
}

fn default_ui_port() -> u16 {
    3838
}

fn default_auto_serve() -> bool {
    true
}

/// Returns the conventional config path for a repo cwd:
/// `<cwd>/.coral/config.toml`. Pure, no I/O.
pub fn config_path(cwd: &Path) -> PathBuf {
    cwd.join(".coral").join("config.toml")
}

/// Enumeration of providers whose credentials live in `.coral/config.toml`.
///
/// Used by [`resolve_provider_credentials`] as the dispatch key. We
/// keep this small and explicit (rather than parsing the
/// `ProviderName` enum from `coral-cli`) so that the core crate has
/// zero dependency on the CLI's flag-parsing types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialProvider {
    /// `[provider.anthropic]` — `api_key` is mandatory in the schema.
    Anthropic,
    /// `[provider.gemini]` — `api_key` is mandatory in the schema.
    Gemini,
}

/// Resolved provider credentials suitable for bridging from
/// `.coral/config.toml` to a runner subprocess.
///
/// Returned by [`resolve_provider_credentials`]. Callers typically
/// inject `api_key` into the spawned subprocess via `cmd.env(...)` so
/// it lands as `ANTHROPIC_API_KEY` / `GEMINI_API_KEY` for the CLI the
/// runner shells out to. `model` is non-`None` because the schema's
/// `serde(default)` populates a sensible default — callers can apply
/// it via `Prompt.model` or leave the user's `--model` flag in charge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProviderCredentials {
    /// API key (never empty if returned `Some`; empty config values
    /// are treated as "no config" and the function returns `None`).
    pub api_key: String,
    /// Model id from the config block.
    pub model: String,
}

/// Bridge `[provider.anthropic]` / `[provider.gemini]` → runner subprocess.
///
/// Resolution mirrors the `[provider.ollama]` → `HttpRunner` pattern
/// added in v0.34.1 ([`coral_cli::commands::runner_helper`]):
///
/// 1. **Config wins**: if the requested block exists in
///    `<cwd>/.coral/config.toml`, return `Some(_)` with the key + model
///    from that block. The caller is expected to override the
///    subprocess env with the returned `api_key` — config explicitly
///    *beats* a pre-set env var, so a user who switches keys via
///    `coral doctor --wizard` doesn't have to also `unset
///    ANTHROPIC_API_KEY` in their shell.
/// 2. **Fall through**: returns `None` when the file is absent, the
///    block is absent, the `api_key` is empty/whitespace, or the file
///    is malformed (one-line stderr warning emitted for malformed
///    files — operational continuity beats a hard crash).
///
/// Empty `api_key` is treated as "no config" so that a half-finished
/// wizard run (or a manually-edited file with `api_key = ""`) doesn't
/// silently inject an empty env var that the CLI subprocess would
/// surface as a confusing auth error.
///
/// BC contract (v0.33 → v0.34.x): the caller layers env-var fallback
/// on top — if this returns `None`, callers continue to inherit
/// `ANTHROPIC_API_KEY` / `GEMINI_API_KEY` from the parent process env,
/// preserving every v0.33-era pipeline that exports keys via env.
pub fn resolve_provider_credentials(
    provider: CredentialProvider,
    cwd: &Path,
) -> Option<ResolvedProviderCredentials> {
    let cfg = match load_from_repo(cwd) {
        Ok(c) => c,
        Err(e) => {
            // Match the resolver-helper pattern in `coral_cli::commands::runner_helper`:
            // a malformed config file is NOT fatal here. A user mid-wizard
            // with a half-written file should still be able to fall back
            // to their env var. One-line stderr breadcrumb so the missed
            // config is visible.
            eprintln!(
                "warning: could not parse {}/.coral/config.toml ({e}); \
                 falling back to env-var auth",
                cwd.display()
            );
            return None;
        }
    };

    match provider {
        CredentialProvider::Anthropic => cfg.provider.anthropic.as_ref().and_then(|a| {
            let key = a.api_key.trim();
            if key.is_empty() {
                None
            } else {
                Some(ResolvedProviderCredentials {
                    api_key: key.to_string(),
                    model: a.model.clone(),
                })
            }
        }),
        CredentialProvider::Gemini => cfg.provider.gemini.as_ref().and_then(|g| {
            let key = g.api_key.trim();
            if key.is_empty() {
                None
            } else {
                Some(ResolvedProviderCredentials {
                    api_key: key.to_string(),
                    model: g.model.clone(),
                })
            }
        }),
    }
}

/// Loads `.coral/config.toml` from `<cwd>/`. Returns `Self::default()`
/// when the file is absent — that is the v0.33 → v0.34 zero-state
/// migration: v0.33 users never wrote one.
///
/// Hard-fails when `schema_version` is GREATER than
/// [`CONFIG_SCHEMA_VERSION`] (the binary is older than the file —
/// we cannot know what fields were dropped/re-typed in the newer
/// schema). Older schema versions are accepted and silently treated
/// as "no `schema_version_X_only_fields`" because additive evolution
/// is non-breaking.
pub fn load_from_repo(cwd: &Path) -> Result<CoralConfig> {
    let path = config_path(cwd);
    if !path.exists() {
        return Ok(CoralConfig::default());
    }
    let raw = std::fs::read_to_string(&path).map_err(|source| CoralError::Io {
        path: path.clone(),
        source,
    })?;
    let cfg: CoralConfig = toml::from_str(&raw)
        .map_err(|e| CoralError::Manifest(format!("{}: {e}", path.display())))?;
    if cfg.schema_version > CONFIG_SCHEMA_VERSION {
        return Err(CoralError::Manifest(format!(
            ".coral/config.toml schema_version {} > supported {}; upgrade coral",
            cfg.schema_version, CONFIG_SCHEMA_VERSION
        )));
    }
    Ok(cfg)
}

/// Upserts a top-level section (`provider.anthropic`, `provider.gemini`,
/// `provider.ollama`, `provider.claude_cli`, `bootstrap`, `ui`) into
/// `.coral/config.toml`. Lock-then-merge-then-atomic-write. On Unix,
/// post-write chmod 600 (best-effort).
///
/// `content` is the TOML *body* of the section — i.e. the lines that
/// belong **inside** `[<section>]`. Example:
///
/// ```rust,ignore
/// upsert_provider_section(
///     &cwd,
///     "provider.anthropic",
///     r#"
///     api_key = "sk-ant-..."
///     model = "claude-sonnet-4-5"
///     "#,
/// )?;
/// ```
///
/// Idempotent: re-running with the same content rewrites the file
/// byte-equal to the previous state (within `toml`'s formatting
/// stability — see test). The flock guarantees no two concurrent
/// `coral-doctor` invocations interleave their writes.
pub fn upsert_provider_section(cwd: &Path, section: &str, content: &str) -> Result<()> {
    let path = config_path(cwd);
    // Ensure the parent `.coral/` exists before locking. The lock
    // file lives next to the target, so the parent must exist for
    // `with_exclusive_lock` to open the sentinel.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| CoralError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    crate::atomic::with_exclusive_lock(&path, || {
        // Read-merge-write: parse the existing file (or start from
        // default), splice the new section in, serialize back. The
        // intermediate `toml::Value` representation lets us mutate
        // by section name without round-tripping through the strongly-
        // typed `CoralConfig` (which would force us to enumerate
        // every section name in code).
        let existing_text = if path.exists() {
            std::fs::read_to_string(&path).map_err(|source| CoralError::Io {
                path: path.clone(),
                source,
            })?
        } else {
            // Seed with the bare-minimum schema_version so the merge
            // below has a non-empty table to splice into.
            format!("schema_version = {CONFIG_SCHEMA_VERSION}\n")
        };

        let mut doc: toml::Value = toml::from_str(&existing_text)
            .map_err(|e| CoralError::Manifest(format!("{}: {e}", path.display())))?;

        let new_section: toml::Value = toml::from_str(content)
            .map_err(|e| CoralError::Manifest(format!("inline section content: {e}")))?;

        merge_section(&mut doc, section, new_section)?;

        // Always re-stamp schema_version so a wizard-written file
        // always advertises the version it understands.
        if let Some(table) = doc.as_table_mut() {
            table.insert(
                "schema_version".to_string(),
                toml::Value::Integer(i64::from(CONFIG_SCHEMA_VERSION)),
            );
        }

        let serialized = toml::to_string_pretty(&doc)
            .map_err(|e| CoralError::Manifest(format!("serialize: {e}")))?;

        crate::atomic::atomic_write_string(&path, &serialized)?;
        Ok(())
    })?;

    // Unix: tighten permissions so a shared-machine user can't read
    // freshly-written API keys. Windows: ACLs don't have a tidy
    // equivalent — we rely on the user's profile permissions and
    // document the limitation in PRD Appendix E.
    #[cfg(unix)]
    {
        set_perm_600_unix(&path)?;
    }

    Ok(())
}

/// Splice `value` into `doc` at the dotted path `section`. Creates
/// missing intermediate tables. Replaces a pre-existing table at the
/// target path (overwrite, not merge — the caller controls atomicity
/// at the section level).
fn merge_section(doc: &mut toml::Value, section: &str, value: toml::Value) -> Result<()> {
    let parts: Vec<&str> = section.split('.').collect();
    if parts.is_empty() {
        return Err(CoralError::Manifest("empty section path".into()));
    }
    let mut current = doc
        .as_table_mut()
        .ok_or_else(|| CoralError::Manifest("root is not a table".into()))?;

    // Walk intermediate keys, creating tables as needed.
    for key in &parts[..parts.len() - 1] {
        let entry = current
            .entry(key.to_string())
            .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
        // `as_table_mut()` returns `None` iff the entry isn't a table; we
        // surface that as a `Manifest` error rather than panicking so the
        // CLI can report which key collided.
        current = entry.as_table_mut().ok_or_else(|| {
            CoralError::Manifest(format!(
                "section path collides with non-table value at `{key}`"
            ))
        })?;
    }

    let final_key = parts[parts.len() - 1].to_string();
    current.insert(final_key, value);
    Ok(())
}

/// Best-effort chmod 600 on Unix. Errors are surfaced — callers should
/// treat a chmod failure as a security issue (the API key just landed
/// world-readable), but we still bubble it up through `Result` rather
/// than silently swallowing.
#[cfg(unix)]
fn set_perm_600_unix(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms).map_err(|source| CoralError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Missing file → `Self::default()` (zero-state migration).
    #[test]
    fn load_returns_default_when_file_missing() {
        let dir = TempDir::new().unwrap();
        let cfg = load_from_repo(dir.path()).expect("load default");
        assert_eq!(cfg.schema_version, CONFIG_SCHEMA_VERSION);
        assert!(cfg.provider.anthropic.is_none());
        assert_eq!(cfg.ui.port, 3838);
    }

    /// A complete, valid file round-trips into the strongly-typed
    /// struct. Defaults apply to omitted sections.
    #[test]
    fn load_parses_valid_minimal_file() {
        let dir = TempDir::new().unwrap();
        let coral_dir = dir.path().join(".coral");
        std::fs::create_dir_all(&coral_dir).unwrap();
        let body = r#"
schema_version = 1

[provider.anthropic]
api_key = "sk-ant-test"
model = "claude-sonnet-4-5"
max_tokens_per_page = 4096
"#;
        std::fs::write(coral_dir.join("config.toml"), body).unwrap();
        let cfg = load_from_repo(dir.path()).expect("load");
        assert_eq!(cfg.schema_version, 1);
        let anthropic = cfg
            .provider
            .anthropic
            .as_ref()
            .expect("anthropic block present");
        assert_eq!(anthropic.api_key, "sk-ant-test");
        assert_eq!(anthropic.model, "claude-sonnet-4-5");
        // Section we didn't write must still default-populate.
        assert_eq!(cfg.bootstrap.warn_threshold_usd, 1.00);
    }

    /// schema_version higher than the binary supports → hard fail.
    #[test]
    fn load_hard_fails_on_future_schema_version() {
        let dir = TempDir::new().unwrap();
        let coral_dir = dir.path().join(".coral");
        std::fs::create_dir_all(&coral_dir).unwrap();
        let future = CONFIG_SCHEMA_VERSION + 1;
        let body = format!("schema_version = {future}\n");
        std::fs::write(coral_dir.join("config.toml"), body).unwrap();
        let err = load_from_repo(dir.path()).expect_err("should refuse newer schema");
        let msg = err.to_string();
        assert!(
            msg.contains(&future.to_string()) && msg.contains("upgrade coral"),
            "expected schema-version mismatch message, got: {msg}"
        );
    }

    /// upsert_provider_section is idempotent: running it twice with
    /// the same content yields a byte-equal file the second time.
    /// This is the property `serde_json` preserve_order guarantees
    /// at the JSON layer; for TOML we rely on `toml::to_string_pretty`
    /// being deterministic when the input `Value` is byte-equal.
    #[test]
    fn upsert_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let section = r#"
api_key = "sk-ant-test"
model = "claude-sonnet-4-5"
max_tokens_per_page = 4096
"#;
        upsert_provider_section(dir.path(), "provider.anthropic", section).expect("first write");
        let first =
            std::fs::read_to_string(config_path(dir.path())).expect("read after first write");

        upsert_provider_section(dir.path(), "provider.anthropic", section).expect("second write");
        let second =
            std::fs::read_to_string(config_path(dir.path())).expect("read after second write");

        assert_eq!(
            first, second,
            "second upsert with identical content must produce a byte-equal file"
        );

        // And the result must still parse cleanly through the
        // strongly-typed loader (no formatting drift the deserializer
        // rejects).
        let cfg = load_from_repo(dir.path()).expect("reload");
        let anth = cfg
            .provider
            .anthropic
            .expect("anthropic survives round-trip");
        assert_eq!(anth.api_key, "sk-ant-test");
    }

    // ── resolve_provider_credentials tests ───────────────────────────
    //
    // The function is the read half of the wizard → runner bridge: the
    // wizard writes `[provider.anthropic]` / `[provider.gemini]` blocks,
    // these tests verify the read path. Sister of
    // `resolve_http_endpoint_*` in `coral_cli::commands::runner_helper`.

    /// No config file → `None`. This is the v0.33 fast-path: a user
    /// who never ran the wizard has no `.coral/config.toml` and must
    /// keep falling back to env vars.
    #[test]
    fn resolve_credentials_returns_none_when_no_config() {
        let dir = TempDir::new().unwrap();
        assert!(resolve_provider_credentials(CredentialProvider::Anthropic, dir.path()).is_none());
        assert!(resolve_provider_credentials(CredentialProvider::Gemini, dir.path()).is_none());
    }

    /// `[provider.anthropic]` block in config returns the key + model.
    /// Whitespace around the key is trimmed (paranoia for hand-edited
    /// files).
    #[test]
    fn resolve_credentials_reads_anthropic_block() {
        let dir = TempDir::new().unwrap();
        let coral = dir.path().join(".coral");
        std::fs::create_dir_all(&coral).unwrap();
        std::fs::write(
            coral.join("config.toml"),
            r#"schema_version = 1
[provider.anthropic]
api_key = "sk-ant-abc"
model = "claude-haiku-4-5"
"#,
        )
        .unwrap();

        let creds = resolve_provider_credentials(CredentialProvider::Anthropic, dir.path())
            .expect("anthropic creds must resolve");
        assert_eq!(creds.api_key, "sk-ant-abc");
        assert_eq!(creds.model, "claude-haiku-4-5");

        // Same file, gemini block absent → gemini lookup returns None.
        assert!(
            resolve_provider_credentials(CredentialProvider::Gemini, dir.path()).is_none(),
            "gemini block was not written; lookup must not invent credentials"
        );
    }

    /// `[provider.gemini]` block similarly. Default model field
    /// applies when omitted (via the schema's `serde(default)`).
    #[test]
    fn resolve_credentials_reads_gemini_block_with_default_model() {
        let dir = TempDir::new().unwrap();
        let coral = dir.path().join(".coral");
        std::fs::create_dir_all(&coral).unwrap();
        // Deliberately omit `model` to exercise the serde default.
        std::fs::write(
            coral.join("config.toml"),
            r#"schema_version = 1
[provider.gemini]
api_key = "AIza-xyz"
"#,
        )
        .unwrap();

        let creds = resolve_provider_credentials(CredentialProvider::Gemini, dir.path())
            .expect("gemini creds must resolve");
        assert_eq!(creds.api_key, "AIza-xyz");
        assert_eq!(creds.model, "gemini-2.0-flash");
    }

    /// Both providers can coexist in a single config.toml. v0.34.x
    /// wizard re-run overwrites only the chosen block, so this shape
    /// occurs in practice for users who pick Anthropic then later add
    /// Gemini.
    #[test]
    fn resolve_credentials_reads_both_providers_from_same_file() {
        let dir = TempDir::new().unwrap();
        let coral = dir.path().join(".coral");
        std::fs::create_dir_all(&coral).unwrap();
        std::fs::write(
            coral.join("config.toml"),
            r#"schema_version = 1
[provider.anthropic]
api_key = "sk-ant-1"
model = "claude-sonnet-4-5"

[provider.gemini]
api_key = "gem-2"
model = "gemini-2.0-flash"
"#,
        )
        .unwrap();

        let a = resolve_provider_credentials(CredentialProvider::Anthropic, dir.path())
            .expect("anthropic");
        let g =
            resolve_provider_credentials(CredentialProvider::Gemini, dir.path()).expect("gemini");
        assert_eq!(a.api_key, "sk-ant-1");
        assert_eq!(a.model, "claude-sonnet-4-5");
        assert_eq!(g.api_key, "gem-2");
        assert_eq!(g.model, "gemini-2.0-flash");
    }

    /// Empty `api_key` field returns `None`, not `Some("")`. A half-
    /// finished wizard run (user hit cancel mid-prompt and a previous
    /// flow left an empty string) shouldn't inject `ANTHROPIC_API_KEY=""`
    /// into the subprocess — that would surface as a confusing
    /// "invalid_api_key" auth error rather than the more actionable
    /// "no key configured" path.
    #[test]
    fn resolve_credentials_treats_empty_api_key_as_none() {
        let dir = TempDir::new().unwrap();
        let coral = dir.path().join(".coral");
        std::fs::create_dir_all(&coral).unwrap();
        std::fs::write(
            coral.join("config.toml"),
            r#"schema_version = 1
[provider.anthropic]
api_key = ""
model = "claude-haiku-4-5"
"#,
        )
        .unwrap();
        assert!(
            resolve_provider_credentials(CredentialProvider::Anthropic, dir.path()).is_none(),
            "empty api_key must be treated as 'no credentials'"
        );
    }

    /// Malformed config doesn't crash — operational continuity: a
    /// user whose config is corrupt should still be able to run with
    /// an explicit env var. Mirrors the same fallthrough rule
    /// `resolve_http_endpoint` follows for the Ollama bridge.
    #[test]
    fn resolve_credentials_returns_none_on_malformed_config() {
        let dir = TempDir::new().unwrap();
        let coral = dir.path().join(".coral");
        std::fs::create_dir_all(&coral).unwrap();
        std::fs::write(
            coral.join("config.toml"),
            "schema_version = 1\n[provider.anthropic\n",
        )
        .unwrap();
        // No panic, returns None — caller will fall through to env var.
        assert!(resolve_provider_credentials(CredentialProvider::Anthropic, dir.path()).is_none());
    }

    /// Unix-only: post-write the file is chmod 600 (owner rw, group/
    /// other none). On Windows this test is skipped — the platform
    /// has no equivalent in std and the wizard relies on the user's
    /// profile ACL defaults.
    #[cfg(unix)]
    #[test]
    fn upsert_sets_unix_permissions_to_600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        upsert_provider_section(dir.path(), "provider.anthropic", r#"api_key = "secret""#)
            .expect("upsert");
        let meta = std::fs::metadata(config_path(dir.path())).expect("stat");
        // Mask to permission bits (top bits encode file type on
        // some platforms).
        assert_eq!(
            meta.permissions().mode() & 0o777,
            0o600,
            "freshly-written .coral/config.toml must be chmod 600 on Unix \
             so a co-tenant cannot read the API key"
        );
    }
}
