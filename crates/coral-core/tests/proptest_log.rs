//! Property-based tests for `coral_core::log::WikiLog` round-trip + invariants.
//!
//! Same harness pattern as the lint / search / wikilinks proptest files
//! (ProptestConfig::with_cases(64)).
//!
//! Properties:
//! 1. `wikilog_round_trip` — build a `WikiLog` with N entries, render to
//!    a string, parse back. The recovered entries match (op + summary
//!    field-for-field, timestamp via `to_rfc3339()` which is the same
//!    canonical form the serializer uses).
//! 2. `wikilog_append_increases_count` — starting from a log with N
//!    entries, calling `append(op, summary)` results in a log with
//!    exactly N + 1 entries; the last entry is the one just appended.
//! 3. `wikilog_parse_handles_empty_body` — content that's just the
//!    canonical header (`---\n type: log\n---\n\n# Wiki operation log\n`)
//!    parses to a `WikiLog` with 0 entries.
//! 4. `wikilog_parse_skips_garbage_silently` — random ASCII garbage
//!    that doesn't match the entry regex is skipped silently and
//!    `parse` returns Ok with the entries that DID match (which may
//!    be zero). The impl is tolerant per its docstring, not strict.

use chrono::{DateTime, Duration, TimeZone, Utc};
use coral_core::log::{LogEntry, WikiLog};
use proptest::prelude::*;

// -----------------------------------------------------------------------------
// Generators
// -----------------------------------------------------------------------------

/// A timestamp drawn from a small range around 2026-01-01 so the rfc3339
/// re-serialization is byte-identical to the original (no DST-edge madness).
fn timestamp_strategy() -> impl Strategy<Value = DateTime<Utc>> {
    // 0..=1_000_000 seconds offset from a fixed anchor.
    (0i64..=1_000_000i64).prop_map(|secs| {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .single()
            .expect("anchor")
            + Duration::seconds(secs)
    })
}

/// An op token — the regex `\w[\w-]*` is what `WikiLog::parse` accepts,
/// so we restrict to that shape so we never feed our generator something
/// the parser wouldn't accept.
fn op_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9-]{0,15}".prop_map(|s| s)
}

/// A summary line — single-line, no `\n`, non-empty (the regex requires
/// at least one char in group 3). The parser does `line.trim_end()` before
/// the regex match, so we must exclude trailing whitespace; otherwise the
/// round-trip is lossy by design (the summary loses its trailing spaces).
/// We also force the first character to be non-whitespace so an all-blank
/// string is impossible.
fn summary_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9.,!?:_-][a-zA-Z0-9 .,!?:_-]{0,79}"
        .prop_map(|s| s.trim_end().to_string())
        // After trim_end, the first-char-non-blank guarantee still holds and
        // the string is non-empty.
        .prop_filter("non-empty after trim_end", |s| !s.is_empty())
}

fn entry_strategy() -> impl Strategy<Value = LogEntry> {
    (timestamp_strategy(), op_strategy(), summary_strategy()).prop_map(
        |(timestamp, op, summary)| LogEntry {
            timestamp,
            op,
            summary,
        },
    )
}

/// 0..=10 entries — enough to exercise the iteration but small enough
/// to keep the proptest runs fast.
fn entries_strategy() -> impl Strategy<Value = Vec<LogEntry>> {
    prop::collection::vec(entry_strategy(), 0..=10)
}

fn build_log(entries: Vec<LogEntry>) -> WikiLog {
    WikiLog { entries }
}

// -----------------------------------------------------------------------------
// Properties
// -----------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// `WikiLog` → string → `WikiLog` is identity on op + summary, and
    /// preserves timestamps modulo rfc3339 round-trip (which is the
    /// canonical form `to_string` uses anyway).
    #[test]
    fn wikilog_round_trip(entries in entries_strategy()) {
        let log = build_log(entries.clone());
        let serialized = log.to_string();
        let reparsed = WikiLog::parse(&serialized).expect("parse");
        prop_assert_eq!(reparsed.entries.len(), entries.len());
        for (orig, got) in entries.iter().zip(reparsed.entries.iter()) {
            prop_assert_eq!(&got.op, &orig.op);
            prop_assert_eq!(&got.summary, &orig.summary);
            // Compare via rfc3339 — the serializer formats as rfc3339 and
            // parses the same form back, so this is the canonical check.
            prop_assert_eq!(got.timestamp.to_rfc3339(), orig.timestamp.to_rfc3339());
        }
    }

    /// Appending one entry to a log with N entries gives a log with N+1
    /// entries; the new entry is at index N with the supplied op + summary.
    #[test]
    fn wikilog_append_increases_count(
        entries in entries_strategy(),
        op in op_strategy(),
        summary in summary_strategy(),
    ) {
        let mut log = build_log(entries.clone());
        let n = log.entries.len();
        let appended_op = op.clone();
        let appended_summary = summary.clone();
        log.append(op, summary);
        prop_assert_eq!(log.entries.len(), n + 1);
        let last = log.entries.last().expect("just appended");
        prop_assert_eq!(&last.op, &appended_op);
        prop_assert_eq!(&last.summary, &appended_summary);
    }

    /// Random ASCII garbage that doesn't match the entry regex is
    /// silently skipped. `parse` returns Ok and the entry count is
    /// `<=` the number of input lines (almost always 0 for pure
    /// garbage). This pins the "tolerant parser" contract.
    #[test]
    fn wikilog_parse_skips_garbage_silently(
        garbage in prop::collection::vec(
            "[a-zA-Z0-9 ,.;:!?'-]{0,80}",
            0..=20,
        )
    ) {
        let content = garbage.join("\n");
        let parsed = WikiLog::parse(&content).expect("tolerant parser returns Ok");
        // We can't make a stronger statement than "<= input line count"
        // because the random generator could in theory emit a string
        // that happens to look like a log line. With this charset
        // (no leading `- `) the count is essentially always 0 in
        // practice, but we don't need to over-pin.
        prop_assert!(parsed.entries.len() <= content.lines().count());
    }
}

// -----------------------------------------------------------------------------
// Non-proptest properties (single-shot)
// -----------------------------------------------------------------------------

/// An empty log with just the canonical header parses to 0 entries.
#[test]
fn wikilog_parse_handles_empty_body() {
    let content = "---\ntype: log\n---\n\n# Wiki operation log\n\n";
    let log = WikiLog::parse(content).expect("parse empty body");
    assert!(log.entries.is_empty());
}

/// Pure empty content also parses cleanly to 0 entries.
#[test]
fn wikilog_parse_handles_truly_empty_input() {
    let log = WikiLog::parse("").expect("parse empty");
    assert!(log.entries.is_empty());
}

/// Default `WikiLog::new()` round-trips cleanly through to_string + parse.
#[test]
fn wikilog_new_round_trip() {
    let log = WikiLog::new();
    let s = log.to_string();
    let parsed = WikiLog::parse(&s).expect("parse");
    assert!(parsed.entries.is_empty());
}
