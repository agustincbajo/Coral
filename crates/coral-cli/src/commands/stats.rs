use anyhow::{Context, Result};
use clap::Args;
use coral_core::symbols::{self, SymbolKind};
use coral_core::walk;
use coral_stats::StatsReport;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug, Default)]
pub struct StatsArgs {
    /// Output format: markdown (default) or json.
    #[arg(long, default_value = "markdown")]
    pub format: String,

    /// Include source-code symbol extraction in stats output.
    #[arg(long, default_value_t = false)]
    pub symbols: bool,
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

    if args.symbols {
        // Scan the project root (parent of .wiki) for source symbols.
        let project_root = root.parent().unwrap_or_else(|| Path::new("."));
        let extensions = &["rs", "ts", "tsx", "py", "go"];
        let syms = symbols::extract_from_dir(project_root, extensions);

        // Breakdown by kind.
        let mut by_kind: HashMap<SymbolKind, usize> = HashMap::new();
        for sym in &syms {
            *by_kind.entry(sym.kind).or_default() += 1;
        }

        // Breakdown by language (extension).
        let mut by_lang: HashMap<String, usize> = HashMap::new();
        for sym in &syms {
            let lang = sym
                .file
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("unknown")
                .to_string();
            *by_lang.entry(lang).or_default() += 1;
        }

        // Auto-linkage: find symbols matching wiki page slugs.
        let page_slugs: Vec<String> = pages
            .iter()
            .filter_map(|p| {
                p.path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            })
            .collect();

        let mut linkage_candidates: Vec<(&str, usize)> = Vec::new();
        for slug in &page_slugs {
            let matches = symbols::find_symbols_for_slug(&syms, slug);
            if !matches.is_empty() {
                linkage_candidates.push((slug.as_str(), matches.len()));
            }
        }

        match args.format.as_str() {
            "json" => {
                let symbols_json = serde_json::json!({
                    "total": syms.len(),
                    "by_kind": by_kind.iter()
                        .map(|(k, v)| (k.to_string(), *v))
                        .collect::<HashMap<String, usize>>(),
                    "by_language": by_lang,
                    "auto_linkage_candidates": linkage_candidates.iter()
                        .map(|(slug, count)| serde_json::json!({"slug": slug, "matches": count}))
                        .collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&symbols_json)?);
            }
            _ => {
                println!("\n## Symbols\n");
                println!("Total symbols found: {}\n", syms.len());

                println!("### By kind\n");
                let mut kinds: Vec<_> = by_kind.iter().collect();
                kinds.sort_by_key(|(_, v)| std::cmp::Reverse(**v));
                for (kind, count) in &kinds {
                    println!("- {}: {}", kind, count);
                }

                println!("\n### By language\n");
                let mut langs: Vec<_> = by_lang.iter().collect();
                langs.sort_by_key(|(_, v)| std::cmp::Reverse(**v));
                for (lang, count) in &langs {
                    println!("- .{}: {}", lang, count);
                }

                if !linkage_candidates.is_empty() {
                    println!("\n### Auto-linkage candidates\n");
                    for (slug, count) in &linkage_candidates {
                        println!("- {} ({} symbol matches)", slug, count);
                    }
                }
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}
