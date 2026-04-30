use anyhow::{Context, Result};
use clap::Args;
use coral_core::walk;
use coral_runner::{Prompt, Runner};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct OnboardArgs {
    /// Profile of the reader (e.g., "backend dev", "data engineer", "PM").
    #[arg(long, default_value = "engineer")]
    pub profile: String,
    #[arg(long)]
    pub model: Option<String>,
}

pub fn run(args: OnboardArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let runner = super::runner_helper::default_runner();
    run_with_runner(args, wiki_root, runner.as_ref())
}

pub fn run_with_runner(
    args: OnboardArgs,
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
            "You are the Coral wiki onboarding guide. Suggest a reading path tailored to a profile."
                .into(),
        ),
        user: format!(
            "Profile: {}\n\nPages:\n{}\n\nSuggest the optimal 5–10 page reading path with 1-line rationales. Output Markdown list.",
            args.profile, summary
        ),
        model: args.model,
        cwd: None,
        timeout: None,
    };

    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("runner failed: {e}"))?;
    println!("# Onboarding path for: {}\n", args.profile);
    println!("{}", out.stdout);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_runner::MockRunner;
    use tempfile::TempDir;

    #[test]
    fn onboard_invokes_runner_with_profile() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(&wiki).unwrap();
        let runner = MockRunner::new();
        runner.push_ok("1. [[order]] — start here.");
        let exit = run_with_runner(
            OnboardArgs {
                profile: "backend dev".into(),
                model: None,
            },
            Some(wiki.as_path()),
            &runner,
        )
        .unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        let calls = runner.calls();
        assert!(calls[0].user.contains("backend dev"));
    }
}
