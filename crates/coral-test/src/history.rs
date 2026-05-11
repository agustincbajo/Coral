//! Test history ledger and flake-rate computation.
//!
//! Each `coral test` run appends one JSONL line per test case to
//! `.coral/test-history.jsonl`. `coral test flakes` reads this ledger
//! and reports cases with flake_rate > threshold.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A single test execution record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRecord {
    pub case_id: String,
    pub status: String, // "pass", "fail", "skip"
    pub duration_ms: u64,
    pub timestamp: String, // ISO 8601
}

/// Flake report for a single test case.
#[derive(Debug, Clone)]
pub struct FlakeEntry {
    pub case_id: String,
    pub total_runs: usize,
    pub pass_count: usize,
    pub fail_count: usize,
    pub flake_rate: f64,
    pub quarantined: bool,
}

const HISTORY_FILE: &str = ".coral/test-history.jsonl";
const QUARANTINE_THRESHOLD: f64 = 0.2;

/// Append test records to the history ledger.
pub fn append_records(project_root: &Path, records: &[TestRecord]) -> std::io::Result<()> {
    let path = project_root.join(HISTORY_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    use std::io::Write;
    for record in records {
        let line = serde_json::to_string(record).unwrap_or_default();
        writeln!(file, "{}", line)?;
    }
    Ok(())
}

/// Read all records from history.
pub fn read_history(project_root: &Path) -> Vec<TestRecord> {
    let path = project_root.join(HISTORY_FILE);
    if !path.exists() {
        return Vec::new();
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Compute flake rates from history, optionally filtered to last N days.
pub fn compute_flakes(records: &[TestRecord], max_age_days: Option<u64>) -> Vec<FlakeEntry> {
    let cutoff = max_age_days.map(|days| chrono::Utc::now() - chrono::Duration::days(days as i64));

    let mut stats: HashMap<&str, (usize, usize)> = HashMap::new(); // (pass, fail)

    for r in records {
        if let Some(ref cut) = cutoff {
            if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&r.timestamp) {
                if ts < *cut {
                    continue;
                }
            }
        }
        let entry = stats.entry(r.case_id.as_str()).or_insert((0, 0));
        match r.status.as_str() {
            "pass" => entry.0 += 1,
            "fail" => entry.1 += 1,
            _ => {}
        }
    }

    let mut flakes: Vec<FlakeEntry> = stats
        .into_iter()
        .filter_map(|(case_id, (pass, fail))| {
            let total = pass + fail;
            if total < 2 {
                return None; // Need at least 2 runs to detect flakiness
            }
            let flake_rate = if total > 0 {
                let minority = pass.min(fail) as f64;
                minority / total as f64
            } else {
                0.0
            };
            if flake_rate > 0.0 {
                Some(FlakeEntry {
                    case_id: case_id.to_string(),
                    total_runs: total,
                    pass_count: pass,
                    fail_count: fail,
                    flake_rate,
                    quarantined: flake_rate >= QUARANTINE_THRESHOLD,
                })
            } else {
                None
            }
        })
        .collect();

    flakes.sort_by(|a, b| {
        b.flake_rate
            .partial_cmp(&a.flake_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    flakes
}

/// Render flake report as markdown.
pub fn render_markdown(flakes: &[FlakeEntry]) -> String {
    if flakes.is_empty() {
        return "No flaky tests detected.\n".to_string();
    }
    let mut out = String::from("# Flaky Tests Report\n\n");
    out.push_str("| Test | Runs | Pass | Fail | Flake Rate | Status |\n");
    out.push_str("|------|------|------|------|------------|--------|\n");
    for f in flakes {
        let status = if f.quarantined {
            "quarantine"
        } else {
            "flaky"
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {:.1}% | {} |\n",
            f.case_id,
            f.total_runs,
            f.pass_count,
            f.fail_count,
            f.flake_rate * 100.0,
            status
        ));
    }
    out
}

/// Render flake report as JSON.
pub fn render_json(flakes: &[FlakeEntry]) -> serde_json::Value {
    serde_json::json!({
        "flaky_count": flakes.len(),
        "quarantined_count": flakes.iter().filter(|f| f.quarantined).count(),
        "tests": flakes.iter().map(|f| serde_json::json!({
            "case_id": f.case_id,
            "total_runs": f.total_runs,
            "pass_count": f.pass_count,
            "fail_count": f.fail_count,
            "flake_rate": f.flake_rate,
            "quarantined": f.quarantined,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_records() -> Vec<TestRecord> {
        vec![
            TestRecord {
                case_id: "test_a".into(),
                status: "pass".into(),
                duration_ms: 100,
                timestamp: "2026-05-01T10:00:00Z".into(),
            },
            TestRecord {
                case_id: "test_a".into(),
                status: "fail".into(),
                duration_ms: 120,
                timestamp: "2026-05-02T10:00:00Z".into(),
            },
            TestRecord {
                case_id: "test_a".into(),
                status: "pass".into(),
                duration_ms: 110,
                timestamp: "2026-05-03T10:00:00Z".into(),
            },
            TestRecord {
                case_id: "test_b".into(),
                status: "pass".into(),
                duration_ms: 50,
                timestamp: "2026-05-01T10:00:00Z".into(),
            },
            TestRecord {
                case_id: "test_b".into(),
                status: "pass".into(),
                duration_ms: 55,
                timestamp: "2026-05-02T10:00:00Z".into(),
            },
            TestRecord {
                case_id: "test_b".into(),
                status: "pass".into(),
                duration_ms: 52,
                timestamp: "2026-05-03T10:00:00Z".into(),
            },
            // test_c: high flake rate (>= 0.2 quarantine threshold)
            TestRecord {
                case_id: "test_c".into(),
                status: "pass".into(),
                duration_ms: 200,
                timestamp: "2026-05-01T10:00:00Z".into(),
            },
            TestRecord {
                case_id: "test_c".into(),
                status: "fail".into(),
                duration_ms: 210,
                timestamp: "2026-05-02T10:00:00Z".into(),
            },
            TestRecord {
                case_id: "test_c".into(),
                status: "fail".into(),
                duration_ms: 215,
                timestamp: "2026-05-03T10:00:00Z".into(),
            },
            TestRecord {
                case_id: "test_c".into(),
                status: "pass".into(),
                duration_ms: 205,
                timestamp: "2026-05-04T10:00:00Z".into(),
            },
        ]
    }

    #[test]
    fn compute_flakes_detects_flaky_tests() {
        let records = sample_records();
        let flakes = compute_flakes(&records, None);

        // test_a: 2 pass, 1 fail => flake_rate = 1/3 ~= 0.333
        // test_b: 3 pass, 0 fail => not flaky (not reported)
        // test_c: 2 pass, 2 fail => flake_rate = 2/4 = 0.5
        assert_eq!(flakes.len(), 2, "expected 2 flaky tests, got {:?}", flakes);

        // Sorted by flake_rate descending: test_c first
        assert_eq!(flakes[0].case_id, "test_c");
        assert!((flakes[0].flake_rate - 0.5).abs() < 0.001);

        assert_eq!(flakes[1].case_id, "test_a");
        assert!((flakes[1].flake_rate - 1.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn all_passes_not_reported() {
        let records = vec![
            TestRecord {
                case_id: "stable".into(),
                status: "pass".into(),
                duration_ms: 10,
                timestamp: "2026-05-01T10:00:00Z".into(),
            },
            TestRecord {
                case_id: "stable".into(),
                status: "pass".into(),
                duration_ms: 12,
                timestamp: "2026-05-02T10:00:00Z".into(),
            },
            TestRecord {
                case_id: "stable".into(),
                status: "pass".into(),
                duration_ms: 11,
                timestamp: "2026-05-03T10:00:00Z".into(),
            },
        ];
        let flakes = compute_flakes(&records, None);
        assert!(
            flakes.is_empty(),
            "stable test should not appear in flakes: {:?}",
            flakes
        );
    }

    #[test]
    fn quarantine_threshold_applied() {
        let records = sample_records();
        let flakes = compute_flakes(&records, None);

        // test_c: flake_rate = 0.5 >= 0.2 threshold => quarantined
        let test_c = flakes.iter().find(|f| f.case_id == "test_c").unwrap();
        assert!(test_c.quarantined, "test_c should be quarantined");

        // test_a: flake_rate = 0.333 >= 0.2 threshold => also quarantined
        let test_a = flakes.iter().find(|f| f.case_id == "test_a").unwrap();
        assert!(test_a.quarantined, "test_a should be quarantined");
    }

    #[test]
    fn below_quarantine_threshold_not_quarantined() {
        // Create a test with exactly 1 fail in 10 runs => flake_rate = 0.1 < 0.2
        let mut records = Vec::new();
        for i in 0..9 {
            records.push(TestRecord {
                case_id: "borderline".into(),
                status: "pass".into(),
                duration_ms: 10,
                timestamp: format!("2026-05-{:02}T10:00:00Z", i + 1),
            });
        }
        records.push(TestRecord {
            case_id: "borderline".into(),
            status: "fail".into(),
            duration_ms: 10,
            timestamp: "2026-05-10T10:00:00Z".into(),
        });

        let flakes = compute_flakes(&records, None);
        assert_eq!(flakes.len(), 1);
        assert_eq!(flakes[0].case_id, "borderline");
        assert!(!flakes[0].quarantined, "borderline should NOT be quarantined");
        assert!((flakes[0].flake_rate - 0.1).abs() < 0.001);
    }

    #[test]
    fn append_and_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let records = vec![
            TestRecord {
                case_id: "rt_test".into(),
                status: "pass".into(),
                duration_ms: 42,
                timestamp: "2026-05-10T12:00:00Z".into(),
            },
            TestRecord {
                case_id: "rt_test".into(),
                status: "fail".into(),
                duration_ms: 43,
                timestamp: "2026-05-10T12:01:00Z".into(),
            },
        ];
        append_records(dir.path(), &records).unwrap();
        let read_back = read_history(dir.path());
        assert_eq!(read_back.len(), 2);
        assert_eq!(read_back[0].case_id, "rt_test");
        assert_eq!(read_back[0].status, "pass");
        assert_eq!(read_back[1].status, "fail");
    }
}
