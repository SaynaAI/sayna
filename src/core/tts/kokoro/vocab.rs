//! Vocabulary mapping for Kokoro TTS
//!
//! This module provides the character-to-index mapping used for tokenization.
//! The vocabulary includes padding, punctuation, letters, and IPA symbols.

#![allow(dead_code)]

use once_cell::sync::Lazy;
use std::collections::HashMap;

/// The vocabulary string containing all supported characters
/// Order: padding ($), punctuation, ASCII letters, IPA symbols
const VOCAB_STRING: &str = concat!(
    "$",                                                                         // Padding
    ";:,.!?\u{00a1}\u{00bf}\u{2014}\u{2026}\"\u{00ab}\u{00bb}\u{201c}\u{201d} ", // Punctuation
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",                      // ASCII letters
    "\u{0251}\u{0250}\u{0252}\u{00e6}\u{0253}\u{0299}\u{03b2}\u{0254}\u{0255}",  // IPA: ɑɐɒæɓʙβɔɕ
    "\u{00e7}\u{0257}\u{0256}\u{00f0}\u{02a4}\u{0259}\u{0258}\u{025a}\u{025b}",  // IPA: çɗɖðʤəɘɚɛ
    "\u{025c}\u{025d}\u{025e}\u{025f}\u{0284}\u{0261}\u{0260}\u{0262}\u{029b}",  // IPA: ɜɝɞɟʄɡɠɢʛ
    "\u{0266}\u{0267}\u{0127}\u{0265}\u{029c}\u{0268}\u{026a}\u{029d}\u{026d}",  // IPA: ɦɧħɥʜɨɪʝɭ
    "\u{026c}\u{026b}\u{026e}\u{029f}\u{0271}\u{026f}\u{0270}\u{014b}\u{0273}",  // IPA: ɬɫɮʟɱɯɰŋɳ
    "\u{0272}\u{0274}\u{00f8}\u{0275}\u{0278}\u{03b8}\u{0153}\u{0276}\u{0298}",  // IPA: ɲɴøɵɸθœɶʘ
    "\u{0279}\u{027a}\u{027e}\u{027b}\u{0280}\u{0281}\u{027d}\u{0282}\u{0283}",  // IPA: ɹɺɾɻʀʁɽʂʃ
    "\u{0288}\u{02a7}\u{0289}\u{028a}\u{028b}\u{2c71}\u{028c}\u{0263}\u{0264}",  // IPA: ʈʧʉʊʋⱱʌɣɤ
    "\u{028d}\u{03c7}\u{028e}\u{028f}\u{0291}\u{0290}\u{0292}\u{0294}\u{02a1}",  // IPA: ʍχʎʏʑʐʒʔʡ
    "\u{0295}\u{02a2}\u{01c0}\u{01c1}\u{01c2}\u{01c3}\u{02c8}\u{02cc}\u{02d0}",  // IPA: ʕʢǀǁǂǃˈˌː
    "\u{02d1}\u{02bc}\u{02b4}\u{02b0}\u{02b1}\u{02b2}\u{02b7}\u{02e0}\u{02e4}",  // IPA: ˑʼʴʰʱʲʷˠˤ
    "\u{02de}\u{2193}\u{2191}\u{2192}\u{2197}\u{2198}\u{2019}\u{0329}\u{2019}",  // IPA: ˞↓↑→↗↘'̩'
    "\u{1d7b}"                                                                   // IPA: ᵻ
);

/// Character to index mapping
pub static VOCAB: Lazy<HashMap<char, usize>> = Lazy::new(|| {
    VOCAB_STRING
        .chars()
        .enumerate()
        .map(|(idx, c)| (c, idx))
        .collect()
});

/// Index to character mapping (reverse vocabulary)
pub static REVERSE_VOCAB: Lazy<HashMap<usize, char>> =
    Lazy::new(|| VOCAB.iter().map(|(&c, &idx)| (idx, c)).collect());

/// Get the vocabulary size
pub fn vocab_size() -> usize {
    VOCAB.len()
}

/// Check if a character is in the vocabulary
pub fn is_valid_char(c: char) -> bool {
    VOCAB.contains_key(&c)
}

/// Get the index for a character, if it exists
pub fn char_to_index(c: char) -> Option<usize> {
    VOCAB.get(&c).copied()
}

/// Get the character for an index, if it exists
pub fn index_to_char(idx: usize) -> Option<char> {
    REVERSE_VOCAB.get(&idx).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vocab_contains_padding() {
        assert!(VOCAB.contains_key(&'$'));
        assert_eq!(VOCAB.get(&'$'), Some(&0));
    }

    #[test]
    fn test_vocab_contains_punctuation() {
        assert!(VOCAB.contains_key(&';'));
        assert!(VOCAB.contains_key(&'.'));
        assert!(VOCAB.contains_key(&'!'));
        assert!(VOCAB.contains_key(&'?'));
        assert!(VOCAB.contains_key(&' '));
    }

    #[test]
    fn test_vocab_contains_letters() {
        for c in 'A'..='Z' {
            assert!(VOCAB.contains_key(&c), "Missing uppercase letter: {}", c);
        }
        for c in 'a'..='z' {
            assert!(VOCAB.contains_key(&c), "Missing lowercase letter: {}", c);
        }
    }

    #[test]
    fn test_vocab_contains_common_ipa() {
        // Common IPA symbols used in English
        let common_ipa = [
            'ɑ', 'æ', 'ə', 'ɪ', 'ʊ', 'ɛ', 'ɔ', 'ʌ', 'ʃ', 'ʒ', 'θ', 'ð', 'ŋ',
        ];
        for c in common_ipa {
            assert!(
                VOCAB.contains_key(&c),
                "Missing IPA symbol: {} (U+{:04X})",
                c,
                c as u32
            );
        }
    }

    #[test]
    fn test_reverse_vocab() {
        // Test round-trip
        for (&c, &idx) in VOCAB.iter() {
            assert_eq!(REVERSE_VOCAB.get(&idx), Some(&c));
        }
    }

    #[test]
    fn test_vocab_size() {
        // The vocabulary should have a consistent size
        let size = vocab_size();
        assert!(size > 100, "Vocab size should be > 100, got {}", size);
        assert_eq!(VOCAB.len(), REVERSE_VOCAB.len());
    }

    #[test]
    fn test_is_valid_char() {
        assert!(is_valid_char('a'));
        assert!(is_valid_char('ə'));
        assert!(!is_valid_char('€')); // Not in vocab
    }

    #[test]
    fn test_char_to_index() {
        assert_eq!(char_to_index('$'), Some(0));
        assert!(char_to_index('a').is_some());
        assert_eq!(char_to_index('€'), None);
    }
}
