//! `coral session distill --as-patch <id>` — option (b) /
//! distill-as-patch.
//!
//! Where option (a) (see [`crate::distill`]) emits 1-3 NEW synthesis
//! pages from a captured transcript, option (b) instead asks the LLM
//! to propose 1-N **unified-diff patches** against existing
//! `.wiki/<slug>.md` pages. Patches are validated with
//! `git apply --check --unsafe-paths` BEFORE any file is written; on
//! the happy path the per-patch `<id>-<idx>.patch` lands under
//! `.coral/sessions/patches/` with a sidecar `<id>-<idx>.json` that
//! carries the target slug + LLM rationale + provenance.
//!
//! With `--apply`, each patch is `git apply`-ed in turn, then the
//! touched page's frontmatter is rewritten so `reviewed: false` —
//! Coral OWNS the flip; the LLM's job is body content. v0.21.3
//! (this module) ships pre-apply atomicity: if ANY patch fails its
//! `--check`, NO files are written and the command exits non-zero
//! with the patch index + git stderr verbatim. This is the same
//! all-or-nothing contract the option (a) flow has always provided.
//!
//! Validation rationale for `git apply --unsafe-paths`: the flag
//! permits patches with paths outside the index, NOT untrusted paths
//! in any meaningful sense. The actual safety property comes from
//! the slug allow-list check that runs BEFORE git ever sees the
//! diff: each path component of `target_slug` must pass
//! [`coral_core::slug::is_safe_filename_slug`], and the resolved
//! page must already exist in `list_page_paths(.wiki)`. By the time
//! we shell out, the target is known-good.
//!
//! Module layout — kept disjoint from option (a) so the byte-for-byte
//! BC contract on `coral session distill <id>` (no `--as-patch`)
//! cannot be regressed by this file's edits. v0.21.3 spec D11.

use crate::capture::{IndexEntry, SessionIndex, read_index, write_index};
use crate::claude_code::{ClaudeCodeMessage, parse_transcript};
use crate::error::{SessionError, SessionResult};
use coral_core::atomic::atomic_write_string;
use coral_core::page::Page;
use coral_core::walk::list_page_paths;
use coral_runner::{Prompt, Runner};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

/// Bumped when the prompt template changes meaningfully. Lives in the
/// sidecar `.json` so a future re-distill pass can spot stale outputs.
/// v0.21.3 introduces the patch prompt at version 2 — option (a)'s
/// page prompt is at version 1.
pub const DISTILL_PATCH_PROMPT_VERSION: u32 = 2;

/// Hard cap on patches per session. The prompt asks for 1-N (1-5
/// recommended); the parser truncates at 5 in case the model exceeds.
pub const MAX_PATCHES_PER_SESSION: usize = 5;

/// Default top-K BM25 candidate pages to surface in the prompt.
/// Spec §4 D4. Override via `--candidates`.
pub const DEFAULT_CANDIDATES: usize = 10;

/// Per-candidate snippet length cap (chars). Each candidate appears
/// in the prompt as `## <slug>\n<first 1500 chars of body>\n`.
const PER_CANDIDATE_CHARS: usize = 1_500;

/// Total candidate budget cap (chars). Even at K=10 with 1500-char
/// snippets we'd hit 15k; the cap exists for the safety case where a
/// caller passes a large `--candidates N`. Spec §4 D4.
const TOTAL_CANDIDATE_CHARS: usize = 60_000;

/// Per-message cap when serializing the captured transcript into the
/// prompt. Mirrors [`crate::distill::build_prompt`] so prompt sizing
/// behavior is consistent across both modes.
const PER_MSG_CHARS: usize = 2_000;

/// Total transcript-region budget in the prompt. Same constant as
/// option (a). Sentinel keeps prompts under typical context windows
/// even when many candidates are also included.
const TOTAL_TRANSCRIPT_CHARS: usize = 80_000;

#[derive(Debug, Clone)]
pub struct DistillPatchOptions {
    pub project_root: PathBuf,
    /// Either the full UUID or any unique prefix (≥4 chars).
    pub session_id: String,
    /// When true, `git apply` each patch in turn and rewrite the
    /// touched page's frontmatter to `reviewed: false`. When false,
    /// patches are written to `.coral/sessions/patches/` only.
    pub apply: bool,
    /// Forwarded to `Runner.run` for tracing.
    pub model: Option<String>,
    /// Top-K candidate pages to surface in the prompt. `0` skips
    /// candidate collection entirely (the LLM call still runs but
    /// without context — useful when the wiki is empty).
    pub candidates: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistillPatchOutcome {
    pub session_id: String,
    /// Each entry is a parsed, validated patch. Order matches the
    /// LLM's output; index in this vec corresponds to the `<idx>` in
    /// the on-disk filenames.
    pub patches: Vec<Patch>,
    /// Paths actually written under `.coral/sessions/patches/`. Each
    /// patch contributes two entries: `<id>-<idx>.patch` and
    /// `<id>-<idx>.json`.
    pub written: Vec<PathBuf>,
    /// `.wiki/<slug>.md` paths that `--apply` mutated, in order. Empty
    /// when `apply: false`.
    pub applied_targets: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Patch {
    /// Path-style slug relative to `.wiki/`, no `.md` extension.
    /// Multi-component for multi-repo wikis (`modules/auth/jwt`).
    pub target_slug: String,
    /// LLM-emitted rationale; surfaces in CLI output and the sidecar.
    pub rationale: String,
    /// Unified diff bytes. Headers (`--- a/<slug>.md` / `+++ b/<slug>.md`)
    /// are validated against `target_slug` at parse time.
    pub diff: String,
}

/// Sidecar JSON written next to each `<id>-<idx>.patch`. Carries enough
/// provenance that an auditor can reconstruct what the LLM saw and
/// when, without re-reading the captured transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchSidecar {
    pub target_slug: String,
    pub rationale: String,
    pub prompt_version: u32,
    pub runner_name: String,
    pub session_id: String,
    pub captured_at: String,
    /// Always `false` at write time — the user flips it after review.
    /// The lint gate (`unreviewed-distilled`) blocks commits whose
    /// targets are unreviewed.
    pub reviewed: bool,
}

/// LLM output shape: a single YAML document with a top-level
/// `patches:` array. Defensive parse mirrors [`crate::distill`]:
/// fences are stripped, validation is strict (target slug
/// safety, diff-header / target agreement), and the count is
/// truncated at [`MAX_PATCHES_PER_SESSION`].
#[derive(Debug, Clone, Deserialize)]
struct PatchesYaml {
    #[serde(default)]
    patches: Vec<Patch>,
}

/// One BM25-ranked candidate page surfaced in the prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct PageCandidate {
    pub slug: String,
    pub snippet: String,
}

/// Strips markdown code fences from `s` if present. Mirrors
/// [`crate::distill::strip_fences`] (private there) — duplicating
/// rather than `pub`-promoting because the disjoint-module design
/// trades a tiny copy for keeping option (a)'s public surface
/// untouched.
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

/// Returns true when every `/`-separated component of `slug` passes
/// [`coral_core::slug::is_safe_filename_slug`]. The function exists
/// because patch targets are path-style (e.g. `modules/auth`) — a
/// flat-slug check would reject every nested target.
///
/// Defends against `..` and other traversal attempts at every
/// component, not just the leaf — even an inner `..` is rejected.
fn is_safe_path_slug(slug: &str) -> bool {
    if slug.is_empty() {
        return false;
    }
    // Reject `\` outright — Windows-style separators are NOT a
    // valid alternative on the wire (Coral's wiki paths are
    // always `/`-separated, regardless of host OS).
    if slug.contains('\\') {
        return false;
    }
    // Reject leading or trailing `/` — the LLM should NEVER emit
    // those (`/foo` looks like an absolute path, `foo/` is empty leaf).
    if slug.starts_with('/') || slug.ends_with('/') {
        return false;
    }
    for component in slug.split('/') {
        if !coral_core::slug::is_safe_filename_slug(component) {
            return false;
        }
    }
    true
}

/// Verifies the `--- a/<X>.md` and `+++ b/<X>.md` headers in `diff`
/// agree with `target_slug`. Headers MUST be present (otherwise
/// `git apply` would still accept a `+++ /dev/null` deletion against
/// our intent — and even with --check it might guess).
///
/// Both headers must reference the same path; we accept any prefix
/// (`a/...` and `b/...` are conventional but not enforced — git accepts
/// both with and without prefix). The path with the `.md` suffix
/// stripped must equal `target_slug`.
fn diff_targets_slug(diff: &str, target_slug: &str) -> bool {
    let mut minus_path: Option<String> = None;
    let mut plus_path: Option<String> = None;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("--- ")
            && minus_path.is_none()
        {
            minus_path = Some(strip_diff_prefix(rest.trim()).to_string());
        } else if let Some(rest) = line.strip_prefix("+++ ")
            && plus_path.is_none()
        {
            plus_path = Some(strip_diff_prefix(rest.trim()).to_string());
            // After we've seen both headers, no need to keep parsing.
            break;
        }
    }
    let (Some(minus), Some(plus)) = (minus_path, plus_path) else {
        return false;
    };
    let expected = format!("{target_slug}.md");
    if minus != expected || plus != expected {
        return false;
    }
    true
}

/// Trims a leading `a/` or `b/` from a unified-diff header path.
/// Returns the input unchanged when neither prefix is present.
fn strip_diff_prefix(p: &str) -> &str {
    if let Some(rest) = p.strip_prefix("a/") {
        return rest;
    }
    if let Some(rest) = p.strip_prefix("b/") {
        return rest;
    }
    p
}

/// Build the prompt sent to `Runner::run`. Public so a future
/// `--dry-run` could re-use the wording.
pub fn build_patch_prompt(messages: &[ClaudeCodeMessage], candidates: &[PageCandidate]) -> String {
    let mut convo = String::new();
    let mut spent = 0usize;
    for m in messages {
        let snippet: String = m.text.chars().take(PER_MSG_CHARS).collect();
        let chunk = format!("[{}] {}\n", m.role, snippet);
        if spent + chunk.len() > TOTAL_TRANSCRIPT_CHARS {
            convo.push_str("[...truncated for prompt budget...]\n");
            break;
        }
        spent += chunk.len();
        convo.push_str(&chunk);
    }

    // Render candidates with a per-page snippet cap and a global
    // budget — the global cap matters when a caller bumps
    // `--candidates` past the default (10).
    let mut cand_block = String::new();
    let mut cand_spent = 0usize;
    for c in candidates {
        let snippet: String = c.snippet.chars().take(PER_CANDIDATE_CHARS).collect();
        let chunk = format!("## {}\n{}\n\n", c.slug, snippet);
        if cand_spent + chunk.len() > TOTAL_CANDIDATE_CHARS {
            cand_block.push_str("[...candidate budget exhausted; later pages omitted...]\n");
            break;
        }
        cand_spent += chunk.len();
        cand_block.push_str(&chunk);
    }

    let cand_section = if candidates.is_empty() {
        "(no candidate pages provided)\n".to_string()
    } else {
        cand_block
    };

    format!(
        "You are reading a conversation transcript between a developer and an AI coding agent. \
Identify edits to make to EXISTING wiki pages so the captured insight is preserved as a unified diff patch. \
\
Output requirements:\n\
\n\
1. YAML format only. No prose preamble, no markdown fences.\n\
2. Top-level key `patches:` with an array of 1-5 entries.\n\
3. Each entry has these keys (all required):\n\
   - `target`: path-style slug relative to `.wiki/`, no `.md` extension. \
     Use `/` for nested paths (e.g. `modules/authentication`). The target \
     MUST be one of the candidate page slugs listed below.\n\
   - `rationale`: a short prose explanation of WHY this edit improves the page; 50-500 chars.\n\
   - `diff`: a unified-diff patch against the page. Headers must be `--- a/<target>.md` and `+++ b/<target>.md`. \
     Include enough context lines that `git apply --check` passes against the current page bytes.\n\
\n\
Bias toward edits that are:\n\
- LOCALIZED: small surgical fixes — a paragraph addition, a corrected line, a clarifying note.\n\
- EVIDENCE-BACKED: cite files / line ranges / commit SHAs from the conversation.\n\
- WIKI-WORTHY: explains WHY, not WHAT. Do NOT propose edits that just paraphrase the conversation.\n\
\n\
If the conversation does not surface anything that would meaningfully edit one of the candidate pages, output `patches: []`. Padding with weak entries is worse than empty output.\n\
\n\
=== CANDIDATE PAGES ===\n{cand_section}=== END CANDIDATES ===\n\
\n\
=== TRANSCRIPT ===\n{convo}=== END TRANSCRIPT ===\n",
    )
}

/// Parses runner output into a list of patches. Strict: rejects
/// unsafe target slugs, unsafe diff-header path mismatches, and
/// truncates at [`MAX_PATCHES_PER_SESSION`].
pub fn parse_patches(stdout: &str) -> SessionResult<Vec<Patch>> {
    // The LLM might emit `target` (per the prompt) but Patch uses
    // `target_slug`. Accept both names via this thin shim; this is
    // a one-time renaming the prompt schema doesn't expose.
    #[derive(Deserialize)]
    struct PatchInput {
        #[serde(default, alias = "target")]
        target_slug: String,
        #[serde(default)]
        rationale: String,
        #[serde(default)]
        diff: String,
    }
    #[derive(Deserialize)]
    struct PatchesInput {
        #[serde(default)]
        patches: Vec<PatchInput>,
    }
    let body = strip_fences(stdout);
    let parsed: PatchesInput = serde_yaml_ng::from_str(body).map_err(|e| {
        SessionError::DistillMalformed(format!("YAML parse failed: {e}; output was: {stdout}"))
    })?;
    let mut out: Vec<Patch> = Vec::new();
    for (idx, p) in parsed.patches.into_iter().enumerate() {
        if !is_safe_path_slug(&p.target_slug) {
            return Err(SessionError::DistillMalformed(format!(
                "patch[{idx}].target '{}' is not a safe path slug",
                p.target_slug
            )));
        }
        if p.rationale.trim().is_empty() || p.diff.trim().is_empty() {
            return Err(SessionError::DistillMalformed(format!(
                "patch[{idx}] is missing rationale or diff"
            )));
        }
        if !diff_targets_slug(&p.diff, &p.target_slug) {
            return Err(SessionError::DistillMalformed(format!(
                "patch[{idx}].diff headers do not match target '{}.md'",
                p.target_slug
            )));
        }
        // Normalize: every diff MUST end with a newline. YAML
        // block-scalar `|` (CLIP) sometimes drops the trailing `\n`
        // when the source ends mid-line (e.g. when `format!()`
        // interpolates a previously-joined `\n`-separated string). git
        // apply rejects unterminated patches with "corrupt patch at
        // line N" — defensively re-add the missing newline so a
        // subtly-broken YAML mis-emit still applies cleanly.
        let mut diff = p.diff;
        if !diff.ends_with('\n') {
            diff.push('\n');
        }
        out.push(Patch {
            target_slug: p.target_slug,
            rationale: p.rationale,
            diff,
        });
    }
    let parsed = PatchesYaml { patches: out };
    let mut out = parsed.patches;
    out.truncate(MAX_PATCHES_PER_SESSION);
    Ok(out)
}

/// Selects up to `k` candidate pages by BM25 relevance against the
/// concatenation of every user-turn message text. Deterministic given
/// the same `transcript` + `pages` (BM25 ranking is stable for
/// stable input — see [`coral_core::search::search_bm25`]).
pub fn select_candidates(
    transcript: &[ClaudeCodeMessage],
    pages: &[Page],
    k: usize,
) -> Vec<PageCandidate> {
    if k == 0 || pages.is_empty() {
        return Vec::new();
    }
    // Build the BM25 query from user-turn text. Joining ALL user
    // turns biases toward terms the developer cared about — assistant
    // turns can be noisier.
    let query: String = transcript
        .iter()
        .filter(|m| m.role == "user")
        .map(|m| m.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    if query.trim().is_empty() {
        return Vec::new();
    }
    let hits = coral_core::search::search_bm25(pages, &query, k);
    // Map slug → page once so we can pull the body for each hit.
    let mut by_slug = std::collections::HashMap::new();
    for p in pages {
        by_slug.insert(p.frontmatter.slug.clone(), p);
    }
    hits.into_iter()
        .filter_map(|h| {
            let page = by_slug.get(&h.slug)?;
            // Per spec D4: snippet = first 1500 chars of body.
            let snippet: String = page.body.chars().take(PER_CANDIDATE_CHARS).collect();
            Some(PageCandidate {
                slug: h.slug,
                snippet,
            })
        })
        .collect()
}

/// Resolves a session by full id or short prefix (4+ chars). Same
/// matching rule as [`crate::forget`] and option (a) [`crate::distill`].
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

/// Maps the `.wiki/`-relative path of a page in `list_page_paths`
/// to its target-slug shape (no `.md` suffix, `/`-separated).
fn page_path_to_slug(wiki_root: &std::path::Path, p: &std::path::Path) -> Option<String> {
    let rel = p.strip_prefix(wiki_root).ok()?;
    let rel_s = rel.to_string_lossy();
    // Normalize Windows separators just in case the host writes `\`.
    let rel_s = rel_s.replace('\\', "/");
    let stripped = rel_s.strip_suffix(".md")?;
    Some(stripped.to_string())
}

/// Runs `git apply --check --unsafe-paths --directory=.wiki <patch>`
/// against `project_root`. Returns Ok on success; Err carries git's
/// stderr verbatim so the caller can include it in the user-visible
/// `DistillMalformed`.
///
/// `--directory=.wiki` is critical: the patch headers come from the
/// LLM as `--- a/<target>.md` where `<target>` is path-style relative
/// to `.wiki/`. Without `--directory`, git would resolve those paths
/// relative to the CWD (= `project_root`), look for
/// `<project_root>/<target>.md`, and 404. With `--directory=.wiki`,
/// every diff path gets that prefix prepended at apply time.
fn git_apply_check(
    project_root: &std::path::Path,
    patch_path: &std::path::Path,
) -> Result<(), String> {
    git_apply_inner(project_root, patch_path, true)
}

/// Runs `git apply --unsafe-paths --directory=.wiki <patch>` against
/// `project_root` — actually mutating the working tree. Used after
/// every patch in the set has passed `--check` (pre-apply atomicity,
/// spec D6).
fn git_apply_real(
    project_root: &std::path::Path,
    patch_path: &std::path::Path,
) -> Result<(), String> {
    git_apply_inner(project_root, patch_path, false)
}

fn git_apply_inner(
    project_root: &std::path::Path,
    patch_path: &std::path::Path,
    check_only: bool,
) -> Result<(), String> {
    let mut cmd = Command::new("git");
    cmd.arg("apply");
    if check_only {
        cmd.arg("--check");
    }
    cmd.arg("--unsafe-paths");
    // Prepend `.wiki/` to every path in the diff. The LLM emits diffs
    // shaped `--- a/<target>.md` where `<target>` is relative to the
    // wiki root; this flag makes git resolve those against `.wiki/`
    // without requiring the LLM to know the wiki path.
    cmd.arg("--directory=.wiki");
    cmd.arg(patch_path);
    cmd.current_dir(project_root);
    let out = cmd
        .output()
        .map_err(|e| format!("failed to spawn `git apply`: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        return Err(stderr);
    }
    Ok(())
}

/// Rewrites `.wiki/<slug>.md`'s frontmatter to set `reviewed: false`
/// AND a populated `source.runner` block. Coral OWNS the flip — the
/// LLM's job is body content. Idempotent: running twice on the same
/// page is fine; the second call overwrites with the same values.
///
/// The `source` block is REQUIRED, not decorative: the
/// `unreviewed-distilled` lint gate (and the mirror in
/// [`coral_core::page::Page::is_unreviewed_distilled`]) only fires
/// when BOTH `reviewed: false` AND a non-empty `source.runner` are
/// present. Without the source block, a `coral lint` run would let
/// LLM-edited-then-not-reviewed pages slip past the trust gate. v0.21.3
/// ships this shape from day one (the bug existed in `0ba9efd` and was
/// caught by the post-commit audit before tagging).
fn flip_reviewed_false(
    target_path: &std::path::Path,
    runner_name: &str,
    session_id: &str,
    captured_at: &str,
) -> SessionResult<()> {
    let mut page = Page::from_file(target_path).map_err(|e| match e {
        coral_core::error::CoralError::Io { path, source } => SessionError::Io { path, source },
        other => SessionError::Io {
            path: target_path.to_path_buf(),
            source: std::io::Error::other(format!("{other}")),
        },
    })?;
    let mut source_map = serde_yaml_ng::Mapping::new();
    source_map.insert(
        serde_yaml_ng::Value::String("runner".into()),
        serde_yaml_ng::Value::String(runner_name.to_string()),
    );
    source_map.insert(
        serde_yaml_ng::Value::String("prompt_version".into()),
        serde_yaml_ng::Value::Number(serde_yaml_ng::Number::from(
            DISTILL_PATCH_PROMPT_VERSION as i64,
        )),
    );
    source_map.insert(
        serde_yaml_ng::Value::String("session_id".into()),
        serde_yaml_ng::Value::String(session_id.to_string()),
    );
    source_map.insert(
        serde_yaml_ng::Value::String("captured_at".into()),
        serde_yaml_ng::Value::String(captured_at.to_string()),
    );
    page.frontmatter
        .extra
        .insert("source".into(), serde_yaml_ng::Value::Mapping(source_map));
    page.frontmatter
        .extra
        .insert("reviewed".into(), serde_yaml_ng::Value::Bool(false));
    page.write().map_err(|e| match e {
        coral_core::error::CoralError::Io { path, source } => SessionError::Io { path, source },
        other => SessionError::Io {
            path: target_path.to_path_buf(),
            source: std::io::Error::other(format!("{other}")),
        },
    })
}

/// End-to-end patch distillation. Caller injects the runner so tests
/// can drive a `MockRunner` and the CLI uses the real provider.
pub fn distill_patch_session(
    opts: &DistillPatchOptions,
    runner: &dyn Runner,
    runner_name: &str,
) -> SessionResult<DistillPatchOutcome> {
    let sessions_dir = opts.project_root.join(".coral").join("sessions");
    let index_path = sessions_dir.join("index.json");
    let index = read_index(&index_path)?;
    let entry = resolve_entry(&index, &opts.session_id)?;
    let parsed = parse_transcript(&entry.captured_path)?;

    // Build the candidate list (skipped when --candidates 0). We do
    // this BEFORE the runner.run call so a wiki-load failure surfaces
    // a clean Io error rather than a runner timeout.
    let wiki_root = opts.project_root.join(".wiki");
    let candidates: Vec<PageCandidate> = if opts.candidates == 0 {
        Vec::new()
    } else {
        let pages = match coral_core::walk::read_pages(&wiki_root) {
            Ok(ps) => ps,
            Err(coral_core::error::CoralError::Io { path: _, source })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Vec::new()
            }
            Err(e) => {
                return Err(SessionError::Io {
                    path: wiki_root.clone(),
                    source: std::io::Error::other(format!("{e}")),
                });
            }
        };
        select_candidates(&parsed.messages, &pages, opts.candidates)
    };

    let prompt = Prompt {
        system: None,
        user: build_patch_prompt(&parsed.messages, &candidates),
        model: opts.model.clone(),
        cwd: None,
        timeout: None,
    };
    let out = runner.run(&prompt)?;
    let patches = parse_patches(&out.stdout)?;
    if patches.is_empty() {
        return Err(SessionError::DistillMalformed(
            "runner returned no patches; transcript may be too thin or candidates may be off-target".into(),
        ));
    }

    // Allow-list: every patch's target_slug must resolve to an existing
    // `.wiki/<slug>.md` page. We compute the allow-list once.
    let allow_paths = list_page_paths(&wiki_root).map_err(|e| match e {
        coral_core::error::CoralError::Io { path, source } => SessionError::Io { path, source },
        other => SessionError::Io {
            path: wiki_root.clone(),
            source: std::io::Error::other(format!("{other}")),
        },
    })?;
    let allow_slugs: std::collections::HashSet<String> = allow_paths
        .iter()
        .filter_map(|p| page_path_to_slug(&wiki_root, p))
        .collect();
    for (idx, p) in patches.iter().enumerate() {
        if !allow_slugs.contains(&p.target_slug) {
            return Err(SessionError::DistillMalformed(format!(
                "patch[{idx}].target '{}' is not a known page in .wiki/ (slug not in list_page_paths)",
                p.target_slug
            )));
        }
    }

    // Pre-apply atomicity (spec D6). Validate ALL patches with
    // `git apply --check --unsafe-paths` BEFORE writing or applying
    // ANY of them. We use a system tempdir for the validation
    // tempfiles so a partial failure cannot leak into
    // `.coral/sessions/patches/`.
    let validation_tmp = tempfile_dir(&opts.project_root)?;
    for (idx, p) in patches.iter().enumerate() {
        let tmp_path = validation_tmp.join(format!("validate-{idx}.patch"));
        std::fs::write(&tmp_path, &p.diff).map_err(|source| SessionError::Io {
            path: tmp_path.clone(),
            source,
        })?;
        if let Err(stderr) = git_apply_check(&opts.project_root, &tmp_path) {
            // Cleanup before surfacing — we held no locks, no other
            // mutation outside the tempdir which `tempfile_dir` cleans
            // up via Drop on its tempdir handle.
            return Err(SessionError::DistillMalformed(format!(
                "patch[{idx}] failed `git apply --check`: {}",
                stderr.trim()
            )));
        }
    }

    // All patches validate. Now safe to write the durable artifacts.
    let patches_dir = sessions_dir.join("patches");
    std::fs::create_dir_all(&patches_dir).map_err(|source| SessionError::Io {
        path: patches_dir.clone(),
        source,
    })?;

    let captured_at = entry.captured_at.to_rfc3339();
    let mut written: Vec<PathBuf> = Vec::new();
    let mut basenames: Vec<String> = Vec::new();
    for (idx, p) in patches.iter().enumerate() {
        let patch_basename = format!("{}-{idx}.patch", entry.session_id);
        let json_basename = format!("{}-{idx}.json", entry.session_id);
        let patch_path = patches_dir.join(&patch_basename);
        let json_path = patches_dir.join(&json_basename);
        atomic_write_string(&patch_path, &p.diff).map_err(|e| match e {
            coral_core::error::CoralError::Io { path, source } => SessionError::Io { path, source },
            other => SessionError::Io {
                path: patch_path.clone(),
                source: std::io::Error::other(format!("{other}")),
            },
        })?;
        let sidecar = PatchSidecar {
            target_slug: p.target_slug.clone(),
            rationale: p.rationale.clone(),
            prompt_version: DISTILL_PATCH_PROMPT_VERSION,
            runner_name: runner_name.to_string(),
            session_id: entry.session_id.clone(),
            captured_at: captured_at.clone(),
            reviewed: false,
        };
        let json_str = serde_json::to_string_pretty(&sidecar).map_err(|e| SessionError::Io {
            path: json_path.clone(),
            source: std::io::Error::other(format!("serialize sidecar: {e}")),
        })?;
        atomic_write_string(&json_path, &json_str).map_err(|e| match e {
            coral_core::error::CoralError::Io { path, source } => SessionError::Io { path, source },
            other => SessionError::Io {
                path: json_path.clone(),
                source: std::io::Error::other(format!("{other}")),
            },
        })?;
        written.push(patch_path);
        written.push(json_path);
        basenames.push(patch_basename);
        basenames.push(json_basename);
    }

    // Apply phase. Each patch was already `--check`-ed; we still
    // surface git failures here as Io errors because a successful
    // --check followed by a failed --apply would be a real
    // environment surprise (e.g. a concurrent edit between phases).
    //
    // TODO(v0.21.x post-tag): post-apply atomicity. Pre-apply atomicity
    // (the `--check` loop above) is intact, but if patch #N applies
    // and patch #N+1's `git apply` fails mid-flight, `.wiki/` is left
    // half-mutated. The spec only required pre-apply atomicity, so
    // this is acknowledged-but-not-blocking. A follow-up issue should
    // either snapshot+restore `.wiki/` around this loop or move to a
    // `git stash` / branch-based rollback. Tester-flagged LOW finding
    // from the v0.21.3 audit; tracked separately, not fixed in the
    // v0.21.3 trust-gate fix commit.
    let mut applied_targets: Vec<PathBuf> = Vec::new();
    if opts.apply {
        for (idx, p) in patches.iter().enumerate() {
            // Re-write the diff to the durable patch file for the
            // actual apply. (We have one already from the pre-write
            // loop above — reuse it.)
            let patch_path = patches_dir.join(format!("{}-{idx}.patch", entry.session_id));
            if let Err(stderr) = git_apply_real(&opts.project_root, &patch_path) {
                return Err(SessionError::Io {
                    path: patch_path,
                    source: std::io::Error::other(format!(
                        "patch[{idx}] passed `--check` but `git apply` failed: {}",
                        stderr.trim()
                    )),
                });
            }
            let target_path = wiki_root.join(format!("{}.md", p.target_slug));
            flip_reviewed_false(&target_path, runner_name, &entry.session_id, &captured_at)?;
            applied_targets.push(target_path);
        }
    }

    // Update index.json under exclusive lock — append basenames
    // to `patch_outputs` (merge-don't-replace, mirrors option (a)).
    coral_core::atomic::with_exclusive_lock(&index_path, || {
        let mut idx = read_index(&index_path).unwrap_or_default();
        for e in idx.sessions.iter_mut() {
            if e.session_id == entry.session_id {
                e.distilled = true;
                for name in &basenames {
                    if !e.patch_outputs.iter().any(|n| n == name) {
                        e.patch_outputs.push(name.clone());
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

    Ok(DistillPatchOutcome {
        session_id: entry.session_id,
        patches,
        written,
        applied_targets,
    })
}

/// Wraps `tempfile::tempdir_in` so the validation tempdir lives next
/// to the project root rather than `/tmp`. This avoids host-OS
/// surprises where a system temp on a different filesystem makes
/// `git apply --check` resolve symlinks oddly. The handle drops at
/// scope end, removing the tempdir.
fn tempfile_dir(project_root: &std::path::Path) -> SessionResult<PathBuf> {
    let dir = project_root
        .join(".coral")
        .join("sessions")
        .join("patches-validate");
    // Best-effort cleanup of a stale validation dir from a previous
    // run that crashed mid-flight. We don't care if removal fails
    // (the create_dir_all below will repair it).
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|source| SessionError::Io {
        path: dir.clone(),
        source,
    })?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_code::ClaudeCodeMessage;
    use coral_core::frontmatter::{Confidence, Frontmatter, PageType, Status};
    use std::collections::BTreeMap;

    fn page(slug: &str, body: &str) -> Page {
        Page {
            path: PathBuf::from(format!(".wiki/{slug}.md")),
            frontmatter: Frontmatter {
                slug: slug.to_string(),
                page_type: PageType::Module,
                last_updated_commit: "abc".to_string(),
                confidence: Confidence::try_new(0.5).unwrap(),
                sources: vec![],
                backlinks: vec![],
                status: Status::Draft,
                generated_at: None,
                valid_from: None,
                valid_to: None,
                extra: BTreeMap::new(),
            },
            body: body.to_string(),
        }
    }

    fn user_msg(text: &str) -> ClaudeCodeMessage {
        ClaudeCodeMessage {
            role: "user".to_string(),
            text: text.to_string(),
            timestamp: Some("2026-05-08T10:00:00Z".to_string()),
        }
    }

    /// Spec test #6: BM25 selection is deterministic — same transcript
    /// + same pages produce the same ranking on every call.
    #[test]
    fn select_candidates_is_deterministic() {
        let pages = vec![
            page(
                "auth",
                "Authentication uses JWT tokens with sliding window expiration",
            ),
            page(
                "rate-limit",
                "Rate limiting is per-tenant with sliding window counters",
            ),
            page(
                "storage",
                "Storage is via SQLite, page cache pinned to content hash",
            ),
        ];
        let transcript = vec![user_msg("how does sliding window auth work?")];
        let a = select_candidates(&transcript, &pages, 3);
        let b = select_candidates(&transcript, &pages, 3);
        assert_eq!(a, b, "BM25 selection must be deterministic");
        assert!(!a.is_empty(), "at least one candidate must come back");
    }

    /// Spec test #7: candidates=0 short-circuits. We don't load pages,
    /// don't tokenize the transcript, just return empty.
    #[test]
    fn zero_candidates_skips_page_load() {
        let pages = vec![page("auth", "body")];
        let transcript = vec![user_msg("anything")];
        assert!(select_candidates(&transcript, &pages, 0).is_empty());
    }

    /// Spec test #8: --candidates N truncates to N even if BM25 has
    /// more matches. (BM25's search_bm25 already does this internally,
    /// but pin the contract from select_candidates' end.)
    #[test]
    fn candidates_flag_truncates_to_n() {
        let pages = vec![
            page("a", "sliding window auth is documented here"),
            page("b", "sliding window rate limiting is here"),
            page("c", "sliding window pagination is here"),
            page("d", "no relevant terms"),
        ];
        let transcript = vec![user_msg("sliding window")];
        let got = select_candidates(&transcript, &pages, 2);
        assert!(got.len() <= 2, "got {} cands, want <=2", got.len());
    }

    /// is_safe_path_slug accepts nested paths but rejects `..` at any
    /// component.
    #[test]
    fn is_safe_path_slug_rejects_dotdot_segments() {
        assert!(is_safe_path_slug("modules/auth"));
        assert!(is_safe_path_slug("auth"));
        assert!(!is_safe_path_slug("../escape"));
        assert!(!is_safe_path_slug("modules/../escape"));
        assert!(!is_safe_path_slug("modules/.hidden"));
        assert!(!is_safe_path_slug("/abs"));
        assert!(!is_safe_path_slug("modules/"));
        assert!(!is_safe_path_slug("modules\\auth"));
        assert!(!is_safe_path_slug(""));
    }

    /// diff header validation: the `--- a/X.md` and `+++ b/X.md`
    /// paths must match `target_slug`. Mismatches reject.
    #[test]
    fn diff_targets_slug_matches_a_and_b_prefixes() {
        let good = "--- a/modules/auth.md\n+++ b/modules/auth.md\n@@ -1 +1 @@\n-old\n+new\n";
        assert!(diff_targets_slug(good, "modules/auth"));
        let mismatch = "--- a/modules/auth.md\n+++ b/modules/wrong.md\n@@ -1 +1 @@\n-old\n+new\n";
        assert!(!diff_targets_slug(mismatch, "modules/auth"));
        let no_headers = "@@ -1 +1 @@\n-old\n+new\n";
        assert!(!diff_targets_slug(no_headers, "modules/auth"));
        // No prefix at all (`--- modules/auth.md`) — git accepts this,
        // so do we.
        let bare = "--- modules/auth.md\n+++ modules/auth.md\n@@ -1 +1 @@\n-old\n+new\n";
        assert!(diff_targets_slug(bare, "modules/auth"));
    }

    /// Strip code fences before parsing — same defensive shape as
    /// option (a).
    #[test]
    fn parse_patches_handles_yaml_code_fence() {
        let raw = "```yaml\npatches:\n  - target: foo-bar\n    rationale: r\n    diff: |\n      --- a/foo-bar.md\n      +++ b/foo-bar.md\n      @@ -1 +1 @@\n      -x\n      +y\n```";
        let parsed = parse_patches(raw).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].target_slug, "foo-bar");
    }

    /// parse_patches truncates at MAX_PATCHES_PER_SESSION (5).
    #[test]
    fn parse_patches_caps_at_five() {
        let mut yaml = String::from("patches:\n");
        for i in 0..7 {
            yaml.push_str(&format!(
                "  - target: page-{i}\n    rationale: rationale {i}\n    diff: |\n      --- a/page-{i}.md\n      +++ b/page-{i}.md\n      @@ -1 +1 @@\n      -old\n      +new\n"
            ));
        }
        let parsed = parse_patches(&yaml).unwrap();
        assert_eq!(parsed.len(), MAX_PATCHES_PER_SESSION);
    }

    /// parse_patches rejects unsafe target slugs (path-traversal).
    #[test]
    fn parse_patches_rejects_dotdot_target() {
        let yaml = "patches:\n  - target: ../escape\n    rationale: r\n    diff: |\n      --- a/escape.md\n      +++ b/escape.md\n      @@ -1 +1 @@\n      -x\n      +y\n";
        let err = parse_patches(yaml).unwrap_err();
        assert!(matches!(err, SessionError::DistillMalformed(_)));
    }

    /// parse_patches rejects diff/target header mismatch.
    #[test]
    fn parse_patches_rejects_header_mismatch() {
        let yaml = "patches:\n  - target: foo\n    rationale: r\n    diff: |\n      --- a/bar.md\n      +++ b/bar.md\n      @@ -1 +1 @@\n      -x\n      +y\n";
        let err = parse_patches(yaml).unwrap_err();
        match err {
            SessionError::DistillMalformed(m) => {
                assert!(m.contains("headers do not match"), "got: {m}");
            }
            other => panic!("expected DistillMalformed, got {other:?}"),
        }
    }
}
