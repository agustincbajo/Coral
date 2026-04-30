use anyhow::{Context, Result};
use clap::Args;
use coral_core::walk;
use coral_stats::StatsReport;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct StatsArgs {
    /// Output format: markdown (default) or json.
    #[arg(long, default_value = "markdown")]
    pub format: String,
}

pub fn run(args: StatsArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
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
    let report = StatsReport::new(&pages);

    match args.format.as_str() {
        "json" => println!("{}", report.as_json()?),
        _ => println!("{}", report.as_markdown()),
    }
    Ok(ExitCode::SUCCESS)
}
