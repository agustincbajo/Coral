//! `coral test discover [--emit yaml] [--commit]` — auto-generate
//! TestCases from OpenAPI specs found in the project's repos. No LLM.
//!
//! Default: print a markdown summary to stdout.
//! `--emit yaml`: print the YAML test suites that would be written.
//! `--commit`: write each suite under `.coral/tests/discovered/`.

use anyhow::{Context, Result};
use clap::Args;
use coral_test::discover::DiscoveredCase;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;

#[derive(Args, Debug)]
pub struct DiscoverArgs {
    #[arg(long, default_value = "markdown")]
    pub emit: Emit,

    /// Write the generated YAML test suites under
    /// `.coral/tests/discovered/<service>.<sha8>.yaml`. Mutually
    /// exclusive with `--emit yaml` (which prints to stdout).
    #[arg(long)]
    pub commit: bool,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum Emit {
    Markdown,
    Yaml,
}

pub fn run(args: DiscoverArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    let cases = coral_test::discover_openapi_in_project(&project.root)
        .context("discovering OpenAPI specs")?;

    if cases.is_empty() {
        println!("no OpenAPI specs found under {}", project.root.display());
        return Ok(ExitCode::SUCCESS);
    }

    if args.commit {
        let out_dir = project.root.join(".coral/tests/discovered");
        std::fs::create_dir_all(&out_dir)
            .with_context(|| format!("creating discovered tests dir {}", out_dir.display()))?;
        let mut written = 0usize;
        for d in &cases {
            let yaml = serde_yaml_ng::to_string(&d.suite).context("serializing suite")?;
            let filename = sanitize_filename(&d.case.id);
            let path = out_dir.join(format!("{filename}.yaml"));
            coral_core::atomic::atomic_write_string(&path, &yaml)
                .with_context(|| format!("writing {}", path.display()))?;
            written += 1;
        }
        println!(
            "✔ wrote {written} discovered test(s) to {}",
            out_dir.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    match args.emit {
        Emit::Markdown => print_markdown(&cases),
        Emit::Yaml => print_yaml(&cases)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn print_markdown(cases: &[DiscoveredCase]) {
    println!("# discovered test cases\n");
    println!("| id | service | source spec |");
    println!("|----|---------|-------------|");
    for d in cases {
        println!(
            "| {} | {} | {} |",
            d.case.id,
            d.case.service.as_deref().unwrap_or("—"),
            d.source_spec.display()
        );
    }
}

fn print_yaml(cases: &[DiscoveredCase]) -> Result<()> {
    for d in cases {
        let yaml = serde_yaml_ng::to_string(&d.suite).context("serializing suite")?;
        println!("---\n# from {}\n{}", d.source_spec.display(), yaml);
    }
    Ok(())
}

fn sanitize_filename(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.len() > 80 {
        out.truncate(80);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_replaces_unsafe_chars() {
        assert_eq!(
            sanitize_filename("openapi:GET:/users"),
            "openapi_GET__users"
        );
        assert_eq!(sanitize_filename("simple"), "simple");
    }
}
