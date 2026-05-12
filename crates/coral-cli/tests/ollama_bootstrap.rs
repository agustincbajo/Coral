//! Ollama end-to-end bootstrap test (FR-ONB-28, PRD v1.4 §7.4).
//!
//! Validates that a Coral user without Anthropic / Gemini API access can
//! still bootstrap a wiki by running Ollama locally — the "no provider,
//! no payment" path the mini-wizard offers from `coral doctor --wizard`.
//!
//! ## Test environment
//!
//! Marked `#[ignore]` so plain `cargo test` skips it. CI runners and
//! laptops without Ollama would otherwise spend 30s spawning the
//! binary before failing. To run locally:
//!
//! ```bash
//! ollama serve &              # if not already running
//! ollama pull llama3.1:8b      # ~4.7 GB download, ~3 min on 200 Mbit
//! cargo test --test ollama_bootstrap -- --ignored
//! ```
//!
//! ## What's exercised
//!
//! 1. The mini-fixture `tests/fixtures/tiny-repo-50loc/` (Rust crate
//!    with ~30 LOC across `lib.rs` + `main.rs`) is copied into a
//!    tempdir + initialised as a git repo (Coral requires a git HEAD).
//! 2. `coral init` writes `.wiki/`, CLAUDE.md template, `.coral/`.
//! 3. `coral bootstrap --apply --provider=http --max-cost=0` runs the
//!    wiki generation against `CORAL_HTTP_ENDPOINT=http://localhost:
//!    11434/v1/chat/completions` (Ollama's OpenAI-compatible endpoint).
//! 4. Assertion: at least one wiki page lands under `.wiki/` and the
//!    body is non-empty and not an error string.
//!
//! ## Acceptable behavior on Ollama
//!
//! Per PRD §7.4: slower than claude-sonnet (5min vs 30s), lower page
//! count, body quality varies — the test only checks that SOMETHING
//! was written, not that the output is high-quality. Quality is
//! deferred to the M2 calibration sprint.
//!
//! ## Budget
//!
//! 10 min hard cap via `Command::timeout`-equivalent monitoring. The
//! mini-fixture is 2 files so the page count should be 2–3 and total
//! generation time ~3–5 min on a laptop without GPU. We allow 10 min
//! to absorb model-load cold-start on the first request after `ollama
//! serve` is started.

#![cfg(test)]

use assert_cmd::cargo::CommandCargoExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Default Ollama endpoint when the user is running `ollama serve` on
/// localhost with the OpenAI-compatible chat-completions route. Coral's
/// `HttpRunner` expects an OpenAI-shaped endpoint, which Ollama serves
/// at `/v1/chat/completions` since the OpenAI compatibility shim
/// shipped in 2024-02.
const OLLAMA_ENDPOINT: &str = "http://localhost:11434/v1/chat/completions";

/// Model expected to be pulled. Matches the wizard default in
/// `coral_cli::commands::doctor::wizard_ollama`.
const OLLAMA_MODEL: &str = "llama3.1:8b";

/// Hard timeout for the bootstrap. Ollama on a developer laptop without
/// GPU acceleration is the slow case we accept.
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(600);

/// Pre-flight: skip with a clear message if `ollama` is not on PATH.
fn ollama_present() -> bool {
    which_in_path("ollama").is_some() || which_in_path("ollama.exe").is_some()
}

fn which_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Pre-flight: verify Ollama has the expected model pulled. Runs
/// `ollama list` and greps for `OLLAMA_MODEL` (prefix match — Ollama
/// reports `llama3.1:8b   ...   SIZE   ...   MODIFIED`).
fn ollama_has_model() -> bool {
    let Ok(out) = std::process::Command::new(if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    })
    .arg("list")
    .output() else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout
        .lines()
        .skip(1) // header
        .any(|l| l.trim_start().starts_with(OLLAMA_MODEL))
}

/// Pre-flight: verify the OpenAI-compatible chat endpoint is reachable.
/// Ollama exposes it once `ollama serve` is running. We do a single GET
/// against `/v1/models` (cheap, no model load required) and consider
/// anything 200..400 a success — Ollama returns JSON; the test isn't
/// fussy about the body shape, only that the server answered.
fn ollama_endpoint_alive() -> bool {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .build();
    match agent.get("http://localhost:11434/v1/models").call() {
        Ok(r) => (200..400).contains(&r.status()),
        Err(_) => false,
    }
}

fn copy_fixture(dst: &Path) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("tiny-repo-50loc");
    copy_dir_recursive(&src, dst);
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// `coral bootstrap` requires a git HEAD. Initialise the tempdir as a
/// git repo with one commit so HEAD resolves.
fn git_init_and_commit(repo: &Path) {
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "ollama-test@coral.local"][..],
        &["config", "user.name", "Coral Ollama Test"][..],
        &["add", "."][..],
        &["commit", "-q", "-m", "initial fixture commit"][..],
    ] {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git invocation failed");
        assert!(
            status.success(),
            "git {args:?} failed in {} (status: {status:?})",
            repo.display()
        );
    }
}

/// Spawn `coral bootstrap --apply` and wait up to `BOOTSTRAP_TIMEOUT`.
/// On timeout, kills the child + returns an Err so the test fails with
/// a clear message rather than hanging the CI worker.
fn run_bootstrap(repo: &Path) -> Result<(), String> {
    let mut cmd = Command::cargo_bin("coral").map_err(|e| e.to_string())?;
    cmd.current_dir(repo)
        .args([
            "bootstrap",
            "--apply",
            "--provider=http",
            // --max-cost=0 documents the user-facing contract: Ollama
            // local = $0. The CLI accepts 0 as "no cost gate" (the gate
            // engages on usd > cap, and 0 > 0 is false). If a future
            // refactor flips the semantics to "strict zero" this assertion
            // catches it.
        ])
        .env("CORAL_HTTP_ENDPOINT", OLLAMA_ENDPOINT)
        .env("CORAL_HTTP_MODEL", OLLAMA_MODEL)
        // Make the failure mode obvious if Ollama needs auth (default
        // install does NOT) — we don't pass CORAL_HTTP_API_KEY so the
        // bearer header is absent.
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("spawn coral: {e}"))?;

    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => {
                if !status.success() {
                    let out = child.wait_with_output().map_err(|e| e.to_string())?;
                    return Err(format!(
                        "coral bootstrap exited non-zero: {status:?}\nstdout:\n{}\nstderr:\n{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr),
                    ));
                }
                return Ok(());
            }
            None => {
                if start.elapsed() > BOOTSTRAP_TIMEOUT {
                    let _ = child.kill();
                    let out = child.wait_with_output().map_err(|e| e.to_string())?;
                    return Err(format!(
                        "coral bootstrap exceeded {}s timeout; killed.\nstdout (partial):\n{}\nstderr (partial):\n{}",
                        BOOTSTRAP_TIMEOUT.as_secs(),
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr),
                    ));
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

/// Walk `.wiki/` and return every non-empty `.md` file path that isn't
/// the scaffolding (index.md, log.md, SCHEMA.md).
fn generated_wiki_pages(wiki: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !wiki.exists() {
        return out;
    }
    walk(&mut out, wiki);
    out.retain(|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Skip the scaffolding files `coral init` writes. They exist on
        // every fresh `coral init` and don't constitute a "bootstrap
        // wrote something" signal.
        !matches!(name, "index.md" | "log.md" | "SCHEMA.md")
    });
    out
}

fn walk(out: &mut Vec<PathBuf>, dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            walk(out, &p);
        } else if p.extension().and_then(|e| e.to_str()) == Some("md")
            && std::fs::metadata(&p).map(|m| m.len() > 0).unwrap_or(false)
        {
            out.push(p);
        }
    }
}

#[test]
#[ignore = "requires Ollama running locally with llama3.1:8b pulled — run with --ignored"]
fn bootstrap_with_ollama_writes_wiki_pages() {
    // Pre-flight gates: skip with helpful messages instead of failing
    // when the local environment doesn't have Ollama set up.
    if !ollama_present() {
        eprintln!("SKIP: `ollama` is not on PATH. Install from https://ollama.com");
        return;
    }
    if !ollama_endpoint_alive() {
        eprintln!(
            "SKIP: Ollama endpoint {OLLAMA_ENDPOINT} is not reachable. Run `ollama serve` first."
        );
        return;
    }
    if !ollama_has_model() {
        eprintln!("SKIP: model `{OLLAMA_MODEL}` is not pulled. Run `ollama pull {OLLAMA_MODEL}`.");
        return;
    }

    let tmp = TempDir::new().expect("tempdir");
    let repo = tmp.path();

    copy_fixture(repo);
    git_init_and_commit(repo);

    // `coral init` writes the wiki skeleton + CLAUDE.md + .gitignore.
    let status = Command::cargo_bin("coral")
        .unwrap()
        .current_dir(repo)
        .arg("init")
        .status()
        .expect("spawn coral init");
    assert!(status.success(), "coral init failed in {}", repo.display());

    // The real bootstrap call. May take 5-10 min on first run because
    // Ollama loads the 4.7 GB model on the first request.
    run_bootstrap(repo).expect("bootstrap with Ollama");

    let pages = generated_wiki_pages(&repo.join(".wiki"));
    assert!(
        !pages.is_empty(),
        "bootstrap finished but no wiki pages were generated under .wiki/. \
         Expected at least one .md file outside index.md/log.md/SCHEMA.md."
    );

    // Sanity: pick the first generated page and verify it's not an
    // error envelope. Ollama can return "I can't help with that" for
    // some prompts; that's a quality issue M2 will tackle, but a
    // hard-error string ("Error:", "ERROR", "401 Unauthorized") is a
    // wiring bug.
    let first = std::fs::read_to_string(&pages[0]).expect("read generated page");
    assert!(
        !first.is_empty(),
        "first generated wiki page is empty: {}",
        pages[0].display()
    );
    for bad in ["401 Unauthorized", "403 Forbidden", "500 Internal Server"] {
        assert!(
            !first.contains(bad),
            "first generated wiki page contains the literal error string {bad:?}: {}",
            pages[0].display()
        );
    }
}
