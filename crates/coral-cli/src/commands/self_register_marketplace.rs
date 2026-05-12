//! `coral self register-marketplace` — opt-in patcher for the user's
//! `.claude/settings.json` that registers the Coral GitHub repo under
//! `extraKnownMarketplaces` (FR-ONB-26).
//!
//! The PRD's design here is "delegation to the binary so install.sh
//! doesn't depend on jq being cross-platform". This module is what
//! `install.sh --with-claude-config` calls after staging the `coral`
//! binary on PATH.
//!
//! Contract:
//!
//! 1. Locate `.claude/settings.json` relative to the current cwd
//!    (when `--scope=project`) or `$HOME/.claude/settings.json` (when
//!    `--scope=user`). Missing file is treated as `{}` — we still
//!    write it.
//! 2. Resolve symlinks via `std::fs::canonicalize` BEFORE the
//!    lock+write so dotfiles-managed configs (XDG-style symlinks into
//!    a repo) keep working: we want to mutate the underlying file,
//!    not break the symlink with a temp-then-rename onto a new inode.
//! 3. Strict JSON (`serde_json`). JSONC (lines containing `//` or
//!    `/* */` outside string literals) is refused with a clear
//!    message — we cannot safely round-trip comments.
//! 4. Backup the existing file to
//!    `.claude/settings.json.coral-backup-<ISO8601-UTC>` BEFORE
//!    touching it.
//! 5. Lock + atomic write the patched JSON. The `preserve_order`
//!    feature on `serde_json` keeps the user's key order intact.
//! 6. Log the backup path so a user who regrets the change can
//!    `mv` it back.

use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use clap::{Args, ValueEnum};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct RegisterMarketplaceArgs {
    /// Which `.claude/settings.json` to patch. `project` (default) is
    /// the cwd's `.claude/settings.json`; `user` is `$HOME/.claude/`.
    #[arg(long, value_enum, default_value_t = Scope::Project)]
    pub scope: Scope,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Project,
    User,
}

/// The marketplace entry we splice into `extraKnownMarketplaces`.
/// Constants so the test module can re-use them without drifting.
const MARKETPLACE_SOURCE: &str = "github";
const MARKETPLACE_REPO: &str = "agustincbajo/Coral";

pub fn run(args: RegisterMarketplaceArgs) -> Result<ExitCode> {
    let settings_path = resolve_settings_path(args.scope)?;
    let outcome = upsert_marketplace(&settings_path)?;
    match outcome {
        Outcome::AlreadyRegistered => {
            println!(
                "Coral marketplace already registered in {} — no changes",
                settings_path.display()
            );
        }
        Outcome::Patched { backup_path } => {
            println!("Coral marketplace registered in {}", settings_path.display());
            if let Some(b) = backup_path {
                println!(
                    "Backup at {}. Restore with: mv \"{}\" \"{}\"",
                    b.display(),
                    b.display(),
                    settings_path.display(),
                );
            }
        }
        Outcome::CreatedFresh => {
            println!(
                "Created {} with Coral marketplace registered (no prior file to back up)",
                settings_path.display()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Where the patch lands depending on `--scope`. Errors when `user`
/// scope is requested but `$HOME` isn't set — we refuse to guess.
fn resolve_settings_path(scope: Scope) -> Result<PathBuf> {
    let base = match scope {
        Scope::Project => std::env::current_dir()?,
        Scope::User => {
            let home = home_dir()
                .ok_or_else(|| anyhow!("--scope=user requires $HOME (or %USERPROFILE%) to be set"))?;
            home
        }
    };
    Ok(base.join(".claude").join("settings.json"))
}

/// Best-effort home directory resolution. We don't take the `dirs`
/// crate dep for one call site; the std-only path covers Unix
/// (`$HOME`) and Windows (`%USERPROFILE%`) which are the only OSes
/// Claude Code supports.
fn home_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        if !profile.is_empty() {
            return Some(PathBuf::from(profile));
        }
    }
    None
}

/// Outcome of a single upsert run. Lets the caller print the right
/// message; `Patched` carries the backup path so the user can undo.
#[derive(Debug)]
enum Outcome {
    AlreadyRegistered,
    Patched { backup_path: Option<PathBuf> },
    CreatedFresh,
}

/// Core idempotent patcher. Pulled out of `run` so tests can drive
/// it without going through the clap dispatcher or touching real
/// `$HOME`.
fn upsert_marketplace(settings_path: &Path) -> Result<Outcome> {
    // Ensure the `.claude/` parent exists (we'll create it for the
    // user) — locking requires the parent dir to exist for the
    // sentinel `.lock` file to live there too.
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Canonicalize the path when the file exists so writes hit the
    // dotfiles-managed inode behind any symlink. `canonicalize`
    // errors on missing files (intentionally on Windows; on Unix it
    // works for some kernels), so we only canonicalize an existing
    // file.
    let resolved_path = if settings_path.exists() {
        std::fs::canonicalize(settings_path).unwrap_or_else(|_| settings_path.to_path_buf())
    } else {
        settings_path.to_path_buf()
    };

    let existing_raw = if resolved_path.exists() {
        Some(
            std::fs::read_to_string(&resolved_path)
                .map_err(|e| anyhow!("reading {}: {e}", resolved_path.display()))?,
        )
    } else {
        None
    };

    // Refuse JSONC. The detection is intentionally permissive —
    // string literals containing "//" generate false positives, but
    // the wording of the abort tells the user what to do (skip
    // --with-claude-config and use paste flow). Failing closed is
    // safer than corrupting their config.
    if let Some(raw) = &existing_raw {
        if looks_like_jsonc(raw) {
            bail!(
                "Refusing to touch JSONC. Strict JSON only in M1. \
                 Either remove `//` or `/* */` comments and re-run, or \
                 skip --with-claude-config and use the 3-line paste flow."
            );
        }
    }

    let original_json = match existing_raw.as_deref() {
        Some(raw) if raw.trim().is_empty() => Value::Object(Map::new()),
        Some(raw) => serde_json::from_str(raw).map_err(|e| {
            anyhow!(
                "{}: not valid JSON ({e}). Aborting without changes. \
                 Either repair the file manually or skip --with-claude-config.",
                resolved_path.display()
            )
        })?,
        None => Value::Object(Map::new()),
    };

    if !matches!(original_json, Value::Object(_)) {
        bail!(
            "{}: root must be a JSON object, found {}",
            resolved_path.display(),
            json_type_name(&original_json)
        );
    }

    // Idempotency check: if the marketplace is already in the
    // array, return early without touching the file. This is the
    // "running install.sh --with-claude-config twice is a no-op"
    // guarantee from the PRD.
    if marketplace_already_present(&original_json) {
        return Ok(Outcome::AlreadyRegistered);
    }

    // Backup BEFORE the lock so an immediate crash on the lock path
    // still leaves the user with both old and new copies.
    let backup_path = if resolved_path.exists() {
        let backup = backup_path_for(&resolved_path, Utc::now());
        std::fs::copy(&resolved_path, &backup)
            .map_err(|e| anyhow!("writing backup at {}: {e}", backup.display()))?;
        Some(backup)
    } else {
        None
    };

    // Splice + atomic write under flock. The lock guards against
    // concurrent invocations (two `coral self register-marketplace`
    // running in parallel against the same settings file).
    let patched = patch_settings_with_marketplace(original_json)?;
    let serialized = serde_json::to_string_pretty(&patched)?;
    coral_core::atomic::with_exclusive_lock(&resolved_path, || {
        coral_core::atomic::atomic_write_string(&resolved_path, &serialized)
    })
    .map_err(|e| anyhow!("atomic write to {}: {e}", resolved_path.display()))?;

    if existing_raw.is_some() {
        Ok(Outcome::Patched { backup_path })
    } else {
        Ok(Outcome::CreatedFresh)
    }
}

/// Inserts the Coral marketplace entry into `extraKnownMarketplaces`
/// (creating the array if absent). Preserves the user's key order
/// because we depend on `serde_json`'s `preserve_order` feature.
fn patch_settings_with_marketplace(mut root: Value) -> Result<Value> {
    let obj = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("root must be a JSON object"))?;

    let array = obj
        .entry("extraKnownMarketplaces")
        .or_insert_with(|| Value::Array(Vec::new()));
    let arr = array
        .as_array_mut()
        .ok_or_else(|| anyhow!("extraKnownMarketplaces must be an array"))?;

    // Re-check inside the borrow so we don't drop the borrow and
    // re-scan; the outer idempotency check already exited if the
    // marketplace was present, but a malformed entry might survive.
    if arr.iter().any(is_coral_marketplace_entry) {
        return Ok(root);
    }
    let mut entry = Map::new();
    entry.insert(
        "source".to_string(),
        Value::String(MARKETPLACE_SOURCE.to_string()),
    );
    entry.insert(
        "repo".to_string(),
        Value::String(MARKETPLACE_REPO.to_string()),
    );
    arr.push(Value::Object(entry));
    Ok(root)
}

/// `true` when `extraKnownMarketplaces` already contains an entry
/// pointing at `agustincbajo/Coral`. Comparison is permissive about
/// the entry shape — older Claude Code versions may have written
/// extra keys we don't care about (we still match on source+repo).
fn marketplace_already_present(root: &Value) -> bool {
    root.as_object()
        .and_then(|o| o.get("extraKnownMarketplaces"))
        .and_then(|v| v.as_array())
        .is_some_and(|arr| arr.iter().any(is_coral_marketplace_entry))
}

fn is_coral_marketplace_entry(entry: &Value) -> bool {
    let Some(obj) = entry.as_object() else {
        return false;
    };
    let source = obj.get("source").and_then(Value::as_str);
    let repo = obj.get("repo").and_then(Value::as_str);
    source == Some(MARKETPLACE_SOURCE) && repo == Some(MARKETPLACE_REPO)
}

/// Heuristic JSONC detection: scan the file character-by-character,
/// tracking whether we're inside a double-quoted string, and flag
/// any `//` or `/*` that appears OUTSIDE a string. Inside a string,
/// `\"` is treated as an escape. This is intentionally simple — the
/// downside of a false positive is a clear user error message;
/// silently corrupting a comment is the catastrophic case we
/// guard against.
fn looks_like_jsonc(raw: &str) -> bool {
    let mut in_string = false;
    let mut prev = '\0';
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            if c == '"' && prev != '\\' {
                in_string = false;
            }
            prev = c;
            continue;
        }
        if c == '"' {
            in_string = true;
        } else if c == '/' {
            if let Some(&next) = chars.peek() {
                if next == '/' || next == '*' {
                    return true;
                }
            }
        }
        prev = c;
    }
    false
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Backup filename: `<settings.json>.coral-backup-<ISO8601-UTC>`. The
/// timestamp uses the no-colon RFC 3339 form so it's a valid filename
/// on every supported OS (Windows refuses `:` in filenames).
fn backup_path_for(settings_path: &Path, now: chrono::DateTime<Utc>) -> PathBuf {
    // 2026-05-12T19:34:11Z → 2026-05-12T193411Z (drop the colons
    // before file extension). We don't use `format!("{:?}")` because
    // chrono's debug shape isn't filename-safe.
    let stamp = now.format("%Y-%m-%dT%H%M%SZ").to_string();
    let mut name = settings_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "settings.json".into());
    name.push(format!(".coral-backup-{stamp}"));
    settings_path.with_file_name(name)
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Empty / missing settings.json → created fresh with the
    /// marketplace inserted. No backup file because nothing existed.
    #[test]
    fn empty_path_creates_fresh_settings_with_marketplace() {
        let dir = TempDir::new().unwrap();
        let settings = dir.path().join(".claude").join("settings.json");
        let outcome = upsert_marketplace(&settings).expect("upsert");
        assert!(matches!(outcome, Outcome::CreatedFresh));
        let raw = std::fs::read_to_string(&settings).unwrap();
        let parsed: Value = serde_json::from_str(&raw).unwrap();
        assert!(marketplace_already_present(&parsed));
        // No backup created for a fresh file.
        let backup_count = std::fs::read_dir(settings.parent().unwrap())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("coral-backup")
            })
            .count();
        assert_eq!(backup_count, 0, "no backup expected for a fresh write");
    }

    /// Existing valid settings.json with user keys: marketplace gets
    /// appended; backup file is created; user keys survive.
    #[test]
    fn valid_existing_settings_preserves_user_keys_and_creates_backup() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let settings = claude_dir.join("settings.json");
        let user_content = r#"{
  "theme": "dark",
  "extraKnownMarketplaces": [
    { "source": "github", "repo": "someone-else/their-plugin" }
  ]
}"#;
        std::fs::write(&settings, user_content).unwrap();

        let outcome = upsert_marketplace(&settings).expect("upsert");
        assert!(matches!(outcome, Outcome::Patched { .. }));

        let raw = std::fs::read_to_string(&settings).unwrap();
        let parsed: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.get("theme").and_then(Value::as_str), Some("dark"));
        let arr = parsed
            .get("extraKnownMarketplaces")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(arr.len(), 2, "user's existing entry must be preserved");
        assert!(arr.iter().any(is_coral_marketplace_entry));

        // Backup exists with the original content.
        let backups: Vec<_> = std::fs::read_dir(&claude_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains("coral-backup")
            })
            .collect();
        assert_eq!(backups.len(), 1, "exactly one backup expected");
        let backup_raw = std::fs::read_to_string(backups[0].path()).unwrap();
        assert_eq!(backup_raw, user_content, "backup must be byte-equal to the original");
    }

    /// Marketplace already present: upsert is a no-op. The file is
    /// NOT rewritten (so its mtime is preserved, and no spurious
    /// backup is created).
    #[test]
    fn already_present_marketplace_is_a_noop() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let settings = claude_dir.join("settings.json");
        let body = r#"{
  "extraKnownMarketplaces": [
    { "source": "github", "repo": "agustincbajo/Coral" }
  ]
}"#;
        std::fs::write(&settings, body).unwrap();
        let outcome = upsert_marketplace(&settings).expect("upsert");
        assert!(matches!(outcome, Outcome::AlreadyRegistered));
        // No backup was created.
        let backup_count = std::fs::read_dir(&claude_dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("coral-backup")
            })
            .count();
        assert_eq!(backup_count, 0);
    }

    /// JSONC (comments) detection refuses to touch the file.
    #[test]
    fn jsonc_aborts_with_clear_message() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let settings = claude_dir.join("settings.json");
        let body = r#"{
  // user has comments in their settings
  "theme": "dark"
}"#;
        std::fs::write(&settings, body).unwrap();
        let err = upsert_marketplace(&settings).expect_err("should refuse JSONC");
        let msg = err.to_string();
        assert!(
            msg.contains("JSONC") && msg.contains("paste"),
            "expected jsonc-refusal message, got: {msg}"
        );
        // File was not touched.
        let after = std::fs::read_to_string(&settings).unwrap();
        assert_eq!(after, body);
    }

    /// Corrupt JSON (missing closing brace): refuse with a clear
    /// abort message and leave the file untouched.
    #[test]
    fn corrupt_json_aborts_without_touching_file() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        let settings = claude_dir.join("settings.json");
        let body = r#"{ "theme": "dark" "#;
        std::fs::write(&settings, body).unwrap();
        let err = upsert_marketplace(&settings).expect_err("should refuse invalid JSON");
        let msg = err.to_string();
        assert!(
            msg.contains("not valid JSON"),
            "expected json-parse-error message, got: {msg}"
        );
        let after = std::fs::read_to_string(&settings).unwrap();
        assert_eq!(after, body, "corrupt file must not be rewritten");
    }

    /// The backup filename format is filename-safe across OSes
    /// (no colons — Windows refuses those in NTFS filenames).
    #[test]
    fn backup_path_has_no_colons() {
        let settings = PathBuf::from("/tmp/repo/.claude/settings.json");
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-12T19:34:11Z")
            .unwrap()
            .with_timezone(&Utc);
        let backup = backup_path_for(&settings, now);
        let name = backup.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("settings.json.coral-backup-"),
            "backup file should sit next to the original: {name}"
        );
        assert!(
            !name.contains(':'),
            "backup filename must not contain ':' (Windows refuses it): {name}"
        );
    }

    /// JSONC detector handles `//` inside string literals correctly
    /// — those are NOT comments and must not flip the heuristic.
    #[test]
    fn jsonc_detector_ignores_slashes_inside_strings() {
        let body = r#"{ "url": "https://example.com" }"#;
        assert!(
            !looks_like_jsonc(body),
            "an `https://` inside a string is not a JSONC comment"
        );
    }
}
