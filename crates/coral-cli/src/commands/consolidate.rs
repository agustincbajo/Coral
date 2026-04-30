use anyhow::{Context, Result};
use clap::Args;
use coral_core::walk;
use coral_runner::{Prompt, Runner};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct ConsolidateArgs {
    #[arg(long)]
    pub model: Option<String>,
}

pub fn run(args: ConsolidateArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let runner = super::runner_helper::default_runner();
    run_with_runner(args, wiki_root, runner.as_ref())
}

pub fn run_with_runner(
    args: ConsolidateArgs,
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
    let pages = walk::read_pages(&root)
        .with_context(|| format!("reading pages from {}", root.display()))?;

    let summary = pages
        .iter()
        .take(80)
        .map(|p| {
            format!(
                "- {} ({})",
                p.frontmatter.slug,
                serde_json::to_value(p.frontmatter.page_type)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = Prompt {
        system: Some(
            "You are the Coral wiki bibliotecario. Suggest page consolidations and archive candidates."
                .into(),
        ),
        user: format!("Pages:\n{summary}\n\nProposed consolidations? Output YAML."),
        model: args.model,
        cwd: None,
        timeout: None,
    };

    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("runner failed: {e}"))?;
    println!("# Consolidation suggestions (preview)\n");
    println!("{}", out.stdout);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_runner::MockRunner;
    use tempfile::TempDir;

    #[test]
    fn consolidate_invokes_runner() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        let runner = MockRunner::new();
        runner.push_ok("- merge: [a, b] -> a-b\n  rationale: redundant");
        let exit =
            run_with_runner(ConsolidateArgs::default(), Some(wiki.as_path()), &runner).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        assert_eq!(runner.calls().len(), 1);
    }
}
