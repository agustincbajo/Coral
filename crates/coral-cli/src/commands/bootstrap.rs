use anyhow::{Context, Result};
use clap::Args;
use coral_runner::{Prompt, Runner};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct BootstrapArgs {
    /// Optional model override.
    #[arg(long)]
    pub model: Option<String>,
}

pub fn run(args: BootstrapArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let runner = super::runner_helper::default_runner();
    run_with_runner(args, wiki_root, runner.as_ref())
}

pub fn run_with_runner(
    args: BootstrapArgs,
    wiki_root: Option<&Path>,
    runner: &dyn Runner,
) -> Result<ExitCode> {
    let root: PathBuf = wiki_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".wiki"));
    if !root.exists() {
        anyhow::bail!(
            "wiki root not found: {}. Run `coral init` first.",
            root.display()
        );
    }

    // Walk repo (exclude .git, .wiki, target, node_modules) to collect file list.
    let cwd = std::env::current_dir().context("getting cwd")?;
    let files = collect_repo_files(&cwd)?;
    let listing = files
        .iter()
        .take(200) // cap to keep prompts bounded
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = Prompt {
        system: Some(BOOTSTRAP_SYSTEM.to_string()),
        user: format!(
            "Repo file listing (truncated to 200):\n{listing}\n\nSuggest 5–15 page slugs to seed `.wiki/`. For each, give: slug, type, 1-line rationale. Output YAML list."
        ),
        model: args.model,
        cwd: None,
        timeout: None,
    };

    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("runner failed: {e}"))?;

    println!("# Bootstrap suggestions (review before applying)\n");
    println!("{}", out.stdout);
    println!("\n# (v0.1 prints suggestions only — apply manually. v0.2 will write pages.)");
    Ok(ExitCode::SUCCESS)
}

const BOOTSTRAP_SYSTEM: &str = "You are the Coral wiki bibliotecario. Suggest initial wiki pages based on a repo file listing. Be terse and pragmatic.";

fn collect_repo_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git" | "target" | "node_modules" | ".wiki" | ".idea" | ".vscode"
            )
        })
    {
        let entry = entry.context("walking repo")?;
        if entry.file_type().is_file() {
            files.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|_| entry.path().to_path_buf()),
            );
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_runner::MockRunner;
    use tempfile::TempDir;

    #[test]
    fn bootstrap_invokes_runner_with_file_listing() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".wiki")).unwrap();
        std::fs::write(tmp.path().join("README.md"), "# repo").unwrap();
        std::fs::write(tmp.path().join("src.rs"), "// code").unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();

        let runner = MockRunner::new();
        runner.push_ok("- slug: readme\n  type: source\n  rationale: top-level overview");
        let exit = run_with_runner(
            BootstrapArgs::default(),
            Some(&tmp.path().join(".wiki")),
            &runner,
        )
        .unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].user.contains("README.md") || calls[0].user.contains("src.rs"));
    }
}
