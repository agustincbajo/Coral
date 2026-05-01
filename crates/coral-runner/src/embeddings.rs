//! Embeddings-provider abstraction.
//!
//! Mirrors the [`Runner`](crate::Runner) trait pattern but for vector embedding
//! providers (Voyage, future OpenAI / Anthropic). The trait lets the search
//! command and tests swap providers without recompiling against a specific
//! HTTP shape. v0.4 ships [`VoyageProvider`] and [`MockEmbeddingsProvider`]; a
//! second real provider can land as one new file in this module.

use serde::{Deserialize, Serialize};
use std::process::Command;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmbeddingsError {
    #[error(
        "embeddings provider auth failed. Set the provider API key (e.g. VOYAGE_API_KEY) in this shell.\n\nProvider response:\n{0}"
    )]
    AuthFailed(String),
    #[error("embeddings provider call failed (exit {code:?}): {detail}")]
    ProviderCall { code: Option<i32>, detail: String },
    #[error("io error invoking embeddings provider: {0}")]
    Io(#[from] std::io::Error),
    #[error("parsing embeddings response: {0}")]
    Parse(String),
}

pub type EmbedResult<T> = std::result::Result<T, EmbeddingsError>;

/// An embeddings provider: turns batches of text into fixed-dimension vectors.
///
/// Implementations should chunk internally to respect provider batch limits
/// (Voyage caps at 128) and preserve input ordering on output.
pub trait EmbeddingsProvider: Send + Sync {
    /// Stable identifier for cache keying. Format is provider-specific but
    /// stable across versions of the same provider+model — changing it
    /// invalidates the on-disk vector cache.
    fn name(&self) -> &str;

    /// Vector dimensionality. Must match the cache's `dim` field.
    fn dim(&self) -> usize;

    /// Embed a batch of texts. `input_type` is provider-specific; common
    /// values are `"query"`, `"document"`, or `None`.
    fn embed_batch(&self, texts: &[String], input_type: Option<&str>)
    -> EmbedResult<Vec<Vec<f32>>>;
}

// --- Voyage AI ---------------------------------------------------------------

pub const DEFAULT_VOYAGE_MODEL: &str = "voyage-3";
pub const DEFAULT_VOYAGE_DIM: usize = 1024;
const VOYAGE_ENDPOINT: &str = "https://api.voyageai.com/v1/embeddings";
const VOYAGE_MAX_BATCH: usize = 128;

#[derive(Serialize)]
struct VoyageRequest<'a> {
    input: Vec<&'a str>,
    model: &'a str,
    input_type: Option<&'a str>,
}

#[derive(Deserialize)]
struct VoyageResponse {
    data: Vec<VoyageItem>,
}

#[derive(Deserialize)]
struct VoyageItem {
    embedding: Vec<f32>,
    index: usize,
}

/// Voyage AI embeddings provider. Shells to `curl` (same pattern as the
/// `notion-push` subcommand) to keep the binary lean and avoid pulling in
/// `reqwest` + `tokio` for a sync CLI.
pub struct VoyageProvider {
    api_key: String,
    model: String,
    dim: usize,
}

impl VoyageProvider {
    /// Build a Voyage provider with an explicit model + dimensionality.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>, dim: usize) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            dim,
        }
    }

    /// Convenience for the default `voyage-3` model (1024-dim).
    pub fn voyage_3(api_key: impl Into<String>) -> Self {
        Self::new(api_key, DEFAULT_VOYAGE_MODEL, DEFAULT_VOYAGE_DIM)
    }
}

impl EmbeddingsProvider for VoyageProvider {
    fn name(&self) -> &str {
        &self.model
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed_batch(
        &self,
        texts: &[String],
        input_type: Option<&str>,
    ) -> EmbedResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(VOYAGE_MAX_BATCH) {
            let req = VoyageRequest {
                input: chunk.iter().map(String::as_str).collect(),
                model: &self.model,
                input_type,
            };
            let body = serde_json::to_string(&req)
                .map_err(|e| EmbeddingsError::Parse(format!("serializing request: {e}")))?;
            let output = Command::new("curl")
                .args([
                    "-s",
                    "--fail-with-body",
                    "-X",
                    "POST",
                    VOYAGE_ENDPOINT,
                    "-H",
                    &format!("Authorization: Bearer {}", self.api_key),
                    "-H",
                    "Content-Type: application/json",
                    "-d",
                    &body,
                ])
                .output()?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let combined = match (stderr.is_empty(), stdout.is_empty()) {
                    (true, true) => String::new(),
                    (true, false) => stdout,
                    (false, true) => stderr,
                    (false, false) => format!("{stderr}\n{stdout}"),
                };
                if is_auth_failure(&combined) {
                    return Err(EmbeddingsError::AuthFailed(combined));
                }
                return Err(EmbeddingsError::ProviderCall {
                    code: output.status.code(),
                    detail: combined,
                });
            }
            let parsed: VoyageResponse = serde_json::from_slice(&output.stdout).map_err(|e| {
                EmbeddingsError::Parse(format!(
                    "voyage response: {e}; body={}",
                    String::from_utf8_lossy(&output.stdout)
                ))
            })?;
            let mut by_index: Vec<Option<Vec<f32>>> = vec![None; chunk.len()];
            for item in parsed.data {
                if item.index < by_index.len() {
                    by_index[item.index] = Some(item.embedding);
                }
            }
            for (i, slot) in by_index.into_iter().enumerate() {
                let v = slot.ok_or_else(|| {
                    EmbeddingsError::Parse(format!("voyage missing embedding at index {i}"))
                })?;
                out.push(v);
            }
        }
        Ok(out)
    }
}

fn is_auth_failure(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("401")
        || lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || lower.contains("authentication")
}

// --- MockEmbeddingsProvider --------------------------------------------------

/// Deterministic in-memory provider for tests. Returns one-hot-ish vectors
/// derived from a stable hash of each input string. Two equal inputs always
/// embed to the same vector; near-equal inputs produce vectors with a
/// sensible cosine similarity for assertion-style ranking tests.
pub struct MockEmbeddingsProvider {
    name: String,
    dim: usize,
}

impl MockEmbeddingsProvider {
    pub fn new(dim: usize) -> Self {
        Self {
            name: format!("mock-{dim}"),
            dim,
        }
    }

    fn embed_one(&self, text: &str) -> Vec<f32> {
        // Bag-of-bytes histogram normalized to unit length: deterministic,
        // stable, and gives sensible cosine similarity to similar inputs.
        let mut buckets = vec![0f32; self.dim];
        for b in text.as_bytes() {
            let idx = (*b as usize) % self.dim;
            buckets[idx] += 1.0;
        }
        let norm = buckets.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut buckets {
                *x /= norm;
            }
        }
        buckets
    }
}

impl EmbeddingsProvider for MockEmbeddingsProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed_batch(
        &self,
        texts: &[String],
        _input_type: Option<&str>,
    ) -> EmbedResult<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed_one(t)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voyage_provider_empty_input_returns_empty_without_curl() {
        // No curl invocation when input is empty; safe to run without API key.
        let p = VoyageProvider::voyage_3("fake-key");
        let result = p.embed_batch(&[], None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn voyage_provider_advertises_model_and_dim() {
        let p = VoyageProvider::voyage_3("fake-key");
        assert_eq!(p.name(), "voyage-3");
        assert_eq!(p.dim(), 1024);

        let p2 = VoyageProvider::new("k", "voyage-3-lite", 512);
        assert_eq!(p2.name(), "voyage-3-lite");
        assert_eq!(p2.dim(), 512);
    }

    #[test]
    fn voyage_request_serializes_with_input_type() {
        let req = VoyageRequest {
            input: vec!["hello", "world"],
            model: "voyage-3",
            input_type: Some("query"),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"input\":[\"hello\",\"world\"]"));
        assert!(json.contains("\"model\":\"voyage-3\""));
        assert!(json.contains("\"input_type\":\"query\""));
    }

    #[test]
    fn mock_provider_is_deterministic() {
        let p = MockEmbeddingsProvider::new(64);
        let texts = vec![
            "hello".to_string(),
            "hello".to_string(),
            "world".to_string(),
        ];
        let vs = p.embed_batch(&texts, None).unwrap();
        assert_eq!(vs.len(), 3);
        // Equal inputs → equal vectors.
        assert_eq!(vs[0], vs[1]);
        // Different inputs → different vectors.
        assert_ne!(vs[0], vs[2]);
        // Unit length.
        let norm = vs[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn mock_provider_swappable_through_trait_object() {
        // Demonstrates the trait swap that the search command relies on.
        let providers: Vec<Box<dyn EmbeddingsProvider>> = vec![
            Box::new(MockEmbeddingsProvider::new(32)),
            Box::new(VoyageProvider::voyage_3("fake-key")),
        ];
        assert_eq!(providers[0].name(), "mock-32");
        assert_eq!(providers[1].name(), "voyage-3");
        // We don't call embed_batch on the Voyage one — that would hit the network.
        let v = providers[0].embed_batch(&["hi".to_string()], None).unwrap();
        assert_eq!(v[0].len(), 32);
    }

    #[test]
    fn is_auth_failure_recognizes_voyage_signatures() {
        assert!(is_auth_failure("HTTP 401 Unauthorized"));
        assert!(is_auth_failure("Invalid API key"));
        assert!(is_auth_failure("authentication failed"));
        assert!(!is_auth_failure("rate limit exceeded"));
        assert!(!is_auth_failure("model overloaded"));
    }

    /// Real API integration test (requires VOYAGE_API_KEY).
    #[test]
    #[ignore]
    fn voyage_provider_real_smoke() {
        let key =
            std::env::var("VOYAGE_API_KEY").expect("VOYAGE_API_KEY required for this ignored test");
        let p = VoyageProvider::voyage_3(key);
        let v = p
            .embed_batch(&["hello world".to_string()], Some("query"))
            .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].len(), DEFAULT_VOYAGE_DIM);
    }
}
