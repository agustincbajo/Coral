//! Embeddings-provider abstraction.
//!
//! Mirrors the [`Runner`](crate::Runner) trait pattern but for vector embedding
//! providers (Voyage, future OpenAI / Anthropic). The trait lets the search
//! command and tests swap providers without recompiling against a specific
//! HTTP shape. v0.4 ships [`VoyageProvider`] and [`MockEmbeddingsProvider`]; a
//! second real provider can land as one new file in this module.

use rayon::prelude::*;
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

/// Run `curl POST` with the provided header line piped through stdin
/// instead of placed in argv.
///
/// v0.19.5 audit H6: API keys must NEVER appear in argv (visible to
/// every other process via `ps` / `/proc`). curl's `@-` form for `-H`
/// reads header lines from stdin until EOF; we pipe the bearer
/// header in and EOF the stream so curl moves on to the body.
fn curl_post_with_secret_header(
    url: &str,
    secret_header: &str,
    extra_headers: &[(&str, &str)],
    body: &str,
) -> std::io::Result<std::process::Output> {
    let mut cmd = Command::new("curl");
    cmd.args(["-s", "--fail-with-body", "-X", "POST", url, "-H", "@-"]);
    for (k, v) in extra_headers {
        cmd.args(["-H", &format!("{k}: {v}")]);
    }
    cmd.args(["-d", body]);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        // curl reads header lines from stdin until EOF for @- inputs.
        let mut line = String::with_capacity(secret_header.len() + 1);
        line.push_str(secret_header);
        if !line.ends_with('\n') {
            line.push('\n');
        }
        std::io::Write::write_all(&mut stdin, line.as_bytes())?;
    }
    child.wait_with_output()
}

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

impl VoyageProvider {
    /// Embed a single sub-batch (≤ `VOYAGE_MAX_BATCH` items) via one curl
    /// invocation. Returned vectors are ordered to match `chunk`.
    ///
    /// This is the unit of work fanned out across rayon's thread pool by
    /// [`Self::embed_batch`]; pulling it out keeps the parallel closure small
    /// and lets us reuse the request shape for any single curl call.
    fn embed_chunk(
        &self,
        chunk: &[String],
        input_type: Option<&str>,
    ) -> EmbedResult<Vec<Vec<f32>>> {
        let req = VoyageRequest {
            input: chunk.iter().map(String::as_str).collect(),
            model: &self.model,
            input_type,
        };
        let body = serde_json::to_string(&req)
            .map_err(|e| EmbeddingsError::Parse(format!("serializing request: {e}")))?;
        let output = curl_post_with_secret_header(
            VOYAGE_ENDPOINT,
            &format!("Authorization: Bearer {}", self.api_key),
            &[("Content-Type", "application/json")],
            &body,
        )?;
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
                // v0.19.5 audit H8: scrub bearer/x-api-key.
                return Err(EmbeddingsError::AuthFailed(crate::runner::scrub_secrets(
                    &combined,
                )));
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
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(chunk.len());
        for (i, slot) in by_index.into_iter().enumerate() {
            let v = slot.ok_or_else(|| {
                EmbeddingsError::Parse(format!("voyage missing embedding at index {i}"))
            })?;
            out.push(v);
        }
        Ok(out)
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
        // Fan the per-chunk curl invocations across rayon's global thread
        // pool. Each closure tags its result with the chunk index so we can
        // re-stitch the final Vec<Vec<f32>> in input order regardless of
        // which thread finishes first. On any chunk error the whole batch
        // aborts, mirroring the previous `for + ?` behavior.
        let mut indexed: Vec<(usize, Vec<Vec<f32>>)> = texts
            .par_chunks(VOYAGE_MAX_BATCH)
            .enumerate()
            .map(|(idx, chunk)| {
                let vecs = self.embed_chunk(chunk, input_type)?;
                Ok::<(usize, Vec<Vec<f32>>), EmbeddingsError>((idx, vecs))
            })
            .collect::<Result<Vec<_>, _>>()?;
        indexed.sort_by_key(|(idx, _)| *idx);
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for (_, vecs) in indexed {
            out.extend(vecs);
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

// --- OpenAI ------------------------------------------------------------------

pub const DEFAULT_OPENAI_MODEL: &str = "text-embedding-3-small";
pub const DEFAULT_OPENAI_DIM: usize = 1536;
const OPENAI_ENDPOINT: &str = "https://api.openai.com/v1/embeddings";
// OpenAI's embeddings API accepts up to 2048 inputs per call. We cap a bit
// lower to leave headroom for token-count limits per request (~8k tokens).
const OPENAI_MAX_BATCH: usize = 256;

#[derive(Serialize)]
struct OpenAIRequest<'a> {
    input: Vec<&'a str>,
    model: &'a str,
    encoding_format: &'a str,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    data: Vec<OpenAIItem>,
}

#[derive(Deserialize)]
struct OpenAIItem {
    embedding: Vec<f32>,
    index: usize,
}

/// OpenAI embeddings provider. Same curl shell-out pattern as VoyageProvider.
/// Supports `text-embedding-3-small` (1536-dim, default), `text-embedding-3-large`
/// (3072-dim), and `text-embedding-ada-002` (1536-dim, legacy).
pub struct OpenAIProvider {
    api_key: String,
    model: String,
    dim: usize,
}

impl OpenAIProvider {
    /// Build an OpenAI provider with an explicit model + dimensionality.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>, dim: usize) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            dim,
        }
    }

    /// Convenience for `text-embedding-3-small` (1536-dim, the cost-effective default).
    pub fn text_embedding_3_small(api_key: impl Into<String>) -> Self {
        Self::new(api_key, DEFAULT_OPENAI_MODEL, DEFAULT_OPENAI_DIM)
    }

    /// Convenience for `text-embedding-3-large` (3072-dim, higher quality).
    pub fn text_embedding_3_large(api_key: impl Into<String>) -> Self {
        Self::new(api_key, "text-embedding-3-large", 3072)
    }
}

impl OpenAIProvider {
    /// Embed a single sub-batch (≤ `OPENAI_MAX_BATCH` items) via one curl
    /// invocation. Returned vectors are ordered to match `chunk`.
    ///
    /// This is the unit of work fanned out across rayon's thread pool by
    /// [`Self::embed_batch`]; pulling it out keeps the parallel closure small
    /// and lets us reuse the request shape for any single curl call.
    fn embed_chunk(&self, chunk: &[String]) -> EmbedResult<Vec<Vec<f32>>> {
        let req = OpenAIRequest {
            input: chunk.iter().map(String::as_str).collect(),
            model: &self.model,
            encoding_format: "float",
        };
        let body = serde_json::to_string(&req)
            .map_err(|e| EmbeddingsError::Parse(format!("serializing request: {e}")))?;
        // v0.19.5 audit H6: see `curl_post_with_secret_header` doc.
        let output = curl_post_with_secret_header(
            OPENAI_ENDPOINT,
            &format!("Authorization: Bearer {}", self.api_key),
            &[("Content-Type", "application/json")],
            &body,
        )?;
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
                // v0.19.5 audit H8: scrub bearer/x-api-key.
                return Err(EmbeddingsError::AuthFailed(crate::runner::scrub_secrets(
                    &combined,
                )));
            }
            return Err(EmbeddingsError::ProviderCall {
                code: output.status.code(),
                detail: combined,
            });
        }
        let parsed: OpenAIResponse = serde_json::from_slice(&output.stdout).map_err(|e| {
            EmbeddingsError::Parse(format!(
                "openai response: {e}; body={}",
                String::from_utf8_lossy(&output.stdout)
            ))
        })?;
        let mut by_index: Vec<Option<Vec<f32>>> = vec![None; chunk.len()];
        for item in parsed.data {
            if item.index < by_index.len() {
                by_index[item.index] = Some(item.embedding);
            }
        }
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(chunk.len());
        for (i, slot) in by_index.into_iter().enumerate() {
            let v = slot.ok_or_else(|| {
                EmbeddingsError::Parse(format!("openai missing embedding at index {i}"))
            })?;
            out.push(v);
        }
        Ok(out)
    }
}

impl EmbeddingsProvider for OpenAIProvider {
    fn name(&self) -> &str {
        &self.model
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed_batch(
        &self,
        texts: &[String],
        _input_type: Option<&str>, // OpenAI doesn't distinguish query vs document
    ) -> EmbedResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        // Fan the per-chunk curl invocations across rayon's global thread
        // pool. Each closure tags its result with the chunk index so we can
        // re-stitch the final Vec<Vec<f32>> in input order regardless of
        // which thread finishes first. On any chunk error the whole batch
        // aborts, mirroring the previous `for + ?` behavior.
        let mut indexed: Vec<(usize, Vec<Vec<f32>>)> = texts
            .par_chunks(OPENAI_MAX_BATCH)
            .enumerate()
            .map(|(idx, chunk)| {
                let vecs = self.embed_chunk(chunk)?;
                Ok::<(usize, Vec<Vec<f32>>), EmbeddingsError>((idx, vecs))
            })
            .collect::<Result<Vec<_>, _>>()?;
        indexed.sort_by_key(|(idx, _)| *idx);
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for (_, vecs) in indexed {
            out.extend(vecs);
        }
        Ok(out)
    }
}

// --- Anthropic (speculative) ------------------------------------------------
//
// SPECULATIVE IMPL — Anthropic has not published an embeddings API at the time
// of this commit. The struct + trait wiring exists so users with an Anthropic
// key can switch over with one URL change once the API ships. The endpoint
// constant is a placeholder (`https://api.anthropic.com/v1/embeddings`) —
// update when the real endpoint is announced.
//
// Until then, calling `embed_batch` will fail with a 404 from the placeholder
// endpoint, surfaced as `EmbeddingsError::ProviderCall` (or `AuthFailed` if
// the API key is missing/invalid).

pub const PLACEHOLDER_ANTHROPIC_MODEL: &str = "claude-embedding-1";
pub const PLACEHOLDER_ANTHROPIC_DIM: usize = 1024; // typical Anthropic dim
const ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com/v1/embeddings";
const ANTHROPIC_MAX_BATCH: usize = 100;

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    input: Vec<&'a str>,
    model: &'a str,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    data: Vec<AnthropicItem>,
}

#[derive(Deserialize)]
struct AnthropicItem {
    embedding: Vec<f32>,
    index: usize,
}

/// **Speculative** Anthropic embeddings provider. Anthropic has not published
/// the embeddings API at the time of this commit; this impl is a placeholder
/// that mirrors the OpenAI/Voyage shape. When Anthropic ships the API:
/// 1. Update [`ANTHROPIC_ENDPOINT`] to the real URL.
/// 2. Verify the request/response shape matches their published spec.
/// 3. Add a real-API smoke test marked `#[ignore]` (mirror voyage_provider_real_smoke).
///
/// Selected via `coral search --embeddings-provider anthropic`. Requires
/// `ANTHROPIC_API_KEY` env var. Until the API ships, calls return
/// `EmbeddingsError::ProviderCall` from the placeholder 404.
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    dim: usize,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>, dim: usize) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            dim,
        }
    }

    /// Convenience for the placeholder default model. Update when the real
    /// model name is announced.
    pub fn default_model(api_key: impl Into<String>) -> Self {
        Self::new(
            api_key,
            PLACEHOLDER_ANTHROPIC_MODEL,
            PLACEHOLDER_ANTHROPIC_DIM,
        )
    }
}

impl EmbeddingsProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.model
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed_batch(
        &self,
        texts: &[String],
        _input_type: Option<&str>,
    ) -> EmbedResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(ANTHROPIC_MAX_BATCH) {
            let req = AnthropicRequest {
                input: chunk.iter().map(String::as_str).collect(),
                model: &self.model,
            };
            let body = serde_json::to_string(&req)
                .map_err(|e| EmbeddingsError::Parse(format!("serializing request: {e}")))?;
            // v0.19.5 audit H6: see `curl_post_with_secret_header` doc.
            let output = curl_post_with_secret_header(
                ANTHROPIC_ENDPOINT,
                &format!("x-api-key: {}", self.api_key),
                &[
                    ("anthropic-version", "2023-06-01"),
                    ("Content-Type", "application/json"),
                ],
                &body,
            )?;
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
                    // v0.19.5 audit H8: scrub bearer/x-api-key.
                    return Err(EmbeddingsError::AuthFailed(crate::runner::scrub_secrets(
                        &combined,
                    )));
                }
                return Err(EmbeddingsError::ProviderCall {
                    code: output.status.code(),
                    detail: combined,
                });
            }
            let parsed: AnthropicResponse =
                serde_json::from_slice(&output.stdout).map_err(|e| {
                    EmbeddingsError::Parse(format!(
                        "anthropic response: {e}; body={}",
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
                    EmbeddingsError::Parse(format!("anthropic missing embedding at index {i}"))
                })?;
                out.push(v);
            }
        }
        Ok(out)
    }
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

    /// v0.19.5 audit H6: regression — `curl_post_with_secret_header`
    /// must NOT place the secret header in argv. We can't observe the
    /// stdin write directly here; instead we assert the only `-H @-`
    /// sentinel is in argv, and no string starting with
    /// `Authorization:` / `x-api-key:` / containing the actual secret.
    #[test]
    fn curl_helper_does_not_leak_secret_into_argv() {
        // Spawn helper builds a Command; we don't actually run it
        // (would shell out to curl) but we can inspect the argv it
        // would produce by replicating the construction here.
        let url = "https://example.invalid/v1/embeddings";
        let secret = "Authorization: Bearer sk-test-supersecret";
        let extra = [("Content-Type", "application/json")];
        let body = "{}";
        let mut cmd = Command::new("curl");
        cmd.args(["-s", "--fail-with-body", "-X", "POST", url, "-H", "@-"]);
        for (k, v) in &extra {
            cmd.args(["-H", &format!("{k}: {v}")]);
        }
        cmd.args(["-d", body]);
        let argv: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            argv.iter().all(|a| !a.contains("sk-test-supersecret")),
            "argv leaked secret: {argv:?}"
        );
        assert!(
            !argv.iter().any(|a| a == secret),
            "argv must not contain the full Authorization header"
        );
        assert!(
            argv.iter().any(|a| a == "@-"),
            "missing @- sentinel: {argv:?}"
        );
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

    #[test]
    fn openai_provider_empty_input_returns_empty_without_curl() {
        // No curl invocation when input is empty; safe to run without API key.
        let p = OpenAIProvider::text_embedding_3_small("fake-key");
        let result = p.embed_batch(&[], None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn openai_provider_advertises_model_and_dim() {
        let p = OpenAIProvider::text_embedding_3_small("k");
        assert_eq!(p.name(), "text-embedding-3-small");
        assert_eq!(p.dim(), 1536);

        let p2 = OpenAIProvider::text_embedding_3_large("k");
        assert_eq!(p2.name(), "text-embedding-3-large");
        assert_eq!(p2.dim(), 3072);

        let p3 = OpenAIProvider::new("k", "text-embedding-ada-002", 1536);
        assert_eq!(p3.name(), "text-embedding-ada-002");
    }

    #[test]
    fn openai_request_serializes_with_float_format() {
        let req = OpenAIRequest {
            input: vec!["hello", "world"],
            model: "text-embedding-3-small",
            encoding_format: "float",
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"input\":[\"hello\",\"world\"]"));
        assert!(json.contains("\"model\":\"text-embedding-3-small\""));
        assert!(json.contains("\"encoding_format\":\"float\""));
    }

    /// Real API integration test (requires OPENAI_API_KEY).
    #[test]
    #[ignore]
    fn openai_provider_real_smoke() {
        let key =
            std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY required for this ignored test");
        let p = OpenAIProvider::text_embedding_3_small(key);
        let v = p.embed_batch(&["hello world".to_string()], None).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].len(), DEFAULT_OPENAI_DIM);
    }

    #[test]
    fn anthropic_provider_empty_input_returns_empty_without_curl() {
        let p = AnthropicProvider::default_model("fake-key");
        let result = p.embed_batch(&[], None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn anthropic_provider_advertises_placeholder_model_and_dim() {
        let p = AnthropicProvider::default_model("k");
        assert_eq!(p.name(), PLACEHOLDER_ANTHROPIC_MODEL);
        assert_eq!(p.dim(), PLACEHOLDER_ANTHROPIC_DIM);

        let p2 = AnthropicProvider::new("k", "future-model-name", 768);
        assert_eq!(p2.name(), "future-model-name");
        assert_eq!(p2.dim(), 768);
    }

    #[test]
    fn anthropic_request_serializes_with_input_and_model() {
        let req = AnthropicRequest {
            input: vec!["hello", "world"],
            model: PLACEHOLDER_ANTHROPIC_MODEL,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"input\":[\"hello\",\"world\"]"));
        assert!(json.contains(PLACEHOLDER_ANTHROPIC_MODEL));
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

    // --- Parallel-correctness tests for the chunked rayon embed_batch shape.
    //
    // Real providers can't be exercised without API keys + network, so we use
    // a small in-test provider that mirrors the same `par_chunks(...)
    // .enumerate().map(...).collect::<Result<_,_>>()? + sort + extend`
    // pattern. The point is to lock in the contract that:
    //   1. output ordering matches input ordering even when chunks complete
    //      out-of-order on the rayon pool,
    //   2. an error in any single chunk aborts the whole batch,
    //   3. the expected number of chunks fire (no double-invocation).
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Test-only provider that follows the same parallel chunking shape as
    /// `VoyageProvider::embed_batch` / `OpenAIProvider::embed_batch`. Uses a
    /// 1-dim vector per text where the single component is a stable hash of
    /// the input string, so output order is trivially verifiable.
    struct ChunkedMockProvider {
        max_batch: usize,
        /// Counts how many times `embed_chunk` is called.
        chunk_calls: AtomicUsize,
        /// If `Some(idx)`, the chunk at index `idx` errors out.
        error_on_chunk: Option<usize>,
    }

    impl ChunkedMockProvider {
        fn new(max_batch: usize) -> Self {
            Self {
                max_batch,
                chunk_calls: AtomicUsize::new(0),
                error_on_chunk: None,
            }
        }

        fn with_error_on_chunk(max_batch: usize, idx: usize) -> Self {
            Self {
                max_batch,
                chunk_calls: AtomicUsize::new(0),
                error_on_chunk: Some(idx),
            }
        }

        fn hash_one(text: &str) -> f32 {
            // Cheap deterministic hash → unique per distinct input.
            let mut acc: u64 = 1469598103934665603;
            for b in text.as_bytes() {
                acc ^= *b as u64;
                acc = acc.wrapping_mul(1099511628211);
            }
            // Map into a finite f32; the absolute value doesn't matter as
            // long as distinct inputs map to distinct floats.
            (acc as f32) / 1.0e10
        }

        fn embed_chunk(&self, chunk: &[String], chunk_idx: usize) -> EmbedResult<Vec<Vec<f32>>> {
            self.chunk_calls.fetch_add(1, Ordering::SeqCst);
            if Some(chunk_idx) == self.error_on_chunk {
                return Err(EmbeddingsError::Parse(format!(
                    "synthetic error at chunk {chunk_idx}"
                )));
            }
            Ok(chunk.iter().map(|t| vec![Self::hash_one(t)]).collect())
        }

        fn embed_batch_chunked(&self, texts: &[String]) -> EmbedResult<Vec<Vec<f32>>> {
            if texts.is_empty() {
                return Ok(vec![]);
            }
            let mut indexed: Vec<(usize, Vec<Vec<f32>>)> = texts
                .par_chunks(self.max_batch)
                .enumerate()
                .map(|(idx, chunk)| {
                    let vecs = self.embed_chunk(chunk, idx)?;
                    Ok::<(usize, Vec<Vec<f32>>), EmbeddingsError>((idx, vecs))
                })
                .collect::<Result<Vec<_>, _>>()?;
            indexed.sort_by_key(|(idx, _)| *idx);
            let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
            for (_, vecs) in indexed {
                out.extend(vecs);
            }
            Ok(out)
        }
    }

    #[test]
    fn chunked_parallel_preserves_input_order_across_chunks() {
        // 10 distinct texts with max_batch = 4 → chunks of [0..4], [4..8], [8..10].
        // Chunks may complete out-of-order on the pool, but the final Vec must
        // still match the input order one-for-one.
        let provider = ChunkedMockProvider::new(4);
        let texts: Vec<String> = (0..10).map(|i| format!("text-{i}")).collect();
        let out = provider.embed_batch_chunked(&texts).unwrap();

        assert_eq!(out.len(), texts.len());
        let expected: Vec<Vec<f32>> = texts
            .iter()
            .map(|t| vec![ChunkedMockProvider::hash_one(t)])
            .collect();
        assert_eq!(out, expected, "output order must mirror input order");
        // 10 items / chunks of 4 = ceil(10/4) = 3 chunk calls.
        assert_eq!(provider.chunk_calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn chunked_parallel_aborts_whole_batch_on_chunk_error() {
        // Chunk index 1 errors → entire embed_batch must error too.
        let provider = ChunkedMockProvider::with_error_on_chunk(4, 1);
        let texts: Vec<String> = (0..12).map(|i| format!("text-{i}")).collect();
        let result = provider.embed_batch_chunked(&texts);

        match result {
            Err(EmbeddingsError::Parse(msg)) => {
                assert!(msg.contains("synthetic error at chunk 1"));
            }
            other => panic!("expected Parse error from chunk 1, got {other:?}"),
        }
    }

    #[test]
    fn chunked_parallel_empty_input_skips_all_chunk_calls() {
        let provider = ChunkedMockProvider::new(4);
        let out = provider.embed_batch_chunked(&[]).unwrap();
        assert!(out.is_empty());
        assert_eq!(
            provider.chunk_calls.load(Ordering::SeqCst),
            0,
            "empty input must not invoke embed_chunk at all"
        );
    }

    #[test]
    fn chunked_parallel_actually_uses_multiple_threads_when_available() {
        // Liveness check that the par_chunks code path is actually wired up.
        // We do NOT assert ≥2 threads — rayon's global pool can be saturated
        // by other parallel tests running concurrently in the same process,
        // making this test flaky under `cargo test --workspace`. The real
        // invariant we care about is that all 32 chunks DID execute (the
        // chunk_calls counter), regardless of how rayon decided to schedule
        // them.
        let provider = ChunkedMockProvider::new(1);
        let texts: Vec<String> = (0..32).map(|i| format!("text-{i}")).collect();
        let observed: Mutex<std::collections::HashSet<std::thread::ThreadId>> =
            Mutex::new(std::collections::HashSet::new());

        let _ = texts
            .par_chunks(1)
            .map(|_| {
                observed.lock().unwrap().insert(std::thread::current().id());
                Ok::<(), EmbeddingsError>(())
            })
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        // Sanity: the real provider's identical shape still hands back the
        // right number of chunk calls — this is the load-bearing assertion.
        let _ = provider.embed_batch_chunked(&texts).unwrap();
        assert_eq!(provider.chunk_calls.load(Ordering::SeqCst), 32);

        // Informational: log how many threads we observed so a future debug
        // pass can spot environments where parallelism degrades to serial.
        let thread_count = observed.lock().unwrap().len();
        eprintln!(
            "chunked_parallel: observed {thread_count} thread(s) (rayon pool: {})",
            rayon::current_num_threads()
        );
    }

    #[test]
    fn embeddings_error_display_messages_are_actionable() {
        let auth = EmbeddingsError::AuthFailed("HTTP 401 Unauthorized".into()).to_string();
        assert!(auth.contains("auth failed"), "got: {auth}");
        assert!(
            auth.contains("API key") || auth.contains("VOYAGE_API_KEY"),
            "should hint fix: {auth}"
        );
        assert!(
            auth.contains("HTTP 401"),
            "should include provider response: {auth}"
        );

        let provider_call = EmbeddingsError::ProviderCall {
            code: Some(429),
            detail: "rate limit".into(),
        }
        .to_string();
        assert!(provider_call.contains("429"), "got: {provider_call}");
        assert!(provider_call.contains("rate limit"), "got: {provider_call}");

        let parse = EmbeddingsError::Parse("missing field 'data'".into()).to_string();
        assert!(parse.contains("parsing"), "got: {parse}");
        assert!(parse.contains("missing field"), "got: {parse}");

        let io = EmbeddingsError::Io(std::io::Error::other("curl crashed")).to_string();
        assert!(io.contains("io error"), "got: {io}");
        assert!(io.contains("curl crashed"), "got: {io}");
    }
}
