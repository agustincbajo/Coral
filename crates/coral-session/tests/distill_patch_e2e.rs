//! End-to-end tests for `distill_patch_session` — option (b) /
//! distill-as-patch (v0.21.3).
//!
//! These tests drive a [`MockRunner`] so the LLM call is deterministic.
//! Every test seeds:
//!
//! - `.coral/sessions/<file>.jsonl` (a captured transcript) and
//!   `.coral/sessions/index.json` (with one entry pointing at it),
//! - `.wiki/<slug>.md` (one or more candidate pages),
//! - a [`MockRunner`] queued with one YAML response.
//!
//! Behavior pinned across the suite (per the v0.21.3 spec):
//!
//! 1. `--as-patch` writes pairs of `<id>-<idx>.patch` + `<id>-<idx>.json`
//!    under `.coral/sessions/patches/`.
//! 2. Validation runs BEFORE writes (pre-apply atomicity).
//! 3. `--apply` mutates `.wiki/<target>.md` AND rewrites the touched
//!    page's frontmatter to `reviewed: false`.
//! 4. Path-traversal targets, target-not-in-wiki, malformed diff
//!    headers all reject without writing anything.

use chrono::TimeZone;
use coral_runner::MockRunner;
use coral_session::capture::{CaptureSource, IndexEntry, SessionIndex, read_index, write_index};
use coral_session::distill_patch::{
    DISTILL_PATCH_PROMPT_VERSION, DistillPatchOptions, distill_patch_session,
};
use coral_session::error::SessionError;
use std::path::Path;
use tempfile::TempDir;

/// Seeds a project root with:
///   - `.coral/sessions/index.json` containing one entry for `session_id`
///   - `.coral/sessions/<file>.jsonl` containing a tiny user/assistant
///     transcript so `parse_transcript` returns at least one user turn.
///   - `.wiki/<slug>.md` for each `(slug, body)` pair in `pages`.
fn seed_project(root: &Path, session_id: &str, pages: &[(&str, &str)]) -> std::path::PathBuf {
    let sessions_dir = root.join(".coral/sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let captured_path = sessions_dir.join(format!("{session_id}.jsonl"));
    let transcript = format!(
        r#"{{"type":"user","sessionId":"{session_id}","timestamp":"2026-05-08T10:00:00Z","cwd":"/x","message":{{"content":"how does sliding window auth actually work?"}}}}
{{"type":"assistant","sessionId":"{session_id}","timestamp":"2026-05-08T10:00:01Z","message":{{"role":"assistant","content":[{{"type":"text","text":"It uses a sliding window per request"}}]}}}}
"#
    );
    std::fs::write(&captured_path, transcript).unwrap();
    let entry = IndexEntry {
        session_id: session_id.into(),
        source: CaptureSource::ClaudeCode,
        captured_at: chrono::Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap(),
        captured_path: captured_path.clone(),
        message_count: 2,
        redaction_count: 0,
        distilled: false,
        distilled_outputs: Vec::new(),
        patch_outputs: Vec::new(),
    };
    let idx = SessionIndex {
        sessions: vec![entry],
    };
    write_index(&sessions_dir.join("index.json"), &idx).unwrap();

    for (slug, body) in pages {
        let target = root.join(".wiki").join(format!("{slug}.md"));
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, body).unwrap();
    }
    captured_path
}

/// A minimally valid wiki page (frontmatter + body) suitable as a patch
/// target. The body has 5 distinct lines so a 3-line context patch
/// resolves cleanly.
fn page_body(slug: &str) -> String {
    format!(
        r#"---
slug: {slug}
type: module
last_updated_commit: aaa
confidence: 0.5
status: draft
---

# {slug}

Line one of body content.
Line two of body content.
Line three of body content.
"#
    )
}

/// Builds a unified-diff that appends a single line to the trailing
/// 3-line block of `page_body(slug)`.
fn append_line_diff(slug: &str, new_line: &str) -> String {
    format!(
        r#"--- a/{slug}.md
+++ b/{slug}.md
@@ -10,3 +10,4 @@
 Line one of body content.
 Line two of body content.
 Line three of body content.
+{new_line}
"#
    )
}

/// Spec test #1: happy path without --apply. Two patches → 4 files
/// written under `.coral/sessions/patches/`. `.wiki/` untouched.
#[test]
fn distill_patch_writes_pairs_under_patches_dir() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    seed_project(
        root,
        "session-aaaa-0001",
        &[
            ("modules/auth", &page_body("auth")),
            ("modules/limit", &page_body("limit")),
        ],
    );
    // Snapshot the wiki bytes pre-distill so the no-apply assertion
    // can prove byte-identity.
    let auth_pre = std::fs::read_to_string(root.join(".wiki/modules/auth.md")).unwrap();
    let limit_pre = std::fs::read_to_string(root.join(".wiki/modules/limit.md")).unwrap();

    let yaml = format!(
        r#"patches:
  - target: modules/auth
    rationale: |
      The session revealed auth uses a sliding window.
    diff: |
{}
  - target: modules/limit
    rationale: |
      Rate-limiting note clarified during the session.
    diff: |
{}
"#,
        indent(&append_line_diff(
            "modules/auth",
            "Sliding-window note added."
        )),
        indent(&append_line_diff("modules/limit", "Limit note added."))
    );
    let runner = MockRunner::new();
    runner.push_ok(&yaml);

    let opts = DistillPatchOptions {
        project_root: root.to_path_buf(),
        session_id: "session-aaaa-0001".into(),
        apply: false,
        model: None,
        candidates: 10,
    };
    let outcome = distill_patch_session(&opts, &runner, "mock").expect("ok");
    assert_eq!(outcome.patches.len(), 2);
    assert_eq!(
        outcome.written.len(),
        4,
        "two patches → 4 files (.patch + .json each)"
    );
    assert!(outcome.applied_targets.is_empty(), "no --apply, no targets");

    // Specific filenames per the spec: `<id>-<idx>.patch` + `.json`,
    // 0-indexed, no zero-padding.
    let patches_dir = root.join(".coral/sessions/patches");
    for idx in 0..2 {
        assert!(
            patches_dir
                .join(format!("session-aaaa-0001-{idx}.patch"))
                .exists()
        );
        assert!(
            patches_dir
                .join(format!("session-aaaa-0001-{idx}.json"))
                .exists()
        );
    }

    // Sidecar shape carries provenance fields per spec.
    let sidecar = std::fs::read_to_string(patches_dir.join("session-aaaa-0001-0.json")).unwrap();
    let sidecar_v: serde_json::Value = serde_json::from_str(&sidecar).unwrap();
    assert_eq!(sidecar_v["target_slug"], "modules/auth");
    assert_eq!(sidecar_v["prompt_version"], DISTILL_PATCH_PROMPT_VERSION);
    assert_eq!(sidecar_v["runner_name"], "mock");
    assert_eq!(sidecar_v["session_id"], "session-aaaa-0001");
    assert_eq!(sidecar_v["reviewed"], false);

    // Index updated: entry has both basenames per patch in patch_outputs.
    let idx = read_index(&root.join(".coral/sessions/index.json")).unwrap();
    let entry = &idx.sessions[0];
    assert!(entry.distilled, "distilled flag must flip");
    assert_eq!(
        entry.patch_outputs.len(),
        4,
        "patch_outputs records 4 basenames"
    );
    assert!(
        entry
            .patch_outputs
            .contains(&"session-aaaa-0001-0.patch".to_string())
    );
    assert!(
        entry
            .patch_outputs
            .contains(&"session-aaaa-0001-0.json".to_string())
    );

    // Wiki bytes UNCHANGED — pre-fix this could regress to mutating
    // wiki even on the no-apply path.
    assert_eq!(
        std::fs::read_to_string(root.join(".wiki/modules/auth.md")).unwrap(),
        auth_pre,
        ".wiki/modules/auth.md must be byte-identical without --apply"
    );
    assert_eq!(
        std::fs::read_to_string(root.join(".wiki/modules/limit.md")).unwrap(),
        limit_pre
    );
}

/// Spec test #2: happy path with --apply. Wiki page mutates AND its
/// frontmatter `reviewed: false` is set by Coral (regardless of what
/// the LLM emitted).
#[test]
fn distill_patch_apply_mutates_wiki_and_resets_reviewed() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    seed_project(
        root,
        "session-bbbb-0002",
        &[("modules/auth", &page_body("auth"))],
    );
    let yaml = format!(
        r#"patches:
  - target: modules/auth
    rationale: |
      Adds a sliding-window clarification.
    diff: |
{}
"#,
        indent(&append_line_diff("modules/auth", "Sliding-window note."))
    );
    let runner = MockRunner::new();
    runner.push_ok(&yaml);

    let opts = DistillPatchOptions {
        project_root: root.to_path_buf(),
        session_id: "session-bbbb-0002".into(),
        apply: true,
        model: None,
        candidates: 5,
    };
    let outcome = distill_patch_session(&opts, &runner, "mock").expect("ok");
    assert_eq!(outcome.applied_targets.len(), 1);
    assert_eq!(
        outcome.applied_targets[0],
        root.join(".wiki/modules/auth.md")
    );

    // Wiki page mutated.
    let after = std::fs::read_to_string(root.join(".wiki/modules/auth.md")).unwrap();
    assert!(
        after.contains("Sliding-window note."),
        "wiki page must include the appended line; got:\n{after}"
    );

    // `reviewed: false` lives in the rewritten frontmatter.
    assert!(
        after.contains("reviewed: false"),
        "frontmatter must carry reviewed: false; got:\n{after}"
    );
}

/// Spec test #3: target slug not in `list_page_paths(.wiki)` rejects
/// before any file is written.
#[test]
fn patch_with_unknown_target_rejects_pre_io() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    seed_project(
        root,
        "session-cccc-0003",
        &[("modules/auth", &page_body("auth"))],
    );
    let yaml = format!(
        r#"patches:
  - target: modules/missing-page
    rationale: |
      Edits a page that doesn't exist in this wiki.
    diff: |
{}
"#,
        indent(&append_line_diff("modules/missing-page", "Doomed."))
    );
    let runner = MockRunner::new();
    runner.push_ok(&yaml);
    let opts = DistillPatchOptions {
        project_root: root.to_path_buf(),
        session_id: "session-cccc-0003".into(),
        apply: false,
        model: None,
        candidates: 5,
    };
    let err = distill_patch_session(&opts, &runner, "mock").unwrap_err();
    match err {
        SessionError::DistillMalformed(m) => {
            assert!(m.contains("not a known page"), "got: {m}");
        }
        other => panic!("expected DistillMalformed, got {other:?}"),
    }
    // No files written under .coral/sessions/patches/.
    let patches_dir = root.join(".coral/sessions/patches");
    if patches_dir.exists() {
        let entries: Vec<_> = std::fs::read_dir(&patches_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|s| matches!(s, "patch" | "json"))
                    .unwrap_or(false)
            })
            .collect();
        assert!(
            entries.is_empty(),
            "no .patch/.json should be written; got {} entries",
            entries.len()
        );
    }
}

/// Spec test #4: malformed diff (header mismatch) rejects atomically —
/// caught at parse time before `git apply --check`.
#[test]
fn malformed_diff_rejects_atomically() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    seed_project(
        root,
        "session-dddd-0004",
        &[("modules/auth", &page_body("auth"))],
    );
    // Diff headers say `b/wrong.md` while target is `modules/auth`.
    let yaml = r#"patches:
  - target: modules/auth
    rationale: |
      Headers disagree with target.
    diff: |
      --- a/modules/auth.md
      +++ b/modules/wrong.md
      @@ -10,3 +10,4 @@
       Line one of body content.
       Line two of body content.
       Line three of body content.
      +Doomed line.
"#;
    let runner = MockRunner::new();
    runner.push_ok(yaml);
    let opts = DistillPatchOptions {
        project_root: root.to_path_buf(),
        session_id: "session-dddd-0004".into(),
        apply: false,
        model: None,
        candidates: 5,
    };
    let err = distill_patch_session(&opts, &runner, "mock").unwrap_err();
    match err {
        SessionError::DistillMalformed(m) => {
            assert!(m.contains("headers do not match"), "got: {m}");
        }
        other => panic!("expected DistillMalformed, got {other:?}"),
    }
}

/// Spec test #5: one bad patch rolls back ALL — even if the first
/// patch validates, the second's `git apply --check` failure must
/// keep `.coral/sessions/patches/` empty.
#[test]
fn one_bad_patch_rolls_back_all() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    seed_project(
        root,
        "session-eeee-0005",
        &[
            ("modules/auth", &page_body("auth")),
            ("modules/limit", &page_body("limit")),
        ],
    );
    // First patch is valid. Second targets a real page but with a
    // garbage hunk that won't `git apply --check` — wrong line numbers
    // pointing at content the page doesn't have.
    let bad_diff = "--- a/modules/limit.md\n+++ b/modules/limit.md\n@@ -1000,3 +1000,4 @@\n NOT_PRESENT_one\n NOT_PRESENT_two\n NOT_PRESENT_three\n+broken\n";
    let yaml = format!(
        r#"patches:
  - target: modules/auth
    rationale: |
      Valid first patch.
    diff: |
{}
  - target: modules/limit
    rationale: |
      Doomed second patch with garbage hunk header.
    diff: |
{}
"#,
        indent(&append_line_diff("modules/auth", "Valid line.")),
        indent(bad_diff)
    );
    let runner = MockRunner::new();
    runner.push_ok(&yaml);
    let opts = DistillPatchOptions {
        project_root: root.to_path_buf(),
        session_id: "session-eeee-0005".into(),
        apply: false,
        model: None,
        candidates: 5,
    };
    let err = distill_patch_session(&opts, &runner, "mock").unwrap_err();
    match err {
        SessionError::DistillMalformed(m) => {
            assert!(
                m.contains("git apply --check"),
                "expected git apply --check failure mention; got: {m}"
            );
            assert!(m.contains("patch[1]"), "expected patch index 1; got: {m}");
        }
        other => panic!("expected DistillMalformed, got {other:?}"),
    }
    // No durable files: even though patch[0] was valid, the all-or-nothing
    // contract means nothing is written.
    let patches_dir = root.join(".coral/sessions/patches");
    if patches_dir.exists() {
        let entries: Vec<_> = std::fs::read_dir(&patches_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|s| matches!(s, "patch" | "json"))
                    .unwrap_or(false)
            })
            .collect();
        assert!(
            entries.is_empty(),
            "no .patch/.json should land when one patch fails; got {} entries",
            entries.len()
        );
    }
    // index.json's patch_outputs untouched (still empty).
    let idx = read_index(&root.join(".coral/sessions/index.json")).unwrap();
    assert!(idx.sessions[0].patch_outputs.is_empty());
}

/// Spec test #11: a patch with `..` in target_slug rejects at parse
/// time, before `git apply --check`.
#[test]
fn patch_with_dotdot_target_rejects() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    seed_project(
        root,
        "session-ffff-0006",
        &[("modules/auth", &page_body("auth"))],
    );
    let yaml = "patches:\n  - target: ../escape\n    rationale: |\n      malicious\n    diff: |\n      --- a/escape.md\n      +++ b/escape.md\n      @@ -1 +1 @@\n      -x\n      +y\n";
    let runner = MockRunner::new();
    runner.push_ok(yaml);
    let opts = DistillPatchOptions {
        project_root: root.to_path_buf(),
        session_id: "session-ffff-0006".into(),
        apply: false,
        model: None,
        candidates: 5,
    };
    let err = distill_patch_session(&opts, &runner, "mock").unwrap_err();
    match err {
        SessionError::DistillMalformed(m) => {
            assert!(m.contains("safe path slug"), "got: {m}");
        }
        other => panic!("expected DistillMalformed, got {other:?}"),
    }
}

/// Spec test #12 (parse-side regression): diff headers disagree with
/// target_slug — caught at parse time. Already covered by
/// `malformed_diff_rejects_atomically`, but pin a second case where
/// only one of the two headers is wrong (a real LLM mis-emit shape).
#[test]
fn diff_header_mismatch_rejects_when_only_minus_is_wrong() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    seed_project(
        root,
        "session-gggg-0007",
        &[("modules/auth", &page_body("auth"))],
    );
    let yaml = "patches:\n  - target: modules/auth\n    rationale: |\n      Bad minus header.\n    diff: |\n      --- a/modules/wrong.md\n      +++ b/modules/auth.md\n      @@ -1 +1 @@\n      -x\n      +y\n";
    let runner = MockRunner::new();
    runner.push_ok(yaml);
    let opts = DistillPatchOptions {
        project_root: root.to_path_buf(),
        session_id: "session-gggg-0007".into(),
        apply: false,
        model: None,
        candidates: 5,
    };
    let err = distill_patch_session(&opts, &runner, "mock").unwrap_err();
    assert!(matches!(err, SessionError::DistillMalformed(_)));
}

/// Spec test #13: more than 5 patches truncates at MAX_PATCHES_PER_SESSION.
#[test]
fn patch_count_capped_at_five() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let pages: Vec<(String, String)> = (0..7)
        .map(|i| (format!("modules/p{i}"), page_body(&format!("p{i}"))))
        .collect();
    let pages_ref: Vec<(&str, &str)> = pages
        .iter()
        .map(|(s, b)| (s.as_str(), b.as_str()))
        .collect();
    seed_project(root, "session-hhhh-0008", &pages_ref);

    let mut yaml = String::from("patches:\n");
    for i in 0..7 {
        let slug = format!("modules/p{i}");
        let body_diff = append_line_diff(&slug, &format!("Note {i}."));
        yaml.push_str(&format!(
            "  - target: {slug}\n    rationale: |\n      Note for p{i}.\n    diff: |\n{}\n",
            indent(&body_diff)
        ));
    }
    let runner = MockRunner::new();
    runner.push_ok(&yaml);
    let opts = DistillPatchOptions {
        project_root: root.to_path_buf(),
        session_id: "session-hhhh-0008".into(),
        apply: false,
        model: None,
        candidates: 5,
    };
    let outcome = distill_patch_session(&opts, &runner, "mock").expect("ok");
    assert_eq!(outcome.patches.len(), 5, "capped at 5");
    assert_eq!(outcome.written.len(), 10, "5 patches × 2 files each");
}

/// Spec test #9: `forget_session` cleans BOTH `distilled_outputs`
/// AND `patch_outputs` from `.coral/sessions/patches/`. `.wiki/`
/// mutations from `--apply` are NOT undone (the apply contract says
/// the user owns the wiki post-apply).
#[test]
fn forget_removes_patch_basenames() {
    use coral_session::forget::{ForgetOptions, forget_session};
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    seed_project(
        root,
        "session-iiii-0009",
        &[("modules/auth", &page_body("auth"))],
    );

    // Distill --as-patch with --apply so the wiki AND the patches
    // dir both have artifacts.
    let yaml = format!(
        r#"patches:
  - target: modules/auth
    rationale: |
      Sliding-window note.
    diff: |
{}
"#,
        indent(&append_line_diff("modules/auth", "Note added."))
    );
    let runner = MockRunner::new();
    runner.push_ok(&yaml);
    let opts = DistillPatchOptions {
        project_root: root.to_path_buf(),
        session_id: "session-iiii-0009".into(),
        apply: true,
        model: None,
        candidates: 5,
    };
    distill_patch_session(&opts, &runner, "mock").unwrap();

    // Sanity: the patch + sidecar + wiki mutation all exist.
    let patches_dir = root.join(".coral/sessions/patches");
    let patch_path = patches_dir.join("session-iiii-0009-0.patch");
    let json_path = patches_dir.join("session-iiii-0009-0.json");
    assert!(patch_path.exists());
    assert!(json_path.exists());
    let wiki_after = std::fs::read_to_string(root.join(".wiki/modules/auth.md")).unwrap();
    assert!(wiki_after.contains("Note added."));

    // Forget — both files in `.coral/sessions/patches/` go away.
    let forget_opts = ForgetOptions {
        project_root: root.to_path_buf(),
        session_id: "session-iiii-0009".into(),
    };
    forget_session(&forget_opts).unwrap();
    assert!(!patch_path.exists(), ".patch must be swept");
    assert!(!json_path.exists(), ".json must be swept");
    // `.wiki/` mutation NOT undone — distill's apply is one-way.
    let wiki_post_forget = std::fs::read_to_string(root.join(".wiki/modules/auth.md")).unwrap();
    assert!(
        wiki_post_forget.contains("Note added."),
        "forget must NOT undo `.wiki/` mutations"
    );
}

/// Spec acceptance #5 + #11 cross-cut: option (a) byte-identity is
/// checked from `distill::tests` directly (this file is option (b)
/// only). But pin here that running both flows in the same session
/// works — call distill_session AFTER distill_patch_session and the
/// distilled_outputs / patch_outputs lists are kept disjoint.
#[test]
fn distilled_and_patch_outputs_track_independently() {
    use coral_session::distill::{DistillOptions, distill_session};
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    seed_project(
        root,
        "session-jjjj-0010",
        &[("modules/auth", &page_body("auth"))],
    );

    // First, run distill --as-patch.
    let patch_yaml = format!(
        r#"patches:
  - target: modules/auth
    rationale: |
      Patch flow ran first.
    diff: |
{}
"#,
        indent(&append_line_diff("modules/auth", "Patch path note."))
    );
    let runner = MockRunner::new();
    runner.push_ok(&patch_yaml);
    distill_patch_session(
        &DistillPatchOptions {
            project_root: root.to_path_buf(),
            session_id: "session-jjjj-0010".into(),
            apply: false,
            model: None,
            candidates: 5,
        },
        &runner,
        "mock",
    )
    .unwrap();

    // Then, the page-emit flow.
    let page_runner = MockRunner::new();
    page_runner.push_ok(
        "findings:\n  - slug: cross-cut\n    title: Cross-cut\n    body: |\n      A real-feeling body that satisfies the body check on the page emit path.\n    sources: []\n",
    );
    distill_session(
        &DistillOptions {
            project_root: root.to_path_buf(),
            session_id: "session-jjjj-0010".into(),
            apply: false,
            model: None,
        },
        &page_runner,
        "mock",
    )
    .unwrap();

    let idx = read_index(&root.join(".coral/sessions/index.json")).unwrap();
    let entry = &idx.sessions[0];
    assert!(
        !entry.distilled_outputs.is_empty(),
        "page-emit basenames recorded"
    );
    assert!(
        !entry.patch_outputs.is_empty(),
        "patch-emit basenames recorded"
    );
    // No bleed-through: page outputs end with `.md`, patch outputs end
    // with `.patch` or `.json`.
    for n in &entry.distilled_outputs {
        assert!(n.ends_with(".md"), "distilled output: {n}");
    }
    for n in &entry.patch_outputs {
        assert!(
            n.ends_with(".patch") || n.ends_with(".json"),
            "patch output: {n}"
        );
    }
}

/// Indents every line of `s` by 6 spaces (matching the YAML block
/// scalar indent we use in the literal patch templates above). We
/// keep this as a tiny utility so the test patch construction is
/// readable.
///
/// Note: appends a trailing `\n` so the YAML block-scalar literal
/// preserves the final hunk line. Without it, the YAML parse strips
/// the unterminated last line (looks like a chomp-strip from the
/// parser's POV) and `git apply --check` later barfs with
/// "corrupt patch at line N".
fn indent(s: &str) -> String {
    let mut out: String = s
        .lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("      {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}
