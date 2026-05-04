//! `.coral-pins.toml` schema + load/save with migration from the legacy
//! `.coral-template-version` marker file.
//!
//! Schema:
//! ```toml
//! default = "v0.2.0"
//! [pins]
//! "agents/wiki-bibliotecario" = "v0.3.0"
//! "prompts/ingest" = "v0.2.0"
//! ```
//!
//! `default` applies to anything unpinned. `pins` are keyed by template
//! relative path (without extension by convention but free-form).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Pins {
    /// Default version applied to anything unpinned.
    #[serde(default)]
    pub default: String,
    /// Per-file pins, keyed by relative path inside the template (e.g.,
    /// `"agents/wiki-bibliotecario"`).
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
}

impl Pins {
    pub const FILENAME: &'static str = ".coral-pins.toml";
    pub const LEGACY_FILENAME: &'static str = ".coral-template-version";

    /// Load from cwd, preferring the new TOML over the legacy single-line file.
    /// Returns `Ok(None)` when neither file exists.
    pub fn load(cwd: &Path) -> Result<Option<Self>> {
        let new_path = cwd.join(Self::FILENAME);
        if new_path.exists() {
            let content = std::fs::read_to_string(&new_path)
                .with_context(|| format!("reading {}", new_path.display()))?;
            let pins: Self = toml::from_str(&content)
                .with_context(|| format!("parsing {}", new_path.display()))?;
            return Ok(Some(pins));
        }
        let legacy_path = cwd.join(Self::LEGACY_FILENAME);
        if legacy_path.exists() {
            let content = std::fs::read_to_string(&legacy_path)
                .with_context(|| format!("reading {}", legacy_path.display()))?;
            return Ok(Some(Self {
                default: content.trim().to_string(),
                pins: BTreeMap::new(),
            }));
        }
        Ok(None)
    }

    /// Serialize and write to `<cwd>/.coral-pins.toml`. Returns the written path.
    ///
    /// v0.19.5 audit N1: writes atomically (temp + rename) so a
    /// concurrent reader (e.g. another invocation of `coral validate-pin`)
    /// sees either the OLD or the NEW content, never a half-written
    /// file.
    pub fn save(&self, cwd: &Path) -> Result<PathBuf> {
        let path = cwd.join(Self::FILENAME);
        let content = toml::to_string_pretty(self).context("serializing pins TOML")?;
        coral_core::atomic::atomic_write_string(&path, &content)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    /// Insert/update a per-file pin. `key` example: `"agents/wiki-bibliotecario"`.
    pub fn set_pin(&mut self, key: impl Into<String>, version: impl Into<String>) {
        self.pins.insert(key.into(), version.into());
    }

    /// Remove a per-file pin. Returns `true` if the key was present.
    pub fn unpin(&mut self, key: &str) -> bool {
        self.pins.remove(key).is_some()
    }

    /// Resolve the version to use for a given file path key.
    ///
    /// Falls back to `default` when the key is not pinned.
    pub fn resolve(&self, key: &str) -> &str {
        self.pins
            .get(key)
            .map(String::as_str)
            .unwrap_or(&self.default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_returns_none_when_neither_present() {
        let tmp = TempDir::new().unwrap();
        assert!(Pins::load(tmp.path()).unwrap().is_none());
    }

    #[test]
    fn load_migrates_legacy_template_version() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join(Pins::LEGACY_FILENAME), "v0.1.0\n").unwrap();
        let pins = Pins::load(tmp.path()).unwrap().unwrap();
        assert_eq!(pins.default, "v0.1.0");
        assert!(pins.pins.is_empty());
    }

    #[test]
    fn pins_toml_takes_priority_over_legacy() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join(Pins::LEGACY_FILENAME), "v0.1.0\n").unwrap();
        let new_pins = Pins {
            default: "v0.2.0".into(),
            pins: BTreeMap::from([("agents/x".into(), "v0.3.0".into())]),
        };
        new_pins.save(tmp.path()).unwrap();
        let loaded = Pins::load(tmp.path()).unwrap().unwrap();
        assert_eq!(loaded.default, "v0.2.0");
        assert_eq!(loaded.resolve("agents/x"), "v0.3.0");
        assert_eq!(loaded.resolve("prompts/y"), "v0.2.0");
    }

    #[test]
    fn resolve_unpinned_returns_default() {
        let pins = Pins {
            default: "v1.0.0".into(),
            pins: BTreeMap::new(),
        };
        assert_eq!(pins.resolve("anything"), "v1.0.0");
    }

    #[test]
    fn set_pin_then_unpin_clears_it() {
        let mut pins = Pins {
            default: "v1.0.0".into(),
            pins: BTreeMap::new(),
        };
        pins.set_pin("agents/x", "v0.9.0");
        assert_eq!(pins.resolve("agents/x"), "v0.9.0");
        assert!(pins.unpin("agents/x"));
        assert_eq!(pins.resolve("agents/x"), "v1.0.0");
    }

    #[test]
    fn save_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let mut pins = Pins {
            default: "v0.2.0".into(),
            pins: BTreeMap::new(),
        };
        pins.set_pin("prompts/ingest", "v0.3.0");
        pins.save(tmp.path()).unwrap();
        let loaded = Pins::load(tmp.path()).unwrap().unwrap();
        assert_eq!(loaded, pins);
    }
}
