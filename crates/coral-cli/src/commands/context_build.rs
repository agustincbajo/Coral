//! `coral context build --query <q> --budget <tokens>`
//!
//! Smart context loader: greedy fill of the `--budget` token target
//! by ranking wiki pages via TF-IDF (already in coral-core), then
//! BFS-walking `backlinks` to pull adjacent context, sorted by
//! confidence. Output is a single Markdown blob ready to paste into
//! any prompt — Claude, Cursor, ChatGPT, what-have-you.
//!
//! Differentiator (PRD §3.7): full-context Opus 1M is expensive and
//! suffers context rot; pure RAG needs a vector DB; this loader is
//! curated structured note-taking under an explicit budget.

use anyhow::{Context, Result};
use clap::Args;
use coral_core::walk;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;

#[derive(Args, Debug)]
pub struct ContextBuildArgs {
    /// The natural-language query that ranks pages.
    #[arg(long)]
    pub query: String,

    /// Token budget. Approximate: we use 4 chars/token as a heuristic
    /// (works well enough for English Markdown; the model will count
    /// more precisely on its end).
    #[arg(long, default_value_t = 50_000)]
    pub budget: usize,

    /// Output format. Markdown is the default (paste-friendly); JSON
    /// is for programmatic callers.
    #[arg(long, default_value = "markdown")]
    pub format: Format,

    /// How many seed pages to pull before expanding via backlinks.
    /// Default 8 covers most "explain X" queries; raise for broader
    /// surveys.
    #[arg(long, default_value_t = 8)]
    pub seeds: usize,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum Format {
    Markdown,
    Json,
}

pub fn run(args: ContextBuildArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    let pages = walk::read_pages(project.wiki_root())
        .with_context(|| format!("walking wiki at {}", project.wiki_root().display()))?;
    if pages.is_empty() {
        anyhow::bail!(
            "no wiki pages found at {}; run `coral init` first",
            project.wiki_root().display()
        );
    }

    let ranking = rank_pages(&pages, &args.query);
    let mut selected_slugs: Vec<String> = ranking
        .iter()
        .take(args.seeds)
        .map(|(slug, _)| slug.clone())
        .collect();

    // BFS over backlinks until budget exhausts. We approximate the
    // "tokens used so far" by counting characters / 4.
    let mut included = std::collections::BTreeSet::new();
    included.extend(selected_slugs.iter().cloned());
    let mut chars_used = 0usize;
    let budget_chars = args.budget.saturating_mul(4);

    let mut pending: std::collections::VecDeque<String> = selected_slugs.iter().cloned().collect();
    while let Some(slug) = pending.pop_front() {
        let page = match pages.iter().find(|p| p.frontmatter.slug == slug) {
            Some(p) => p,
            None => continue,
        };
        chars_used += page.body.len();
        if chars_used >= budget_chars {
            break;
        }
        for backlink in &page.frontmatter.backlinks {
            if !included.contains(backlink) {
                included.insert(backlink.clone());
                pending.push_back(backlink.clone());
                selected_slugs.push(backlink.clone());
            }
        }
    }

    // Filter selection back to pages we actually have on disk +
    // re-sort by `(confidence desc, body length asc)` so the prompt
    // is led by the most-trusted concise sources.
    let mut result_pages: Vec<&coral_core::page::Page> = pages
        .iter()
        .filter(|p| included.contains(&p.frontmatter.slug))
        .collect();
    result_pages.sort_by(|a, b| {
        b.frontmatter
            .confidence
            .as_f64()
            .partial_cmp(&a.frontmatter.confidence.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.body.len().cmp(&b.body.len()))
    });

    match args.format {
        Format::Markdown => print_markdown(&args.query, &result_pages, args.budget),
        Format::Json => print_json(&args.query, &result_pages)?,
    }

    Ok(ExitCode::SUCCESS)
}

/// TF-IDF-style ranking — minimal hand-rolled: each page scored by
/// the count of query terms found in its body + slug. Identical to
/// the heuristic `coral search` uses internally so the output is
/// comparable across commands.
fn rank_pages(pages: &[coral_core::page::Page], query: &str) -> Vec<(String, f64)> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() >= 2)
        .collect();
    let mut scored: Vec<(String, f64)> = pages
        .iter()
        .map(|p| {
            let haystack = format!("{} {}", p.frontmatter.slug, p.body).to_lowercase();
            let raw_score: usize = terms
                .iter()
                .map(|t| haystack.matches(t.as_str()).count())
                .sum();
            let length_norm = (p.body.len() as f64).sqrt().max(1.0);
            let score = (raw_score as f64) / length_norm;
            (p.frontmatter.slug.clone(), score)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    scored
}

fn print_markdown(query: &str, pages: &[&coral_core::page::Page], budget: usize) {
    println!("# Coral context for: \"{query}\"\n");
    println!(
        "_Loaded {} page(s) under a budget of {} tokens (~4 chars/token)._\n",
        pages.len(),
        budget
    );
    for p in pages {
        println!("---\n");
        println!(
            "## {} (confidence={:.2})",
            p.frontmatter.slug,
            p.frontmatter.confidence.as_f64()
        );
        if !p.frontmatter.sources.is_empty() {
            println!("\n**Sources:** {}\n", p.frontmatter.sources.join(", "));
        }
        println!("{}\n", p.body.trim());
    }
}

fn print_json(query: &str, pages: &[&coral_core::page::Page]) -> Result<()> {
    let json = serde_json::json!({
        "query": query,
        "pages": pages
            .iter()
            .map(|p| serde_json::json!({
                "slug": p.frontmatter.slug,
                "confidence": p.frontmatter.confidence.as_f64(),
                "sources": p.frontmatter.sources,
                "body": p.body
            }))
            .collect::<Vec<_>>()
    });
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}
