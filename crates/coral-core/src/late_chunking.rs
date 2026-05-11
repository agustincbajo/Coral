//! Late Chunking for embeddings with surrounding context.
//!
//! Traditional chunking splits text into fixed-size windows and embeds each
//! independently. Late Chunking preserves surrounding context with each chunk,
//! enabling embeddings providers to produce better representations for
//! boundary tokens.
//!
//! Each [`Chunk`] carries `context_before` and `context_after` fields so that
//! downstream embedding calls can feed a wider window to the model while still
//! indexing the chunk boundaries precisely.

/// A text chunk with surrounding context for late-chunking embeddings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The primary text content of this chunk.
    pub text: String,
    /// Text immediately before this chunk (for context).
    pub context_before: String,
    /// Text immediately after this chunk (for context).
    pub context_after: String,
    /// Byte offset of `text` within the original document.
    pub offset: usize,
}

/// Split `text` into chunks of approximately `chunk_size` characters, with
/// `overlap` characters shared between adjacent chunks. Each chunk carries
/// surrounding context from the original text.
///
/// # Arguments
/// - `text` — the full document text to chunk.
/// - `chunk_size` — target size (in characters) for each chunk's `text` field.
/// - `overlap` — number of characters to overlap between consecutive chunks.
///
/// # Returns
/// A `Vec<Chunk>` where each chunk's `text` is at most `chunk_size` characters,
/// `context_before` contains up to `chunk_size` characters before the chunk start,
/// and `context_after` contains up to `chunk_size` characters after the chunk end.
///
/// # Edge cases
/// - Empty text returns an empty vector.
/// - Text shorter than `chunk_size` returns a single chunk.
/// - `overlap >= chunk_size` is clamped to `chunk_size - 1` to guarantee progress.
/// - `chunk_size == 0` is treated as `chunk_size = 1`.
pub fn chunk_with_context(text: &str, chunk_size: usize, overlap: usize) -> Vec<Chunk> {
    if text.is_empty() {
        return vec![];
    }

    // Clamp parameters to ensure forward progress.
    let chunk_size = chunk_size.max(1);
    let overlap = overlap.min(chunk_size - 1);
    let step = chunk_size - overlap;

    let chars: Vec<char> = text.chars().collect();
    let total_chars = chars.len();
    let mut chunks = Vec::new();
    let mut pos = 0;

    while pos < total_chars {
        let end = (pos + chunk_size).min(total_chars);

        // Extract chunk text (character-based boundaries).
        let chunk_text: String = chars[pos..end].iter().collect();

        // Context before: up to `chunk_size` characters before `pos`.
        let ctx_before_start = pos.saturating_sub(chunk_size);
        let context_before: String = chars[ctx_before_start..pos].iter().collect();

        // Context after: up to `chunk_size` characters after `end`.
        let ctx_after_end = (end + chunk_size).min(total_chars);
        let context_after: String = chars[end..ctx_after_end].iter().collect();

        // Calculate byte offset for `pos`.
        let byte_offset: usize = chars[..pos].iter().map(|c| c.len_utf8()).sum();

        chunks.push(Chunk {
            text: chunk_text,
            context_before,
            context_after,
            offset: byte_offset,
        });

        // Advance by step. If we'd stay in place (impossible with our clamp),
        // force at least one character forward.
        pos += step;
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_returns_empty() {
        let chunks = chunk_with_context("", 100, 20);
        assert!(chunks.is_empty());
    }

    #[test]
    fn text_shorter_than_chunk_size_returns_single_chunk() {
        let text = "hello world";
        let chunks = chunk_with_context(text, 100, 10);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "hello world");
        assert_eq!(chunks[0].context_before, "");
        assert_eq!(chunks[0].context_after, "");
        assert_eq!(chunks[0].offset, 0);
    }

    #[test]
    fn chunks_have_correct_context() {
        // "abcdefghij" with chunk_size=4, overlap=1 → step=3
        // Chunks: [0..4], [3..7], [6..10]
        let text = "abcdefghij";
        let chunks = chunk_with_context(text, 4, 1);

        assert_eq!(chunks.len(), 4); // positions 0, 3, 6, 9

        // First chunk: "abcd", no context_before, context_after="efgh"
        assert_eq!(chunks[0].text, "abcd");
        assert_eq!(chunks[0].context_before, "");
        assert_eq!(chunks[0].context_after, "efgh");
        assert_eq!(chunks[0].offset, 0);

        // Second chunk: "defg", context_before="abc", context_after="hij"
        assert_eq!(chunks[1].text, "defg");
        assert_eq!(chunks[1].context_before, "abc");
        assert_eq!(chunks[1].context_after, "hij");
        assert_eq!(chunks[1].offset, 3);

        // Third chunk: "ghij", context_before="cdef", context_after=""
        assert_eq!(chunks[2].text, "ghij");
        assert_eq!(chunks[2].context_before, "cdef");
        assert_eq!(chunks[2].context_after, "");
        assert_eq!(chunks[2].offset, 6);
    }

    #[test]
    fn overlap_calculation_ensures_coverage() {
        // With chunk_size=5, overlap=2 → step=3
        // Text of 12 chars: positions 0, 3, 6, 9
        let text = "abcdefghijkl";
        let chunks = chunk_with_context(text, 5, 2);

        // Verify overlap: chunk[0] ends at 5, chunk[1] starts at 3 → overlap=2
        assert_eq!(chunks[0].text, "abcde");
        assert_eq!(chunks[1].text, "defgh");
        // Characters d,e are shared between chunk 0 and chunk 1.
        assert!(chunks[0].text.ends_with("de"));
        assert!(chunks[1].text.starts_with("de"));
    }

    #[test]
    fn zero_chunk_size_is_clamped() {
        let text = "abc";
        let chunks = chunk_with_context(text, 0, 0);
        // chunk_size clamped to 1, so we get 3 chunks of 1 char each
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].text, "a");
        assert_eq!(chunks[1].text, "b");
        assert_eq!(chunks[2].text, "c");
    }

    #[test]
    fn overlap_clamped_when_exceeds_chunk_size() {
        // overlap > chunk_size should be clamped to chunk_size - 1
        let text = "abcdefgh";
        let chunks = chunk_with_context(text, 3, 100);
        // overlap clamped to 2, step = 1
        assert!(!chunks.is_empty());
        // With step=1 every chunk advances by 1 char
        assert_eq!(chunks[0].text, "abc");
        assert_eq!(chunks[1].text, "bcd");
    }

    #[test]
    fn multibyte_characters_handled_correctly() {
        // Each emoji is 4 bytes in UTF-8
        let text = "Hello\u{1F600}World\u{1F601}End";
        let chunks = chunk_with_context(text, 6, 2);
        // Just verify no panic and offsets are byte-correct
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            // Verify offset is a valid byte boundary
            assert!(
                text.is_char_boundary(chunk.offset),
                "offset {} is not a char boundary in {:?}",
                chunk.offset,
                text
            );
        }
    }

    #[test]
    fn single_char_text() {
        let chunks = chunk_with_context("x", 10, 5);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "x");
        assert_eq!(chunks[0].offset, 0);
    }
}
