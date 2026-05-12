//! Performance baseline tracking and regression detection.
//!
//! After each test run, latency data is stored in `.coral/perf-baseline.json`.
//! `coral test perf` compares current run against baseline and reports
//! regressions beyond a configurable threshold.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const BASELINE_FILE: &str = ".coral/perf-baseline.json";

/// Per-test latency stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyStats {
    pub samples: Vec<u64>, // durations in ms
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub mean: u64,
}

/// Full baseline: map of case_id -> LatencyStats.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PerfBaseline {
    pub cases: BTreeMap<String, LatencyStats>,
    pub updated_at: String,
}

/// A single regression finding.
#[derive(Debug, Clone)]
pub struct PerfRegression {
    pub case_id: String,
    pub baseline_p95: u64,
    pub current_p95: u64,
    pub delta_percent: f64,
}

/// Performance report.
#[derive(Debug, Clone)]
pub struct PerfReport {
    pub regressions: Vec<PerfRegression>,
    pub improvements: Vec<PerfRegression>,
    pub total_cases: usize,
    pub threshold_percent: f64,
}

impl LatencyStats {
    pub fn from_samples(mut samples: Vec<u64>) -> Self {
        samples.sort();
        let len = samples.len();
        if len == 0 {
            return Self {
                samples: vec![],
                p50: 0,
                p95: 0,
                p99: 0,
                mean: 0,
            };
        }
        let p50 = samples[len * 50 / 100];
        let p95 = samples[(len * 95 / 100).min(len - 1)];
        let p99 = samples[(len * 99 / 100).min(len - 1)];
        let mean = samples.iter().sum::<u64>() / len as u64;
        Self {
            samples,
            p50,
            p95,
            p99,
            mean,
        }
    }
}

/// Load baseline from disk.
pub fn load_baseline(project_root: &Path) -> PerfBaseline {
    let path = project_root.join(BASELINE_FILE);
    if !path.exists() {
        return PerfBaseline::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save baseline to disk.
pub fn save_baseline(project_root: &Path, baseline: &PerfBaseline) -> std::io::Result<()> {
    let path = project_root.join(BASELINE_FILE);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(baseline).unwrap();
    std::fs::write(path, json)
}

/// Update baseline with new samples.
pub fn update_baseline(baseline: &mut PerfBaseline, case_id: &str, duration_ms: u64) {
    let entry = baseline
        .cases
        .entry(case_id.to_string())
        .or_insert_with(|| LatencyStats {
            samples: Vec::new(),
            p50: 0,
            p95: 0,
            p99: 0,
            mean: 0,
        });
    entry.samples.push(duration_ms);
    // Keep only last 100 samples
    if entry.samples.len() > 100 {
        let drain = entry.samples.len() - 100;
        entry.samples.drain(..drain);
    }
    *entry = LatencyStats::from_samples(entry.samples.clone());
    baseline.updated_at = chrono::Utc::now().to_rfc3339();
}

/// Compare current run against baseline. Returns regressions beyond threshold.
pub fn detect_regressions(
    baseline: &PerfBaseline,
    current: &BTreeMap<String, u64>,
    threshold_percent: f64,
) -> PerfReport {
    let mut regressions = Vec::new();
    let mut improvements = Vec::new();

    for (case_id, &current_ms) in current {
        if let Some(stats) = baseline.cases.get(case_id) {
            if stats.p95 == 0 {
                continue;
            }
            let delta = (current_ms as f64 - stats.p95 as f64) / stats.p95 as f64 * 100.0;
            let reg = PerfRegression {
                case_id: case_id.clone(),
                baseline_p95: stats.p95,
                current_p95: current_ms,
                delta_percent: delta,
            };
            if delta > threshold_percent {
                regressions.push(reg);
            } else if delta < -threshold_percent {
                improvements.push(reg);
            }
        }
    }

    regressions.sort_by(|a, b| {
        b.delta_percent
            .partial_cmp(&a.delta_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    improvements.sort_by(|a, b| {
        a.delta_percent
            .partial_cmp(&b.delta_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    PerfReport {
        regressions,
        improvements,
        total_cases: current.len(),
        threshold_percent,
    }
}

/// Render perf report as markdown.
pub fn render_markdown(report: &PerfReport) -> String {
    let mut out = String::from("# Performance Report\n\n");
    out.push_str(&format!(
        "Threshold: +/-{:.0}% p95 regression\n\n",
        report.threshold_percent
    ));

    if report.regressions.is_empty() && report.improvements.is_empty() {
        out.push_str("No performance regressions detected.\n");
        return out;
    }

    if !report.regressions.is_empty() {
        out.push_str("## Regressions\n\n| Test | Baseline p95 | Current | Delta |\n|------|-------------|---------|-------|\n");
        for r in &report.regressions {
            out.push_str(&format!(
                "| {} | {}ms | {}ms | +{:.1}% |\n",
                r.case_id, r.baseline_p95, r.current_p95, r.delta_percent
            ));
        }
        out.push('\n');
    }

    if !report.improvements.is_empty() {
        out.push_str("## Improvements\n\n| Test | Baseline p95 | Current | Delta |\n|------|-------------|---------|-------|\n");
        for r in &report.improvements {
            out.push_str(&format!(
                "| {} | {}ms | {}ms | {:.1}% |\n",
                r.case_id, r.baseline_p95, r.current_p95, r.delta_percent
            ));
        }
    }

    out
}

/// Render as JSON.
pub fn render_json(report: &PerfReport) -> serde_json::Value {
    serde_json::json!({
        "threshold_percent": report.threshold_percent,
        "total_cases": report.total_cases,
        "regression_count": report.regressions.len(),
        "improvement_count": report.improvements.len(),
        "regressions": report.regressions.iter().map(|r| serde_json::json!({
            "case_id": r.case_id,
            "baseline_p95_ms": r.baseline_p95,
            "current_p95_ms": r.current_p95,
            "delta_percent": r.delta_percent,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_stats_from_empty_samples() {
        let stats = LatencyStats::from_samples(vec![]);
        assert_eq!(stats.p50, 0);
        assert_eq!(stats.p95, 0);
        assert_eq!(stats.p99, 0);
        assert_eq!(stats.mean, 0);
    }

    #[test]
    fn latency_stats_from_single_sample() {
        let stats = LatencyStats::from_samples(vec![42]);
        assert_eq!(stats.p50, 42);
        assert_eq!(stats.p95, 42);
        assert_eq!(stats.p99, 42);
        assert_eq!(stats.mean, 42);
    }

    #[test]
    fn latency_stats_percentiles_correct() {
        // 100 samples: 1..=100 (indices 0..99 after sort)
        let samples: Vec<u64> = (1..=100).collect();
        let stats = LatencyStats::from_samples(samples);
        // index 50 => value 51
        assert_eq!(stats.p50, 51);
        // index 95 => value 96
        assert_eq!(stats.p95, 96);
        // index min(99, 99) = 99 => value 100
        assert_eq!(stats.p99, 100);
        // avg of 1..=100 = 5050/100 = 50
        assert_eq!(stats.mean, 50);
    }

    #[test]
    fn detect_regressions_above_threshold() {
        let mut baseline = PerfBaseline::default();
        baseline.cases.insert(
            "test_a".to_string(),
            LatencyStats::from_samples(vec![100; 10]),
        );
        baseline.cases.insert(
            "test_b".to_string(),
            LatencyStats::from_samples(vec![200; 10]),
        );

        let mut current = BTreeMap::new();
        current.insert("test_a".to_string(), 150); // +50%
        current.insert("test_b".to_string(), 210); // +5%

        let report = detect_regressions(&baseline, &current, 20.0);
        assert_eq!(report.regressions.len(), 1);
        assert_eq!(report.regressions[0].case_id, "test_a");
        assert!(report.regressions[0].delta_percent > 49.0);
        assert!(report.improvements.is_empty());
    }

    #[test]
    fn detect_improvements_below_negative_threshold() {
        let mut baseline = PerfBaseline::default();
        baseline.cases.insert(
            "test_fast".to_string(),
            LatencyStats::from_samples(vec![100; 10]),
        );

        let mut current = BTreeMap::new();
        current.insert("test_fast".to_string(), 50); // -50%

        let report = detect_regressions(&baseline, &current, 20.0);
        assert!(report.regressions.is_empty());
        assert_eq!(report.improvements.len(), 1);
        assert_eq!(report.improvements[0].case_id, "test_fast");
    }

    #[test]
    fn update_baseline_caps_at_100_samples() {
        let mut baseline = PerfBaseline::default();
        for i in 0..120 {
            update_baseline(&mut baseline, "test_x", i);
        }
        let stats = baseline.cases.get("test_x").unwrap();
        assert_eq!(stats.samples.len(), 100);
        // The oldest samples (0..20) should have been drained
        assert_eq!(stats.samples[0], 20);
    }

    #[test]
    fn render_markdown_no_regressions() {
        let report = PerfReport {
            regressions: vec![],
            improvements: vec![],
            total_cases: 5,
            threshold_percent: 20.0,
        };
        let md = render_markdown(&report);
        assert!(md.contains("No performance regressions detected"));
    }

    #[test]
    fn render_json_structure() {
        let report = PerfReport {
            regressions: vec![PerfRegression {
                case_id: "slow_test".to_string(),
                baseline_p95: 100,
                current_p95: 150,
                delta_percent: 50.0,
            }],
            improvements: vec![],
            total_cases: 1,
            threshold_percent: 20.0,
        };
        let json = render_json(&report);
        assert_eq!(json["regression_count"], 1);
        assert_eq!(json["regressions"][0]["case_id"], "slow_test");
    }
}
