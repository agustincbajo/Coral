//! Retrieval evaluation metrics (RAGAs-inspired).
//!
//! Given a goldset of (query, expected_slugs) pairs and a search
//! function, compute precision@k, recall@k, and MRR (Mean Reciprocal Rank).

use serde::{Deserialize, Serialize};

/// A single goldset entry: a query and the slugs that should appear.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldsetEntry {
    pub query: String,
    pub expected_slugs: Vec<String>,
}

/// Aggregated evaluation metrics.
#[derive(Debug, Clone, Serialize)]
pub struct EvalReport {
    pub num_queries: usize,
    pub mean_precision_at_k: f64,
    pub mean_recall_at_k: f64,
    pub mrr: f64, // Mean Reciprocal Rank
    pub k: usize,
    pub per_query: Vec<QueryEvalSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryEvalSummary {
    pub query: String,
    pub precision: f64,
    pub recall: f64,
    pub reciprocal_rank: f64,
}

/// Compute precision@k: fraction of the k slots that contain relevant results.
pub fn precision_at_k(retrieved: &[String], expected: &[String], k: usize) -> f64 {
    if k == 0 {
        return 0.0;
    }
    let top_k: Vec<&String> = retrieved.iter().take(k).collect();
    let relevant = top_k.iter().filter(|r| expected.contains(r)).count();
    relevant as f64 / k as f64
}

/// Compute recall@k: fraction of relevant items found in top-k.
pub fn recall_at_k(retrieved: &[String], expected: &[String], k: usize) -> f64 {
    if expected.is_empty() {
        return 1.0;
    }
    let top_k: Vec<&String> = retrieved.iter().take(k).collect();
    let found = expected.iter().filter(|e| top_k.contains(e)).count();
    found as f64 / expected.len() as f64
}

/// Compute reciprocal rank: 1/position of first relevant result.
pub fn reciprocal_rank(retrieved: &[String], expected: &[String]) -> f64 {
    for (i, slug) in retrieved.iter().enumerate() {
        if expected.contains(slug) {
            return 1.0 / (i + 1) as f64;
        }
    }
    0.0
}

/// Run evaluation against a goldset.
///
/// `search_fn` takes a query string and returns ranked slug results.
pub fn evaluate<F>(goldset: &[GoldsetEntry], k: usize, search_fn: F) -> EvalReport
where
    F: Fn(&str) -> Vec<String>,
{
    let mut total_precision = 0.0;
    let mut total_recall = 0.0;
    let mut total_rr = 0.0;
    let mut per_query = Vec::new();

    for entry in goldset {
        let retrieved = search_fn(&entry.query);
        let p = precision_at_k(&retrieved, &entry.expected_slugs, k);
        let r = recall_at_k(&retrieved, &entry.expected_slugs, k);
        let rr = reciprocal_rank(&retrieved, &entry.expected_slugs);

        total_precision += p;
        total_recall += r;
        total_rr += rr;

        per_query.push(QueryEvalSummary {
            query: entry.query.clone(),
            precision: p,
            recall: r,
            reciprocal_rank: rr,
        });
    }

    let n = goldset.len().max(1) as f64;
    EvalReport {
        num_queries: goldset.len(),
        mean_precision_at_k: total_precision / n,
        mean_recall_at_k: total_recall / n,
        mrr: total_rr / n,
        k,
        per_query,
    }
}

/// Load goldset from a JSON file.
pub fn load_goldset(path: &std::path::Path) -> Result<Vec<GoldsetEntry>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("reading goldset: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parsing goldset JSON: {e}"))
}

/// Render eval report as markdown.
pub fn render_markdown(report: &EvalReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Search Evaluation Report (k={})\n\n", report.k));
    out.push_str("| Metric | Value |\n|--------|-------|\n");
    out.push_str(&format!(
        "| Precision@{} | {:.3} |\n",
        report.k, report.mean_precision_at_k
    ));
    out.push_str(&format!(
        "| Recall@{} | {:.3} |\n",
        report.k, report.mean_recall_at_k
    ));
    out.push_str(&format!("| MRR | {:.3} |\n", report.mrr));
    out.push_str(&format!("| Queries | {} |\n\n", report.num_queries));

    out.push_str("## Per-query breakdown\n\n");
    out.push_str("| Query | P@k | R@k | RR |\n|-------|-----|-----|----|\n");
    for q in &report.per_query {
        out.push_str(&format!(
            "| {} | {:.2} | {:.2} | {:.2} |\n",
            q.query.chars().take(40).collect::<String>(),
            q.precision,
            q.recall,
            q.reciprocal_rank
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precision_at_k_all_relevant() {
        let retrieved = vec!["a".into(), "b".into(), "c".into()];
        let expected = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(precision_at_k(&retrieved, &expected, 3), 1.0);
    }

    #[test]
    fn precision_at_k_none_relevant() {
        let retrieved = vec!["x".into(), "y".into()];
        let expected = vec!["a".into(), "b".into()];
        assert_eq!(precision_at_k(&retrieved, &expected, 2), 0.0);
    }

    #[test]
    fn recall_at_k_partial() {
        let retrieved = vec!["a".into(), "x".into(), "b".into()];
        let expected = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(recall_at_k(&retrieved, &expected, 3), 2.0 / 3.0);
    }

    #[test]
    fn reciprocal_rank_first() {
        let retrieved = vec!["a".into(), "b".into()];
        let expected = vec!["a".into()];
        assert_eq!(reciprocal_rank(&retrieved, &expected), 1.0);
    }

    #[test]
    fn reciprocal_rank_second() {
        let retrieved = vec!["x".into(), "a".into()];
        let expected = vec!["a".into()];
        assert_eq!(reciprocal_rank(&retrieved, &expected), 0.5);
    }

    #[test]
    fn reciprocal_rank_not_found() {
        let retrieved = vec!["x".into(), "y".into()];
        let expected = vec!["a".into()];
        assert_eq!(reciprocal_rank(&retrieved, &expected), 0.0);
    }

    #[test]
    fn evaluate_full_pipeline() {
        let goldset = vec![
            GoldsetEntry {
                query: "order".into(),
                expected_slugs: vec!["order".into()],
            },
            GoldsetEntry {
                query: "payment".into(),
                expected_slugs: vec!["payment".into()],
            },
        ];
        let report = evaluate(&goldset, 5, |q| vec![q.to_string()]);
        assert_eq!(report.num_queries, 2);
        assert_eq!(report.mrr, 1.0);
        assert_eq!(report.mean_precision_at_k, 1.0 / 5.0); // 1 relevant in top-5 -> 0.2 precision
    }
}
