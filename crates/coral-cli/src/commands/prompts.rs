//! `coral prompts` — inspect prompt sources (local override, embedded, fallback).

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::Path;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct PromptsArgs {
    #[command(subcommand)]
    pub action: PromptsAction,
}

#[derive(Subcommand, Debug)]
pub enum PromptsAction {
    /// List all known prompts with their resolved source.
    List,
}

pub fn run(args: PromptsArgs, _wiki_root: Option<&Path>) -> Result<ExitCode> {
    match args.action {
        PromptsAction::List => list_action(),
    }
}

fn list_action() -> Result<ExitCode> {
    use super::prompt_loader::{PromptSource, list_prompts};
    let prompts = list_prompts();
    println!("# Prompts\n");
    println!("| Name | Source |");
    println!("|------|--------|");
    for p in prompts {
        let source = match &p.source {
            PromptSource::Local(path) => format!("local ({})", path.display()),
            PromptSource::Embedded(key) => format!("embedded ({key})"),
            PromptSource::Fallback => "fallback (in code)".to_string(),
        };
        println!("| {} | {} |", p.name, source);
    }
    Ok(ExitCode::SUCCESS)
}
