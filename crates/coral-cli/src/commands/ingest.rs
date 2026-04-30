use anyhow::{Context, Result};
use clap::Args;
use coral_core::gitdiff;
use coral_runner::{Prompt, Runner};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct IngestArgs {
    /// Override start commit. If not provided, reads `last_commit` from .wiki/index.md.
    #[arg(long)]
    pub from: Option<String>,
    /// Optional model override.
    #[arg(long)]
    pub model: Option<String>,
}

pub fn run(args: IngestArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let runner = super::runner_helper::default_runner();
    run_with_runner(args, wiki_root, runner.as_ref())
}

pub fn run_with_runner(
    args: IngestArgs,
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
    let cwd = std::env::current_dir().context("getting cwd")?;

    let from = match args.from {
        Some(f) => f,
        None => {
            let idx_path = root.join("index.md");
            let idx_content =
                std::fs::read_to_string(&idx_path).context("reading .wiki/index.md")?;
            let idx = coral_core::index::WikiIndex::parse(&idx_content)?;
            idx.last_commit
        }
    };
    let head = gitdiff::head_sha(&cwd).unwrap_or_else(|_| "HEAD".to_string());
    let range = format!("{from}..{head}");

    let entries = gitdiff::run(&cwd, &range).unwrap_or_default();
    let summary = entries
        .iter()
        .map(|e| format!("{:?} {}", e.kind, e.path.display()))
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = Prompt {
        system: Some(INGEST_SYSTEM.to_string()),
        user: format!(
            "Diff range: {range}\n\nChanged files:\n{summary}\n\nWhich pages of the wiki should be created or updated? Output YAML list of {{slug, action, rationale}} where action is one of: create | update | retire."
        ),
        model: args.model,
        cwd: None,
        timeout: None,
    };

    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("runner failed: {e}"))?;

    println!("# Ingest plan for range {range} (preview)\n");
    println!("{}", out.stdout);
    println!("\n# (v0.1 prints plan only — apply manually. v0.2 will mutate pages.)");
    Ok(ExitCode::SUCCESS)
}

const INGEST_SYSTEM: &str =
    "You are the Coral wiki bibliotecario. Translate a git diff into a wiki update plan. Be terse.";

#[cfg(test)]
mod tests {
    use super::*;
    use coral_runner::MockRunner;
    use tempfile::TempDir;

    #[test]
    fn ingest_invokes_runner_with_range() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        // Minimal index.md so `from` resolves.
        let idx_content = format!(
            "---\nlast_commit: abc123\ngenerated_at: {}\n---\n\n# index\n",
            chrono::Utc::now().to_rfc3339()
        );
        std::fs::write(wiki.join("index.md"), idx_content).unwrap();

        let runner = MockRunner::new();
        runner.push_ok("- slug: order\n  action: update\n  rationale: handler signature changed");
        let exit = run_with_runner(
            IngestArgs {
                from: Some("abc".into()),
                model: None,
            },
            Some(wiki.as_path()),
            &runner,
        )
        .unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].user.contains("abc.."));
    }
}
