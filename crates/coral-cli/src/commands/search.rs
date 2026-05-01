use anyhow::{Context, Result};
use clap::Args;
use coral_core::{search, walk};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Search query.
    pub query: String,
    /// Max results to display (default: 5).
    #[arg(long, default_value_t = 5)]
    pub limit: usize,
    /// Output format: markdown (default) or json.
    #[arg(long, default_value = "markdown")]
    pub format: String,
}

pub fn run(args: SearchArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
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
    let results = search::search(&pages, &args.query, args.limit);

    if args.format == "json" {
        let json: Vec<_> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "slug": r.slug,
                    "score": r.score,
                    "snippet": r.snippet,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"results": json}))?
        );
    } else if results.is_empty() {
        println!("No results found for: {}", args.query);
    } else {
        println!("# Search results for: {}\n", args.query);
        for r in &results {
            println!(
                "- **[[{}]]** (score: {:.3})\n  {}\n",
                r.slug,
                r.score,
                r.snippet.trim()
            );
        }
        println!("\n_(v0.2 ranks via TF-IDF; v0.3 will switch to embeddings — see ADR 0006.)_");
    }
    Ok(ExitCode::SUCCESS)
}
