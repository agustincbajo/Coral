//! `coral export-agents --format <agents-md|claude-md|cursor-rules|copilot|llms-txt>`
//!
//! **Manifest-driven, NOT LLM-driven.** Empirical context-engineering
//! guidance (see [Anthropic's published recommendations][ace]) consistently
//! finds that LLM-synthesised AGENTS.md files degrade agent task success
//! vs. deterministic templates rendered from structured config.
//!
//! Today (v0.19.3) the renderer reads only the fields the manifest parser
//! actually exposes: `[project.name]`, `[[repos]] {name, depends_on,
//! tags}`, and the manifest path itself. The output points agents at the
//! wiki via `coral mcp serve` for content. The `[project.agents_md]` and
//! `[hooks]` blocks promised by earlier drafts of this docstring are not
//! parsed yet — those land in v0.20+ when `MultiStepRunner` does too.
//!
//! [ace]: https://www.anthropic.com/engineering/context-engineering

use anyhow::{Context, Result};
use clap::Args;
use coral_core::project::Project;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;

/// `coral export-agents --format <fmt>` arguments dedicated to the
/// agents-md family. Different from the existing `coral export` (wiki
/// bundle export — see `commands::export`); we keep `export-agents` as
/// its own top-level subcommand to avoid confusing the two namespaces.
#[derive(Args, Debug)]
pub struct ExportAgentsArgs {
    #[arg(long, default_value = "agents-md")]
    pub format: AgentFormat,

    /// Write to a default path instead of stdout. The default path
    /// depends on `--format`:
    /// - `agents-md` → `AGENTS.md`
    /// - `claude-md` → `CLAUDE.md`
    /// - `cursor-rules` → `.cursor/rules/coral.mdc`
    /// - `copilot` → `.github/copilot-instructions.md`
    /// - `llms-txt` → `llms.txt`
    #[arg(long)]
    pub write: bool,

    /// Override the destination path (only meaningful with `--write`).
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
}

#[derive(clap::ValueEnum, Clone, Debug, Copy)]
pub enum AgentFormat {
    AgentsMd,
    ClaudeMd,
    CursorRules,
    Copilot,
    LlmsTxt,
}

pub fn run(args: ExportAgentsArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    let body = render(&project, args.format);
    if args.write {
        let target = args
            .out
            .unwrap_or_else(|| default_path(&project, args.format));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating dir {}", parent.display()))?;
        }
        coral_core::atomic::atomic_write_string(&target, &body)
            .with_context(|| format!("writing {}", target.display()))?;
        println!("✔ wrote {}", target.display());
    } else {
        print!("{body}");
    }
    Ok(ExitCode::SUCCESS)
}

fn default_path(project: &Project, fmt: AgentFormat) -> std::path::PathBuf {
    let root = project.root.clone();
    match fmt {
        AgentFormat::AgentsMd => root.join("AGENTS.md"),
        AgentFormat::ClaudeMd => root.join("CLAUDE.md"),
        AgentFormat::CursorRules => root.join(".cursor/rules/coral.mdc"),
        AgentFormat::Copilot => root.join(".github/copilot-instructions.md"),
        AgentFormat::LlmsTxt => root.join("llms.txt"),
    }
}

pub fn render(project: &Project, fmt: AgentFormat) -> String {
    match fmt {
        AgentFormat::AgentsMd => render_agents_md(project),
        AgentFormat::ClaudeMd => render_claude_md(project),
        AgentFormat::CursorRules => render_cursor_rules(project),
        AgentFormat::Copilot => render_copilot(project),
        AgentFormat::LlmsTxt => render_llms_txt(project),
    }
}

/// Escape a free-text token (project name, repo name) for safe
/// interpolation into rendered Markdown.
///
/// v0.20.2 audit-followup #47: pre-fix, an attacker who controlled
/// `[project.name]` (e.g. via a poisoned `coral.toml` in a forked
/// repo) could land arbitrary Markdown in AGENTS.md / CLAUDE.md by
/// embedding `\n## malicious heading` or backticks/asterisks.
/// Coding agents read every byte of these files; a subtle injection
/// could nudge LLM behavior.
///
/// The escape rules:
///
/// - **Newlines (`\n`, `\r`, `\r\n`)** become the literal two-character
///   sequence `\n` so the name stays on its own line. This is the
///   load-bearing fix — without it `name = "evil\n## new section"`
///   creates a real Markdown heading.
/// - **Backticks** are escaped as `\``. Otherwise a name like
///   ``evil`code` `` would close an inline-code span and reopen with
///   attacker-controlled content.
/// - **Markdown emphasis chars** (`*`, `_`) are escaped via backslash.
///   Single occurrences would turn into bold/italic; doubled
///   occurrences would too.
/// - **Brackets and parens** (`[`, `]`, `(`, `)`) are escaped so a
///   name like `[[wikilink]]` doesn't render as a link.
/// - **Backslashes** are escaped first so the other escapes survive.
///
/// The output is suitable for direct interpolation into Markdown
/// running text. It is NOT suitable for HTML output (we don't escape
/// `<` / `>`); the agents-md / claude-md / cursor-rules / copilot /
/// llms-txt formats are all Markdown.
pub(crate) fn escape_markdown_token(name: &str) -> String {
    // Pre-allocate generously — most names are short and don't
    // contain any of the special chars below.
    let mut out = String::with_capacity(name.len() + 8);
    // Process \r\n as a single linebreak so we emit one `\n` literal
    // instead of two.
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.push('\\');
                out.push('\\');
            }
            '\r' => {
                // Collapse \r\n → \n (then escape).
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str("\\n");
            }
            '\n' => out.push_str("\\n"),
            // Inline-code delimiter — escape so the closing backtick
            // can't break out into attacker-controlled content.
            '`' => {
                out.push('\\');
                out.push('`');
            }
            // Emphasis chars — escape singly. Doubled (`**`, `__`)
            // forms are also covered because we escape each char.
            '*' | '_' => {
                out.push('\\');
                out.push(c);
            }
            // Link / wikilink syntax — escape so the name can't
            // become a clickable link or `[[wikilink]]`.
            '[' | ']' | '(' | ')' => {
                out.push('\\');
                out.push(c);
            }
            other => out.push(other),
        }
    }
    out
}

fn render_agents_md(project: &Project) -> String {
    let mut out = String::new();
    out.push_str("# AGENTS.md\n\n");
    out.push_str(&format!(
        "_Generated by `coral export-agents --format agents-md` from {}_\n\n",
        project
            .manifest_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("coral.toml")
    ));

    out.push_str("## Project\n\n");
    // v0.20.2 audit-followup #47: escape project / repo names so a
    // poisoned `coral.toml` (e.g. `name = "evil\n## injection"`)
    // can't land arbitrary Markdown in AGENTS.md / CLAUDE.md. See
    // `escape_markdown_token` for the escape rules.
    out.push_str(&format!(
        "- name: `{}`\n",
        escape_markdown_token(&project.name)
    ));
    out.push_str(&format!("- repos: {}\n", project.repos.len()));
    out.push('\n');

    out.push_str("## Repos\n\n");
    if project.repos.is_empty() {
        out.push_str("_(no repos declared)_\n\n");
    } else {
        out.push_str("| name | tags | depends_on |\n");
        out.push_str("|------|------|------------|\n");
        for r in &project.repos {
            let tags = if r.tags.is_empty() {
                "—".to_string()
            } else {
                r.tags.join(",")
            };
            let deps = if r.depends_on.is_empty() {
                "—".to_string()
            } else {
                r.depends_on.join(",")
            };
            out.push_str(&format!(
                "| `{}` | {} | {} |\n",
                escape_markdown_token(&r.name),
                tags,
                deps
            ));
        }
        out.push('\n');
    }

    out.push_str("## Build & test commands\n\n");
    out.push_str("```bash\n");
    out.push_str("coral up --env dev          # bring up the dev environment\n");
    out.push_str("coral verify                # liveness check (<30s)\n");
    out.push_str("coral test --tag smoke      # functional smoke (<2min)\n");
    out.push_str("coral test                  # full suite\n");
    out.push_str("coral down                  # tear down\n");
    out.push_str("```\n\n");

    out.push_str("## Project context\n\n");
    out.push_str(
        "Coral exposes the wiki + manifest as a Model Context Protocol server. \
Agents that speak MCP can connect via `coral mcp serve` (stdio transport) and read:\n\n",
    );
    out.push_str("- `coral://manifest` — coral.toml as JSON\n");
    out.push_str("- `coral://lock` — coral.lock with resolved SHAs\n");
    out.push_str("- `coral://graph` — repo dependency graph\n");
    out.push_str("- `coral://wiki/_index` — aggregated wiki listing\n");
    out.push_str("- `coral://stats` — wiki health stats\n");
    out.push_str("- `coral://test-report/latest` — last test run\n\n");

    out.push_str("## Conventions\n\n");
    out.push_str(
        "- v0.16+ multi-repo: edits to `coral.toml` should keep `[[repos]]` alphabetical.\n",
    );
    out.push_str(
        "- v0.17 environments: declare healthchecks alongside services so `coral verify` works.\n",
    );
    out.push_str("- v0.18 testing: place new YAML suites under `.coral/tests/`, Hurl files use `.hurl` extension.\n");
    out.push('\n');

    out
}

fn render_claude_md(project: &Project) -> String {
    // v0.19.5 audit C6: previously this delegated to render_agents_md
    // and only appended a stanza, leaving the H1 as `# AGENTS.md` and
    // the generation marker pointing at `--format agents-md`. Replace
    // both so a CLAUDE.md file actually self-identifies as such.
    let mut out = render_agents_md(project);
    if let Some(rest) = out.strip_prefix("# AGENTS.md\n") {
        out = format!("# CLAUDE.md\n{rest}");
    }
    out = out.replace(
        "_Generated by `coral export-agents --format agents-md`",
        "_Generated by `coral export-agents --format claude-md`",
    );
    out.push_str("## Coral wiki layout (Claude Code specific)\n\n");
    out.push_str(
        "When `coral mcp serve` is running, Claude Code can read wiki pages on demand. \
The wiki lives at `.wiki/` (single-repo) or aggregated across repos when `coral.toml` \
declares more than one. Slugs are namespaced as `<repo>/<slug>` in multi-repo \
projects.\n\n",
    );
    out
}

fn render_cursor_rules(project: &Project) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    // v0.20.2 audit-followup #47: name lands in YAML frontmatter
    // here. The escape we apply works for Markdown and is also safe
    // for single-quoted YAML scalars (we don't introduce single
    // quotes; we don't drop backslashes that would break the YAML
    // either — the only thing that would break the YAML literal is
    // an unescaped newline, which we collapse to `\n` literally).
    out.push_str(&format!(
        "description: Coral project '{}' conventions\n",
        escape_markdown_token(&project.name)
    ));
    out.push_str("alwaysApply: true\n");
    out.push_str("globs: [\"**/*\"]\n");
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", escape_markdown_token(&project.name)));
    out.push_str(&format!(
        "This is a multi-repo Coral project ({} repo(s)). Run `coral mcp serve` to \
expose the wiki + manifest as MCP resources, then use `coral query \"<question>\"` to \
ground answers in the wiki.\n\n",
        project.repos.len()
    ));
    out.push_str("## Build commands\n\n");
    out.push_str("- `coral up --env dev` — bring up the dev environment\n");
    out.push_str("- `coral verify` — liveness check\n");
    out.push_str("- `coral test --tag smoke` — functional smoke\n");
    out.push_str("- `coral test` — full suite\n");
    out
}

fn render_copilot(project: &Project) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Copilot instructions — {}\n\n",
        escape_markdown_token(&project.name)
    ));
    out.push_str(
        "This repository is managed by [Coral](https://github.com/agustincbajo/Coral). \
The `coral.toml` declares the multi-repo project layout, and `coral mcp serve` exposes \
the wiki as MCP resources to coding agents.\n\n",
    );
    out.push_str("## Conventions\n\n");
    out.push_str(
        "- Prefer `coral query` over reading individual wiki pages — answers are cited.\n",
    );
    out.push_str("- Run `coral verify` before committing changes that touch services.\n");
    out.push_str("- Use `coral test --tag smoke` to validate functional behavior.\n\n");
    out.push_str("## Repos\n\n");
    for r in &project.repos {
        out.push_str(&format!("- `{}`\n", escape_markdown_token(&r.name)));
    }
    out
}

fn render_llms_txt(project: &Project) -> String {
    // https://llmstxt.org/ — a flat index agents fetch as their first
    // pass over a project. We render an index pointing at the wiki +
    // a one-paragraph project summary.
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", escape_markdown_token(&project.name)));
    out.push_str(&format!(
        "> Coral-managed multi-repo project ({} repos). The wiki is the source of truth.\n\n",
        project.repos.len()
    ));
    out.push_str("## Project context\n\n");
    out.push_str(
        "- [coral.toml](coral.toml): manifest declaring `[[repos]]` and `[[environments]]`.\n",
    );
    out.push_str("- [coral.lock](coral.lock): resolved SHAs for reproducibility.\n");
    out.push_str("- [.wiki/index.md](.wiki/index.md): aggregated wiki index across repos.\n\n");
    out.push_str("## How to query\n\n");
    out.push_str("- `coral query \"<question>\"` — LLM-cited answer from the wiki.\n");
    out.push_str("- `coral mcp serve` — Model Context Protocol server (stdio).\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_project() -> Project {
        let mut p = Project::single_repo("demo".into(), std::path::PathBuf::from("/tmp/demo"));
        p.repos[0].tags = vec!["service".into()];
        p.manifest_path = std::path::PathBuf::from("/tmp/demo/coral.toml");
        p
    }

    #[test]
    fn agents_md_includes_project_name_and_commands() {
        let project = fixture_project();
        let body = render(&project, AgentFormat::AgentsMd);
        assert!(body.contains("# AGENTS.md"));
        assert!(body.contains("`demo`"));
        assert!(body.contains("coral up --env dev"));
        assert!(body.contains("coral verify"));
    }

    #[test]
    fn claude_md_extends_agents_md_with_wiki_layout_section() {
        let project = fixture_project();
        let body = render(&project, AgentFormat::ClaudeMd);
        assert!(body.contains("Coral wiki layout"));
    }

    /// v0.19.5 audit C6: previous render_claude_md emitted
    /// `# AGENTS.md` as the top-level header, with the generation
    /// marker pointing at `--format agents-md`. Both must match the
    /// claude-md format now.
    #[test]
    fn claude_md_starts_with_claude_md_header() {
        let project = fixture_project();
        let body = render(&project, AgentFormat::ClaudeMd);
        let first = body.lines().next().unwrap_or("");
        assert_eq!(first, "# CLAUDE.md", "first line should be `# CLAUDE.md`");
        assert!(
            body.contains("`coral export-agents --format claude-md`"),
            "generation marker should reference claude-md format: {body}"
        );
        assert!(
            !body.contains("`coral export-agents --format agents-md`"),
            "generation marker must not reference agents-md format: {body}"
        );
    }

    #[test]
    fn cursor_rules_emits_always_apply_frontmatter() {
        let project = fixture_project();
        let body = render(&project, AgentFormat::CursorRules);
        assert!(body.starts_with("---\n"));
        assert!(body.contains("alwaysApply: true"));
        assert!(body.contains("# demo"));
    }

    #[test]
    fn copilot_format_lists_repos() {
        let project = fixture_project();
        let body = render(&project, AgentFormat::Copilot);
        assert!(body.contains("Copilot instructions"));
        assert!(body.contains("- `demo`"));
    }

    #[test]
    fn llms_txt_format_is_flat_index() {
        let project = fixture_project();
        let body = render(&project, AgentFormat::LlmsTxt);
        assert!(body.starts_with("# demo\n"));
        assert!(body.contains("coral.toml"));
        assert!(body.contains("coral.lock"));
    }

    /// v0.20.2 audit-followup #47: project names with literal
    /// newlines must NOT escape into a fresh Markdown line. Pre-fix
    /// `name = "evil\n## injection"` would land a real heading in
    /// AGENTS.md; post-fix it survives as the literal `\n`
    /// two-character sequence.
    #[test]
    fn agents_md_escapes_newlines_in_project_name() {
        let mut project = fixture_project();
        project.name = "evil\n## injected heading".into();
        let body = render(&project, AgentFormat::AgentsMd);
        // The synthetic heading must NOT appear as a real heading
        // (i.e. there must not be a fresh `## injected heading` line
        // outside our own `## Project` / `## Repos` etc.).
        let lines: Vec<&str> = body.lines().collect();
        assert!(
            !lines.contains(&"## injected heading"),
            "newline-injected heading reached output: {body}"
        );
        // The escaped name should appear with the literal `\n`.
        assert!(
            body.contains("evil\\n## injected heading"),
            "expected literal escaped name in output: {body}"
        );
    }

    /// v0.20.2 audit-followup #47: backticks in a name must NOT
    /// close an inline-code span. Post-fix the backtick is escaped
    /// as `\``.
    #[test]
    fn agents_md_escapes_backticks_in_project_name() {
        let mut project = fixture_project();
        project.name = "evil`code`injection".into();
        let body = render(&project, AgentFormat::AgentsMd);
        // The escaped form is `evil\`code\`injection` inside the
        // backtick-wrapped span.
        assert!(
            body.contains("evil\\`code\\`injection"),
            "expected backtick-escaped name: {body}"
        );
    }

    /// v0.20.2 audit-followup #47: wikilink-shaped name must NOT
    /// render as a real link.
    #[test]
    fn agents_md_escapes_wikilink_brackets_in_project_name() {
        let mut project = fixture_project();
        project.name = "[[evil-link]]".into();
        let body = render(&project, AgentFormat::AgentsMd);
        // Each `[` and `]` should be escaped.
        assert!(
            body.contains("\\[\\[evil-link\\]\\]"),
            "expected escaped brackets in name: {body}"
        );
    }

    /// v0.20.2 audit-followup #47: emphasis chars (`*`, `_`) must
    /// NOT make text bold/italic.
    #[test]
    fn agents_md_escapes_emphasis_chars_in_project_name() {
        let mut project = fixture_project();
        project.name = "name *with* _emphasis_".into();
        let body = render(&project, AgentFormat::AgentsMd);
        assert!(
            body.contains("name \\*with\\* \\_emphasis\\_"),
            "expected escaped emphasis chars: {body}"
        );
    }

    /// v0.20.2 audit-followup #47: \r\n line endings (Windows-style)
    /// must collapse to a single literal `\n`.
    #[test]
    fn agents_md_escapes_crlf_in_project_name() {
        let mut project = fixture_project();
        project.name = "evil\r\n## crlf injection".into();
        let body = render(&project, AgentFormat::AgentsMd);
        let lines: Vec<&str> = body.lines().collect();
        assert!(
            !lines.contains(&"## crlf injection"),
            "CRLF-injected heading reached output: {body}"
        );
        // Should be exactly ONE `\n` literal, not two (the \r is
        // collapsed into the \n).
        assert!(
            body.contains("evil\\n## crlf injection"),
            "expected single \\n literal: {body}"
        );
        assert!(
            !body.contains("evil\\n\\n## crlf injection"),
            "CRLF should collapse to one \\n, got double: {body}"
        );
    }

    /// v0.20.2 audit-followup #47: repo names follow the same
    /// escape rules. The repo name allowlist already rejects `../`
    /// etc, but a backtick is technically valid in a slug allowlist
    /// configured permissively, so escape defensively.
    #[test]
    fn agents_md_escapes_repo_names_too() {
        let mut project = fixture_project();
        project.repos[0].name = "evil\nrepo".into();
        let body = render(&project, AgentFormat::AgentsMd);
        // Repo name lands in the table row; the literal `\n` should
        // be there, not a newline that breaks the table.
        assert!(
            body.contains("`evil\\nrepo`"),
            "expected escaped repo name in table: {body}"
        );
    }

    /// v0.20.2 audit-followup #47: the escape is also applied in
    /// CLAUDE.md (which delegates to render_agents_md).
    #[test]
    fn claude_md_inherits_escape_from_agents_md() {
        let mut project = fixture_project();
        project.name = "evil\n## injection".into();
        let body = render(&project, AgentFormat::ClaudeMd);
        let lines: Vec<&str> = body.lines().collect();
        assert!(
            !lines.contains(&"## injection"),
            "injection reached CLAUDE.md: {body}"
        );
    }

    /// v0.20.2 audit-followup #47: the escape is also applied to
    /// the cursor-rules / copilot / llms-txt formats.
    #[test]
    fn cursor_rules_copilot_llms_txt_all_escape_project_name() {
        let mut project = fixture_project();
        project.name = "evil\n## injection".into();
        for fmt in [
            AgentFormat::CursorRules,
            AgentFormat::Copilot,
            AgentFormat::LlmsTxt,
        ] {
            let body = render(&project, fmt);
            let lines: Vec<&str> = body.lines().collect();
            assert!(
                !lines.contains(&"## injection"),
                "{fmt:?} did not escape newline in project name: {body}"
            );
        }
    }

    /// v0.20.2 audit-followup #47: pure-text well-formed names go
    /// through unchanged so the happy path stays readable.
    #[test]
    fn escape_markdown_token_is_pass_through_for_safe_input() {
        for safe in ["demo", "demo-app", "Demo App", "alpha-beta-9"] {
            assert_eq!(escape_markdown_token(safe), safe);
        }
    }

    /// v0.20.2 audit-followup #47: backslash itself is escaped so
    /// downstream `\n` literal can't be re-interpreted as an escape
    /// sequence.
    #[test]
    fn escape_markdown_token_doubles_existing_backslash() {
        assert_eq!(escape_markdown_token("a\\b"), "a\\\\b");
    }

    #[test]
    fn default_paths_match_format_conventions() {
        let project = fixture_project();
        assert_eq!(
            default_path(&project, AgentFormat::AgentsMd),
            std::path::PathBuf::from("/tmp/demo/AGENTS.md")
        );
        assert_eq!(
            default_path(&project, AgentFormat::ClaudeMd),
            std::path::PathBuf::from("/tmp/demo/CLAUDE.md")
        );
        assert_eq!(
            default_path(&project, AgentFormat::CursorRules),
            std::path::PathBuf::from("/tmp/demo/.cursor/rules/coral.mdc")
        );
        assert_eq!(
            default_path(&project, AgentFormat::Copilot),
            std::path::PathBuf::from("/tmp/demo/.github/copilot-instructions.md")
        );
        assert_eq!(
            default_path(&project, AgentFormat::LlmsTxt),
            std::path::PathBuf::from("/tmp/demo/llms.txt")
        );
    }
}
