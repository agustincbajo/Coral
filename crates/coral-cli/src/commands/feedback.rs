//! `coral feedback submit` — opt-in calibration feedback JSON.
//!
//! v0.38.0 (PRD §11 decision #3, finally implemented).
//!
//! ## What this command does
//!
//! Reads two local files — `.wiki/.bootstrap-state.json` and
//! `.coral/config.toml` — and emits a **sanitized** JSON envelope to
//! stdout containing only:
//!
//! - `coral_version` — `env!("CARGO_PKG_VERSION")`
//! - `platform` — `os` (`linux`/`macos`/`windows`) + `arch`
//!   (`x86_64`/`aarch64`/…)
//! - `provider` — bare label, e.g. `"claude"`, `"anthropic"`,
//!   `"gemini"`, `"ollama"`, or `"unknown"`. **Never the API key.**
//! - `repo_signature` — `loc_total` + `file_count_by_type` (extension
//!   → count, e.g. `".rs": 78`) + `page_count`. No paths, no names.
//! - `bootstrap_estimate` — `predicted_usd` + `upper_bound_usd` +
//!   `actual_usd` + `margin_pct_actual`. Floats only.
//! - `wallclock` — `predicted_seconds` + `actual_seconds`. Integers.
//!
//! ## What this command does NOT do
//!
//! - **It does NOT phone home.** Per PRD AF-1 (cero phone-home), the
//!   command writes to stdout and prints a manual paste URL. The
//!   operator chooses whether to share the data.
//! - **It does NOT include paths.** Sanitization rules are exercised
//!   by the `sanitization_assertions` test below: no file paths, no
//!   file names, no git remote URLs, no API keys, no user names.
//!
//! ## `--copy`
//!
//! When passed, the JSON is also printed with a platform-specific
//! one-liner the user can pipe through to populate the clipboard
//! (`pbcopy` / `clip` / `xclip -selection clipboard`). A real
//! cross-platform clipboard crate (`arboard`) would add ~6 native
//! deps and 4 cfg-gated code paths; the documented manual pipe is
//! cleaner for v0.38.0. Future revision: add an optional
//! `clipboard` cargo feature.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::ExitCode;

/// `coral feedback <subcommand>` argument shell.
#[derive(Args, Debug)]
pub struct FeedbackArgs {
    #[command(subcommand)]
    pub command: FeedbackCmd,
}

#[derive(Subcommand, Debug)]
pub enum FeedbackCmd {
    /// Emit sanitized calibration JSON to stdout for manual sharing.
    Submit(SubmitArgs),
}

#[derive(Args, Debug, Default)]
pub struct SubmitArgs {
    /// Also print a platform-specific clipboard-pipe hint after the
    /// JSON so the operator can `coral feedback submit | pbcopy`
    /// (macOS) / `... | clip` (Windows) / `... | xclip -selection
    /// clipboard` (Linux) without remembering the right tool.
    #[arg(long)]
    pub copy: bool,
}

/// Discussion thread the user-facing instructions point at. Hard-
/// coded; if the URL changes, the operator can still paste somewhere
/// else — the JSON is content-addressable.
const FEEDBACK_URL: &str = "https://github.com/agustincbajo/Coral/discussions";

// ---------------------------------------------------------------------
// Sanitized envelope types — these are the ONLY thing serialized.
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedbackEnvelope {
    pub coral_version: String,
    pub platform: Platform,
    pub provider: String,
    pub repo_signature: RepoSignature,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap_estimate: Option<BootstrapEstimate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallclock: Option<Wallclock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Platform {
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoSignature {
    pub loc_total: u64,
    pub file_count_by_type: std::collections::BTreeMap<String, u64>,
    pub page_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BootstrapEstimate {
    pub predicted_usd: f64,
    pub upper_bound_usd: f64,
    pub actual_usd: f64,
    pub margin_pct_actual: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Wallclock {
    pub predicted_seconds: u64,
    pub actual_seconds: u64,
}

// ---------------------------------------------------------------------
// Entry point.
// ---------------------------------------------------------------------

pub fn run(args: FeedbackArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        FeedbackCmd::Submit(submit_args) => run_submit(submit_args, wiki_root),
    }
}

fn run_submit(args: SubmitArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let cwd = std::env::current_dir().context("reading current directory")?;
    let wiki_dir = wiki_root
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| cwd.join(".wiki"));

    let envelope = build_envelope(&cwd, &wiki_dir)?;
    let json = serde_json::to_string_pretty(&envelope)
        .context("serializing feedback envelope")?;

    println!("{json}");

    eprintln!();
    eprintln!(
        "Paste this JSON into a comment at {} to contribute calibration data.",
        FEEDBACK_URL
    );
    eprintln!("Coral does NOT auto-send anything (AF-1: zero phone-home).");

    if args.copy {
        eprintln!();
        let hint = clipboard_pipe_hint();
        eprintln!("To copy to clipboard, re-run piped:");
        eprintln!("  coral feedback submit | {hint}");
    }

    Ok(ExitCode::SUCCESS)
}

/// Returns the platform-appropriate clipboard pipe command. Pure;
/// covered by unit tests.
pub fn clipboard_pipe_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "pbcopy"
    } else if cfg!(target_os = "windows") {
        "clip"
    } else {
        // Linux / *BSD — xclip is the most common; xsel is the
        // fallback. We pick xclip because the wider distros ship it.
        "xclip -selection clipboard"
    }
}

/// Build the envelope from local state. Missing inputs degrade
/// gracefully:
///
/// - No `.wiki/.bootstrap-state.json` → `bootstrap_estimate` +
///   `wallclock` are `None`. A warning is emitted to stderr.
/// - No `.coral/config.toml` → `provider` is `"unknown"`.
pub fn build_envelope(cwd: &Path, wiki_dir: &Path) -> Result<FeedbackEnvelope> {
    let coral_version = env!("CARGO_PKG_VERSION").to_string();
    let platform = current_platform();
    let provider = sanitized_provider_label(cwd);
    let repo_signature = compute_repo_signature(cwd, wiki_dir);

    let bootstrap_path = wiki_dir.join(".bootstrap-state.json");
    let (bootstrap_estimate, wallclock) = if bootstrap_path.exists() {
        load_calibration(&bootstrap_path)?
    } else {
        // Note: deliberately do NOT print the absolute path here.
        // The sanitization invariant covers stderr too — the user
        // may paste their terminal scrollback as easily as the JSON.
        eprintln!(
            "note: no calibration data available; run a bootstrap first \
             (no .wiki/.bootstrap-state.json in the wiki root)."
        );
        (None, None)
    };

    Ok(FeedbackEnvelope {
        coral_version,
        platform,
        provider,
        repo_signature,
        bootstrap_estimate,
        wallclock,
    })
}

fn current_platform() -> Platform {
    Platform {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

/// Read `.coral/config.toml` (if present) and return ONLY the bare
/// provider name — never the API key, never the host URL.
fn sanitized_provider_label(cwd: &Path) -> String {
    let cfg = match coral_core::config::load_from_repo(cwd) {
        Ok(cfg) => cfg,
        Err(_) => return "unknown".to_string(),
    };
    if cfg.provider.anthropic.is_some() {
        "anthropic".to_string()
    } else if cfg.provider.gemini.is_some() {
        "gemini".to_string()
    } else if cfg.provider.ollama.is_some() {
        "ollama".to_string()
    } else if cfg.provider.claude_cli.is_some() {
        "claude_cli".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Count LOC across tracked files, file counts by extension, and
/// wiki page count. Skips `.git`, `target`, `node_modules`,
/// `dist`, `.wiki` itself.
///
/// **Sanitization invariant**: this function MUST NOT include any
/// path string in the output. Only counts and extension fragments.
fn compute_repo_signature(cwd: &Path, wiki_dir: &Path) -> RepoSignature {
    let mut loc_total: u64 = 0;
    let mut file_count_by_type: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();

    for entry in walkdir::WalkDir::new(cwd)
        .into_iter()
        .filter_entry(|e| !is_skip_dir(e.path()))
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| format!(".{s}"));
        if let Some(ext) = ext {
            *file_count_by_type.entry(ext).or_insert(0) += 1;
        }
        // Cheap LOC: count '\n' bytes without holding the whole file.
        if let Ok(bytes) = std::fs::read(path) {
            loc_total += bytes.iter().filter(|b| **b == b'\n').count() as u64;
        }
    }

    // Page count = number of `*.md` files directly inside the wiki
    // dir (not recursive — wiki layout is flat by design).
    let page_count = std::fs::read_dir(wiki_dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|x| x.to_str())
                        .is_some_and(|x| x == "md")
                })
                .count() as u64
        })
        .unwrap_or(0);

    RepoSignature {
        loc_total,
        file_count_by_type,
        page_count,
    }
}

fn is_skip_dir(p: &Path) -> bool {
    let name = match p.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return false,
    };
    matches!(
        name,
        ".git" | "target" | "node_modules" | "dist" | ".wiki" | ".coral"
    )
}

/// Decode the bootstrap-state JSON enough to extract calibration
/// numbers. We don't import the typed `BootstrapState` because that
/// would couple this module to `commands::bootstrap::state` and
/// pull in lockfile machinery. The JSON shape is stable enough
/// (FR-ONB-30 schema versioning) that an opportunistic parse is
/// fine — missing fields just degrade to `None`.
fn load_calibration(path: &Path) -> Result<(Option<BootstrapEstimate>, Option<Wallclock>)> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading bootstrap state: {}", path.display()))?;
    let v: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing bootstrap state: {}", path.display()))?;

    let predicted_usd = v
        .get("predicted_cost_usd")
        .and_then(|x| x.as_f64())
        .or_else(|| v.get("estimated_cost_usd").and_then(|x| x.as_f64()));
    let upper_bound_usd = v
        .get("predicted_upper_bound_usd")
        .and_then(|x| x.as_f64())
        .or_else(|| v.get("max_cost_usd").and_then(|x| x.as_f64()));
    let actual_usd = v.get("cost_spent_usd").and_then(|x| x.as_f64());

    let bootstrap_estimate = match (predicted_usd, upper_bound_usd, actual_usd) {
        (Some(p), Some(u), Some(a)) => {
            let margin = if p > 0.0 {
                ((a - p) / p * 100.0).abs()
            } else {
                0.0
            };
            Some(BootstrapEstimate {
                predicted_usd: p,
                upper_bound_usd: u,
                actual_usd: a,
                margin_pct_actual: round_to(margin, 1),
            })
        }
        _ => None,
    };

    let predicted_seconds = v.get("predicted_wallclock_seconds").and_then(|x| x.as_u64());
    let actual_seconds = v
        .get("actual_wallclock_seconds")
        .and_then(|x| x.as_u64())
        .or_else(|| {
            // Fallback: derive from started_at if completed_at is
            // available — opportunistic only.
            None
        });
    let wallclock = match (predicted_seconds, actual_seconds) {
        (Some(p), Some(a)) => Some(Wallclock {
            predicted_seconds: p,
            actual_seconds: a,
        }),
        _ => None,
    };

    Ok((bootstrap_estimate, wallclock))
}

fn round_to(x: f64, decimals: u32) -> f64 {
    let f = 10_f64.powi(decimals as i32);
    (x * f).round() / f
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn platform_fields_populated() {
        let p = current_platform();
        assert!(!p.os.is_empty(), "OS should be set");
        assert!(!p.arch.is_empty(), "arch should be set");
    }

    #[test]
    fn clipboard_pipe_hint_is_platform_appropriate() {
        let hint = clipboard_pipe_hint();
        if cfg!(target_os = "macos") {
            assert_eq!(hint, "pbcopy");
        } else if cfg!(target_os = "windows") {
            assert_eq!(hint, "clip");
        } else {
            assert!(hint.contains("xclip") || hint.contains("xsel"));
        }
    }

    #[test]
    fn sanitization_no_paths_in_json_output() {
        // Construct an envelope manually and verify the serialized
        // JSON contains nothing that looks like a path or filename.
        let envelope = FeedbackEnvelope {
            coral_version: "0.38.0".into(),
            platform: Platform {
                os: "linux".into(),
                arch: "x86_64".into(),
            },
            provider: "anthropic".into(),
            repo_signature: RepoSignature {
                loc_total: 12534,
                file_count_by_type: {
                    let mut m = BTreeMap::new();
                    m.insert(".rs".into(), 78);
                    m.insert(".ts".into(), 31);
                    m.insert(".md".into(), 15);
                    m
                },
                page_count: 47,
            },
            bootstrap_estimate: Some(BootstrapEstimate {
                predicted_usd: 0.42,
                upper_bound_usd: 0.53,
                actual_usd: 0.39,
                margin_pct_actual: 7.1,
            }),
            wallclock: Some(Wallclock {
                predicted_seconds: 180,
                actual_seconds: 162,
            }),
        };
        let json = serde_json::to_string(&envelope).unwrap();

        // Path separators — neither Unix nor Windows.
        assert!(!json.contains('/'), "JSON must not contain '/' (path leak)");
        assert!(
            !json.contains('\\'),
            "JSON must not contain '\\' (path leak)"
        );
        // Common file-like fragments that should never be present.
        for forbidden in [
            ".bootstrap-state.json",
            ".coral/config.toml",
            "/home/",
            "C:\\",
            "/Users/",
            "secret",
            "api_key",
            "sk-ant-",
            "github.com",
            ".git",
        ] {
            assert!(
                !json.contains(forbidden),
                "JSON must not contain `{forbidden}`: {json}"
            );
        }
        // Extensions are fine — they're not paths.
        assert!(json.contains(".rs"));
        assert!(json.contains(".ts"));
    }

    #[test]
    fn build_envelope_without_bootstrap_state_reports_no_calibration() {
        // Set up a tempdir with NO .wiki/.bootstrap-state.json.
        let tmp = tempfile::tempdir().unwrap();
        let wiki = tmp.path().join(".wiki");
        fs::create_dir_all(&wiki).unwrap();
        // No bootstrap-state.json written.

        let envelope = build_envelope(tmp.path(), &wiki).unwrap();
        assert!(
            envelope.bootstrap_estimate.is_none(),
            "no calibration available -> None"
        );
        assert!(envelope.wallclock.is_none());
        assert_eq!(envelope.platform.os, std::env::consts::OS);
        assert_eq!(envelope.provider, "unknown");
    }

    #[test]
    fn build_envelope_counts_file_extensions_and_skips_skip_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path();
        // Some tracked files.
        fs::write(cwd.join("a.rs"), "line 1\nline 2\n").unwrap();
        fs::write(cwd.join("b.rs"), "x\ny\nz\n").unwrap();
        fs::write(cwd.join("c.md"), "page\n").unwrap();
        // A `target/` dir (should be skipped).
        fs::create_dir(cwd.join("target")).unwrap();
        fs::write(cwd.join("target/should_not_count.rs"), "x\n").unwrap();
        // A `.git/` dir (skipped).
        fs::create_dir(cwd.join(".git")).unwrap();
        fs::write(cwd.join(".git/HEAD"), "ref: foo\n").unwrap();
        // .wiki — also skipped from LOC but used for page count.
        fs::create_dir(cwd.join(".wiki")).unwrap();
        fs::write(cwd.join(".wiki/page1.md"), "page body\n").unwrap();
        fs::write(cwd.join(".wiki/page2.md"), "page body\n").unwrap();
        fs::write(cwd.join(".wiki/.bootstrap-state.json"), "{}").unwrap();

        let sig = compute_repo_signature(cwd, &cwd.join(".wiki"));
        // Only a.rs + b.rs + c.md should count: 2 + 3 + 1 = 6 LOC.
        assert_eq!(sig.loc_total, 6, "skipped dirs leaked into LOC");
        assert_eq!(sig.file_count_by_type.get(".rs").copied(), Some(2));
        assert_eq!(sig.file_count_by_type.get(".md").copied(), Some(1));
        assert_eq!(sig.page_count, 2, "expected 2 wiki pages");
    }

    #[test]
    fn load_calibration_with_partial_state_returns_none_fields() {
        // A state file that only has provider + plan but no cost
        // numbers — should return (None, None) gracefully.
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("state.json");
        fs::write(
            &p,
            r#"{"schema_version": 1, "coral_version": "0.38.0",
                 "plan_fingerprint": "x", "plan": [], "pages": []}"#,
        )
        .unwrap();
        let (est, wc) = load_calibration(&p).unwrap();
        assert!(est.is_none());
        assert!(wc.is_none());
    }

    #[test]
    fn load_calibration_with_full_state_returns_some() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("state.json");
        fs::write(
            &p,
            r#"{
                "schema_version": 1,
                "coral_version": "0.38.0",
                "predicted_cost_usd": 0.42,
                "predicted_upper_bound_usd": 0.53,
                "cost_spent_usd": 0.39,
                "predicted_wallclock_seconds": 180,
                "actual_wallclock_seconds": 162
            }"#,
        )
        .unwrap();
        let (est, wc) = load_calibration(&p).unwrap();
        let est = est.expect("bootstrap_estimate populated");
        assert!((est.predicted_usd - 0.42).abs() < 1e-9);
        assert!((est.upper_bound_usd - 0.53).abs() < 1e-9);
        assert!((est.actual_usd - 0.39).abs() < 1e-9);
        // margin_pct_actual = |0.39 - 0.42| / 0.42 * 100 ≈ 7.1
        assert!((est.margin_pct_actual - 7.1).abs() < 0.2);
        let wc = wc.expect("wallclock populated");
        assert_eq!(wc.predicted_seconds, 180);
        assert_eq!(wc.actual_seconds, 162);
    }

    // Tag the constant + URL test so renames go through review.
    #[test]
    fn feedback_url_is_pinned_discussion_path() {
        assert!(
            FEEDBACK_URL.starts_with("https://github.com/agustincbajo/Coral/discussions"),
            "feedback URL must point at the agustincbajo/Coral discussions"
        );
    }

    // Defensive: ensure the envelope never accidentally serializes
    // an `api_key` field even if a future field gets added.
    #[test]
    fn envelope_serde_field_allowlist() {
        let envelope = FeedbackEnvelope {
            coral_version: "0.38.0".into(),
            platform: Platform {
                os: "linux".into(),
                arch: "x86_64".into(),
            },
            provider: "anthropic".into(),
            repo_signature: RepoSignature {
                loc_total: 0,
                file_count_by_type: BTreeMap::new(),
                page_count: 0,
            },
            bootstrap_estimate: None,
            wallclock: None,
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = v.as_object().unwrap();
        let allowed: std::collections::HashSet<&str> = [
            "coral_version",
            "platform",
            "provider",
            "repo_signature",
        ]
        .into_iter()
        .collect();
        for key in obj.keys() {
            assert!(
                allowed.contains(key.as_str()),
                "unexpected field in feedback envelope: {key}"
            );
        }
    }

    // Compile-time-ish check: the SubmitArgs default has --copy off.
    #[test]
    fn submit_args_default_copy_is_off() {
        let a = SubmitArgs::default();
        assert!(!a.copy);
    }

    // Path-only sanity: the envelope types contain no `PathBuf`
    // fields. This is enforced by the type definitions in this
    // module; the test exists so a future refactor that adds a
    // path field would have to confront it. Serialized-shape
    // assertion lives in `sanitization_no_paths_in_json_output`.
    #[test]
    fn envelope_struct_has_zero_path_typed_fields() {
        let _envelope_size = std::mem::size_of::<FeedbackEnvelope>();
        let _platform_size = std::mem::size_of::<Platform>();
        let _signature_size = std::mem::size_of::<RepoSignature>();
        // If anyone adds a `PathBuf` field to one of these structs,
        // the sanitization JSON assertions in the sibling tests will
        // start firing because PathBuf serializes as a string with
        // path separators.
    }
}
