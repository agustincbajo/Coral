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
    /// Search engine: `tfidf` (default; offline, no API key) or `embeddings`
    /// (semantic, requires VOYAGE_API_KEY).
    #[arg(long, default_value = "tfidf")]
    pub engine: String,
    /// Force a re-embed of all pages, ignoring the cached vectors.
    #[arg(long)]
    pub reindex: bool,
    /// Optional embeddings model override (default: voyage-3).
    #[arg(long)]
    pub embeddings_model: Option<String>,
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

    match args.engine.as_str() {
        "tfidf" => run_tfidf(&pages, &args),
        "embeddings" => run_embeddings(&pages, &args, &root),
        other => anyhow::bail!("unknown engine: {other}. Choose: tfidf | embeddings"),
    }
}

fn run_tfidf(pages: &[coral_core::page::Page], args: &SearchArgs) -> Result<ExitCode> {
    let results = search::search(pages, &args.query, args.limit);

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
        println!(
            "\n_(TF-IDF default; pass `--engine embeddings` for semantic search via Voyage AI.)_"
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn run_embeddings(
    pages: &[coral_core::page::Page],
    args: &SearchArgs,
    wiki_root: &Path,
) -> Result<ExitCode> {
    use coral_core::cache::WalkCache;
    use coral_core::embeddings::EmbeddingsIndex;

    let api_key = std::env::var("VOYAGE_API_KEY")
        .context("VOYAGE_API_KEY required for --engine embeddings (or use --engine tfidf)")?;
    let model = args
        .embeddings_model
        .as_deref()
        .unwrap_or(super::voyage::DEFAULT_MODEL);

    let mut index = EmbeddingsIndex::load(wiki_root)?;
    if index.dim == 0 || index.provider != model {
        index = EmbeddingsIndex::empty(model, super::voyage::DEFAULT_DIM);
    }

    // Determine which pages need (re-)embedding.
    let mut to_embed: Vec<(String, String, i64)> = Vec::new(); // (slug, text, mtime)
    for p in pages {
        let mtime = WalkCache::mtime_of(&p.path).unwrap_or(0);
        if args.reindex || !index.is_fresh(&p.frontmatter.slug, mtime) {
            let text = format!(
                "{}\n{}",
                p.frontmatter.slug,
                p.body.chars().take(8000).collect::<String>()
            );
            to_embed.push((p.frontmatter.slug.clone(), text, mtime));
        }
    }

    if !to_embed.is_empty() {
        eprintln!("Embedding {} page(s) via {model}…", to_embed.len());
        let texts: Vec<String> = to_embed.iter().map(|(_, t, _)| t.clone()).collect();
        let vectors = super::voyage::embed_batch(&texts, model, &api_key, Some("document"))
            .context("embedding pages")?;
        for ((slug, _, mtime), vec) in to_embed.into_iter().zip(vectors.into_iter()) {
            index.upsert(slug, mtime, vec);
        }
    }

    // Prune removed pages.
    let live: std::collections::HashSet<String> =
        pages.iter().map(|p| p.frontmatter.slug.clone()).collect();
    index.prune(&live);
    index.save(wiki_root).context("saving embeddings index")?;

    // Embed query.
    let query_vecs = super::voyage::embed_batch(
        std::slice::from_ref(&args.query),
        model,
        &api_key,
        Some("query"),
    )
    .context("embedding query")?;
    let query_vec = query_vecs
        .into_iter()
        .next()
        .context("no query vector returned")?;

    let scored = index.search(&query_vec, args.limit);

    if args.format == "json" {
        let json: Vec<_> = scored
            .iter()
            .map(|(slug, score)| {
                serde_json::json!({
                    "slug": slug,
                    "score": score,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "engine": "embeddings",
                "model": model,
                "results": json,
            }))?
        );
    } else if scored.is_empty() {
        println!("No results found for: {}", args.query);
    } else {
        println!("# Search results for: {} ({})\n", args.query, model);
        for (slug, score) in &scored {
            // Find the page to get a snippet.
            let snippet = pages
                .iter()
                .find(|p| &p.frontmatter.slug == slug)
                .map(|p| p.body.chars().take(200).collect::<String>())
                .unwrap_or_default();
            println!(
                "- **[[{slug}]]** (cosine: {:.3})\n  {}\n",
                score,
                snippet.trim()
            );
        }
        println!(
            "\n_(Embeddings via {model} cached at .coral-embeddings.json. Pass `--engine tfidf` for offline mode.)_"
        );
    }

    Ok(ExitCode::SUCCESS)
}
