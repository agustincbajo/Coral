//! `coral wiki at <ref>` — time-travel wiki access.
//!
//! Extracts the wiki directory as it existed at any git ref (tag, commit,
//! branch) and presents it for querying. ★ killer feature #3 from the
//! v0.24 PRD.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

#[derive(Args, Debug)]
pub struct WikiArgs {
    #[command(subcommand)]
    pub command: WikiCmd,
}

#[derive(Subcommand, Debug)]
pub enum WikiCmd {
    /// View the wiki as it existed at a git ref (tag, commit, branch).
    At(AtArgs),
}

#[derive(Args, Debug)]
pub struct AtArgs {
    /// Git ref to check out (tag, branch, commit SHA).
    pub git_ref: String,

    /// Show only pages matching this slug pattern (substring match).
    #[arg(long)]
    pub filter: Option<String>,

    /// Show full page content for matching pages (otherwise just summary).
    #[arg(long)]
    pub full: bool,

    /// Search the historical wiki with this query (BM25).
    #[arg(long)]
    pub search: Option<String>,

    /// Number of search results to return.
    #[arg(long, default_value = "10")]
    pub limit: usize,
}

pub fn run(args: WikiArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.command {
        WikiCmd::At(at_args) => run_at(at_args, wiki_root),
    }
}

fn run_at(args: AtArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let wiki_rel = wiki_root
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".wiki".to_string());

    // Verify the ref exists
    let ref_check = Command::new("git")
        .args(["rev-parse", "--verify", &format!("{}^{{commit}}", args.git_ref)])
        .output()
        .context("failed to run git rev-parse")?;
    if !ref_check.status.success() {
        bail!(
            "git ref '{}' does not resolve to a commit in this repository",
            args.git_ref
        );
    }
    let resolved_sha = String::from_utf8_lossy(&ref_check.stdout)
        .trim()
        .to_string();

    // Check that the wiki directory exists at that ref
    let tree_check = Command::new("git")
        .args([
            "ls-tree",
            "--name-only",
            &args.git_ref,
            &format!("{}/", wiki_rel),
        ])
        .output()
        .context("failed to run git ls-tree")?;
    if !tree_check.status.success() || tree_check.stdout.is_empty() {
        bail!(
            "no '{}' directory found at ref '{}'",
            wiki_rel,
            args.git_ref
        );
    }

    // Extract wiki at that ref to a temp directory using git archive + tar
    let tmp = tempfile::TempDir::new().context("failed to create temp directory")?;
    let archive_output = Command::new("git")
        .args(["archive", &args.git_ref, "--", &wiki_rel])
        .output()
        .context("failed to run git archive")?;
    if !archive_output.status.success() {
        bail!(
            "git archive failed: {}",
            String::from_utf8_lossy(&archive_output.stderr)
        );
    }

    // Pipe the tar archive into tar for extraction
    let mut tar_proc = Command::new("tar")
        .args(["xf", "-", "-C"])
        .arg(tmp.path())
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to spawn tar")?;
    tar_proc
        .stdin
        .as_mut()
        .expect("stdin piped")
        .write_all(&archive_output.stdout)
        .context("failed to write to tar stdin")?;
    let tar_status = tar_proc.wait().context("tar failed")?;
    if !tar_status.success() {
        bail!("tar extraction failed");
    }

    let extracted_wiki = tmp.path().join(&wiki_rel);
    if !extracted_wiki.exists() {
        bail!("wiki directory not found after extraction");
    }

    // Read pages from the extracted wiki
    let pages = coral_core::walk::read_pages(&extracted_wiki)
        .context("failed to read pages from historical wiki")?;

    // Header
    let short_sha = &resolved_sha[..7.min(resolved_sha.len())];
    eprintln!(
        "coral wiki at {} ({}) — {} pages",
        args.git_ref,
        short_sha,
        pages.len()
    );
    eprintln!();

    // If search is requested, do BM25 search
    if let Some(query) = &args.search {
        let results = coral_core::search::search_bm25(&pages, query, args.limit);
        if results.is_empty() {
            eprintln!("  No results for '{query}'");
        } else {
            for r in &results {
                println!("  {:.4}  {}", r.score, r.slug);
                if !r.snippet.is_empty() {
                    let snippet: String = r.snippet.chars().take(120).collect();
                    println!("         {}", snippet.replace('\n', " "));
                }
            }
        }
        return Ok(ExitCode::SUCCESS);
    }

    // Otherwise, list pages (with optional filter)
    let mut shown = 0usize;
    for page in &pages {
        let slug = &page.frontmatter.slug;
        if let Some(ref filter) = args.filter {
            if !slug.contains(filter.as_str()) {
                continue;
            }
        }
        if args.full {
            println!("--- {} ---", slug);
            println!("{}", page.body);
            println!();
        } else {
            println!(
                "  {:40} {:10} conf={:.1} sources={}",
                slug,
                format!("{:?}", page.frontmatter.page_type),
                page.frontmatter.confidence.as_f64(),
                page.frontmatter.sources.len()
            );
        }
        shown += 1;
    }
    if !args.full {
        eprintln!("\n  Showing {shown}/{} pages", pages.len());
    }

    Ok(ExitCode::SUCCESS)
}
