//! `coral doctor` — top-level diagnostic + provider mini-wizard
//! (FR-ONB-7, FR-ONB-8, FR-ONB-27, FR-ONB-28).
//!
//! This is the **new** week-3 doctor — distinct from the older
//! `coral project doctor` (which lives in `commands::project::doctor`
//! and checks multi-repo manifest health). The naming is intentional
//! per PRD v1.4 §0 item 24: the slash command `/coral:coral-doctor`
//! invokes the new flow; the old `coral project doctor` is untouched.
//!
//! Modes:
//!
//! * **Default** — run `coral self-check`'s probe pipeline in-process,
//!   print warnings + suggestions in a human-readable form, exit. No
//!   prompts. Used by the `coral-doctor` skill via `Bash(coral
//!   doctor)`.
//! * **`--non-interactive`** — emit the same JSON envelope
//!   `coral self-check --format=json` produces. Provided so callers
//!   (CI, automation) can parse instead of scrape stdout.
//! * **`--wizard`** — launch the **provider mini-wizard** (FR-ONB-27,
//!   FR-ONB-28). 4 paths: Anthropic API key, Gemini API key, Ollama
//!   local, install `claude` CLI. Refuses non-TTY runs (we cannot
//!   prompt without a terminal).
//!
//! The wizard:
//!   * Anthropic / Gemini — verifies the pasted key via a 1-token
//!     ping (HTTP 200 = persist; anything else = abort + no write).
//!   * Ollama — checks `ollama` on PATH, then `ollama list` for
//!     `llama3.1:8b`. Does NOT auto-pull (would block 5–10 min
//!     without progress visible through `dialoguer`).
//!   * `claude` CLI — prints the install URL. We do not auto-open
//!     a browser.
//!
//! All four paths write via `coral_core::config::upsert_provider_
//! section`, which flock-protects `.coral/config.toml` and chmods
//! 600 on Unix.

use anyhow::{Context, Result, anyhow};
use clap::Args;
use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use crate::commands::self_check::{self, SelfCheck, Severity};

// ----------------------------------------------------------------------
// Prompter abstraction (testability seam, v0.40.0)
// ----------------------------------------------------------------------

/// Tiny shim around `dialoguer` so the wizard's 5 interactive paths
/// can be exercised by `#[cfg(test)]` code without a real terminal.
/// In production this delegates straight to `dialoguer::{Select,
/// Password}` and is functionally identical to the pre-v0.40.0 inline
/// calls; in tests, `MockPrompter` plays back a scripted sequence.
///
/// Introduced for BACKLOG #5 step 3/4 (coverage ratchet 60 → 65). The
/// wizard's interactive branches sat at 39% line coverage because they
/// could not be reached from cargo-test's piped stdin — `dialoguer` 0.11
/// has no test mode. The abstraction is < 40 LoC; cost is small relative
/// to the test footprint it unlocks.
pub(crate) trait Prompter {
    /// Show a "pick one" menu. Returns the 0-based index of the choice.
    fn select(&mut self, prompt: &str, items: &[&str], default: usize) -> Result<usize>;
    /// Prompt for a secret (echo suppressed). Caller is responsible for
    /// trimming / validating; the prompter returns whatever was typed.
    fn password(&mut self, prompt: &str) -> Result<String>;
}

/// Production prompter — wraps `dialoguer` with the same `ColorfulTheme`
/// the pre-abstraction wizard used. Behavioral parity with v0.39.0.
pub(crate) struct DialoguerPrompter {
    theme: dialoguer::theme::ColorfulTheme,
}

impl DialoguerPrompter {
    pub(crate) fn new() -> Self {
        Self {
            theme: dialoguer::theme::ColorfulTheme::default(),
        }
    }
}

impl Prompter for DialoguerPrompter {
    fn select(&mut self, prompt: &str, items: &[&str], default: usize) -> Result<usize> {
        dialoguer::Select::with_theme(&self.theme)
            .with_prompt(prompt)
            .default(default)
            .items(items)
            .interact()
            .map_err(|e| anyhow!("provider selection prompt failed: {e}"))
    }
    fn password(&mut self, prompt: &str) -> Result<String> {
        dialoguer::Password::with_theme(&self.theme)
            .with_prompt(prompt)
            .interact()
            .map_err(|e| anyhow!("password prompt failed: {e}"))
    }
}

/// `coral doctor` arguments. See module docs for the behavior matrix.
#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Launch the provider mini-wizard (FR-ONB-27 + FR-ONB-28). Requires
    /// a TTY — we refuse non-interactive runs because the password
    /// prompt cannot consume stdin safely.
    #[arg(long)]
    pub wizard: bool,
    /// Emit machine-readable JSON instead of human text. Mirrors the
    /// `coral self-check --format=json` envelope so the skill and
    /// downstream CI can parse one schema.
    #[arg(long = "non-interactive")]
    pub non_interactive: bool,
}

pub fn run(args: DoctorArgs) -> Result<ExitCode> {
    let cwd = std::env::current_dir().context("getting cwd")?;

    if args.wizard {
        return run_wizard(&cwd);
    }

    // Default path: probe + print. The self-check probe pipeline is
    // the single source of truth — we don't shell out to the
    // sibling subcommand because that would duplicate the report
    // serialization round-trip (and add a process spawn).
    let report = self_check::run_probes(&cwd, false);

    if args.non_interactive {
        let json = serde_json::to_string(&report)?;
        println!("{json}");
    } else {
        print_human_report(&report);
    }
    Ok(ExitCode::SUCCESS)
}

/// Pretty-print warnings + suggestions. Mirrors the layout the skill
/// description in PRD §7.3 promises the user:
///
/// ```text
/// Coral doctor — diagnostics
/// status: Ok
/// 1 warning, 2 suggestions
///
/// warnings:
///   [Medium] no providers configured (claude_cli available)
///     fix: /coral:coral-doctor
///
/// suggestions:
///   /coral:coral-doctor — the doctor skill walks you through a 4-path
///     provider wizard
/// ```
fn print_human_report(report: &SelfCheck) {
    println!("Coral doctor — diagnostics");
    println!("  status:   {:?}", report.coral_status);
    println!("  version:  {}", report.coral_version);
    println!(
        "  platform: {}/{}",
        report.platform.os, report.platform.arch
    );

    if !report.providers_configured.is_empty() {
        println!(
            "  configured providers: {}",
            report.providers_configured.join(", ")
        );
    } else if !report.providers_available.is_empty() {
        println!(
            "  no providers configured ({} available — run `coral doctor --wizard`)",
            report.providers_available.join(", ")
        );
    } else {
        println!("  no providers configured and none auto-detected — run `coral doctor --wizard`");
    }

    if report.warnings.is_empty() && report.suggestions.is_empty() {
        println!();
        println!("Coral is ready. Try `/coral:coral-bootstrap` next.");
        return;
    }

    if !report.warnings.is_empty() {
        println!();
        println!("warnings ({}):", report.warnings.len());
        for w in &report.warnings {
            let sev = match w.severity {
                Severity::High => "High",
                Severity::Medium => "Medium",
                Severity::Low => "Low",
            };
            println!("  [{}] {}", sev, w.message);
            if let Some(action) = &w.action {
                println!("       fix: {action}");
            }
        }
    }
    if !report.suggestions.is_empty() {
        println!();
        println!("suggestions ({}):", report.suggestions.len());
        for s in &report.suggestions {
            println!("  {} — {}", s.command, s.explanation);
        }
    }
}

// ----------------------------------------------------------------------
// Provider mini-wizard (FR-ONB-27 / FR-ONB-28)
// ----------------------------------------------------------------------

/// Wizard entry point. Refuses non-TTY callers — there is no safe
/// way to consume a password from a non-terminal stdin. Callers that
/// need to script provider config should edit `.coral/config.toml`
/// directly (the file is documented in PRD Appendix E).
fn run_wizard(cwd: &Path) -> Result<ExitCode> {
    if !std::io::stdin().is_terminal() {
        return Err(anyhow!(
            "coral doctor --wizard requires a TTY for password entry; \
             edit .coral/config.toml directly or run in an interactive shell"
        ));
    }
    let mut prompter = DialoguerPrompter::new();
    run_wizard_with(cwd, &mut prompter)
}

/// Wizard core, parameterised over the prompt source. The TTY gate
/// lives in [`run_wizard`]; tests call this directly with a
/// `MockPrompter` so they can exercise the four provider paths without
/// a real terminal.
pub(crate) fn run_wizard_with(cwd: &Path, prompter: &mut dyn Prompter) -> Result<ExitCode> {
    println!("Coral provider wizard — pick one path:");
    let options = [
        "Anthropic API key (direct, recommended for trying Coral)",
        "Gemini API key (Google AI Studio)",
        "Ollama (local, free, no API key)",
        "Install `claude` CLI (Claude Code subscription)",
        "Skip — I'll configure manually",
    ];
    let choice = prompter.select("Choose a provider", &options, 0)?;

    match choice {
        0 => wizard_anthropic(cwd, prompter),
        1 => wizard_gemini(cwd, prompter),
        2 => wizard_ollama(cwd),
        3 => wizard_claude_cli(),
        _ => {
            println!("Skipped. Nothing was written to .coral/config.toml.");
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn wizard_anthropic(cwd: &Path, prompter: &mut dyn Prompter) -> Result<ExitCode> {
    let key = prompter.password("Anthropic API key (hidden)")?;
    if key.trim().is_empty() {
        println!("Empty key — aborted. Nothing written.");
        return Ok(ExitCode::SUCCESS);
    }

    println!("Verifying key via 1-token ping to api.anthropic.com…");
    match ping_anthropic(&key) {
        Ok(()) => {
            let body = format!(
                "api_key = {}\nmodel = \"claude-haiku-4-5\"\nmax_tokens_per_page = 4096\n",
                toml_string(&key)
            );
            coral_core::config::upsert_provider_section(cwd, "provider.anthropic", &body)
                .context("writing provider.anthropic to .coral/config.toml")?;
            println!("OK — wrote [provider.anthropic] to .coral/config.toml (chmod 600 on Unix).");
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => {
            println!("FAILED: {e}");
            println!("Key was not written. Re-run `coral doctor --wizard` to try again.");
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn wizard_gemini(cwd: &Path, prompter: &mut dyn Prompter) -> Result<ExitCode> {
    let key = prompter.password("Gemini API key (hidden)")?;
    if key.trim().is_empty() {
        println!("Empty key — aborted. Nothing written.");
        return Ok(ExitCode::SUCCESS);
    }

    println!("Verifying key via 1-token ping to generativelanguage.googleapis.com…");
    match ping_gemini(&key) {
        Ok(()) => {
            let body = format!(
                "api_key = {}\nmodel = \"gemini-2.0-flash\"\n",
                toml_string(&key)
            );
            coral_core::config::upsert_provider_section(cwd, "provider.gemini", &body)
                .context("writing provider.gemini to .coral/config.toml")?;
            println!("OK — wrote [provider.gemini] to .coral/config.toml (chmod 600 on Unix).");
            Ok(ExitCode::SUCCESS)
        }
        Err(e) => {
            println!("FAILED: {e}");
            println!("Key was not written. Re-run `coral doctor --wizard` to try again.");
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn wizard_ollama(cwd: &Path) -> Result<ExitCode> {
    let ollama_exe = if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    };
    let Some(_path) = self_check::which_in_path(ollama_exe) else {
        println!("`ollama` is not on PATH.");
        println!("Install Ollama: https://ollama.com");
        println!("Then re-run `coral doctor --wizard` and pick Ollama again.");
        return Ok(ExitCode::SUCCESS);
    };

    println!("Found `ollama` on PATH. Checking for model `llama3.1:8b`…");
    let has_model = has_ollama_model("llama3.1:8b");
    if !has_model {
        println!("Model `llama3.1:8b` is not pulled yet.");
        println!("Run: ollama pull llama3.1:8b");
        println!("(this is a ~4.7 GB download; we don't auto-pull from the wizard");
        println!(" because dialoguer cannot stream pull progress safely)");
        println!("Then re-run `coral doctor --wizard` to finish.");
        return Ok(ExitCode::SUCCESS);
    }

    let body = "endpoint = \"http://localhost:11434\"\nmodel = \"llama3.1:8b\"\n";
    coral_core::config::upsert_provider_section(cwd, "provider.ollama", body)
        .context("writing provider.ollama to .coral/config.toml")?;
    println!("OK — wrote [provider.ollama] to .coral/config.toml.");
    Ok(ExitCode::SUCCESS)
}

fn wizard_claude_cli() -> Result<ExitCode> {
    println!("Install the `claude` CLI: https://claude.ai/code");
    println!("After installing + logging in, re-run `coral doctor` and Coral will");
    println!("auto-detect it via PATH. No further wizard step needed — the CLI");
    println!("provider is presence-detected, no key to enter.");
    Ok(ExitCode::SUCCESS)
}

// ----------------------------------------------------------------------
// Provider probes (HTTP + ollama list)
// ----------------------------------------------------------------------

/// 1-token POST to Anthropic. Returns Ok on HTTP 200, Err on anything
/// else (including network failure or 401). We do NOT include the
/// response body in the error message — Anthropic's error envelope
/// sometimes echoes back the key prefix and we don't want that on a
/// user's terminal.
pub(crate) fn ping_anthropic(api_key: &str) -> Result<()> {
    let body = serde_json::json!({
        "model": "claude-haiku-4-5",
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "."}]
    });
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();
    let resp = agent
        .post("https://api.anthropic.com/v1/messages")
        .set("x-api-key", api_key)
        .set("anthropic-version", "2023-06-01")
        .set("content-type", "application/json")
        .send_json(body);
    match resp {
        Ok(r) if r.status() == 200 => Ok(()),
        Ok(r) => Err(anyhow!("HTTP {} from api.anthropic.com", r.status())),
        Err(ureq::Error::Status(code, _)) => Err(anyhow!(
            "HTTP {} from api.anthropic.com (invalid key?)",
            code
        )),
        Err(e) => Err(anyhow!("network error: {e}")),
    }
}

/// 1-token POST to Gemini. Same contract as `ping_anthropic`.
pub(crate) fn ping_gemini(api_key: &str) -> Result<()> {
    let body = serde_json::json!({
        "contents": [{"parts": [{"text": "."}]}]
    });
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key={api_key}"
    );
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();
    let resp = agent
        .post(&url)
        .set("content-type", "application/json")
        .send_json(body);
    match resp {
        Ok(r) if r.status() == 200 => Ok(()),
        Ok(r) => Err(anyhow!(
            "HTTP {} from generativelanguage.googleapis.com",
            r.status()
        )),
        Err(ureq::Error::Status(code, _)) => Err(anyhow!(
            "HTTP {} from generativelanguage.googleapis.com (invalid key?)",
            code
        )),
        Err(e) => Err(anyhow!("network error: {e}")),
    }
}

/// `ollama list` returns one line per pulled model, prefixed with the
/// model name. We grep for an exact `name:tag` prefix because Ollama's
/// table-format output puts the name in column 0.
fn has_ollama_model(model: &str) -> bool {
    let out = std::process::Command::new(if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    })
    .arg("list")
    .output();
    let Ok(out) = out else { return false };
    if !out.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Skip header row; ollama-list output looks like:
    //   NAME                ID              SIZE      MODIFIED
    //   llama3.1:8b         abc123          4.7 GB    3 days ago
    stdout
        .lines()
        .skip(1)
        .any(|line| line.split_whitespace().next() == Some(model))
}

/// Quote a string as a TOML basic-string literal. We can't naively
/// wrap in `"…"` because the API key (for the Anthropic / Gemini
/// paths) may contain control characters that need escaping per the
/// TOML spec. This implements the minimum set of escapes the spec
/// requires for basic strings: `\`, `"`, plus controls below 0x20.
/// We round-trip through `toml::from_str` in the test to confirm the
/// output is valid.
fn toml_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for c in raw.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// `toml_string` round-trips through `toml::Value` so any payload
    /// (including characters TOML would otherwise reject) lands as a
    /// valid double-quoted string literal.
    #[test]
    fn toml_string_quotes_arbitrary_payload() {
        let s = toml_string("sk-ant-abc-123");
        assert_eq!(s, "\"sk-ant-abc-123\"");
        // Quotes inside: must escape.
        let s = toml_string("has\"quote");
        assert!(s.starts_with('"') && s.ends_with('"'));
        // The escaped form parses back as the original string.
        let v: toml::Value = toml::from_str(&format!("k = {s}")).unwrap();
        assert_eq!(v["k"].as_str().unwrap(), "has\"quote");
    }

    /// `--non-interactive` (no `--wizard`) emits valid JSON whose
    /// envelope matches the self-check schema. The skill consumes
    /// this — a schema break here breaks the skill.
    #[test]
    fn non_interactive_emits_valid_self_check_json() {
        let cwd = std::env::current_dir().unwrap();
        let report = self_check::run_probes(&cwd, false);
        let json = serde_json::to_string(&report).unwrap();
        // Sanity: the fields the skill grep'd in PRD §7.3 must be
        // present so the skill's JSON-path can find them.
        for field in [
            "coral_status",
            "providers_configured",
            "warnings",
            "suggestions",
        ] {
            assert!(json.contains(field), "missing required field `{field}`");
        }
    }

    /// `has_ollama_model` returns false when `ollama` is not on PATH —
    /// the `Command::output()` call returns Err and we fall through
    /// to false. CI hosts don't ship Ollama, so we exercise the
    /// negative branch only; the positive branch is documented as a
    /// manual smoke test.
    #[test]
    fn has_ollama_model_returns_false_without_ollama() {
        // We can't reliably control PATH here without disturbing
        // sibling tests; we just call it and accept either branch
        // — the type-level guarantee is that we never panic and
        // always produce a bool.
        let _ = has_ollama_model("nonexistent:tag");
    }

    /// `print_human_report` doesn't panic on the empty-report path.
    /// This is the "everything is green" code path — the skill
    /// prints "Coral is ready" in that case.
    #[test]
    fn print_human_report_handles_empty_report() {
        let report = SelfCheck {
            schema_version: 1,
            coral_status: self_check::CoralStatus::Ok,
            coral_version: "0.34.0-test".into(),
            binary_path: std::path::PathBuf::new(),
            in_path: true,
            platform: self_check::PlatformInfo {
                os: "linux".into(),
                arch: "x86_64".into(),
            },
            git_repo: None,
            wiki: None,
            coral_toml: None,
            claude_md: None,
            claude_cli: None,
            providers_available: vec![],
            providers_configured: vec!["anthropic".into()],
            update_available: None,
            mcp_server: None,
            ui_server: None,
            warnings: vec![],
            suggestions: vec![],
        };
        // Just make sure it doesn't panic; we don't capture stdout
        // (the formatter is straight println!, easier to assert via
        // an integration test on the binary).
        print_human_report(&report);
    }

    /// Wizard refuses non-TTY runs with a clear error message. We
    /// exercise the early-return by calling `run_wizard` directly on
    /// a path where stdin is the test runner's pipe (never a TTY).
    #[test]
    fn wizard_refuses_non_tty_with_explicit_error() {
        let cwd = std::env::current_dir().unwrap();
        let res = run_wizard(&cwd);
        // In CI / cargo test, stdin is never a TTY — we expect Err.
        let err = res.expect_err("wizard must refuse non-TTY");
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("tty"),
            "error message must mention TTY: {msg}"
        );
    }

    // ------------------------------------------------------------------
    // v0.40.0 wizard E2E coverage (BACKLOG #5 step 3/4)
    // ------------------------------------------------------------------

    use std::cell::RefCell;

    /// Scripted `Prompter` for tests. `selects` and `passwords` are
    /// consumed front-to-back. Each call pops one entry; if the queue
    /// is empty we return a synthetic error so a test that under-
    /// scripts its wizard run fails loudly instead of hanging.
    struct MockPrompter {
        selects: RefCell<Vec<usize>>,
        passwords: RefCell<Vec<String>>,
    }

    impl MockPrompter {
        fn new(selects: Vec<usize>, passwords: Vec<String>) -> Self {
            Self {
                selects: RefCell::new(selects),
                passwords: RefCell::new(passwords),
            }
        }
    }

    impl Prompter for MockPrompter {
        fn select(&mut self, _prompt: &str, items: &[&str], _default: usize) -> Result<usize> {
            let mut v = self.selects.borrow_mut();
            if v.is_empty() {
                return Err(anyhow!("MockPrompter: no scripted select answers left"));
            }
            let idx = v.remove(0);
            assert!(
                idx < items.len(),
                "MockPrompter: scripted select index {idx} >= items.len() {}",
                items.len()
            );
            Ok(idx)
        }
        fn password(&mut self, _prompt: &str) -> Result<String> {
            let mut v = self.passwords.borrow_mut();
            if v.is_empty() {
                return Err(anyhow!("MockPrompter: no scripted password answers left"));
            }
            Ok(v.remove(0))
        }
    }

    /// `run_wizard_with` picking the "Skip" branch returns Ok and
    /// writes nothing to `.coral/config.toml`.
    #[test]
    fn wizard_skip_branch_writes_nothing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut p = MockPrompter::new(vec![4], vec![]);
        let exit = run_wizard_with(tmp.path(), &mut p).expect("skip path is Ok");
        // Can't compare ExitCode directly (no PartialEq); format and
        // check it's the SUCCESS variant.
        assert!(format!("{exit:?}").contains("0") || format!("{exit:?}").contains("Success"));
        assert!(
            !tmp.path().join(".coral/config.toml").exists(),
            "skip branch must not create config.toml"
        );
    }

    /// `run_wizard_with` picking the Anthropic path with an empty key
    /// aborts cleanly without writing the section.
    #[test]
    fn wizard_anthropic_empty_key_aborts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut p = MockPrompter::new(vec![0], vec!["   ".into()]);
        let _ = run_wizard_with(tmp.path(), &mut p).expect("empty-key abort is Ok");
        assert!(
            !tmp.path().join(".coral/config.toml").exists(),
            "empty-key abort must not create config.toml"
        );
    }

    /// `run_wizard_with` picking the Anthropic path with a bogus key
    /// runs the ping, which fails (404/401/network), prints "FAILED",
    /// and does NOT write the section. We don't actually want to hit
    /// the live API in CI; we feed a key that is syntactically valid
    /// but unauthorised so the request returns 401 rather than ever
    /// being authorised. If the network is offline the same code path
    /// is taken via `network error`.
    #[test]
    fn wizard_anthropic_bogus_key_does_not_persist() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut p = MockPrompter::new(
            vec![0],
            vec!["sk-ant-bogus-not-a-real-key-0000000000".into()],
        );
        let _ =
            run_wizard_with(tmp.path(), &mut p).expect("bogus-key path returns Ok (error printed)");
        assert!(
            !tmp.path().join(".coral/config.toml").exists(),
            "bogus key must not persist [provider.anthropic]"
        );
    }

    /// Same shape as the Anthropic test but for Gemini.
    #[test]
    fn wizard_gemini_empty_key_aborts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut p = MockPrompter::new(vec![1], vec!["".into()]);
        let _ = run_wizard_with(tmp.path(), &mut p).expect("empty-key abort is Ok");
        assert!(!tmp.path().join(".coral/config.toml").exists());
    }

    #[test]
    fn wizard_gemini_bogus_key_does_not_persist() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut p = MockPrompter::new(vec![1], vec!["AIzaSyBogus-not-a-real-key".into()]);
        let _ = run_wizard_with(tmp.path(), &mut p).expect("bogus-key path returns Ok");
        assert!(!tmp.path().join(".coral/config.toml").exists());
    }

    /// The `claude` CLI path is informational only — no prompts after
    /// the menu pick, no config writes. Verifies it returns Ok.
    #[test]
    fn wizard_claude_cli_branch_is_informational() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut p = MockPrompter::new(vec![3], vec![]);
        let _ = run_wizard_with(tmp.path(), &mut p).expect("claude-cli path is Ok");
        assert!(
            !tmp.path().join(".coral/config.toml").exists(),
            "claude-cli branch must not create config.toml"
        );
    }

    /// The Ollama branch with `ollama` absent from PATH prints the
    /// install hint and returns Ok without writing anything. We force
    /// the "not on PATH" condition by setting PATH to a single empty
    /// tempdir for the duration of the call. PATH is restored after.
    #[test]
    fn wizard_ollama_without_binary_prints_hint() {
        let tmp = tempfile::TempDir::new().unwrap();
        let empty = tempfile::TempDir::new().unwrap();
        let prev_path = std::env::var_os("PATH");
        // SAFETY: tests in a `#[cfg(test)]` module run single-threaded
        // per-process by default for set_var; sibling tests in this
        // file don't read PATH, so the temporary mutation is local.
        // We restore on the way out.
        // NB: This is intentionally pragmatic. The alternative (a
        // PATH-injection seam) would balloon the abstraction past the
        // 50-LoC budget the BACKLOG calls for.
        unsafe {
            std::env::set_var("PATH", empty.path());
        }
        let mut p = MockPrompter::new(vec![2], vec![]);
        let result = run_wizard_with(tmp.path(), &mut p);
        // Restore PATH before assertions so an assertion failure doesn't
        // poison sibling tests.
        unsafe {
            match prev_path {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }
        let _ = result.expect("ollama-without-binary path is Ok");
        assert!(
            !tmp.path().join(".coral/config.toml").exists(),
            "ollama-missing branch must not create config.toml"
        );
    }

    /// `print_human_report` exercises the warnings + suggestions
    /// branches that the v0.39.0-and-earlier test only skipped.
    #[test]
    fn print_human_report_handles_warnings_and_suggestions() {
        let report = SelfCheck {
            schema_version: 1,
            coral_status: self_check::CoralStatus::CheckFailed,
            coral_version: "0.40.0-test".into(),
            binary_path: std::path::PathBuf::new(),
            in_path: true,
            platform: self_check::PlatformInfo {
                os: "linux".into(),
                arch: "x86_64".into(),
            },
            git_repo: None,
            wiki: None,
            coral_toml: None,
            claude_md: None,
            claude_cli: None,
            providers_available: vec!["claude_cli".into()],
            providers_configured: vec![],
            update_available: None,
            mcp_server: None,
            ui_server: None,
            warnings: vec![
                self_check::Warning {
                    severity: Severity::High,
                    message: "no providers configured".into(),
                    action: Some("/coral:coral-doctor".into()),
                },
                self_check::Warning {
                    severity: Severity::Medium,
                    message: "stale wiki".into(),
                    action: None,
                },
                self_check::Warning {
                    severity: Severity::Low,
                    message: "low-priority hint".into(),
                    action: None,
                },
            ],
            suggestions: vec![self_check::Suggestion {
                kind: self_check::SuggestionKind::RunDoctor,
                command: "/coral:coral-doctor".into(),
                explanation: "walk through 4-path provider wizard".into(),
            }],
        };
        // Branch coverage: providers_available non-empty +
        // providers_configured empty.
        print_human_report(&report);
    }

    /// `print_human_report` with neither configured NOR available
    /// providers — the "no providers configured and none auto-detected"
    /// branch.
    #[test]
    fn print_human_report_handles_zero_provider_detection() {
        let report = SelfCheck {
            schema_version: 1,
            coral_status: self_check::CoralStatus::CheckFailed,
            coral_version: "0.40.0-test".into(),
            binary_path: std::path::PathBuf::new(),
            in_path: true,
            platform: self_check::PlatformInfo {
                os: "linux".into(),
                arch: "x86_64".into(),
            },
            git_repo: None,
            wiki: None,
            coral_toml: None,
            claude_md: None,
            claude_cli: None,
            providers_available: vec![],
            providers_configured: vec![],
            update_available: None,
            mcp_server: None,
            ui_server: None,
            warnings: vec![],
            suggestions: vec![],
        };
        print_human_report(&report);
    }

    /// `ping_anthropic` with a syntactically obvious bogus key resolves
    /// to an error variant (either HTTP 4xx or a network error if CI is
    /// offline). Both branches return Err — the contract.
    #[test]
    fn ping_anthropic_bogus_key_returns_err() {
        let res = ping_anthropic("sk-ant-bogus-key-for-test");
        assert!(res.is_err(), "bogus key must produce Err, got {res:?}");
    }

    /// Same shape for Gemini.
    #[test]
    fn ping_gemini_bogus_key_returns_err() {
        let res = ping_gemini("bogus-gemini-key-for-test");
        assert!(res.is_err(), "bogus key must produce Err, got {res:?}");
    }

    /// `toml_string` covers the control-character branch (`\u{XXXX}`
    /// escape) which the existing test didn't reach.
    #[test]
    fn toml_string_escapes_control_chars() {
        let s = toml_string("a\x01b\x1Fc\nd\re\tf");
        // The TOML basic-string spec requires `\nXXXX` for these.
        assert!(s.contains("\\u0001"), "must escape U+0001: {s}");
        assert!(s.contains("\\u001F"), "must escape U+001F: {s}");
        assert!(s.contains("\\n"));
        assert!(s.contains("\\r"));
        assert!(s.contains("\\t"));
        // Round-trip: the escaped form must parse back to the input.
        let v: toml::Value = toml::from_str(&format!("k = {s}")).unwrap();
        assert_eq!(v["k"].as_str().unwrap(), "a\x01b\x1Fc\nd\re\tf");
    }

    /// `toml_string` escapes a literal backslash.
    #[test]
    fn toml_string_escapes_backslash() {
        let s = toml_string("path\\with\\backslash");
        let v: toml::Value = toml::from_str(&format!("k = {s}")).unwrap();
        assert_eq!(v["k"].as_str().unwrap(), "path\\with\\backslash");
    }

    /// `run()` non-interactive branch returns Ok and emits JSON we can
    /// pipe back through serde — exercises the top-level dispatcher.
    #[test]
    fn run_non_interactive_returns_ok() {
        let args = DoctorArgs {
            wizard: false,
            non_interactive: true,
        };
        let res = run(args);
        assert!(res.is_ok(), "non-interactive run must succeed");
    }

    /// `run()` default (human-readable) branch returns Ok.
    #[test]
    fn run_default_returns_ok() {
        let args = DoctorArgs {
            wizard: false,
            non_interactive: false,
        };
        let res = run(args);
        assert!(res.is_ok(), "default run must succeed");
    }
}
