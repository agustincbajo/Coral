use anyhow::{Context, Result};
use clap::Args;
use include_dir::{Dir, include_dir};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct SyncArgs {
    /// Pin a specific Coral version. In v0.1, only the embedded bundle is supported;
    /// passing a version different from the running binary's version aborts.
    #[arg(long)]
    pub version: Option<String>,
    /// Overwrite existing files in target dirs.
    #[arg(long)]
    pub force: bool,
}

static TEMPLATE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../template");

pub fn run(args: SyncArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    if let Some(v) = &args.version {
        let current = env!("CARGO_PKG_VERSION");
        if v.trim_start_matches('v') != current {
            anyhow::bail!(
                "requested version {v} differs from embedded {current}; remote sync not yet supported in v0.1"
            );
        }
    }

    let cwd = std::env::current_dir().context("getting cwd")?;
    // Lay files under <cwd>/template/ so the consumer can pick what to use.
    // Phase F will expand this to map agents/ → .claude/agents/, etc.
    let dest = cwd.join("template");
    std::fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;
    let mut written = 0usize;

    extract_recursive(&TEMPLATE, &dest, args.force, &mut written)?;

    // Mark version at the cwd root (not under template/).
    let marker = cwd.join(".coral-template-version");
    std::fs::write(&marker, format!("v{}\n", env!("CARGO_PKG_VERSION")))
        .with_context(|| format!("writing {}", marker.display()))?;

    println!(
        "✔ Synced {} files to {}. Pinned: v{}",
        written,
        dest.display(),
        env!("CARGO_PKG_VERSION")
    );
    Ok(ExitCode::SUCCESS)
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
