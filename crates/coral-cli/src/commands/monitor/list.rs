//! `coral monitor list` — print declared monitors with best-effort
//! running/stopped status (v0.23.1).
//!
//! "Status" here is a heuristic, not authoritative: the JSONL file's
//! mtime (more precisely, the timestamp of the last line) is compared
//! against `now()`. Within `interval_seconds × 2` of the last tick →
//! `running`. Older or missing → `stopped`. The `× 2` window absorbs
//! one missed tick (slow iteration, GC pause, brief network blip)
//! before flipping the flag.
//!
//! AC #10: when no JSONL exists yet, status is `stopped`.

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Args;
use std::path::Path;
use std::process::ExitCode;

use crate::commands::common::resolve_project;
use crate::commands::env_resolve::{default_env_name, parse_all};
use crate::commands::monitor::run::jsonl_path;

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Filter to a single environment (default: every env in coral.toml).
    #[arg(long)]
    pub env: Option<String>,
}

pub fn run(args: ListArgs, wiki_root: Option<&Path>) -> Result<ExitCode> {
    let project = resolve_project(wiki_root)?;
    if project.environments_raw.is_empty() {
        println!("(no [[environments]] declared)");
        return Ok(ExitCode::SUCCESS);
    }
    let envs = parse_all(&project)?;
    let pick = args
        .env
        .clone()
        .unwrap_or_else(|| default_env_name(&project));

    println!("{:<12} {:<24} {:<10} last_tick", "env", "monitor", "status");
    let mut printed = 0usize;
    for spec in envs {
        if !args.env_matches(&pick, &spec.name) {
            continue;
        }
        for m in &spec.monitors {
            let path = jsonl_path(&project.root, &spec.name, &m.name);
            let (status, last) = status_for(&path, m.interval_seconds);
            println!("{:<12} {:<24} {:<10} {}", spec.name, m.name, status, last);
            printed += 1;
        }
    }
    if printed == 0 {
        println!("(no monitors declared in env '{}')", pick);
    }
    Ok(ExitCode::SUCCESS)
}

impl ListArgs {
    /// `--env name` matches a single env; without `--env` we list every
    /// env. The default is the first declared (per `default_env_name`).
    fn env_matches(&self, default: &str, candidate: &str) -> bool {
        match &self.env {
            Some(e) => e == candidate,
            None => candidate == default,
        }
    }
}

/// Best-effort status. Reads the last JSONL line, parses its
/// `timestamp`, compares against `now()`. Within `interval_seconds × 2`
/// → "running"; otherwise → "stopped". Missing file → "stopped".
pub(crate) fn status_for(path: &Path, interval_seconds: u64) -> (&'static str, String) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return ("stopped", "(no runs)".into());
    };
    let last_line = text.lines().rev().find(|l| !l.trim().is_empty());
    let Some(line) = last_line else {
        return ("stopped", "(no runs)".into());
    };
    let parsed: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return ("stopped", "(unreadable last line)".into()),
    };
    let ts_str = parsed
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let ts = match DateTime::parse_from_rfc3339(ts_str) {
        Ok(t) => t.with_timezone(&Utc),
        Err(_) => return ("stopped", format!("(bad timestamp: {ts_str})")),
    };
    let now = Utc::now();
    let elapsed = now.signed_duration_since(ts).num_seconds();
    let window = (interval_seconds.saturating_mul(2)) as i64;
    let status = if elapsed < window {
        "running"
    } else {
        "stopped"
    };
    (status, ts.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn status_for_reports_stopped_when_no_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("missing.jsonl");
        let (status, _) = status_for(&path, 60);
        assert_eq!(status, "stopped");
    }

    #[test]
    fn status_for_reports_running_when_recent_tick() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("recent.jsonl");
        let now = Utc::now();
        let line = serde_json::json!({
            "timestamp": now.to_rfc3339(),
            "env": "dev",
            "monitor_name": "smoke",
            "total": 1, "passed": 1, "failed": 0, "duration_ms": 5
        });
        fs::write(&path, line.to_string() + "\n").unwrap();
        let (status, _) = status_for(&path, 60);
        assert_eq!(status, "running");
    }

    #[test]
    fn status_for_reports_stopped_when_tick_outside_window() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("stale.jsonl");
        // 10 minutes ago is well outside `interval × 2` for a 60s
        // monitor (window = 120s).
        let stale = Utc::now() - chrono::Duration::seconds(600);
        let line = serde_json::json!({
            "timestamp": stale.to_rfc3339(),
            "env": "dev",
            "monitor_name": "smoke",
            "total": 1, "passed": 1, "failed": 0, "duration_ms": 5
        });
        fs::write(&path, line.to_string() + "\n").unwrap();
        let (status, _) = status_for(&path, 60);
        assert_eq!(status, "stopped");
    }

    #[test]
    fn status_for_handles_unparseable_last_line() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("garbage.jsonl");
        fs::write(&path, "not json\n").unwrap();
        let (status, _) = status_for(&path, 60);
        assert_eq!(status, "stopped");
    }
}
