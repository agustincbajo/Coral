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
        if !entry.is_table() {
            return Err(CoralError::Manifest(format!(
                "section path collides with non-table value at `{key}`"
            )));
        }
        current = entry.as_table_mut().expect("just checked is_table");
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
