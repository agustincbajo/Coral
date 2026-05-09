//! `coral monitor history --env NAME --monitor NAME [--tail N]` (v0.23.1).
//!
//! Reads the JSONL file at `.coral/monitors/<env>-<monitor>.jsonl`
//! and prints the last N lines (default 20). No JSON parsing — the
//! file is line-oriented by construction; we just stream raw bytes.
//! Each line is one `MonitorRun` and stays human-readable so an
//! operator can `cat`/`tail` the file directly without `coral`.

use anyhow::{Context, Result};
use clap::Args;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;
use crate::commands::monitor::run::jsonl_path;

#[derive(Args, Debug)]
pub struct HistoryArgs {
    #[arg(long)]
    pub env: String,
    #[arg(long = "monitor")]
    pub monitor: String,
    /// Print the last N lines (default 20).
    #[arg(long, default_value_t = 20)]
    pub tail: usize,
}

pub fn run(args: HistoryArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    let path = jsonl_path(&project.root, &args.env, &args.monitor);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!(
                "no JSONL file at {}; has `coral monitor up --env {} --monitor {}` been run?",
                path.display(),
                args.env,
                args.monitor
            );
            return Ok(ExitCode::from(2));
        }
        Err(e) => {
            return Err(e).with_context(|| format!("reading {}", path.display()));
        }
    };
    for line in tail_lines(&text, args.tail) {
        println!("{line}");
    }
    Ok(ExitCode::SUCCESS)
}

/// Return the last `n` non-empty lines of `text` in original order.
/// Empty lines (blank trailing newlines) are skipped — the JSONL
/// format treats them as no-ops.
pub(crate) fn tail_lines(text: &str, n: usize) -> Vec<&str> {
    let mut lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() > n {
        let drop = lines.len() - n;
        lines.drain(0..drop);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **T5 — history --tail 5** on a fixture with 10 lines returns
    /// lines 6..=10 in original order.
    #[test]
    fn tail_lines_returns_last_n() {
        let mut text = String::new();
        for i in 1..=10 {
            text.push_str(&format!("line {i}\n"));
        }
        let last5 = tail_lines(&text, 5);
        assert_eq!(
            last5,
            vec!["line 6", "line 7", "line 8", "line 9", "line 10"]
        );
    }

    #[test]
    fn tail_lines_handles_fewer_lines_than_n() {
        let text = "a\nb\n";
        let lines = tail_lines(text, 5);
        assert_eq!(lines, vec!["a", "b"]);
    }

    #[test]
    fn tail_lines_skips_blank_trailing_newlines() {
        let text = "x\n\n\ny\n\n";
        let lines = tail_lines(text, 5);
        assert_eq!(lines, vec!["x", "y"]);
    }

    #[test]
    fn tail_lines_handles_empty() {
        assert!(tail_lines("", 5).is_empty());
    }
}
