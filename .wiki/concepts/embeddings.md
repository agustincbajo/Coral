---
slug: embeddings
type: concept
last_updated_commit: 721050563f1ed29954b279fe334bf6bc8c8e2c34
confidence: 0.9
sources:
- crates/coral-runner/src/embeddings.rs
- crates/coral-core/src/embeddings.rs
- crates/coral-core/src/embeddings_sqlite.rs
backlinks: []
status: draft
generated_at: 2026-05-02T23:48:12.483332+00:00
---

# Embeddings

Coral's embeddings subsystem converts wiki page bodies into fixed-
dimension float vectors for semantic similarity [[search]].

## EmbeddingsProvider trait (`coral-runner/src/embeddings.rs`)