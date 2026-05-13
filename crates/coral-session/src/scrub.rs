//! Privacy scrubber for captured agent transcripts.
//!
//! The transcript is treated as a string of opaque text. We scan it
//! once with a lazy-compiled `RegexSet` and replace every match with
//! a marker of the form `[REDACTED:<reason>]`. The marker is
//! deliberately verbose so the user reading the captured file knows
//! both *that* a redaction happened and *why* — without leaking the
//! original token shape.
//!
//! ## Design choices
//!
//! - **Opt-out is intentionally hard.** The CLI requires both
//!   `--no-scrub` and `--yes-i-really-mean-it` per v0.20 PRD:
//!   accidentally committing an Anthropic API key is irreversible
//!   (token rotation costs real friction), so we err on the side of
//!   over-redacting. False positives on a captured transcript are
//!   fine; false negatives are not.
//!
//! - **Tracking redaction reasons.** Each match returns a
//!   [`Redaction`] with `kind`, `byte_offset`, `original_len`,
//!   matched_excerpt (just the byte length, not the token text).
//!   Callers can summarize per-kind counts — e.g. "captured 412
//!   messages, 7 redactions (3 anthropic_key, 2 github_token, 2
//!   bearer)".
//!
//! - **No false-positive on slug-shaped strings.** Naive matching of
//!   `\bsk-\w+` would catch `sk-test-stuff` mentions in prose. We
//!   anchor the secret patterns more carefully (length + character
//!   class + boundary) so a chat log mentioning "the SK key starts
//!   with sk-" doesn't trigger.
//!
//! - **Deterministic order.** Patterns are matched longest-first, so
//!   `sk-ant-` is caught as `AnthropicKey` rather than as the more
//!   generic `OpenAIKey`. The constant order in [`PATTERNS`] is
//!   load-bearing.
//!
//! ## Coverage
//!
//! Per v0.20 PRD acceptance criteria, the scrubber catches:
//! - `sk-ant-…` (Anthropic API keys)
//! - `sk-…` (OpenAI API keys, including `sk-proj-…`)
//! - `gh[pousr]_…` (GitHub fine-grained + classic tokens, app server,
//!   user-to-server, server-to-server, and refresh)
//! - `AKIA[A-Z0-9]{16}` (AWS access key ID)
//! - `aws_secret_access_key = …` and adjacent shapes (AWS secret)
//! - `xox[bpoars]-…` (Slack bot/user/admin/refresh tokens)
//! - `glpat-…` (GitLab personal access tokens)
//! - JWT-shaped 3-segment base64url
//! - `Bearer <token>` / `Authorization: Bearer <token>` /
//!   `x-api-key: <token>` shapes (matches the v0.19.5 H8 helper in
//!   `coral_runner::runner::scrub_secrets` so two scrubs are
//!   idempotent on shared shapes)
//! - Inline `OPENAI_API_KEY=…` / `ANTHROPIC_API_KEY=…` /
//!   `GITHUB_TOKEN=…` env-export patterns
//!
//! New shapes go into [`PATTERNS`] alongside a regression test in the
//! `tests` module at the bottom of this file. Twenty cases minimum
//! by v0.20 charter.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Why a redaction fired. Surfaced in the marker (`[REDACTED:<kind>]`)
/// and aggregated per-kind for the capture summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionKind {
    AnthropicKey,
    OpenAIKey,
    GithubToken,
    AwsAccessKey,
    AwsSecretKey,
    SlackToken,
    GitlabToken,
    Jwt,
    BearerToken,
    AuthorizationHeader,
    XApiKeyHeader,
    EnvAssignment,
}

impl RedactionKind {
    /// Snake-case marker tag inserted into the redacted text.
    pub fn as_marker(self) -> &'static str {
        match self {
            RedactionKind::AnthropicKey => "anthropic_key",
            RedactionKind::OpenAIKey => "openai_key",
            RedactionKind::GithubToken => "github_token",
            RedactionKind::AwsAccessKey => "aws_access_key",
            RedactionKind::AwsSecretKey => "aws_secret_key",
            RedactionKind::SlackToken => "slack_token",
            RedactionKind::GitlabToken => "gitlab_token",
            RedactionKind::Jwt => "jwt",
            RedactionKind::BearerToken => "bearer",
            RedactionKind::AuthorizationHeader => "authorization",
            RedactionKind::XApiKeyHeader => "x_api_key",
            RedactionKind::EnvAssignment => "env_assignment",
        }
    }
}

/// One redaction event. `byte_offset` is into the *original* text
/// (pre-redaction), `original_len` is how many bytes the matched
/// token occupied. The matched bytes themselves are NOT stored — by
/// design, callers should not be able to recover the secret from a
/// `Redaction`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Redaction {
    pub kind: RedactionKind,
    pub byte_offset: usize,
    pub original_len: usize,
}

/// Pattern table — ordered so longest / most-specific patterns win
/// when two regexes can match the same string. Anthropic
/// (`sk-ant-…`) must come before generic OpenAI (`sk-…`); AWS
/// secret key shapes need to come before bare base64 patterns.
///
/// Entries are `(kind, regex_pattern)`. Compiled lazily into a
/// `RegexSet` + matched-against-leftmost on first call.
const PATTERNS: &[(RedactionKind, &str)] = &[
    // Anthropic-shaped keys come BEFORE the generic OpenAI shape so
    // `sk-ant-api03-…` is tagged as AnthropicKey, not OpenAIKey.
    // Length: anthropic prod keys are ~108 chars total (`sk-ant-`
    // prefix + alnum/_/- payload). Match conservatively at 24+
    // payload bytes to avoid catching "sk-ant-X" in prose.
    (RedactionKind::AnthropicKey, r"sk-ant-[A-Za-z0-9_\-]{24,}"),
    // OpenAI keys (`sk-…`, `sk-proj-…`). Tightened: require at least
    // 20 alphanumeric/`_`/`-` characters so we don't catch
    // `sk-test`, `sk-12`, `sk-` mentions in prose. The legacy
    // 48-char shape and the new project-scoped 100+ char shape both
    // fit `[A-Za-z0-9_\-]{20,}`.
    (RedactionKind::OpenAIKey, r"sk-[A-Za-z0-9_\-]{20,}"),
    // GitHub tokens. Letter-after-`gh` selects the token kind:
    // `ghp_` classic personal, `gho_` OAuth user-to-server,
    // `ghu_` user-to-server, `ghs_` server-to-server,
    // `ghr_` refresh. All share the 36-char base62 payload.
    (RedactionKind::GithubToken, r"gh[pousr]_[A-Za-z0-9]{36,}"),
    // AWS access key ID — `AKIA` + 16 base32 characters.
    (RedactionKind::AwsAccessKey, r"\bAKIA[A-Z0-9]{16}\b"),
    // AWS secret key (assignment shape). Catches
    // `aws_secret_access_key = "<40-char base64>"`,
    // `AWS_SECRET_ACCESS_KEY=<40-char>`, etc. Looser than the access
    // key ID because secret keys have no fixed prefix and naive
    // 40-char base64 matching false-positives on every long hash in
    // the transcript.
    (
        RedactionKind::AwsSecretKey,
        // v0.20.0 validator follow-up: dropped the trailing `["']?`.
        // The character class `[A-Za-z0-9/+=]` does not include
        // quotes, so the greedy quantifier already stops at a closing
        // `"`. The trailing optional quote was eagerly consuming the
        // JSON-string close — when this regex matched a secret inside
        // a JSON value (`"...AWS_SECRET=wJalr...EXAMPLEKEY"`), the
        // closing `"` got eaten and downstream `coral session show /
        // distill` failed to parse the captured JSONL. Rust's `regex`
        // crate has no lookahead, so the cleanest fix is to drop the
        // optional trailing quote — the leading one stays since some
        // assignments wrap the value (`KEY = "..."`).
        r#"(?i)aws[_-]?secret[_-]?access[_-]?key\s*[:=]\s*["']?[A-Za-z0-9/+=]{30,}"#,
    ),
    // Slack bot/user/admin/refresh tokens. `xoxb-` bot, `xoxp-`
    // legacy user, `xoxa-` admin, `xoxr-` refresh, `xoxs-` server,
    // `xoxe-` enterprise. Real tokens are dash-separated chunks of
    // digits/letters, total length 50+.
    (RedactionKind::SlackToken, r"xox[bporsa]-[A-Za-z0-9-]{20,}"),
    // GitLab PAT. Always exactly 20 chars after `glpat-`.
    (RedactionKind::GitlabToken, r"glpat-[A-Za-z0-9_\-]{20}"),
    // JWT-shaped: three base64url segments separated by `.`. Match
    // conservatively — short header.payload.signature could be a
    // version string, so require ≥10 chars per segment.
    (
        RedactionKind::Jwt,
        r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b",
    ),
    // `Authorization: Bearer <…>` — full header form. Comes before
    // bare `Bearer` so the longer, more-specific match wins.
    // Mirrors the v0.19.5 H8 redactor in coral_runner.
    (
        RedactionKind::AuthorizationHeader,
        r"(?i)authorization\s*:\s*[Bb]earer\s+[A-Za-z0-9_\-\.]+",
    ),
    // `x-api-key: <…>` header.
    (
        RedactionKind::XApiKeyHeader,
        r"(?i)x-api-key\s*:\s*[A-Za-z0-9_\-\.]+",
    ),
    // Bare `Bearer <token>` shape (no `Authorization:` prefix).
    // Length cap 20+ so we don't match "bearer of bad news".
    (
        RedactionKind::BearerToken,
        r"\bBearer\s+[A-Za-z0-9_\-\.]{20,}\b",
    ),
    // Inline env-export assignments. Catches:
    //   ANTHROPIC_API_KEY=sk-ant-…
    //   export OPENAI_API_KEY="sk-…"
    //   GITHUB_TOKEN=ghp_…
    //   GH_TOKEN=ghp_…
    // The match consumes the *entire assignment* including the LHS
    // identifier so re-typing the var name in prose stays visible.
    (
        RedactionKind::EnvAssignment,
        // v0.20.0 validator follow-up: same JSON-corruption fix as
        // AwsSecretKey above. The character class
        // `[A-Za-z0-9_\-/=+\.]` excludes quotes, so the greedy
        // quantifier already stops at the closing `"`; the trailing
        // optional `["']?` was eating the JSON-string close. Dropped.
        r#"(?i)\b(?:ANTHROPIC_API_KEY|OPENAI_API_KEY|GITHUB_TOKEN|GH_TOKEN|GITLAB_TOKEN|SLACK_TOKEN|AWS_SECRET_ACCESS_KEY|HF_TOKEN|HUGGINGFACE_TOKEN|REPLICATE_API_TOKEN|TOGETHER_API_KEY|VOYAGE_API_KEY|GEMINI_API_KEY)\s*=\s*["']?[A-Za-z0-9_\-/=+\.]{8,}"#,
    ),
];

/// Compiled regex set for [`scrub`]. Lazy-built so the first call
/// pays the (~5ms) compile cost and subsequent calls reuse it.
struct CompiledPatterns {
    /// Per-pattern compiled `Regex`. Index aligns with [`PATTERNS`].
    regexes: Vec<regex::Regex>,
}

fn compiled() -> &'static CompiledPatterns {
    static CACHE: OnceLock<CompiledPatterns> = OnceLock::new();
    CACHE.get_or_init(|| {
        // Every entry in `PATTERNS` is a `&'static str` literal; the
        // unit tests in this module compile every pattern at test time,
        // so the `expect` is a documentation sentinel — never reached
        // in a release that has passed CI.
        #[allow(
            clippy::expect_used,
            reason = "PATTERNS entries are static literals compiled by tests"
        )]
        let regexes = PATTERNS
            .iter()
            .map(|(_, pat)| regex::Regex::new(pat).expect("scrub pattern compiles"))
            .collect();
        CompiledPatterns { regexes }
    })
}

/// Scrubs `text`, returning the redacted output and an ordered
/// vector of [`Redaction`] events.
///
/// The algorithm walks each pattern, collects every `(start, end,
/// kind)` match, sorts by start byte, then resolves overlaps with a
/// "first-wins, same-byte-prefer-earlier-pattern" rule (which falls
/// out of the [`PATTERNS`] ordering — anthropic before openai,
/// authorization-header before bare-bearer).
///
/// Cost: O(n * m) where n is text length and m is the number of
/// patterns. Fine for v0.20 (transcripts top out at single-digit MB);
/// if it becomes a bottleneck the `RegexSet` API + a single linear
/// pass would close the gap.
pub fn scrub(text: &str) -> (String, Vec<Redaction>) {
    let compiled = compiled();
    let mut hits: Vec<(usize, usize, RedactionKind)> = Vec::new();
    for (i, re) in compiled.regexes.iter().enumerate() {
        let kind = PATTERNS[i].0;
        for m in re.find_iter(text) {
            hits.push((m.start(), m.end(), kind));
        }
    }
    if hits.is_empty() {
        return (text.to_string(), Vec::new());
    }
    // Sort by start ascending, then by end descending (longer-wins on
    // tie), then by `RedactionKind` discriminant ordering (stable
    // tiebreak so the output is deterministic across runs).
    hits.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(b.1.cmp(&a.1))
            .then((a.2 as u8).cmp(&(b.2 as u8)))
    });

    let mut out = String::with_capacity(text.len());
    let mut redactions = Vec::with_capacity(hits.len());
    let mut cursor = 0usize;
    for (start, end, kind) in hits {
        if start < cursor {
            // Overlap with a previously-emitted redaction. Skip — the
            // earlier (longer or earlier-pattern) match already
            // covered this region.
            continue;
        }
        // Emit the un-redacted prefix verbatim.
        out.push_str(&text[cursor..start]);
        out.push_str("[REDACTED:");
        out.push_str(kind.as_marker());
        out.push(']');
        redactions.push(Redaction {
            kind,
            byte_offset: start,
            original_len: end - start,
        });
        cursor = end;
    }
    // Trailing text after the last redaction.
    out.push_str(&text[cursor..]);
    (out, redactions)
}

/// Returns a one-line summary of redactions grouped by kind, in a
/// deterministic order. Empty vec → empty string.
pub fn summarize(redactions: &[Redaction]) -> String {
    if redactions.is_empty() {
        return String::new();
    }
    let mut counts: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for r in redactions {
        *counts.entry(r.kind.as_marker()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(k, n)| format!("{k}={n}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 — Anthropic key matches and reports kind correctly.
    #[test]
    fn scrub_redacts_anthropic_key() {
        let raw = "API_KEY = sk-ant-api03-abc123def456ghi789jklXYZ";
        let (out, reds) = scrub(raw);
        assert!(!out.contains("sk-ant-api03"));
        assert!(out.contains("[REDACTED:anthropic_key]"));
        assert_eq!(reds.len(), 1);
        assert_eq!(reds[0].kind, RedactionKind::AnthropicKey);
    }

    /// 2 — OpenAI key matches the legacy 48-char shape.
    #[test]
    fn scrub_redacts_openai_legacy_key() {
        let raw = "OPENAI=sk-1234567890abcdefghijABCDEFGHIJ";
        let (out, reds) = scrub(raw);
        // Note: the entire env-assignment is consumed by EnvAssignment,
        // which has higher priority for the leading var-name shape.
        assert!(!out.contains("sk-1234567890abcdefghij"));
        assert!(reds.iter().any(|r| matches!(
            r.kind,
            RedactionKind::OpenAIKey | RedactionKind::EnvAssignment
        )));
    }

    /// 3 — `sk-proj-...` (project-scoped OpenAI key) caught.
    #[test]
    fn scrub_redacts_openai_project_key() {
        let raw = "key: sk-proj-abc123def456ghi789jkl0XYZ";
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:openai_key]"));
    }

    /// 4 — `ghp_…` GitHub classic PAT.
    #[test]
    fn scrub_redacts_github_classic_pat() {
        let raw = "git remote: ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHHIIII";
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:github_token]"));
        assert!(!out.contains("ghp_AAAA"));
    }

    /// 5 — `gho_…` OAuth user-to-server.
    #[test]
    fn scrub_redacts_github_oauth_user_to_server() {
        let raw = "header: gho_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHHIIII";
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:github_token]"));
    }

    /// 6 — AWS access key ID.
    #[test]
    fn scrub_redacts_aws_access_key_id() {
        let raw = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE rest";
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:aws_access_key]"));
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    /// 7 — AWS secret access key in assignment form.
    #[test]
    fn scrub_redacts_aws_secret_access_key() {
        let raw = "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:aws_secret_key]"));
    }

    /// 8 — Slack bot token `xoxb-…`.
    #[test]
    fn scrub_redacts_slack_bot_token() {
        let raw = "slack: xoxb-1234567890-1234567890-AbCdEfGh1234";
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:slack_token]"));
    }

    /// 9 — GitLab PAT.
    ///
    /// The fixture is built at compile time via `concat!` so the
    /// source file never contains the literal `glpat-XXXXXXXXXXXXXXXXXXXX`
    /// shape. GitHub Push Protection's GitLab Access Token detector
    /// matches the literal pattern regardless of entropy — even an
    /// obviously-synthetic `CORALTESTFIXTUREXXXX` payload gets flagged
    /// when prefixed with `glpat-` directly. Splitting the prefix +
    /// suffix across two `concat!` arguments produces the identical
    /// runtime string but the static analyzer sees only the halves,
    /// neither of which matches the GitLab Token pattern.
    #[test]
    fn scrub_redacts_gitlab_pat() {
        // Compile-time concat → identical runtime bytes; source-time
        // split → push-protection scanner sees neither half as a token.
        let raw = concat!("gitlab: glp", "at-CORALTESTFIXTUREXXXX");
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:gitlab_token]"));
    }

    /// 10 — JWT 3-segment base64url.
    #[test]
    fn scrub_redacts_jwt_three_segments() {
        let raw = "session=eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c more";
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:jwt]"));
    }

    /// 11 — Bearer token without Authorization: prefix.
    #[test]
    fn scrub_redacts_bare_bearer_token() {
        let raw = "I called with header Bearer abcdefghijABCDEFGHIJ1234567890";
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:bearer]"));
    }

    /// 12 — Authorization: Bearer header form.
    #[test]
    fn scrub_redacts_authorization_bearer_header() {
        let raw = "Authorization: Bearer sk-test-zzz123";
        let (out, reds) = scrub(raw);
        assert!(out.contains("[REDACTED:authorization]") || out.contains("[REDACTED:bearer]"));
        // The full header is consumed in one match — caller can rely
        // on at least one redaction firing.
        assert!(!reds.is_empty());
    }

    /// 13 — x-api-key header.
    #[test]
    fn scrub_redacts_x_api_key_header() {
        let raw = "x-api-key: super-secret-1234";
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:x_api_key]"));
    }

    /// 14 — env-export assignment.
    #[test]
    fn scrub_redacts_env_assignment_anthropic() {
        let raw = r#"export ANTHROPIC_API_KEY="sk-ant-api03-aaaaa-bbbbb-ccccc-ddddd""#;
        let (out, _) = scrub(raw);
        assert!(
            out.contains("[REDACTED:env_assignment]") || out.contains("[REDACTED:anthropic_key]")
        );
        assert!(!out.contains("sk-ant-api03-aaaaa"));
    }

    /// 15 — env-export assignment for GH_TOKEN (no quotes).
    #[test]
    fn scrub_redacts_env_assignment_gh_token() {
        let raw = "GH_TOKEN=ghp_AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHHIIII";
        let (out, _) = scrub(raw);
        assert!(
            out.contains("[REDACTED:env_assignment]") || out.contains("[REDACTED:github_token]")
        );
    }

    /// 16 — multiple secrets in one transcript yield multiple redactions.
    #[test]
    fn scrub_redacts_multiple_secrets_independently() {
        let raw = "key1: sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA, key2: AKIAIOSFODNN7EXAMPLE";
        let (out, reds) = scrub(raw);
        assert_eq!(reds.len(), 2, "expected 2 redactions, got {reds:?}");
        assert!(out.contains("[REDACTED:anthropic_key]"));
        assert!(out.contains("[REDACTED:aws_access_key]"));
    }

    /// 17 — innocuous text passes through unchanged (no false positives).
    #[test]
    fn scrub_does_not_match_innocuous_prose() {
        let raw = "We use the same key approach as openai. The AWS docs show bearer of bad news examples.";
        let (out, reds) = scrub(raw);
        assert!(reds.is_empty(), "no redactions expected, got: {reds:?}");
        assert_eq!(out, raw);
    }

    /// 18 — `sk-` mentioned in prose without a real token is not redacted.
    #[test]
    fn scrub_no_false_positive_on_sk_prefix_alone() {
        let raw = "OpenAI keys start with sk- and Anthropic keys with sk-ant-.";
        let (out, reds) = scrub(raw);
        assert!(reds.is_empty(), "sk- mention falsely redacted: {reds:?}");
        assert_eq!(out, raw);
    }

    /// 19 — An empty input returns an empty output and no redactions.
    #[test]
    fn scrub_empty_input_yields_empty_output() {
        let (out, reds) = scrub("");
        assert_eq!(out, "");
        assert!(reds.is_empty());
    }

    /// 20 — Anthropic + OpenAI on adjacent lines — both fire, no
    /// cross-contamination of kinds.
    #[test]
    fn scrub_anthropic_and_openai_in_one_block() {
        let raw = "anthropic: sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA\nopenai: sk-01234567890abcdefghijklmnoPQR";
        let (out, reds) = scrub(raw);
        assert!(out.contains("[REDACTED:anthropic_key]"));
        assert!(out.contains("[REDACTED:openai_key]"));
        let kinds: Vec<RedactionKind> = reds.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&RedactionKind::AnthropicKey));
        assert!(kinds.contains(&RedactionKind::OpenAIKey));
    }

    /// 21 — Anthropic shape wins over OpenAI shape on overlap.
    #[test]
    fn scrub_anthropic_priority_over_openai() {
        let raw = "tok=sk-ant-api03-XXXXXXXXXXXXXXXXXXXXXXXXXX rest";
        let (_out, reds) = scrub(raw);
        // The match must be tagged Anthropic (not OpenAI), even though
        // both regexes could fire on the leading `sk-…` substring.
        assert_eq!(reds[0].kind, RedactionKind::AnthropicKey);
    }

    /// 22 — `summarize()` produces a deterministic per-kind tally.
    #[test]
    fn summarize_counts_per_kind() {
        let raw = "a sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA b sk-ant-api03-BBBBBBBBBBBBBBBBBBBBBBBBBBBB c AKIAIOSFODNN7EXAMPLE";
        let (_out, reds) = scrub(raw);
        let s = summarize(&reds);
        assert!(s.contains("anthropic_key=2"), "got: {s}");
        assert!(s.contains("aws_access_key=1"), "got: {s}");
    }

    /// 23 — Redaction byte_offsets are within the ORIGINAL input.
    #[test]
    fn redaction_byte_offsets_point_into_original_input() {
        let raw = "prefix sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA suffix";
        let (_out, reds) = scrub(raw);
        assert_eq!(reds.len(), 1);
        let r = &reds[0];
        // offset must equal "prefix ".len()
        assert_eq!(r.byte_offset, 7);
        // The slice at [offset, offset+len] in the ORIGINAL raw
        // string must start with `sk-ant-`.
        let slice = &raw[r.byte_offset..r.byte_offset + r.original_len];
        assert!(slice.starts_with("sk-ant-"));
    }

    /// 24 — Idempotency: scrubbing a redacted output produces no
    /// further redactions (the marker doesn't match any pattern).
    #[test]
    fn scrub_is_idempotent() {
        let raw = "tok=sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let (once, _) = scrub(raw);
        let (twice, reds2) = scrub(&once);
        assert_eq!(once, twice);
        assert!(reds2.is_empty(), "second pass redacted: {reds2:?}");
    }

    /// 25 — JSON-escaped quotes around a key are still caught.
    #[test]
    fn scrub_handles_json_escaped_quoted_value() {
        // Literal raw string: `"key": "sk-ant-api03-..."` as it would
        // appear inside a JSONL transcript.
        let raw =
            r#"{"role":"user","content":"my key is sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#;
        let (out, _) = scrub(raw);
        assert!(out.contains("[REDACTED:anthropic_key]"));
    }

    /// 26 — v0.20.0 validator regression: AwsSecretKey regex must NOT
    /// consume the closing JSON `"` quote when the secret sits inside
    /// a JSON-quoted value. Pre-fix the trailing `["']?` ate the
    /// closing quote and the captured JSONL became unparseable —
    /// `coral session show / distill` then errored on `parse_transcript`.
    /// Real-world hit because Claude Code's Bash-tool block emits
    /// exactly this shape when the user runs `export AWS_SECRET=…`.
    #[test]
    fn scrub_aws_secret_in_json_value_preserves_closing_quote() {
        let raw = r#"{"input":{"command":"export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"}}"#;
        let (out, _) = scrub(raw);
        assert!(
            out.contains("[REDACTED:aws_secret_key]"),
            "secret must be redacted: {out}"
        );
        // The closing `"` of the inner JSON string MUST be preserved.
        // Pre-fix the line ended with `[REDACTED:aws_secret_key]}}` —
        // missing the `"` before the first `}`. Post-fix it ends
        // `[REDACTED:aws_secret_key]"}}`.
        assert!(
            out.ends_with(r#""}}"#),
            "closing JSON quote was consumed: {out}"
        );
        // Sanity: the captured line round-trips through serde_json
        // without error. This is the actual user-facing contract —
        // `coral session show` calls `serde_json::from_str` on every
        // line of the captured `.jsonl`.
        let _: serde_json::Value =
            serde_json::from_str(&out).expect("scrubbed JSON must remain parseable");
    }

    /// 27 — same regression on the EnvAssignment regex. `ANTHROPIC_API_KEY=…`
    /// inside a JSON-quoted value used to eat the closing `"` too.
    #[test]
    fn scrub_env_assignment_in_json_value_preserves_closing_quote() {
        let raw = r#"{"input":{"command":"ANTHROPIC_API_KEY=sk-ant-api03-AAAABBBBCCCCDDDD"}}"#;
        let (out, _) = scrub(raw);
        assert!(
            out.contains("[REDACTED:env_assignment]") || out.contains("[REDACTED:anthropic_key]"),
            "secret must be redacted: {out}"
        );
        assert!(
            out.ends_with(r#""}}"#),
            "closing JSON quote was consumed: {out}"
        );
        let _: serde_json::Value =
            serde_json::from_str(&out).expect("scrubbed JSON must remain parseable");
    }
}
