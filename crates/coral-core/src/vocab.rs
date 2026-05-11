//! Interned vocabulary with `Arc<str>` for memory-efficient token storage (M3.2).
//!
//! Maps string tokens to compact `TokenId` values (`u32`), enabling search
//! data structures to store `Vec<TokenId>` instead of `Vec<String>`.
//! Deduplication is achieved via an `AHashMap<Arc<str>, TokenId>` reverse
//! lookup — each unique token string is stored exactly once in memory.
//!
//! The `Vocabulary` is serializable (serde) for persistence alongside
//! the search index.

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

/// Compact token identifier. A `u32` supports up to ~4 billion unique tokens,
/// far exceeding any realistic wiki vocabulary size.
pub type TokenId = u32;

/// Interned vocabulary mapping tokens to compact IDs.
///
/// Each unique token string is stored once in an `Arc<str>`. The forward
/// vector (`tokens`) maps `TokenId → Arc<str>`, and the reverse hash map
/// (`index`) maps `Arc<str> → TokenId`.
#[derive(Debug, Clone)]
pub struct Vocabulary {
    /// Forward lookup: TokenId → string.
    tokens: Vec<Arc<str>>,
    /// Reverse lookup: string → TokenId.
    index: AHashMap<Arc<str>, TokenId>,
}

impl Vocabulary {
    /// Create an empty vocabulary.
    pub fn new() -> Self {
        Self {
            tokens: Vec::new(),
            index: AHashMap::new(),
        }
    }

    /// Intern a token, returning its ID.
    ///
    /// If the token was previously interned, returns the existing ID.
    /// Otherwise assigns the next sequential ID and stores the token.
    pub fn intern(&mut self, token: &str) -> TokenId {
        if let Some(&id) = self.index.get(token) {
            return id;
        }
        let id = self.tokens.len() as TokenId;
        let arc: Arc<str> = Arc::from(token);
        self.tokens.push(Arc::clone(&arc));
        self.index.insert(arc, id);
        id
    }

    /// Look up the ID for a token without interning it.
    pub fn get_id(&self, token: &str) -> Option<TokenId> {
        self.index.get(token).copied()
    }

    /// Look up the string for a given token ID.
    pub fn get_token(&self, id: TokenId) -> Option<&str> {
        self.tokens.get(id as usize).map(|arc| &**arc)
    }

    /// Number of unique tokens in the vocabulary.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// Whether the vocabulary is empty.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Tokenize text into a vector of token IDs.
    ///
    /// Splits on non-alphanumeric boundaries, lowercases, filters single-char
    /// tokens and stopwords, then interns each resulting token.
    pub fn tokenize(&mut self, text: &str) -> Vec<TokenId> {
        let sw = stopwords();
        text.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() > 1)
            .filter(|t| !sw.contains(t))
            .map(|t| self.intern(t))
            .collect()
    }
}

impl Default for Vocabulary {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Serde support ───
//
// We serialize as a simple list of strings (the `tokens` vec in order).
// On deserialization we rebuild the AHashMap index from the list.

impl Serialize for Vocabulary {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let strings: Vec<&str> = self.tokens.iter().map(|arc| &**arc).collect();
        strings.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Vocabulary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let strings: Vec<String> = Vec::deserialize(deserializer)?;
        let mut vocab = Vocabulary {
            tokens: Vec::with_capacity(strings.len()),
            index: AHashMap::with_capacity(strings.len()),
        };
        for s in strings {
            let arc: Arc<str> = Arc::from(s.as_str());
            let id = vocab.tokens.len() as TokenId;
            vocab.tokens.push(Arc::clone(&arc));
            vocab.index.insert(arc, id);
        }
        Ok(vocab)
    }
}

// ─── Integration function ───

/// Tokenize text using a shared vocabulary, returning token IDs.
///
/// This is the integration point for callers that want vocabulary-backed
/// tokenization without owning the `Vocabulary` directly. It delegates to
/// `Vocabulary::tokenize`.
pub fn tokenize_with_vocab(vocab: &mut Vocabulary, text: &str) -> Vec<TokenId> {
    vocab.tokenize(text)
}

// ─── Internal: stopwords (mirrors search.rs) ───

fn stopwords() -> &'static HashSet<&'static str> {
    static INSTANCE: OnceLock<HashSet<&'static str>> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        [
            "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "he", "in",
            "is", "it", "its", "of", "on", "that", "the", "to", "was", "were", "will", "with",
            // Spanish
            "el", "la", "los", "las", "de", "y", "en", "que", "es", "se", "un", "una", "para",
            "por", "con", "del", "al",
        ]
        .into_iter()
        .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    #[test]
    fn intern_same_token_twice_returns_same_id() {
        let mut vocab = Vocabulary::new();
        let id1 = vocab.intern("hello");
        let id2 = vocab.intern("hello");
        assert_eq!(id1, id2);
        assert_eq!(vocab.len(), 1);
    }

    #[test]
    fn different_tokens_get_different_ids() {
        let mut vocab = Vocabulary::new();
        let id_a = vocab.intern("alpha");
        let id_b = vocab.intern("beta");
        let id_c = vocab.intern("gamma");
        assert_ne!(id_a, id_b);
        assert_ne!(id_b, id_c);
        assert_ne!(id_a, id_c);
        assert_eq!(vocab.len(), 3);
    }

    #[test]
    fn get_token_roundtrip() {
        let mut vocab = Vocabulary::new();
        let id = vocab.intern("roundtrip");
        assert_eq!(vocab.get_token(id), Some("roundtrip"));
    }

    #[test]
    fn get_id_unknown_token_returns_none() {
        let vocab = Vocabulary::new();
        assert_eq!(vocab.get_id("nonexistent"), None);
    }

    #[test]
    fn tokenize_produces_correct_ids() {
        let mut vocab = Vocabulary::new();
        let ids = vocab.tokenize("hello world hello");
        // 3 token occurrences: hello, world, hello
        assert_eq!(ids.len(), 3);

        // hello gets id 0, world gets id 1; second "hello" reuses id 0
        let hello_id = vocab.get_id("hello").unwrap();
        let world_id = vocab.get_id("world").unwrap();
        assert_eq!(ids, vec![hello_id, world_id, hello_id]);
        // Only 2 unique tokens in the vocabulary despite 3 occurrences
        assert_eq!(vocab.len(), 2);
    }

    #[test]
    fn tokenize_filters_stopwords_and_single_chars() {
        let mut vocab = Vocabulary::new();
        let ids = vocab.tokenize("the a I is in hello world");
        // Only "hello" and "world" should survive (stopwords and single-char filtered)
        assert_eq!(ids.len(), 2);
        assert_eq!(vocab.get_token(ids[0]), Some("hello"));
        assert_eq!(vocab.get_token(ids[1]), Some("world"));
    }

    #[test]
    fn vocabulary_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Vocabulary>();
    }

    #[test]
    fn serde_roundtrip() {
        let mut vocab = Vocabulary::new();
        vocab.intern("alpha");
        vocab.intern("beta");
        vocab.intern("gamma");
        let _ = vocab.tokenize("delta epsilon");

        let json = serde_json::to_string(&vocab).unwrap();
        let deserialized: Vocabulary = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.len(), vocab.len());
        for id in 0..vocab.len() as TokenId {
            assert_eq!(
                deserialized.get_token(id),
                vocab.get_token(id),
                "token mismatch at id {id}"
            );
            let token = vocab.get_token(id).unwrap();
            assert_eq!(
                deserialized.get_id(token),
                Some(id),
                "reverse lookup failed for '{token}'"
            );
        }
    }

    #[test]
    fn large_vocabulary_memory_efficiency() {
        // Demonstrate that interned vocabulary uses less memory than Vec<String>
        // for 10k tokens with realistic duplication.
        let words: Vec<String> = (0..1000)
            .map(|i| format!("token_{i:04}"))
            .collect();

        // Simulate a corpus with heavy repetition: 10k token occurrences
        // drawn from 1000 unique tokens.
        let corpus: Vec<&str> = (0..10_000)
            .map(|i| words[i % words.len()].as_str())
            .collect();

        // Vec<String> baseline: each occurrence is a separate heap allocation.
        let vec_string_size: usize = corpus
            .iter()
            .map(|s| mem::size_of::<String>() + s.len())
            .sum();

        // Vocabulary approach: intern all, store only TokenIds.
        let mut vocab = Vocabulary::new();
        let ids: Vec<TokenId> = corpus.iter().map(|s| vocab.intern(s)).collect();

        // Vocabulary overhead: Arc<str> per unique token + AHashMap entry.
        let vocab_unique_storage: usize = vocab
            .tokens
            .iter()
            .map(|arc| mem::size_of::<Arc<str>>() + arc.len())
            .sum::<usize>()
            + vocab.len() * (mem::size_of::<Arc<str>>() + mem::size_of::<TokenId>());

        // Token ID vector: just u32 per occurrence.
        let ids_storage = ids.len() * mem::size_of::<TokenId>();
        let total_vocab_storage = vocab_unique_storage + ids_storage;

        assert!(
            total_vocab_storage < vec_string_size,
            "vocabulary storage ({total_vocab_storage} bytes) should be less than \
             Vec<String> ({vec_string_size} bytes) with heavy repetition"
        );

        // In practice, with 10x repetition the savings should be substantial.
        let ratio = total_vocab_storage as f64 / vec_string_size as f64;
        assert!(
            ratio < 0.5,
            "expected at least 50% reduction; got ratio {ratio:.3}"
        );
    }

    #[test]
    fn empty_vocabulary() {
        let vocab = Vocabulary::new();
        assert!(vocab.is_empty());
        assert_eq!(vocab.len(), 0);
        assert_eq!(vocab.get_id("anything"), None);
        assert_eq!(vocab.get_token(0), None);
    }

    #[test]
    fn tokenize_with_vocab_integration() {
        let mut vocab = Vocabulary::new();
        let ids = tokenize_with_vocab(&mut vocab, "outbox dispatcher pattern");
        assert_eq!(ids.len(), 3);
        assert_eq!(vocab.get_token(ids[0]), Some("outbox"));
        assert_eq!(vocab.get_token(ids[1]), Some("dispatcher"));
        assert_eq!(vocab.get_token(ids[2]), Some("pattern"));
    }
}
