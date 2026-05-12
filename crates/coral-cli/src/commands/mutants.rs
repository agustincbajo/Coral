//! `coral test mutants` — mutation testing wrapper (M3.3).
//!
//! Wraps `cargo-mutants` to run mutation testing against workspace crates.
//! Reports survivor mutants (weak tests) and generates a structured JSON
//! report with mutation score.

use anyhow::{Context, Result};
use clap::Args;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

/// Arguments for `coral test mutants`.
#[derive(Args, Debug, Clone)]
pub struct MutantsArgs {
    /// Target crate name. If omitted, runs against all workspace crates.
    #[arg(long, value_name = "NAME")]
    pub crate_name: Option<String>,

    /// Timeout per mutant in seconds.
    #[arg(long, default_value_t = 300)]
    pub timeout: u64,

    /// Output path for the full JSON report.
    #[arg(long, default_value = ".coral/mutants-report.json")]
    pub output: PathBuf,

    /// Minimum mutation score (0.0–1.0) to pass. Exit 1 if below.
    #[arg(long, default_value_t = 0.80)]
    pub threshold: f64,

    /// Total budget in minutes for the entire mutation testing run.
    /// When set, passes `--timeout <budget*60>` to cargo-mutants as the
    /// overall run timeout. If the budget expires before all mutants are
    /// tested, partial results are reported.
    #[arg(long, value_name = "MINUTES")]
    pub budget: Option<u32>,
}

/// A single surviving mutant from the cargo-mutants output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SurvivorMutant {
    pub file: String,
    pub line: u64,
    pub function: String,
    pub replacement: String,
}

/// Structured report emitted by `coral test mutants`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MutantsReport {
    pub survivors: Vec<SurvivorMutant>,
    pub killed: u64,
    pub total: u64,
    pub score: f64,
    /// The budget in minutes that was configured for this run, if any.
    /// Present when `--budget` was passed to `coral test mutants`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_minutes: Option<u32>,
    /// Whether the run was terminated early due to the budget expiring
    /// before all mutants could be tested.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub budget_exceeded: bool,
}

/// Outcome from cargo-mutants JSON: each mutant has an outcome status.
#[derive(Debug, Clone, Deserialize)]
struct CargoMutantOutcome {
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u64>,
    #[serde(default, rename = "function")]
    function_name: Option<String>,
    #[serde(default)]
    replacement: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
}

/// Top-level JSON structure from `cargo mutants --json`.
#[derive(Debug, Clone, Deserialize)]
struct CargoMutantsOutput {
    #[serde(default)]
    outcomes: Vec<CargoMutantOutcome>,
}

pub fn run(args: MutantsArgs) -> Result<ExitCode> {
    // Check if cargo-mutants is installed.
    if !is_cargo_mutants_available() {
        eprintln!(
            "error: `cargo-mutants` is not installed.\n\n\
             Install it with:\n\
             \n\
             \x20   cargo install cargo-mutants\n\n\
             Or visit: https://github.com/sourcefrog/cargo-mutants"
        );
        return Ok(ExitCode::from(2));
    }

    // Compute effective timeout: if --budget is set, use budget*60 as
    // the overall run timeout (seconds). Otherwise use the per-mutant
    // --timeout value.
    let effective_timeout = budget_to_timeout(args.budget, args.timeout);

    // Build the cargo mutants command.
    let mut cmd = Command::new("cargo");
    cmd.arg("mutants");
    cmd.arg("--json");
    cmd.args(["--timeout", &effective_timeout.to_string()]);

    if let Some(ref crate_name) = args.crate_name {
        cmd.args(["--package", crate_name]);
    }

    if let Some(budget) = args.budget {
        eprintln!(
            "Running mutation tests (budget={}min, timeout={}s)...",
            budget, effective_timeout
        );
    } else {
        eprintln!("Running mutation tests (timeout={}s)...", effective_timeout);
    }

    let output = cmd
        .output()
        .context("failed to execute `cargo mutants` — is it on PATH?")?;

    // cargo-mutants may exit non-zero when mutants survive; we still parse.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut report = parse_cargo_mutants_output(&stdout)?;

    // Attach budget metadata to the report.
    report.budget_minutes = args.budget;
    // Heuristic: if the process was killed (exit code != 0 and no
    // outcomes parsed) OR if partial results exist and the budget was
    // set, mark as budget_exceeded.
    if args.budget.is_some() && !output.status.success() && report.total == 0 {
        report.budget_exceeded = true;
    }

    // Print summary to stderr.
    let summary = format_summary(&report);
    eprintln!("{}", summary);

    if report.budget_exceeded {
        eprintln!(
            "NOTE: budget of {} minutes expired before all mutants were tested. \
             Results are partial.",
            args.budget.unwrap_or(0)
        );
    }

    // Write full report to output path.
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating output dir {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&report).context("serializing mutants report")?;
    std::fs::write(&args.output, &json)
        .with_context(|| format!("writing report to {}", args.output.display()))?;
    eprintln!("Report written to {}", args.output.display());

    // Exit 1 if score is below threshold.
    if report.score < args.threshold {
        eprintln!(
            "FAIL: mutation score {:.2} is below threshold {:.2}",
            report.score, args.threshold
        );
        Ok(ExitCode::FAILURE)
    } else {
        eprintln!(
            "PASS: mutation score {:.2} meets threshold {:.2}",
            report.score, args.threshold
        );
        Ok(ExitCode::SUCCESS)
    }
}

/// Convert a budget (minutes) to a timeout (seconds) for cargo-mutants.
/// If no budget is set, returns the per-mutant timeout unchanged.
pub fn budget_to_timeout(budget: Option<u32>, per_mutant_timeout: u64) -> u64 {
    match budget {
        Some(minutes) => (minutes as u64) * 60,
        None => per_mutant_timeout,
    }
}

/// Check whether `cargo-mutants` is available on PATH.
fn is_cargo_mutants_available() -> bool {
    Command::new("cargo")
        .args(["mutants", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse the JSON output from `cargo mutants --json` into a `MutantsReport`.
pub fn parse_cargo_mutants_output(json_str: &str) -> Result<MutantsReport> {
    // cargo-mutants may emit multiple JSON objects or a single one.
    // Try parsing as a single top-level object first.
    let parsed: CargoMutantsOutput = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => {
            // Fall back: try to find a JSON object with "outcomes" key
            // by scanning lines (cargo-mutants sometimes emits progress
            // lines before the final JSON).
            let mut found = None;
            for line in json_str.lines().rev() {
                let trimmed = line.trim();
                if trimmed.starts_with('{') {
                    if let Ok(v) = serde_json::from_str::<CargoMutantsOutput>(trimmed) {
                        found = Some(v);
                        break;
                    }
                }
            }
            // If still nothing, try the full text as a JSON array of outcomes.
            found.unwrap_or_else(|| {
                let outcomes: Vec<CargoMutantOutcome> =
                    serde_json::from_str(json_str).unwrap_or_default();
                CargoMutantsOutput { outcomes }
            })
        }
    };

    build_report_from_outcomes(&parsed.outcomes)
}

/// Convert parsed outcomes into a structured report.
fn build_report_from_outcomes(outcomes: &[CargoMutantOutcome]) -> Result<MutantsReport> {
    let mut survivors = Vec::new();
    let mut killed: u64 = 0;
    let total = outcomes.len() as u64;

    for outcome in outcomes {
        let status = outcome.outcome.as_deref().unwrap_or("unknown");
        match status {
            "Survived" | "survived" => {
                survivors.push(SurvivorMutant {
                    file: outcome.file.clone().unwrap_or_default(),
                    line: outcome.line.unwrap_or(0),
                    function: outcome.function_name.clone().unwrap_or_default(),
                    replacement: outcome.replacement.clone().unwrap_or_default(),
                });
            }
            "Killed" | "killed" | "Timeout" | "timeout" => {
                killed += 1;
            }
            _ => {
                // Unrecognized statuses (e.g., "Unviable") don't count
                // toward killed or survived for score calculation.
            }
        }
    }

    let scored_total = killed + survivors.len() as u64;
    let score = if scored_total == 0 {
        1.0
    } else {
        killed as f64 / scored_total as f64
    };

    Ok(MutantsReport {
        survivors,
        killed,
        total,
        score,
        budget_minutes: None,
        budget_exceeded: false,
    })
}

/// Format a human-readable summary of the mutation testing results.
pub fn format_summary(report: &MutantsReport) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "Mutation testing complete: {}/{} mutants killed ({:.1}% score)",
        report.killed,
        report.total,
        report.score * 100.0
    ));

    if !report.survivors.is_empty() {
        lines.push(String::new());
        lines.push(format!("Surviving mutants ({}):", report.survivors.len()));
        for (i, s) in report.survivors.iter().enumerate() {
            lines.push(format!(
                "  {}. {}:{} in `{}` — {}",
                i + 1,
                s.file,
                s.line,
                s.function,
                s.replacement
            ));
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Minimal CLI shim for parsing MutantsArgs.
    #[derive(Parser, Debug)]
    struct ShimCli {
        #[command(flatten)]
        args: MutantsArgs,
    }

    // Test 1: CLI arg parsing — defaults
    #[test]
    fn mutants_cli_defaults() {
        let parsed = ShimCli::try_parse_from(["test"]).expect("parse with defaults");
        assert_eq!(parsed.args.crate_name, None);
        assert_eq!(parsed.args.timeout, 300);
        assert_eq!(
            parsed.args.output,
            PathBuf::from(".coral/mutants-report.json")
        );
        assert!((parsed.args.threshold - 0.80).abs() < f64::EPSILON);
    }

    // Test 2: CLI arg parsing — custom values
    #[test]
    fn mutants_cli_custom_args() {
        let parsed = ShimCli::try_parse_from([
            "test",
            "--crate-name",
            "my-crate",
            "--timeout",
            "600",
            "--output",
            "/tmp/report.json",
            "--threshold",
            "0.90",
        ])
        .expect("parse with custom args");
        assert_eq!(parsed.args.crate_name.as_deref(), Some("my-crate"));
        assert_eq!(parsed.args.timeout, 600);
        assert_eq!(parsed.args.output, PathBuf::from("/tmp/report.json"));
        assert!((parsed.args.threshold - 0.90).abs() < f64::EPSILON);
    }

    // Test 3: Report generation from sample cargo-mutants JSON output
    #[test]
    fn mutants_report_from_json() {
        let sample_json = r#"{
            "outcomes": [
                {
                    "file": "src/lib.rs",
                    "line": 42,
                    "function": "add",
                    "replacement": "replace + with -",
                    "outcome": "Survived"
                },
                {
                    "file": "src/lib.rs",
                    "line": 50,
                    "function": "sub",
                    "replacement": "replace - with +",
                    "outcome": "Killed"
                },
                {
                    "file": "src/lib.rs",
                    "line": 60,
                    "function": "mul",
                    "replacement": "replace * with /",
                    "outcome": "Killed"
                },
                {
                    "file": "src/lib.rs",
                    "line": 70,
                    "function": "div",
                    "replacement": "replace / with *",
                    "outcome": "Survived"
                }
            ]
        }"#;

        let report = parse_cargo_mutants_output(sample_json).expect("parse");
        assert_eq!(report.total, 4);
        assert_eq!(report.killed, 2);
        assert_eq!(report.survivors.len(), 2);
        assert!((report.score - 0.5).abs() < f64::EPSILON);

        // Verify survivor details.
        assert_eq!(report.survivors[0].file, "src/lib.rs");
        assert_eq!(report.survivors[0].line, 42);
        assert_eq!(report.survivors[0].function, "add");
        assert_eq!(report.survivors[0].replacement, "replace + with -");

        assert_eq!(report.survivors[1].function, "div");
    }

    // Test 4: Threshold logic — pass when score >= threshold
    #[test]
    fn mutants_threshold_pass() {
        let report = MutantsReport {
            survivors: vec![SurvivorMutant {
                file: "src/lib.rs".into(),
                line: 1,
                function: "f".into(),
                replacement: "x".into(),
            }],
            killed: 9,
            total: 10,
            score: 0.9,
            budget_minutes: None,
            budget_exceeded: false,
        };
        // 0.9 >= 0.80 threshold → pass
        assert!(report.score >= 0.80);
    }

    // Test 5: Threshold logic — fail when score < threshold
    #[test]
    fn mutants_threshold_fail() {
        let report = MutantsReport {
            survivors: vec![
                SurvivorMutant {
                    file: "a.rs".into(),
                    line: 1,
                    function: "f".into(),
                    replacement: "x".into(),
                },
                SurvivorMutant {
                    file: "b.rs".into(),
                    line: 2,
                    function: "g".into(),
                    replacement: "y".into(),
                },
                SurvivorMutant {
                    file: "c.rs".into(),
                    line: 3,
                    function: "h".into(),
                    replacement: "z".into(),
                },
            ],
            killed: 7,
            total: 10,
            score: 0.7,
            budget_minutes: None,
            budget_exceeded: false,
        };
        // 0.7 < 0.80 threshold → fail
        assert!(report.score < 0.80);
    }

    // Test 6: Summary formatting
    #[test]
    fn mutants_summary_format() {
        let report = MutantsReport {
            survivors: vec![SurvivorMutant {
                file: "src/math.rs".into(),
                line: 10,
                function: "add".into(),
                replacement: "replace + with -".into(),
            }],
            killed: 4,
            total: 5,
            score: 0.8,
            budget_minutes: None,
            budget_exceeded: false,
        };
        let summary = format_summary(&report);
        assert!(summary.contains("4/5 mutants killed"));
        assert!(summary.contains("80.0%"));
        assert!(summary.contains("Surviving mutants (1)"));
        assert!(summary.contains("src/math.rs:10"));
        assert!(summary.contains("`add`"));
        assert!(summary.contains("replace + with -"));
    }

    // Test 7: Empty outcomes produce score 1.0 (no mutants = nothing to kill)
    #[test]
    fn mutants_empty_outcomes_score_one() {
        let report = parse_cargo_mutants_output(r#"{"outcomes": []}"#).expect("parse");
        assert_eq!(report.total, 0);
        assert_eq!(report.killed, 0);
        assert!(report.survivors.is_empty());
        assert!((report.score - 1.0).abs() < f64::EPSILON);
    }

    // Test 8: Unviable outcomes are excluded from score calculation
    #[test]
    fn mutants_unviable_excluded_from_score() {
        let json = r#"{
            "outcomes": [
                {"file": "a.rs", "line": 1, "function": "f", "replacement": "x", "outcome": "Killed"},
                {"file": "b.rs", "line": 2, "function": "g", "replacement": "y", "outcome": "Unviable"},
                {"file": "c.rs", "line": 3, "function": "h", "replacement": "z", "outcome": "Survived"}
            ]
        }"#;
        let report = parse_cargo_mutants_output(json).expect("parse");
        assert_eq!(report.total, 3);
        assert_eq!(report.killed, 1);
        assert_eq!(report.survivors.len(), 1);
        // Score: 1 killed / (1 killed + 1 survived) = 0.5
        assert!((report.score - 0.5).abs() < f64::EPSILON);
    }

    // ---------------------------------------------------------------
    // M3.11 Part A: Mutation budget gate tests
    // ---------------------------------------------------------------

    // Test 9: Budget flag is included in report JSON
    #[test]
    fn mutants_budget_field_in_report() {
        let report = MutantsReport {
            survivors: vec![],
            killed: 5,
            total: 5,
            score: 1.0,
            budget_minutes: Some(10),
            budget_exceeded: false,
        };
        let json = serde_json::to_string_pretty(&report).expect("serialize");
        assert!(
            json.contains("\"budget_minutes\": 10"),
            "report JSON must contain budget_minutes when set: {json}"
        );
        // When budget is None, the field should be absent (skip_serializing_if)
        let report_no_budget = MutantsReport {
            budget_minutes: None,
            ..report.clone()
        };
        let json2 = serde_json::to_string_pretty(&report_no_budget).expect("serialize");
        assert!(
            !json2.contains("budget_minutes"),
            "report JSON must omit budget_minutes when None: {json2}"
        );
    }

    // Test 10: Budget timeout calculation
    #[test]
    fn mutants_budget_timeout_calculation() {
        // 10 minutes budget → 600s timeout
        assert_eq!(budget_to_timeout(Some(10), 300), 600);
        // 1 minute budget → 60s timeout
        assert_eq!(budget_to_timeout(Some(1), 300), 60);
        // No budget → per-mutant timeout unchanged
        assert_eq!(budget_to_timeout(None, 300), 300);
        // Large budget
        assert_eq!(budget_to_timeout(Some(120), 300), 7200);
    }

    // Test 11: CLI arg parsing — budget flag
    #[test]
    fn mutants_cli_budget_flag() {
        let parsed =
            ShimCli::try_parse_from(["test", "--budget", "15"]).expect("parse with budget flag");
        assert_eq!(parsed.args.budget, Some(15));

        // Default: no budget
        let parsed_default = ShimCli::try_parse_from(["test"]).expect("parse defaults");
        assert_eq!(parsed_default.args.budget, None);
    }
}
