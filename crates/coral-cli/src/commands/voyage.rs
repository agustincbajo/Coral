//! Voyage AI embeddings provider. Shells to `curl` (same pattern as
//! `notion-push`) — keeps the binary lean. v0.3.1 supports only `voyage-3`;
//! future versions may add `voyage-3-lite` or other providers.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const ENDPOINT: &str = "https://api.voyageai.com/v1/embeddings";
pub const DEFAULT_MODEL: &str = "voyage-3";
pub const DEFAULT_DIM: usize = 1024;
const MAX_BATCH: usize = 128; // Voyage limits

#[derive(Serialize)]
struct EmbedRequest<'a> {
    input: Vec<&'a str>,
    model: &'a str,
    input_type: Option<&'a str>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedItem>,
}

#[derive(Deserialize)]
struct EmbedItem {
    embedding: Vec<f32>,
    index: usize,
}

/// Embed a batch of texts. Splits internally into ≤128-item chunks.
/// `input_type` is one of: "query", "document", or None.
pub fn embed_batch(
    texts: &[String],
    model: &str,
    api_key: &str,
    input_type: Option<&str>,
) -> Result<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Ok(vec![]);
    }
    let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(MAX_BATCH) {
        let req = EmbedRequest {
            input: chunk.iter().map(String::as_str).collect(),
            model,
            input_type,
        };
        let body = serde_json::to_string(&req)?;
        let output = std::process::Command::new("curl")
            .args([
                "-s",
                "--fail-with-body",
                "-X",
                "POST",
                ENDPOINT,
                "-H",
                &format!("Authorization: Bearer {api_key}"),
                "-H",
                "Content-Type: application/json",
                "-d",
                &body,
            ])
            .output()
            .context("invoking curl (is it in PATH?)")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            anyhow::bail!(
                "voyage embeddings call failed (exit {:?}): {stderr}\n{stdout}",
                output.status.code()
            );
        }
        let parsed: EmbedResponse = serde_json::from_slice(&output.stdout).with_context(|| {
            format!(
                "parsing voyage response: {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })?;
        let mut by_index: Vec<Option<Vec<f32>>> = vec![None; chunk.len()];
        for item in parsed.data {
            if item.index < by_index.len() {
                by_index[item.index] = Some(item.embedding);
            }
        }
        for (i, slot) in by_index.into_iter().enumerate() {
            let v = slot.with_context(|| format!("missing embedding at index {i}"))?;
            out.push(v);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_empty_output() {
        // No curl invocation when input is empty; safe to run without API key.
        let result = embed_batch(&[], DEFAULT_MODEL, "fake-key", None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn build_request_serializes_correctly() {
        let req = EmbedRequest {
            input: vec!["hello", "world"],
            model: "voyage-3",
            input_type: Some("query"),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"input\":[\"hello\",\"world\"]"));
        assert!(json.contains("\"model\":\"voyage-3\""));
        assert!(json.contains("\"input_type\":\"query\""));
    }

    // Real API integration test (requires VOYAGE_API_KEY).
    #[test]
    #[ignore]
    fn voyage_real_smoke() {
        let key =
            std::env::var("VOYAGE_API_KEY").expect("VOYAGE_API_KEY required for this ignored test");
        let v = embed_batch(
            &["hello world".to_string()],
            DEFAULT_MODEL,
            &key,
            Some("query"),
        )
        .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].len(), DEFAULT_DIM);
    }
}
