//! `MonitorRun` — the persisted shape of a single monitor iteration.
//!
//! **Schema FROZEN for v0.23.1** — pinned by `monitor_run_jsonl_shape_pinned`.
//! Any future field MUST land with `#[serde(default, skip_serializing_if =
//! ...)]` so existing JSONL files stay readable. Removing or renaming
//! a field is a breaking change requiring a JSONL format version bump.
//!
//! Path convention: `<project_root>/.coral/monitors/<env>-<monitor_name>.jsonl`.
//!
//! Append protocol:
//!   1. Open with `OpenOptions::new().create(true).append(true)`.
//!   2. Write one line via `writeln!(file, "{}", serde_json::to_string(&run)?)`.
//!   3. Call `file.sync_all()` so the line is durable before the
//!      process can be killed mid-tick.
//!
//! We deliberately do NOT use `coral_core::atomic::atomic_write_string`
//! (the wiki layer's tempfile + rename pattern). Atomic-write replaces
//! the entire file; for an append-only log of unbounded length, we
//! want O(1) appends, not O(N) rewrites. The trade-off: a SIGKILL
//! between `writeln!` and `sync_all()` could lose the most recent
//! line. SIGINT/SIGTERM are handled cleanly via `signal-hook` (see
//! `up::run`) so the only at-risk case is a crash; the operator can
//! re-establish the run with a fresh `monitor up`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One persisted monitor iteration — the JSONL row.
///
/// Field order matches `serde_json::to_string`'s output (struct field
/// declaration order); the snapshot test pins the exact 7-field key
/// set + ordering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorRun {
    /// RFC3339 UTC timestamp of when the iteration started.
    pub timestamp: String,
    /// Environment name from `[[environments]].name`.
    pub env: String,
    /// Monitor name from `[[monitors]].name`.
    pub monitor_name: String,
    /// Total cases the iteration ran (post-filter).
    pub total: usize,
    /// Cases that returned `TestStatus::Pass`.
    pub passed: usize,
    /// Cases that returned `TestStatus::Fail` or `TestStatus::Error`.
    /// `Skip` outcomes count toward neither passed nor failed; the
    /// total field is the source of truth for "what got run".
    pub failed: usize,
    /// Wall-clock duration of the iteration in milliseconds.
    pub duration_ms: u64,
}

impl MonitorRun {
    /// Build from a started-at instant + a TestReport slice. Counts
    /// pass / fail / error per `TestStatus`. `Skip` is not tallied —
    /// `total` always equals `reports.len()`.
    pub fn from_reports(
        env: &str,
        monitor_name: &str,
        started_at: DateTime<Utc>,
        duration_ms: u64,
        reports: &[coral_test::TestReport],
    ) -> Self {
        let total = reports.len();
        let passed = reports
            .iter()
            .filter(|r| matches!(r.status, coral_test::TestStatus::Pass))
            .count();
        let failed = reports
            .iter()
            .filter(|r| {
                matches!(
                    r.status,
                    coral_test::TestStatus::Fail { .. } | coral_test::TestStatus::Error { .. }
                )
            })
            .count();
        Self {
            // RFC3339 with `Z` suffix — both human-readable and
            // tooling-friendly (jq, lnav, JSONL viewers).
            timestamp: started_at.to_rfc3339(),
            env: env.to_string(),
            monitor_name: monitor_name.to_string(),
            total,
            passed,
            failed,
            duration_ms,
        }
    }

    /// Serialize to a one-line JSON string suitable for direct
    /// `writeln!` to a JSONL file. Errors propagate as `serde_json::Error`.
    pub fn to_jsonl_line(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// `<project_root>/.coral/monitors/<env>-<monitor_name>.jsonl`.
///
/// Creating the parent directory is the caller's responsibility (the
/// append helper does it lazily so a missing `.coral/monitors/` is
/// not an error on first run).
pub fn jsonl_path(project_root: &Path, env: &str, monitor_name: &str) -> PathBuf {
    project_root
        .join(".coral")
        .join("monitors")
        .join(format!("{env}-{monitor_name}.jsonl"))
}

/// Append one `MonitorRun` to the JSONL file at the conventional path.
/// Creates parent dirs and the file as needed. `sync_all()` after the
/// write so a hard kill between this call and the next one cannot
/// drop the line.
pub fn append_run(project_root: &Path, run: &MonitorRun) -> std::io::Result<PathBuf> {
    let path = jsonl_path(project_root, &run.env, &run.monitor_name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
    let line = run.to_jsonl_line().map_err(std::io::Error::other)?;
    writeln!(f, "{line}")?;
    f.sync_all()?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use coral_test::{Evidence, TestReport, TestStatus};
    use std::time::Duration;
    use tempfile::TempDir;

    fn report(status: TestStatus) -> TestReport {
        // The TestReport::new constructor sets `started_at = Utc::now()`,
        // which we don't care about for these tests.
        let case = coral_test::TestCase {
            id: "x".into(),
            name: "x".into(),
            kind: coral_test::TestKind::Healthcheck,
            service: None,
            tags: vec![],
            source: coral_test::TestSource::Inline,
            spec: coral_test::TestSpec::empty(),
        };
        let mut r = TestReport::new(case, status, Duration::from_millis(0));
        r.evidence = Evidence::default();
        r
    }

    /// **T2 — schema pin.** Pin the 7-field JSON shape so any future
    /// drift (rename / add / remove) is immediately caught. We
    /// hand-write the expected JSON rather than `insta::assert_json`
    /// to keep the dependency footprint minimal — the assert here is
    /// the snapshot.
    #[test]
    fn monitor_run_jsonl_shape_pinned() {
        let ts = Utc.with_ymd_and_hms(2026, 5, 9, 12, 0, 0).unwrap();
        let run = MonitorRun {
            timestamp: ts.to_rfc3339(),
            env: "staging".into(),
            monitor_name: "smoke".into(),
            total: 5,
            passed: 4,
            failed: 1,
            duration_ms: 250,
        };
        let line = run.to_jsonl_line().unwrap();
        // Pin the EXACT field set + order. serde_json emits fields in
        // struct-declaration order. If anyone adds a field above
        // `duration_ms`, this assertion blows up loudly — that's the
        // point.
        assert_eq!(
            line,
            r#"{"timestamp":"2026-05-09T12:00:00+00:00","env":"staging","monitor_name":"smoke","total":5,"passed":4,"failed":1,"duration_ms":250}"#
        );
        // Round-trip check.
        let parsed: MonitorRun = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed, run);
    }

    #[test]
    fn from_reports_counts_pass_fail_error() {
        let reports = vec![
            report(TestStatus::Pass),
            report(TestStatus::Pass),
            report(TestStatus::Fail { reason: "x".into() }),
            report(TestStatus::Error { reason: "y".into() }),
            report(TestStatus::Skip { reason: "z".into() }),
        ];
        let run = MonitorRun::from_reports(
            "dev",
            "smoke",
            Utc.with_ymd_and_hms(2026, 5, 9, 12, 0, 0).unwrap(),
            42,
            &reports,
        );
        // total counts every report (including skipped); passed counts
        // Pass only; failed counts Fail + Error.
        assert_eq!(run.total, 5);
        assert_eq!(run.passed, 2);
        assert_eq!(run.failed, 2);
        assert_eq!(run.duration_ms, 42);
        assert_eq!(run.env, "dev");
        assert_eq!(run.monitor_name, "smoke");
    }

    /// `append_run` creates parent dirs and the file, then appends one
    /// JSONL line. A second append leaves two lines.
    #[test]
    fn append_run_creates_file_and_appends() {
        let tmp = TempDir::new().unwrap();
        let run = MonitorRun {
            timestamp: "2026-05-09T12:00:00+00:00".into(),
            env: "dev".into(),
            monitor_name: "smoke".into(),
            total: 1,
            passed: 1,
            failed: 0,
            duration_ms: 5,
        };
        let path = append_run(tmp.path(), &run).expect("first append");
        let path2 = append_run(tmp.path(), &run).expect("second append");
        assert_eq!(path, path2);
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            // Each line must round-trip back to the same struct.
            let parsed: MonitorRun = serde_json::from_str(line).unwrap();
            assert_eq!(parsed, run);
        }
        // Path layout matches the convention.
        assert!(path.ends_with(".coral/monitors/dev-smoke.jsonl"));
    }

    #[test]
    fn jsonl_path_combines_env_and_monitor_name() {
        let p = jsonl_path(Path::new("/tmp/proj"), "staging", "canary");
        assert_eq!(
            p,
            Path::new("/tmp/proj/.coral/monitors/staging-canary.jsonl")
        );
    }
}
