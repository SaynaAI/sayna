//! Tokenizer for Kokoro TTS
//!
//! Converts phoneme strings to token indices for the ONNX model.
//!
//! The vocabulary includes:
//! - Padding token (`$`) at index 0
//! - Punctuation characters (indices 1-16)
//! - ASCII letters A-Z (indices 17-42) and a-z (indices 43-68)
//! - IPA (International Phonetic Alphabet) symbols (indices 69+)
//!
//! Unknown characters are silently filtered out during tokenization.

#![allow(dead_code)]

use super::vocab::VOCAB;
use thiserror::Error;

/// Maximum number of tokens the model can process (excluding BOS/EOS padding)
pub const MAX_TOKEN_LENGTH: usize = 510;

/// Errors that can occur during tokenization
#[derive(Debug, Error)]
pub enum TokenizerError {
    /// The token sequence exceeds the maximum length supported by the model
    #[error("Sequence too long: {length} tokens (max: {max})")]
    SequenceTooLong {
        /// Actual length of the token sequence
        length: usize,
        /// Maximum allowed length
        max: usize,
    },
}

/// Tokenize a phoneme string into a vector of token indices
///
/// Characters not in the vocabulary are silently skipped.
///
/// # Arguments
/// * `phonemes` - The phoneme string to tokenize
///
/// # Returns
/// A vector of i64 token indices
pub fn tokenize(phonemes: &str) -> Vec<i64> {
    phonemes
        .chars()
        .filter_map(|c| VOCAB.get(&c))
        .map(|&idx| idx as i64)
        .collect()
}

/// Convert tokens back to phonemes (for debugging)
pub fn tokens_to_phonemes(tokens: &[i64]) -> String {
    use super::vocab::REVERSE_VOCAB;

    tokens
        .iter()
        .filter_map(|&t| REVERSE_VOCAB.get(&(t as usize)))
        .collect()
}

/// Pad tokens with BOS/EOS markers (index 0 = '$')
///
/// The Kokoro model expects input tokens to be padded with the padding
/// token at the beginning and end.
pub fn pad_tokens(tokens: Vec<i64>) -> Vec<i64> {
    let mut padded = Vec::with_capacity(tokens.len() + 2);
    padded.push(0); // BOS padding
    padded.extend(tokens);
    padded.push(0); // EOS padding
    padded
}

/// Add silence tokens to the beginning of a token sequence
///
/// Silence tokens (index 30, which is a space character) can be added
/// to create initial silence in the generated audio.
pub fn add_initial_silence(tokens: &mut Vec<i64>, count: usize) {
    const SILENCE_TOKEN: i64 = 16; // Space character index
    for _ in 0..count {
        tokens.insert(0, SILENCE_TOKEN);
    }
}

/// Validates that a token sequence does not exceed the maximum length
///
/// The Kokoro model can process up to [`MAX_TOKEN_LENGTH`] tokens (excluding
/// BOS/EOS padding). This function checks the length and returns an error
/// if it exceeds the limit.
///
/// # Arguments
/// * `tokens` - The token sequence to validate
///
/// # Returns
/// `Ok(())` if the sequence is within the limit, or a [`TokenizerError::SequenceTooLong`]
/// error if it exceeds [`MAX_TOKEN_LENGTH`].
///
/// # Example
/// ```ignore
/// let tokens = tokenize("some phonemes");
/// validate_length(&tokens)?;  // Returns Ok(()) or error
/// ```
pub fn validate_length(tokens: &[i64]) -> Result<(), TokenizerError> {
    if tokens.len() > MAX_TOKEN_LENGTH {
        return Err(TokenizerError::SequenceTooLong {
            length: tokens.len(),
            max: MAX_TOKEN_LENGTH,
        });
    }
    Ok(())
}

/// Tokenize and validate a phoneme string
///
/// This is a convenience function that combines [`tokenize`] and [`validate_length`].
/// It tokenizes the input and validates that the result doesn't exceed the maximum length.
///
/// # Arguments
/// * `phonemes` - The phoneme string to tokenize
///
/// # Returns
/// A `Result` containing the token vector or a [`TokenizerError`] if the sequence is too long.
pub fn tokenize_validated(phonemes: &str) -> Result<Vec<i64>, TokenizerError> {
    let tokens = tokenize(phonemes);
    validate_length(&tokens)?;
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_simple() {
        let phonemes = "hello";
        let tokens = tokenize(phonemes);

        // All lowercase letters should be tokenized
        assert_eq!(tokens.len(), 5);

        // Each token should be valid (non-negative)
        for token in &tokens {
            assert!(*token >= 0);
        }
    }

    #[test]
    fn test_tokenize_with_ipa() {
        let phonemes = "həˈloʊ";
        let tokens = tokenize(phonemes);

        // Should tokenize IPA characters
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_tokenize_empty() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_invalid_chars() {
        let phonemes = "hello€world"; // € is not in vocab
        let tokens = tokenize(phonemes);

        // Should skip invalid character
        assert_eq!(tokens.len(), 10); // "helloworld" = 10 chars
    }

    #[test]
    fn test_pad_tokens() {
        let tokens = vec![1, 2, 3];
        let padded = pad_tokens(tokens);

        assert_eq!(padded.len(), 5);
        assert_eq!(padded[0], 0); // BOS
        assert_eq!(padded[4], 0); // EOS
        assert_eq!(padded[1..4], [1, 2, 3]);
    }

    #[test]
    fn test_tokens_to_phonemes() {
        let phonemes = "hello";
        let tokens = tokenize(phonemes);
        let recovered = tokens_to_phonemes(&tokens);

        assert_eq!(recovered, phonemes);
    }

    #[test]
    fn test_roundtrip() {
        let original = "hɛˈloʊ wˈɜːld";
        let tokens = tokenize(original);
        let recovered = tokens_to_phonemes(&tokens);

        // Should recover the same string (filtered for valid chars)
        let filtered: String = original.chars().filter(|c| VOCAB.contains_key(c)).collect();
        assert_eq!(recovered, filtered);
    }

    // ========================
    // Additional comprehensive tests
    // ========================

    #[test]
    fn test_validate_length_within_limit() {
        // Create tokens within the limit
        let tokens: Vec<i64> = (0..MAX_TOKEN_LENGTH as i64).collect();
        assert!(validate_length(&tokens).is_ok());
    }

    #[test]
    fn test_validate_length_at_limit() {
        // Exactly at the limit should be OK
        let tokens: Vec<i64> = (0..MAX_TOKEN_LENGTH as i64).collect();
        assert_eq!(tokens.len(), MAX_TOKEN_LENGTH);
        assert!(validate_length(&tokens).is_ok());
    }

    #[test]
    fn test_validate_length_exceeds_limit() {
        // One over the limit should error
        let tokens: Vec<i64> = (0..(MAX_TOKEN_LENGTH + 1) as i64).collect();
        let result = validate_length(&tokens);
        assert!(result.is_err());

        if let Err(TokenizerError::SequenceTooLong { length, max }) = result {
            assert_eq!(length, MAX_TOKEN_LENGTH + 1);
            assert_eq!(max, MAX_TOKEN_LENGTH);
        } else {
            panic!("Expected SequenceTooLong error");
        }
    }

    #[test]
    fn test_validate_length_empty() {
        // Empty should be OK
        let tokens: Vec<i64> = vec![];
        assert!(validate_length(&tokens).is_ok());
    }

    #[test]
    fn test_tokenize_validated_success() {
        let phonemes = "hello";
        let result = tokenize_validated(phonemes);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 5);
    }

    #[test]
    fn test_tokenize_validated_too_long() {
        // Create a very long string that exceeds the limit
        let long_phonemes: String = "a".repeat(MAX_TOKEN_LENGTH + 100);
        let result = tokenize_validated(&long_phonemes);
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_chars_filtered_with_emoji() {
        // Emoji should be filtered out, not cause an error
        let phonemes = "hello🎉world";
        let tokens = tokenize(phonemes);
        let recovered = tokens_to_phonemes(&tokens);
        assert_eq!(recovered, "helloworld");
    }

    #[test]
    fn test_max_token_length_constant() {
        // Verify the constant is set correctly
        assert_eq!(MAX_TOKEN_LENGTH, 510);
    }

    #[test]
    fn test_punctuation_tokenization() {
        // Test common punctuation
        let phonemes = "hello, world! what?";
        let tokens = tokenize(phonemes);

        // Should tokenize all characters including punctuation and spaces
        assert_eq!(tokens.len(), phonemes.len());
    }

    #[test]
    fn test_special_ipa_characters() {
        // Test stress marks and length markers
        let phonemes = "ˈhɛˈloʊː";
        let tokens = tokenize(phonemes);
        let recovered = tokens_to_phonemes(&tokens);
        assert_eq!(recovered, phonemes);
    }

    #[test]
    fn test_add_initial_silence() {
        let mut tokens = vec![1, 2, 3];
        add_initial_silence(&mut tokens, 2);

        assert_eq!(tokens.len(), 5);
        assert_eq!(tokens[0], 16); // Silence token
        assert_eq!(tokens[1], 16); // Silence token
        assert_eq!(tokens[2..], [1, 2, 3]);
    }

    #[test]
    fn test_add_initial_silence_zero_count() {
        let mut tokens = vec![1, 2, 3];
        add_initial_silence(&mut tokens, 0);
        assert_eq!(tokens, vec![1, 2, 3]);
    }

    #[test]
    fn test_pad_tokens_empty() {
        let tokens = vec![];
        let padded = pad_tokens(tokens);

        // Even empty tokens get BOS and EOS
        assert_eq!(padded.len(), 2);
        assert_eq!(padded[0], 0); // BOS
        assert_eq!(padded[1], 0); // EOS
    }

    #[test]
    fn test_tokens_to_phonemes_empty() {
        let tokens: Vec<i64> = vec![];
        let result = tokens_to_phonemes(&tokens);
        assert_eq!(result, "");
    }

    #[test]
    fn test_tokens_to_phonemes_with_unknown_ids() {
        // Token IDs that don't exist in the reverse vocab should be skipped
        let tokens = vec![9999, 10000, 50]; // First two are invalid, 50 = 'h'
        let result = tokens_to_phonemes(&tokens);
        assert_eq!(result, "h");
    }

    #[test]
    fn test_error_display() {
        let err = TokenizerError::SequenceTooLong {
            length: 600,
            max: 510,
        };
        let msg = err.to_string();
        assert!(msg.contains("600"));
        assert!(msg.contains("510"));
        assert!(msg.contains("too long"));
    }
}
