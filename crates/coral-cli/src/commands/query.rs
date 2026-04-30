use anyhow::{Context, Result};
use clap::Args;
use coral_core::walk;
use coral_runner::{Prompt, Runner};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct QueryArgs {
    /// The question to ask the wiki.
    pub question: String,
    /// Optional model override (e.g., "sonnet", "haiku", or full id).
    #[arg(long)]
    pub model: Option<String>,
}

pub fn run(args: QueryArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let runner = super::runner_helper::default_runner();
    run_with_runner(args, wiki_root, runner.as_ref())
}

pub fn run_with_runner(
    args: QueryArgs,
    wiki_root: Option<&Path>,
    runner: &dyn Runner,
) -> Result<ExitCode> {
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

    let mut context = String::from("Wiki snapshot (slug, type, body excerpt):\n\n");
    for p in pages.iter().take(40) {
        context.push_str(&format!(
            "- {} ({}): {}\n",
            p.frontmatter.slug,
            slug_type_str(&p.frontmatter),
            p.body
                .chars()
                .take(200)
                .collect::<String>()
                .replace('\n', " ")
        ));
    }

    let prompt = Prompt {
        system: Some(QUERY_SYSTEM.to_string()),
        user: format!(
            "{context}\n\nQuestion: {}\n\nAnswer concisely. Cite the page slugs you used in brackets like [[slug]] at the end.",
            args.question
        ),
        model: args.model,
        cwd: None,
        timeout: None,
    };

    let out = runner
        .run(&prompt)
        .map_err(|e| anyhow::anyhow!("runner failed: {e}"))?;
    println!("{}", out.stdout);
    Ok(ExitCode::SUCCESS)
}

const QUERY_SYSTEM: &str = "You are the Coral wiki bibliotecario. Answer questions using only the wiki snapshot provided. Be terse and cite slugs.";

fn slug_type_str(fm: &coral_core::frontmatter::Frontmatter) -> String {
    serde_json::to_value(fm.page_type)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use coral_runner::MockRunner;
    use tempfile::TempDir;

    fn make_wiki_dir() -> TempDir {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        std::fs::create_dir_all(wiki.join("modules")).unwrap();
        std::fs::write(
            wiki.join("modules/order.md"),
            "---\nslug: order\ntype: module\nlast_updated_commit: abc\nconfidence: 0.8\nstatus: reviewed\n---\n\nOrder feature.",
        )
        .unwrap();
        tmp
    }

    #[test]
    fn query_invokes_runner_and_prints_response() {
        let tmp = make_wiki_dir();
        let wiki = tmp.path().join(".wiki");
        let runner = MockRunner::new();
        runner.push_ok("Order is created via POST /orders. [[order]]");
        let exit = run_with_runner(
            QueryArgs {
                question: "How is an order created?".into(),
                model: None,
            },
            Some(wiki.as_path()),
            &runner,
        )
        .unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].user.contains("How is an order created?"));
        assert!(calls[0].user.contains("order"));
    }

    #[test]
    fn query_fails_when_wiki_missing() {
        let tmp = TempDir::new().unwrap();
        let wiki = tmp.path().join(".wiki");
        let runner = MockRunner::new();
        let res = run_with_runner(
            QueryArgs {
                question: "x".into(),
                model: None,
            },
            Some(wiki.as_path()),
            &runner,
        );
        assert!(res.is_err());
    }
}
