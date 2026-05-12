use anyhow::{Context, Result};
use clap::Args;
use coral_core::symbols::{self, SymbolKind};
use coral_core::walk;
use coral_stats::StatsReport;
use std::collections::BTreeMap;
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

        // v0.30.x audit #007: BTreeMap (not HashMap) so JSON / Markdown
        // emission is byte-deterministic across runs. HashMap's
        // RandomState seed differs per process and produced flaky CI
        // diffs / golden snapshots.
        let mut by_kind: BTreeMap<SymbolKind, usize> = BTreeMap::new();
        for sym in &syms {
            *by_kind.entry(sym.kind).or_default() += 1;
        }

        // Breakdown by language (extension).
        let mut by_lang: BTreeMap<String, usize> = BTreeMap::new();
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
                        .collect::<BTreeMap<String, usize>>(),
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
                // v0.30.x audit #007: count desc, then name asc as tiebreak
                // so equal-count kinds emit in a stable order. `sort_by_key`
                // is unstable across runs; this explicit tiebreak makes
                // Markdown output deterministic.
                kinds.sort_by(|(ak, av), (bk, bv)| {
                    bv.cmp(av).then_with(|| ak.to_string().cmp(&bk.to_string()))
                });
                for (kind, count) in &kinds {
                    println!("- {}: {}", kind, count);
                }

                println!("\n### By language\n");
                let mut langs: Vec<_> = by_lang.iter().collect();
                langs.sort_by(|(ak, av), (bk, bv)| bv.cmp(av).then_with(|| ak.cmp(bk)));
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

/// Build the symbols JSON payload used by `coral stats --symbols --format json`.
///
/// Extracted as a helper so the v0.30.x audit #007 regression test can
/// pin byte-deterministic output without touching stdout. Keep this in
/// sync with the inline `serde_json::json!({...})` site above.
#[doc(hidden)]
pub fn build_symbols_json(
    total: usize,
    by_kind: &BTreeMap<SymbolKind, usize>,
    by_lang: &BTreeMap<String, usize>,
    linkage_candidates: &[(&str, usize)],
) -> Result<String> {
    let symbols_json = serde_json::json!({
        "total": total,
        "by_kind": by_kind.iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect::<BTreeMap<String, usize>>(),
        "by_language": by_lang,
        "auto_linkage_candidates": linkage_candidates.iter()
            .map(|(slug, count)| serde_json::json!({"slug": slug, "matches": count}))
            .collect::<Vec<_>>(),
    });
    Ok(serde_json::to_string_pretty(&symbols_json)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v0.30.x audit #007 regression: the symbols JSON must be
    /// byte-identical across runs. Pre-fix `by_kind` / `by_lang` were
    /// `HashMap`s and the RandomState seed produced flaky key order.
    /// We build a fixed input with >=3 kinds and >=2 languages and
    /// assert two serializations match byte-for-byte.
    #[test]
    fn symbols_json_is_byte_deterministic() {
        let mut by_kind: BTreeMap<SymbolKind, usize> = BTreeMap::new();
        by_kind.insert(SymbolKind::Function, 7);
        by_kind.insert(SymbolKind::Struct, 3);
        by_kind.insert(SymbolKind::Trait, 1);
        by_kind.insert(SymbolKind::Enum, 2);

        let mut by_lang: BTreeMap<String, usize> = BTreeMap::new();
        by_lang.insert("rs".to_string(), 8);
        by_lang.insert("ts".to_string(), 4);
        by_lang.insert("py".to_string(), 1);

        let linkage = vec![("foo", 2usize), ("bar", 1usize)];

        let a = build_symbols_json(13, &by_kind, &by_lang, &linkage).unwrap();
        let b = build_symbols_json(13, &by_kind, &by_lang, &linkage).unwrap();
        assert_eq!(a, b, "symbols JSON must be byte-identical across calls");
        // Spot-check ordering is alphabetical (BTreeMap) so the
        // assertion is sensitive to the regression: enum < function < struct < trait.
        let enum_pos = a.find("\"enum\"").expect("enum present");
        let func_pos = a.find("\"function\"").expect("function present");
        let struct_pos = a.find("\"struct\"").expect("struct present");
        let trait_pos = a.find("\"trait\"").expect("trait present");
        assert!(
            enum_pos < func_pos,
            "by_kind must be sorted (enum < function)"
        );
        assert!(
            func_pos < struct_pos,
            "by_kind must be sorted (function < struct)"
        );
        assert!(
            struct_pos < trait_pos,
            "by_kind must be sorted (struct < trait)"
        );
    }
}
