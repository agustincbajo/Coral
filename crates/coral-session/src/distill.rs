//! `coral session distill <id>` — feed a captured transcript
//! through `Runner` to extract surprising / non-obvious findings,
//! then emit one new wiki page per finding (option a from PRD design
//! Q3) with `reviewed: false` frontmatter.
//!
//! The LLM call is **single-pass** (PRD design Q6 — multi-step
//! deferred to v0.20.x optimization): one `Runner::run` invocation
//! that takes the conversation transcript and returns YAML with
//! `slug, title, body, sources` shape. We parse the YAML, filter
//! to a hard cap of 3 findings (PRD prompt asks 1-3, but the
//! parser still tolerates 4+ defensively), and write each as a
//! `.coral/sessions/distilled/<slug>.md` synthesis page.
//!
//! When `--apply` is set, we additionally write each finding to
//! `.wiki/pages/synthesis/<slug>.md` so it shows up in `coral
//! search` / `coral lint`. Both copies carry `reviewed: false` —
//! the PRD trust-by-curation gate. The user must flip to `true`
//! manually before commit; otherwise `coral lint` raises Critical
//! and the pre-commit hook blocks it.

use crate::capture::{IndexEntry, SessionIndex, read_index, write_index};
use crate::claude_code::{ClaudeCodeMessage, parse_transcript};
use crate::error::{SessionError, SessionResult};
use coral_core::atomic::atomic_write_string;
use coral_runner::{Prompt, Runner};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

/// Versioned to bump when the prompt template changes meaningfully.
/// Surfaced in the distilled page's frontmatter so the maintainer
/// can re-distill old sessions when a new prompt drops.
pub const DISTILL_PROMPT_VERSION: u32 = 1;

/// Hard cap on findings per session. The prompt asks for 1-3; we
/// clamp at 3 in case the model exceeds. v0.20 doesn't ship a way
/// for the user to override this — the right escape hatch is to
/// re-distill with a tighter prompt, not pump up the cap.
const MAX_FINDINGS_PER_SESSION: usize = 3;

#[derive(Debug, Clone)]
pub struct DistillOptions {
    pub project_root: PathBuf,
    /// Either the full UUID or any unique prefix (≥4 chars) of one.
    pub session_id: String,
    /// When true, emit the page additionally under
    /// `.wiki/pages/synthesis/<slug>.md` so it shows up in
    /// `coral lint` / `coral search`. Always `reviewed: false`.
    pub apply: bool,
    /// Forwarded to `Runner.run` for tracing.
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistillOutcome {
    pub session_id: String,
    /// One or more findings — each becomes its own
    /// `<slug>.md` under `.coral/sessions/distilled/`.
    pub findings: Vec<Finding>,
    /// Paths actually written (`<distilled>.md` always; wiki path
    /// only when `--apply` was set).
    pub written: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub slug: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub sources: Vec<String>,
}

/// LLM output shape: a single YAML document with a top-level
/// `findings` array. Defensive: the LLM is told to emit YAML, but
/// will sometimes wrap in fences — we strip those before parse.
#[derive(Debug, Clone, Deserialize)]
struct DistillerYaml {
    #[serde(default)]
    findings: Vec<Finding>,
}

/// Build the prompt sent to `Runner::run`. Public so the CLI layer
/// can re-use the exact same wording for `--dry-run` previews
/// (not in v0.20 MVP, but the function shape is ready for it).
pub fn build_prompt(messages: &[ClaudeCodeMessage]) -> String {
    // Truncate very long bodies so the prompt fits in a typical
    // context window. Per-message cap is 2000 chars; total across
    // all messages is also capped at ~80k chars (configurable
    // via the constant if it ever needs tuning).
    const PER_MSG_CHARS: usize = 2_000;
    const TOTAL_CHARS: usize = 80_000;

    let mut convo = String::new();
    let mut spent = 0usize;
    for m in messages {
        let snippet: String = m.text.chars().take(PER_MSG_CHARS).collect();
        let chunk = format!("[{}] {}\n", m.role, snippet);
        if spent + chunk.len() > TOTAL_CHARS {
            convo.push_str("[...truncated for prompt budget...]\n");
            break;
        }
        spent += chunk.len();
        convo.push_str(&chunk);
    }

    format!(
        "You are reading a conversation transcript between a developer and an AI coding agent. \
Identify 1-3 surprising or non-obvious technical findings about THIS codebase that would be worth recording in a project wiki. \
\
Output requirements:\n\
\n\
1. YAML format only. No prose preamble, no markdown fences.\n\
2. Top-level key `findings:` with an array of 1-3 entries.\n\
3. Each entry has these keys (all required):\n\
   - `slug`: lowercase kebab-case, 3-40 chars, [a-z0-9-] only.\n\
   - `title`: human-readable title, 5-100 chars.\n\
   - `body`: markdown body, 100-2000 chars; explain the finding so a future maintainer learns something.\n\
   - `sources`: list of file paths or URLs that ground the claim. Empty list if none.\n\
\n\
Bias toward findings that are:\n\
- COUNTER-INTUITIVE: something a reader of the code wouldn't expect.\n\
- EVIDENCE-BACKED: cite files / line ranges / commit SHAs from the conversation.\n\
- WIKI-WORTHY: explains WHY, not WHAT. Skip findings that just paraphrase the conversation.\n\
\n\
Skip if the conversation is too thin to support 1 quality finding — output `findings: []` rather than padding with weak entries.\n\
\n\
=== TRANSCRIPT ===\n{convo}\n=== END TRANSCRIPT ===\n",
    )
}

/// Strips markdown code fences from `s` if present. Defensive
/// against runners that wrap YAML in ` ```yaml ... ``` ` blocks.
fn strip_fences(s: &str) -> &str {
    let s = s.trim();
    if let Some(stripped) = s.strip_prefix("```yaml") {
        if let Some(stripped) = stripped.strip_suffix("```") {
            return stripped.trim();
        }
    }
    if let Some(stripped) = s.strip_prefix("```") {
        if let Some(stripped) = stripped.strip_suffix("```") {
            return stripped.trim();
        }
    }
    s
}

/// Parses runner output into a list of findings. Strict slug check —
/// rejects entries with unsafe slugs early so we never write a path
/// outside the synthesis directory.
pub fn parse_findings(stdout: &str) -> SessionResult<Vec<Finding>> {
    let body = strip_fences(stdout);
    let parsed: DistillerYaml = serde_yaml_ng::from_str(body).map_err(|e| {
        SessionError::DistillMalformed(format!("YAML parse failed: {e}; output was: {stdout}"))
    })?;
    let mut out: Vec<Finding> = Vec::new();
    for (idx, f) in parsed.findings.into_iter().enumerate() {
        if !coral_core::slug::is_safe_filename_slug(&f.slug) {
            return Err(SessionError::DistillMalformed(format!(
                "finding[{idx}].slug '{}' is not a safe filename slug",
                f.slug
            )));
        }
        if f.title.trim().is_empty() || f.body.trim().is_empty() {
            return Err(SessionError::DistillMalformed(format!(
                "finding[{idx}] is missing title or body"
            )));
        }
        out.push(f);
    }
    out.truncate(MAX_FINDINGS_PER_SESSION);
    Ok(out)
}

/// Resolves a session by full id or short prefix (4+ chars) — same
/// matching rule as `forget`. Returns the index entry if exactly one
/// match exists.
fn resolve_entry(index: &SessionIndex, id: &str) -> SessionResult<IndexEntry> {
    if id.len() < 4 {
        return Err(SessionError::InvalidInput(
            "session id must be at least 4 chars".into(),
        ));
    }
    let matches: Vec<&IndexEntry> = index
        .sessions
        .iter()
        .filter(|e| e.session_id == id || e.session_id.starts_with(id))
        .collect();
    match matches.len() {
        0 => Err(SessionError::NotFound(id.to_string())),
        1 => Ok(matches[0].clone()),
        n => Err(SessionError::InvalidInput(format!(
            "session id '{id}' matches {n} sessions; use a longer prefix or full id"
        ))),
    }
}

/// Renders a finding into a wiki-shaped Markdown page. Frontmatter
/// follows the v0.20 trust-by-curation contract: `reviewed: false`
/// is a top-level extra field that `coral lint` flags as Critical
/// (see `crates/coral-lint/src/structural.rs::check_unreviewed`).
pub fn render_page(
    finding: &Finding,
    runner_name: &str,
    session_id: &str,
    captured_at: &str,
) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    // We hand-roll the frontmatter rather than route through
    // `coral_core::frontmatter::serialize` because the synthesis
    // page is written with placeholder confidence + status (the
    // human reviewer fills these in). Hand-rolled keeps the body
    // free of stray defaults.
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("slug: {}\n", finding.slug));
    out.push_str("type: synthesis\n");
    out.push_str("last_updated_commit: unknown\n");
    out.push_str("confidence: 0.4\n");
    out.push_str("status: draft\n");
    out.push_str(&format!("generated_at: \"{now}\"\n"));
    if finding.sources.is_empty() {
        out.push_str("sources: []\n");
    } else {
        out.push_str("sources:\n");
        for s in &finding.sources {
            out.push_str(&format!("  - \"{s}\"\n"));
        }
    }
    out.push_str("backlinks: []\n");
    // Trust-by-curation gate: distilled output ALWAYS lands as
    // `reviewed: false` and the lint check rejects committing a
    // page without flipping it to `true` after human review.
    out.push_str("reviewed: false\n");
    out.push_str("source:\n");
    out.push_str(&format!("  runner: \"{runner_name}\"\n"));
    out.push_str(&format!("  prompt_version: {DISTILL_PROMPT_VERSION}\n"));
    out.push_str(&format!("  session_id: \"{session_id}\"\n"));
    out.push_str(&format!("  captured_at: \"{captured_at}\"\n"));
    out.push_str("---\n\n");
    out.push_str(&format!("# {}\n\n", finding.title));
    out.push_str(&finding.body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// End-to-end distillation. Caller injects the runner so tests can
/// drive a `MockRunner` and the CLI uses the real provider.
pub fn distill_session(
    opts: &DistillOptions,
    runner: &dyn Runner,
    runner_name: &str,
) -> SessionResult<DistillOutcome> {
    let sessions_dir = opts.project_root.join(".coral").join("sessions");
    let index_path = sessions_dir.join("index.json");
    let index = read_index(&index_path)?;
    let entry = resolve_entry(&index, &opts.session_id)?;
    let parsed = parse_transcript(&entry.captured_path)?;

    let prompt = Prompt {
        system: None,
        user: build_prompt(&parsed.messages),
        model: opts.model.clone(),
        cwd: None,
        timeout: None,
    };
    let out = runner.run(&prompt)?;
    let findings = parse_findings(&out.stdout)?;

    if findings.is_empty() {
        return Err(SessionError::DistillMalformed(
            "runner returned no findings; transcript may be too thin to distill".into(),
        ));
    }

    let distilled_dir = sessions_dir.join("distilled");
    std::fs::create_dir_all(&distilled_dir).map_err(|source| SessionError::Io {
        path: distilled_dir.clone(),
        source,
    })?;

    let captured_at = entry.captured_at.to_rfc3339();
    let mut written: Vec<PathBuf> = Vec::new();
    // v0.20.1 cycle-4 audit H1: track the basenames of every file we
    // write under `.coral/sessions/distilled/` so `forget` can clean
    // them up. Pre-fix the index didn't record per-finding filenames
    // and `forget` hard-coded `<session_id>.md` (wrong shape) — every
    // distilled output got orphaned on forget.
    let mut distilled_basenames: Vec<String> = Vec::new();
    for finding in &findings {
        let page = render_page(finding, runner_name, &entry.session_id, &captured_at);
        let basename = format!("{}.md", finding.slug);
        let dest = distilled_dir.join(&basename);
        atomic_write_string(&dest, &page).map_err(|e| match e {
            coral_core::error::CoralError::Io { path, source } => SessionError::Io { path, source },
            other => SessionError::Io {
                path: dest.clone(),
                source: std::io::Error::other(format!("{other}")),
            },
        })?;
        written.push(dest);
        distilled_basenames.push(basename.clone());

        if opts.apply {
            let wiki_synthesis_dir = opts.project_root.join(".wiki").join("synthesis");
            std::fs::create_dir_all(&wiki_synthesis_dir).map_err(|source| SessionError::Io {
                path: wiki_synthesis_dir.clone(),
                source,
            })?;
            let wiki_dest = wiki_synthesis_dir.join(&basename);
            atomic_write_string(&wiki_dest, &page).map_err(|e| match e {
                coral_core::error::CoralError::Io { path, source } => {
                    SessionError::Io { path, source }
                }
                other => SessionError::Io {
                    path: wiki_dest.clone(),
                    source: std::io::Error::other(format!("{other}")),
                },
            })?;
            written.push(wiki_dest);
        }
    }

    // Mark the index entry as distilled and record the output filenames
    // so `forget` can clean them up later (audit H1).
    coral_core::atomic::with_exclusive_lock(&index_path, || {
        let mut idx = read_index(&index_path).unwrap_or_default();
        for e in idx.sessions.iter_mut() {
            if e.session_id == entry.session_id {
                e.distilled = true;
                // Merge — if `distill` was re-run with different findings,
                // remembering both lists keeps cleanup conservative.
                for name in &distilled_basenames {
                    if !e.distilled_outputs.iter().any(|n| n == name) {
                        e.distilled_outputs.push(name.clone());
                    }
                }
            }
        }
        write_index(&index_path, &idx)
    })
    .map_err(|e| match e {
        coral_core::error::CoralError::Io { path, source } => SessionError::Io { path, source },
        other => SessionError::Io {
            path: index_path.clone(),
            source: std::io::Error::other(format!("{other}")),
        },
    })?;

    Ok(DistillOutcome {
        session_id: entry.session_id,
        findings,
        written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{CaptureSource, IndexEntry, SessionIndex, write_index};
    use chrono::TimeZone;
    use coral_runner::MockRunner;
    use tempfile::TempDir;

    fn seed_session(root: &Path, session_id: &str, transcript: &str) -> IndexEntry {
        let dir = root.join(".coral/sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let captured_path = dir.join("session.jsonl");
        std::fs::write(&captured_path, transcript).unwrap();
        let entry = IndexEntry {
            session_id: session_id.into(),
            source: CaptureSource::ClaudeCode,
            captured_at: chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap(),
            captured_path,
            message_count: 1,
            redaction_count: 0,
            distilled: false,
            distilled_outputs: Vec::new(),
            patch_outputs: Vec::new(),
        };
        let idx = SessionIndex {
            sessions: vec![entry.clone()],
        };
        write_index(&dir.join("index.json"), &idx).unwrap();
        entry
    }

    fn mock_runner_with_yaml(yaml: &str) -> MockRunner {
        let r = MockRunner::new();
        r.push_ok(yaml);
        r
    }

    /// Happy path: distill writes the expected synthesis page with
    /// `reviewed: false` frontmatter and marks the index entry as
    /// `distilled: true`.
    #[test]
    fn distill_writes_page_with_reviewed_false_and_marks_index() {
        let dir = TempDir::new().unwrap();
        let _entry = seed_session(
            dir.path(),
            "abc123def4567",
            r#"{"type":"user","sessionId":"abc123def4567","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{"content":"why does this work?"}}
{"type":"assistant","sessionId":"abc123def4567","timestamp":"2026-05-08T10:00:01Z","message":{"role":"assistant","content":[{"type":"text","text":"because of X"}]}}
"#,
        );
        let runner_yaml = r#"findings:
  - slug: surprising-x
    title: A surprising thing about X
    body: |
      The codebase uses X in a non-obvious way: while reading lib.rs you'd assume Y, but in fact Z. This matters because the read path silently relies on the inversion.
    sources:
      - "src/lib.rs"
"#;
        let runner = mock_runner_with_yaml(runner_yaml);
        let opts = DistillOptions {
            project_root: dir.path().to_path_buf(),
            session_id: "abc123de".into(),
            apply: false,
            model: None,
        };
        let outcome = distill_session(&opts, &runner, "mock-test").unwrap();
        assert_eq!(outcome.findings.len(), 1);
        assert_eq!(outcome.findings[0].slug, "surprising-x");
        assert_eq!(outcome.written.len(), 1);
        let page = std::fs::read_to_string(&outcome.written[0]).unwrap();
        assert!(page.contains("reviewed: false"), "missing flag: {page}");
        assert!(page.contains("session_id: \"abc123def4567\""));
        assert!(page.contains("prompt_version: 1"));
        assert!(page.contains("# A surprising thing about X"));

        // Index entry's `distilled` flag flipped.
        let idx = read_index(&dir.path().join(".coral/sessions/index.json")).unwrap();
        assert!(idx.sessions[0].distilled);
    }

    /// `--apply` writes the synthesis page additionally under
    /// `.wiki/synthesis/`.
    #[test]
    fn distill_apply_also_writes_under_wiki_synthesis() {
        let dir = TempDir::new().unwrap();
        let _entry = seed_session(
            dir.path(),
            "ssn-apply-1",
            r#"{"type":"user","sessionId":"ssn-apply-1","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{"content":"hi"}}
"#,
        );
        let runner = mock_runner_with_yaml(
            r#"findings:
  - slug: applied-thing
    title: Applied finding
    body: |
      A counter-intuitive aspect of the captured conversation: the read path needs the lock for cross-process safety, not just inside-process — see atomic.rs::with_exclusive_lock for the rationale.
    sources: []
"#,
        );
        let opts = DistillOptions {
            project_root: dir.path().to_path_buf(),
            session_id: "ssn-apply-1".into(),
            apply: true,
            model: None,
        };
        let outcome = distill_session(&opts, &runner, "mock-test").unwrap();
        assert_eq!(outcome.written.len(), 2, "distilled + wiki");
        let wiki_path = dir.path().join(".wiki/synthesis/applied-thing.md");
        assert!(
            wiki_path.exists(),
            "wiki page missing: {}",
            wiki_path.display()
        );
        let wiki = std::fs::read_to_string(&wiki_path).unwrap();
        assert!(wiki.contains("reviewed: false"));
    }

    /// Empty findings array surfaces a clear DistillMalformed
    /// rather than silently doing nothing.
    #[test]
    fn distill_empty_findings_returns_malformed_error() {
        let dir = TempDir::new().unwrap();
        let _entry = seed_session(
            dir.path(),
            "ssn-empty",
            r#"{"type":"user","sessionId":"ssn-empty","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{"content":"x"}}
"#,
        );
        let runner = mock_runner_with_yaml("findings: []\n");
        let opts = DistillOptions {
            project_root: dir.path().to_path_buf(),
            session_id: "ssn-empty".into(),
            apply: false,
            model: None,
        };
        let err = distill_session(&opts, &runner, "mock-test").unwrap_err();
        assert!(matches!(err, SessionError::DistillMalformed(_)));
    }

    /// Unsafe slug rejects rather than writing outside synthesis dir.
    #[test]
    fn distill_unsafe_slug_rejects() {
        let dir = TempDir::new().unwrap();
        let _entry = seed_session(
            dir.path(),
            "ssn-evil",
            r#"{"type":"user","sessionId":"ssn-evil","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{"content":"x"}}
"#,
        );
        let runner = mock_runner_with_yaml(
            r#"findings:
  - slug: ../escape
    title: bad
    body: |
      this would escape the synthesis dir if writes weren't gated by is_safe_filename_slug.
    sources: []
"#,
        );
        let opts = DistillOptions {
            project_root: dir.path().to_path_buf(),
            session_id: "ssn-evil".into(),
            apply: false,
            model: None,
        };
        let err = distill_session(&opts, &runner, "mock-test").unwrap_err();
        match err {
            SessionError::DistillMalformed(m) => assert!(m.contains("safe filename slug")),
            other => panic!("expected DistillMalformed, got {other:?}"),
        }
    }

    /// Hard cap: more than 3 findings get truncated.
    #[test]
    fn distill_caps_findings_at_three() {
        let dir = TempDir::new().unwrap();
        let _entry = seed_session(
            dir.path(),
            "ssn-cap",
            r#"{"type":"user","sessionId":"ssn-cap","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{"content":"x"}}
"#,
        );
        let runner = mock_runner_with_yaml(
            r#"findings:
  - { slug: a-finding, title: A, body: "Body A — a substantial note about the codebase that's worth more than a one-liner.", sources: [] }
  - { slug: b-finding, title: B, body: "Body B — a substantial note about the codebase that's worth more than a one-liner.", sources: [] }
  - { slug: c-finding, title: C, body: "Body C — a substantial note about the codebase that's worth more than a one-liner.", sources: [] }
  - { slug: d-finding, title: D, body: "Body D — a substantial note about the codebase that's worth more than a one-liner.", sources: [] }
"#,
        );
        let opts = DistillOptions {
            project_root: dir.path().to_path_buf(),
            session_id: "ssn-cap".into(),
            apply: false,
            model: None,
        };
        let outcome = distill_session(&opts, &runner, "mock-test").unwrap();
        assert_eq!(outcome.findings.len(), 3);
    }

    /// Strip code fences before parsing.
    #[test]
    fn parse_findings_handles_yaml_code_fence() {
        let raw = "```yaml\nfindings:\n  - {slug: x-y, title: T, body: \"Some body that explains the thing comprehensively for a future maintainer\", sources: []}\n```";
        let findings = parse_findings(raw).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].slug, "x-y");
    }

    /// Strip generic backtick fence too.
    #[test]
    fn parse_findings_handles_generic_code_fence() {
        let raw = "```\nfindings:\n  - {slug: x-y, title: T, body: \"Some body that explains the thing comprehensively for a future maintainer\", sources: []}\n```";
        let findings = parse_findings(raw).unwrap();
        assert_eq!(findings.len(), 1);
    }

    /// `render_page` always emits `reviewed: false` and the source
    /// sub-block.
    #[test]
    fn render_page_always_emits_reviewed_false_frontmatter() {
        let f = Finding {
            slug: "test-slug".into(),
            title: "Test Title".into(),
            body: "Body".into(),
            sources: vec!["src/lib.rs".into()],
        };
        let page = render_page(&f, "claude", "sess-x", "2026-05-08T10:00:00+00:00");
        assert!(page.contains("reviewed: false"));
        assert!(page.contains("session_id: \"sess-x\""));
        assert!(page.contains("captured_at: \"2026-05-08T10:00:00+00:00\""));
        assert!(page.contains("- \"src/lib.rs\""));
    }

    /// v0.21.3 spec test #14 — pin the byte-identical contract on the
    /// option (a) page-emit path. Anything new in v0.21.3 lives strictly
    /// behind `--as-patch`; running `distill_session` (no patch flag)
    /// against a fixed input MUST produce the exact same page bytes
    /// that v0.21.2 produced.
    ///
    /// We pin the page bytes (modulo the `generated_at` timestamp,
    /// which is always `now()` and would defeat any byte-identity
    /// check). The frontmatter / body / source-block shape are the
    /// load-bearing parts.
    #[test]
    fn distill_without_as_patch_byte_identical_to_v0212() {
        let f = Finding {
            slug: "fixed-slug".into(),
            title: "Fixed Title".into(),
            body: "A representative body that v0.21.2 would render verbatim into the page.".into(),
            sources: vec!["src/lib.rs".into(), "src/main.rs".into()],
        };
        let page = render_page(&f, "claude", "sess-fixed", "2026-05-08T10:00:00+00:00");
        // Strip the timestamp line so the rest of the page can be
        // pinned byte-for-byte against v0.21.2 output. We pin the
        // expected envelope here so a future edit that quietly shifts
        // the schema (e.g. moving `confidence:` ahead of `slug:`) is
        // caught at test time.
        let lines: Vec<&str> = page
            .lines()
            .filter(|l| !l.starts_with("generated_at:"))
            .collect();
        let stable = lines.join("\n");
        // NB: `render_page` is a hand-rolled string builder. The
        // sources list and `source:` sub-block are written with a
        // 2-space indent; the rest of the frontmatter is at column
        // zero. This is the v0.21.2 shape — pinned here so a future
        // "fix" that touches the indent is caught (existing on-disk
        // pages would round-trip differently).
        let expected = "---\n\
slug: fixed-slug\n\
type: synthesis\n\
last_updated_commit: unknown\n\
confidence: 0.4\n\
status: draft\n\
sources:\n  \
- \"src/lib.rs\"\n  \
- \"src/main.rs\"\n\
backlinks: []\n\
reviewed: false\n\
source:\n  \
runner: \"claude\"\n  \
prompt_version: 1\n  \
session_id: \"sess-fixed\"\n  \
captured_at: \"2026-05-08T10:00:00+00:00\"\n\
---\n\
\n\
# Fixed Title\n\
\n\
A representative body that v0.21.2 would render verbatim into the page.";
        assert_eq!(
            stable, expected,
            "v0.21.3 must keep page-emit byte-identical to v0.21.2 (modulo `generated_at`).\n\
             Got:\n{stable}\n\nExpected:\n{expected}\n"
        );
    }
}
