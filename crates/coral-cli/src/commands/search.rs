use anyhow::{Context, Result};
use clap::Args;
use coral_core::{search, walk};
use coral_runner::{
    DEFAULT_OPENAI_DIM, DEFAULT_OPENAI_MODEL, DEFAULT_VOYAGE_DIM, DEFAULT_VOYAGE_MODEL,
    EmbeddingsProvider, OpenAIProvider, VoyageProvider,
};
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
    /// (semantic, requires the selected provider's API key).
    #[arg(long, default_value = "tfidf")]
    pub engine: String,
    /// Ranking algorithm for the TF-IDF/BM25 family. `tfidf` (default) or `bm25`.
    /// Both work offline, no API key. BM25 has better precision on 100+ page wikis.
    /// Ignored when `--engine embeddings` is set (the embeddings engine has its
    /// own ranking).
    #[arg(long, default_value = "tfidf")]
    pub algorithm: String,
    /// Force a re-embed of all pages, ignoring the cached vectors.
    #[arg(long)]
    pub reindex: bool,
    /// Embeddings provider when `--engine embeddings`: `voyage` (default,
    /// requires VOYAGE_API_KEY) or `openai` (requires OPENAI_API_KEY).
    #[arg(long, default_value = "voyage")]
    pub embeddings_provider: String,
    /// Optional embeddings model override. Default depends on provider:
    /// `voyage-3` (voyage) or `text-embedding-3-small` (openai).
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
        "embeddings" => {
            let provider = build_embeddings_provider(&args)?;
            run_embeddings(&pages, &args, &root, provider.as_ref())
        }
        other => anyhow::bail!("unknown engine: {other}. Choose: tfidf | embeddings"),
    }
}

fn build_embeddings_provider(args: &SearchArgs) -> Result<Box<dyn EmbeddingsProvider>> {
    match args.embeddings_provider.as_str() {
        "voyage" => {
            let api_key = std::env::var("VOYAGE_API_KEY").context(
                "VOYAGE_API_KEY required for --embeddings-provider voyage (or use --engine tfidf)",
            )?;
            let model = args
                .embeddings_model
                .clone()
                .unwrap_or_else(|| DEFAULT_VOYAGE_MODEL.into());
            Ok(Box::new(VoyageProvider::new(
                api_key,
                model,
                DEFAULT_VOYAGE_DIM,
            )))
        }
        "openai" => {
            let api_key = std::env::var("OPENAI_API_KEY")
                .context("OPENAI_API_KEY required for --embeddings-provider openai")?;
            let model = args
                .embeddings_model
                .clone()
                .unwrap_or_else(|| DEFAULT_OPENAI_MODEL.into());
            // text-embedding-3-large is 3072-dim; everything else defaults to 1536.
            let dim = if model == "text-embedding-3-large" {
                3072
            } else {
                DEFAULT_OPENAI_DIM
            };
            Ok(Box::new(OpenAIProvider::new(api_key, model, dim)))
        }
        other => anyhow::bail!("unknown --embeddings-provider: {other}. Choose: voyage | openai"),
    }
}

fn run_tfidf(pages: &[coral_core::page::Page], args: &SearchArgs) -> Result<ExitCode> {
    let results = match args.algorithm.as_str() {
        "tfidf" => search::search(pages, &args.query, args.limit),
        "bm25" => search::search_bm25(pages, &args.query, args.limit),
        other => anyhow::bail!(
            "unknown --algorithm: {other}. Choose: tfidf | bm25 (or pass --engine embeddings for semantic search)"
        ),
    };

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
            "\n_(Offline {} ranking. Pass `--algorithm bm25` (or `tfidf`) to switch ranking, or `--engine embeddings` for semantic search via Voyage AI.)_",
            args.algorithm
        );
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn run_embeddings(
    pages: &[coral_core::page::Page],
    args: &SearchArgs,
    wiki_root: &Path,
    provider: &dyn EmbeddingsProvider,
) -> Result<ExitCode> {
    use coral_core::cache::WalkCache;
    use coral_core::embeddings::EmbeddingsIndex;

    let model = provider.name();

    let mut index = EmbeddingsIndex::load(wiki_root)?;
    if index.dim == 0 || index.provider != model {
        index = EmbeddingsIndex::empty(model, provider.dim());
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
        let vectors = provider
            .embed_batch(&texts, Some("document"))
            .map_err(|e| anyhow::anyhow!("embedding pages: {e}"))?;
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
    let query_vecs = provider
        .embed_batch(std::slice::from_ref(&args.query), Some("query"))
        .map_err(|e| anyhow::anyhow!("embedding query: {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use coral_core::page::Page;
    use coral_runner::MockEmbeddingsProvider;
    use tempfile::TempDir;

    fn page(slug: &str, body: &str) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/modules/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type: PageType::Module,
                last_updated_commit: "abc".to_string(),
                confidence: Confidence::try_new(0.7).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Reviewed,
                generated_at: None,
                extra: Default::default(),
            },
            body: body.to_string(),
        }
    }

    fn write_md_page(path: &std::path::Path, slug: &str, body: &str) {
        let content = format!(
            "---\nslug: {slug}\ntype: module\nlast_updated_commit: abc123\nconfidence: 0.7\nsources:\n  - src/{slug}.rs\nstatus: reviewed\n---\n\n{body}\n"
        );
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn run_with_algorithm_bm25_succeeds_end_to_end() {
        // End-to-end: tempdir wiki + 3 pages + run(SearchArgs { algorithm: bm25 }).
        // Verifies the --algorithm bm25 path is wired through `run` → `run_tfidf`
        // → `search::search_bm25` and produces SUCCESS exit code with parseable
        // JSON output.
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        write_md_page(
            &wiki.join("modules/outbox.md"),
            "outbox",
            "the outbox dispatcher polls every second",
        );
        write_md_page(
            &wiki.join("modules/order.md"),
            "order",
            "order module references the outbox",
        );
        write_md_page(
            &wiki.join("modules/unrelated.md"),
            "unrelated",
            "lorem ipsum dolor sit amet",
        );

        let args = SearchArgs {
            query: "outbox".into(),
            limit: 5,
            format: "json".into(),
            engine: "tfidf".into(),
            algorithm: "bm25".into(),
            reindex: false,
            embeddings_provider: "voyage".into(),
            embeddings_model: None,
        };
        let exit = run(args, Some(&wiki)).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    #[test]
    fn run_with_unknown_algorithm_errors() {
        // Defensive: the CLI must reject unknown ranking names rather than
        // silently falling through to a default.
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        write_md_page(&wiki.join("modules/x.md"), "x", "outbox");

        let args = SearchArgs {
            query: "outbox".into(),
            limit: 5,
            format: "json".into(),
            engine: "tfidf".into(),
            algorithm: "totally-bogus".into(),
            reindex: false,
            embeddings_provider: "voyage".into(),
            embeddings_model: None,
        };
        let err = run(args, Some(&wiki)).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("totally-bogus") || msg.contains("algorithm"),
            "unexpected error message: {msg}"
        );
    }

    #[test]
    fn run_embeddings_uses_swappable_provider_via_trait() {
        // The whole point of the v0.4 trait: search runs against any
        // EmbeddingsProvider, not just Voyage. This test runs the embeddings
        // path end-to-end against a deterministic Mock so it works offline
        // and never touches the network.
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path();
        let pages = vec![
            page("outbox", "the outbox dispatcher polls every second"),
            page("query", "answers go through the search pipeline"),
        ];
        let provider = MockEmbeddingsProvider::new(64);
        let args = SearchArgs {
            query: "outbox".into(),
            limit: 5,
            format: "json".into(),
            engine: "embeddings".into(),
            algorithm: "tfidf".into(),
            reindex: false,
            embeddings_provider: "voyage".into(),
            embeddings_model: None,
        };
        let exit = run_embeddings(&pages, &args, wiki, &provider).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        // The cache file should have been written with the mock's name.
        let cache_path = wiki.join(".coral-embeddings.json");
        assert!(cache_path.exists(), "cache file was not written");
        let cache = std::fs::read_to_string(&cache_path).unwrap();
        assert!(
            cache.contains("\"provider\""),
            "cache missing provider field: {cache}"
        );
        assert!(
            cache.contains("\"mock-64\""),
            "cache should record the mock provider name: {cache}"
        );
    }
}
