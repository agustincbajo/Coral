//! `coral validate-pin` — verify every version referenced in
//! `.coral-pins.toml` exists as a tag in the remote Coral repo.
//!
//! Useful before a release that is about to consume those pins, or in CI to
//! catch a typo before it surfaces as a `coral sync --remote` failure deep
//! in the build.
//!
//! Implementation: a single `git ls-remote --tags <REMOTE_REPO_URL>` call
//! (no clone, no download) feeds a HashSet of known tags. Each pinned
//! version is then a constant-time lookup. Exit `0` when every pin
//! resolves; exit `1` when any version is missing.

use anyhow::{Context, Result};
use clap::Args;
use std::collections::BTreeSet;
use std::process::ExitCode;

use super::pins::Pins;

const REMOTE_REPO_URL: &str = "https://github.com/agustincbajo/Coral";

#[derive(Args, Debug, Default)]
pub struct ValidatePinArgs {
    /// Override the remote repo URL (useful for forks / mirrors).
    #[arg(long, default_value = REMOTE_REPO_URL)]
    pub remote: String,
}

pub fn run(args: ValidatePinArgs) -> Result<ExitCode> {
    let cwd = std::env::current_dir().context("getting cwd")?;
    let pins = match Pins::load(&cwd)? {
        Some(p) => p,
        None => {
            eprintln!(
                "no {} or {} present at {}; nothing to validate",
                Pins::FILENAME,
                Pins::LEGACY_FILENAME,
                cwd.display()
            );
            return Ok(ExitCode::SUCCESS);
        }
    };
    let needed = collect_versions(&pins);
    if needed.is_empty() {
        eprintln!("no versions referenced in pins; nothing to validate");
        return Ok(ExitCode::SUCCESS);
    }

    let tags = ls_remote_tags(&args.remote)?;
    report_validation(&pins, &needed, &tags)
}

/// Collect every distinct version string referenced by a `Pins` value.
/// Empty `default` is filtered out (uninitialized pins file).
pub(crate) fn collect_versions(pins: &Pins) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    if !pins.default.is_empty() {
        out.insert(pins.default.clone());
    }
    for v in pins.pins.values() {
        if !v.is_empty() {
            out.insert(v.clone());
        }
    }
    out
}

/// Build the `git ls-remote --tags -- <remote>` command for a remote
/// URL.
///
/// Pure construction — does not spawn — so tests can pin the argv
/// shape (specifically: that `--` appears before `remote`). Public to
/// the module so the regression test can call it directly.
///
/// v0.20.2 audit-followup #36: `--` is a defense-in-depth separator
/// against option-injection. Modern git (≥2.30) blocks
/// `--upload-pack=evil` shapes via a `protocol.allow` allowlist, but
/// older git on user CI boxes is vulnerable. Same class as the
/// v0.19.5 git-clone fix (#3).
pub(crate) fn build_ls_remote_command(remote: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    cmd.args(["ls-remote", "--tags", "--", remote]);
    cmd
}

/// Run `git ls-remote --tags -- <url>` and return the set of tag
/// names (without the `refs/tags/` prefix and the `^{}` peel suffix).
fn ls_remote_tags(remote: &str) -> Result<BTreeSet<String>> {
    let output = build_ls_remote_command(remote)
        .output()
        .context("invoking git ls-remote (is git installed?)")?;
    if !output.status.success() {
        anyhow::bail!(
            "git ls-remote failed (exit {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(parse_ls_remote_tags(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// Pure parser for `git ls-remote --tags` output. Each non-empty line is:
/// `<sha>\trefs/tags/<name>` (or `<sha>\trefs/tags/<name>^{}` for
/// annotated-tag commit deref). The `^{}` form is collapsed into the same
/// tag name so a single tag is reported once.
pub(crate) fn parse_ls_remote_tags(stdout: &str) -> BTreeSet<String> {
    let mut tags: BTreeSet<String> = BTreeSet::new();
    for line in stdout.lines() {
        let Some((_, refname)) = line.split_once('\t') else {
            continue;
        };
        let Some(name) = refname.strip_prefix("refs/tags/") else {
            continue;
        };
        let name = name.strip_suffix("^{}").unwrap_or(name);
        tags.insert(name.to_string());
    }
    tags
}

fn report_validation(
    pins: &Pins,
    needed: &BTreeSet<String>,
    tags: &BTreeSet<String>,
) -> Result<ExitCode> {
    let mut missing: Vec<&str> = Vec::new();
    println!("# Pin validation\n");
    if !pins.default.is_empty() {
        if tags.contains(&pins.default) {
            println!("- ✓ default = `{}`", pins.default);
        } else {
            println!("- ✗ default = `{}` — NOT FOUND in remote", pins.default);
            missing.push(&pins.default);
        }
    }
    for (key, version) in &pins.pins {
        if tags.contains(version) {
            println!("- ✓ `{key}` → `{version}`");
        } else {
            println!("- ✗ `{key}` → `{version}` — NOT FOUND in remote");
            missing.push(version);
        }
    }
    println!();
    if missing.is_empty() {
        println!("All {} pinned version(s) resolve.", needed.len());
        Ok(ExitCode::SUCCESS)
    } else {
        let unique_missing: BTreeSet<&str> = missing.iter().copied().collect();
        eprintln!(
            "{} pinned version(s) missing from remote: {}",
            unique_missing.len(),
            unique_missing
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(ExitCode::FAILURE)
    }
}

/// Test-only entry point that takes the cwd explicitly so we don't have to
/// touch process-wide state in unit tests.
#[cfg(test)]
pub(crate) fn run_with_cwd_and_tags(
    cwd: &std::path::Path,
    tags: &BTreeSet<String>,
) -> Result<ExitCode> {
    let pins = match Pins::load(cwd)? {
        Some(p) => p,
        None => return Ok(ExitCode::SUCCESS),
    };
    let needed = collect_versions(&pins);
    if needed.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    report_validation(&pins, &needed, tags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn parse_ls_remote_tags_strips_prefix_and_peel_suffix() {
        let stdout = "\
abc123\trefs/tags/v0.1.0
def456\trefs/tags/v0.2.0
abc123\trefs/tags/v0.1.0^{}
ghi789\trefs/heads/main
";
        let tags = parse_ls_remote_tags(stdout);
        assert!(tags.contains("v0.1.0"));
        assert!(tags.contains("v0.2.0"));
        assert!(!tags.contains("v0.1.0^{}"));
        assert!(!tags.contains("main"));
        assert_eq!(tags.len(), 2);
    }

    #[test]
    fn parse_ls_remote_tags_handles_empty_output() {
        let tags = parse_ls_remote_tags("");
        assert!(tags.is_empty());
    }

    #[test]
    fn collect_versions_dedupes_and_skips_empty() {
        let mut p = Pins {
            default: "v0.3.0".into(),
            ..Default::default()
        };
        p.set_pin("agents/wiki-bibliotecario", "v0.3.0");
        p.set_pin("prompts/ingest", "v0.4.0");
        p.set_pin("agents/wiki-linter", ""); // empty values get skipped
        let v = collect_versions(&p);
        assert_eq!(
            v,
            ["v0.3.0", "v0.4.0"]
                .iter()
                .map(|s| s.to_string())
                .collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn validates_clean_when_all_pins_match_remote() {
        let tmp = TempDir::new().unwrap();
        let pins = Pins {
            default: "v0.4.0".into(),
            pins: pinmap(&[("agents/wiki-bibliotecario", "v0.3.2")]),
        };
        pins.save(tmp.path()).unwrap();
        let tags: BTreeSet<String> = ["v0.4.0", "v0.3.2", "v0.3.1"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let exit = run_with_cwd_and_tags(tmp.path(), &tags).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    #[test]
    fn fails_when_a_pinned_version_is_missing_from_remote() {
        let tmp = TempDir::new().unwrap();
        let pins = Pins {
            default: "v9.9.9".into(),
            ..Default::default()
        };
        pins.save(tmp.path()).unwrap();
        let tags: BTreeSet<String> = ["v0.4.0"].iter().map(|s| s.to_string()).collect();
        let exit = run_with_cwd_and_tags(tmp.path(), &tags).unwrap();
        assert_eq!(exit, ExitCode::FAILURE);
    }

    #[test]
    fn returns_success_when_no_pins_file_present() {
        let tmp = TempDir::new().unwrap();
        let exit = run_with_cwd_and_tags(tmp.path(), &BTreeSet::new()).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    fn pinmap(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// v0.20.2 audit-followup #36: regression — `git ls-remote
    /// --tags <remote>` must include the `--` end-of-options
    /// separator before the user-controlled remote URL. Otherwise a
    /// remote like `--upload-pack=evil` would be parsed by older git
    /// (<2.30) as a flag rather than a positional. Modern git
    /// mitigates this via the `protocol.allow` allowlist; the
    /// separator is defense-in-depth (same shape as the v0.19.5
    /// git-clone fix, #3).
    #[test]
    fn build_ls_remote_command_inserts_double_dash_before_remote() {
        let cmd = build_ls_remote_command("--upload-pack=evil");
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        // Expected shape: ["ls-remote", "--tags", "--", "--upload-pack=evil"]
        let dash_idx = argv
            .iter()
            .position(|a| a == "--")
            .expect("expected `--` separator in argv");
        let remote_idx = argv
            .iter()
            .position(|a| a == "--upload-pack=evil")
            .expect("expected the remote literal in argv");
        assert!(
            dash_idx < remote_idx,
            "`--` must precede the remote URL, got: {argv:?}"
        );
        // Sanity: the args before `--` are exactly `ls-remote
        // --tags`. If a future refactor adds another flag, this
        // assertion will catch the missing-`--` case.
        assert_eq!(&argv[..dash_idx], &["ls-remote", "--tags"]);
        assert_eq!(&argv[remote_idx], "--upload-pack=evil");
    }

    /// v0.20.2 audit-followup #36: a normal remote URL still works
    /// — the `--` separator is unconditional.
    #[test]
    fn build_ls_remote_command_includes_separator_for_benign_remote() {
        let cmd = build_ls_remote_command("https://github.com/agustincbajo/Coral");
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let last = argv.last().expect("argv must not be empty");
        assert_eq!(last, "https://github.com/agustincbajo/Coral");
        assert!(
            argv.contains(&"--".to_string()),
            "`--` separator must always be present: {argv:?}"
        );
    }
}
