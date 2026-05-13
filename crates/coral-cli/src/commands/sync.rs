use anyhow::{Context, Result};
use clap::Args;
use include_dir::{Dir, include_dir};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use super::pins::Pins;

#[derive(Args, Debug, Default)]
pub struct SyncArgs {
    /// Pin a specific Coral version. Without `--remote`, only the embedded
    /// bundle is supported and passing a version different from the running
    /// binary aborts. With `--remote`, fetches that tag from the Coral repo.
    #[arg(long)]
    pub version: Option<String>,
    /// Fetch the template from a remote tag via `git clone --depth=1
    /// --branch=<version>`. Requires `--version`.
    #[arg(long)]
    pub remote: bool,
    /// Overwrite existing files in target dirs.
    #[arg(long)]
    pub force: bool,
    /// Add or update a per-file pin: `--pin agents/wiki-bibliotecario=v0.3.0`.
    /// Can be repeated.
    #[arg(long, value_parser = parse_pin)]
    pub pin: Vec<(String, String)>,
    /// Remove a per-file pin: `--unpin agents/wiki-bibliotecario`. Can be repeated.
    #[arg(long)]
    pub unpin: Vec<String>,
}

fn parse_pin(s: &str) -> Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("expected K=V, got: {s}"))?;
    if k.is_empty() || v.is_empty() {
        return Err(format!("expected non-empty K=V, got: {s}"));
    }
    Ok((k.to_string(), v.to_string()))
}

static TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../template");

const REMOTE_REPO_URL: &str = "https://github.com/agustincbajo/Coral";

pub fn run(args: SyncArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    if args.remote && args.version.is_none() {
        anyhow::bail!("remote sync requires --version");
    }

    let cwd = std::env::current_dir().context("getting cwd")?;
    let dest = cwd.join("template");
    std::fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut written = 0usize;

    let resolved_version: String = if args.remote {
        // `--remote requires --version` was enforced at the top of `run`;
        // we re-surface the check as an explicit error here instead of
        // unwrapping so a refactor that drops the early `bail!` cannot
        // turn into a runtime panic. Cost is one branch on the cold path.
        let version = args
            .version
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("--remote requires --version"))?;
        sync_from_remote(version, &dest, args.force, &mut written)?;
        version.trim_start_matches('v').to_string()
    } else {
        if let Some(v) = &args.version {
            let current = env!("CARGO_PKG_VERSION");
            if v.trim_start_matches('v') != current {
                anyhow::bail!(
                    "requested version {v} differs from embedded {current}; pass --remote to fetch from git"
                );
            }
        }
        extract_recursive(&TEMPLATE, &dest, args.force, &mut written)?;
        env!("CARGO_PKG_VERSION").to_string()
    };

    // Mark version at the cwd root (legacy single-line marker, kept for bcompat).
    let marker = cwd.join(Pins::LEGACY_FILENAME);
    std::fs::write(&marker, format!("v{}\n", resolved_version))
        .with_context(|| format!("writing {}", marker.display()))?;

    // Load (or initialize) pins, apply CLI mutations, then persist.
    let mut pins = Pins::load(&cwd)?.unwrap_or_else(|| Pins {
        default: format!("v{}", resolved_version),
        pins: Default::default(),
    });
    // The default tracks the version we just synced.
    pins.default = format!("v{}", resolved_version);
    for (k, v) in &args.pin {
        pins.set_pin(k, v);
    }
    for k in &args.unpin {
        pins.unpin(k);
    }
    let pins_path = pins.save(&cwd)?;

    println!(
        "✔ Synced {} files to {}. Pinned: v{} (pins: {})",
        written,
        dest.display(),
        resolved_version,
        pins_path.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn sync_from_remote(version: &str, dest: &Path, force: bool, written: &mut usize) -> Result<()> {
    let tmp = tempfile::TempDir::new().context("creating temp dir for git clone")?;
    let tmp_path = tmp.path();
    let status = std::process::Command::new("git")
        .args([
            "clone",
            "--depth=1",
            "--branch",
            version,
            REMOTE_REPO_URL,
            tmp_path
                .to_str()
                .context("temp dir path is not valid UTF-8")?,
        ])
        .status()
        .context("invoking git clone (is git installed?)")?;
    if !status.success() {
        anyhow::bail!(
            "git clone failed for version {version}; check the tag exists at {REMOTE_REPO_URL}"
        );
    }
    let src_template = tmp_path.join("template");
    if !src_template.exists() {
        anyhow::bail!(
            "template/ not found in remote {version} (expected at {})",
            src_template.display()
        );
    }
    extract_fs_dir(&src_template, dest, force, written)?;
    Ok(())
}

fn extract_recursive(dir: &Dir<'_>, dest: &Path, force: bool, written: &mut usize) -> Result<()> {
    for entry in dir.entries() {
        match entry {
            include_dir::DirEntry::Dir(d) => {
                let path = dest.join(d.path());
                std::fs::create_dir_all(&path)
                    .with_context(|| format!("creating {}", path.display()))?;
                extract_recursive(d, dest, force, written)?;
            }
            include_dir::DirEntry::File(f) => {
                let target: PathBuf = dest.join(f.path());
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                if !target.exists() || force {
                    std::fs::write(&target, f.contents())
                        .with_context(|| format!("writing {}", target.display()))?;
                    *written += 1;
                }
            }
        }
    }
    Ok(())
}

/// Filesystem-walk equivalent of `extract_recursive` for the `--remote` path.
/// Walks `src` and copies every regular file into the matching path under
/// `dest`, creating intermediate directories. Honors the same `force` semantics
/// (skip if the target exists).
fn extract_fs_dir(src: &Path, dest: &Path, force: bool, written: &mut usize) -> Result<()> {
    for entry in walkdir::WalkDir::new(src) {
        let entry =
            entry.with_context(|| format!("walking remote template at {}", src.display()))?;
        let path = entry.path();
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = path.strip_prefix(src).with_context(|| {
            format!("stripping prefix {} from {}", src.display(), path.display())
        })?;
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        if !target.exists() || force {
            std::fs::copy(path, &target)
                .with_context(|| format!("copying {} -> {}", path.display(), target.display()))?;
            *written += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pin_accepts_kv() {
        let (k, v) = parse_pin("agents/x=v0.3.0").unwrap();
        assert_eq!(k, "agents/x");
        assert_eq!(v, "v0.3.0");
    }

    #[test]
    fn parse_pin_rejects_no_equals() {
        assert!(parse_pin("nope").is_err());
    }

    #[test]
    fn parse_pin_rejects_empty_sides() {
        assert!(parse_pin("=v1.0").is_err());
        assert!(parse_pin("k=").is_err());
    }
}
