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

/// Run `git ls-remote --tags <url>` and return the set of tag names
/// (without the `refs/tags/` prefix and the `^{}` peel suffix).
fn ls_remote_tags(remote: &str) -> Result<BTreeSet<String>> {
    let output = std::process::Command::new("git")
        .args(["ls-remote", "--tags", remote])
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
}
