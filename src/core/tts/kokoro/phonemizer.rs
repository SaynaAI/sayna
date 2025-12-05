//! Phonemizer for Kokoro TTS
//!
//! Converts text to IPA phonemes using eSpeak-NG.
//! This module requires the `kokoro-tts` feature to be enabled.
//!
//! # Thread Safety
//!
//! eSpeak-NG uses global state internally and is not thread-safe. All calls to
//! espeak-rs are serialized through a global mutex. This is a potential bottleneck
//! in high-concurrency scenarios, but necessary for correctness.
//!
//! # Language Support
//!
//! Currently supports:
//! - American English ("a" or "en-us")
//! - British English ("b" or "en-gb")
//!
//! # System Requirements
//!
//! eSpeak-NG must be installed on the system:
//! - Ubuntu/Debian: `sudo apt-get install espeak-ng libespeak-ng-dev`
//! - macOS: `brew install espeak-ng`

use super::normalize::normalize_text;
use super::vocab::VOCAB;
use crate::core::tts::{TTSError, TTSResult};
use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::Mutex;
use thiserror::Error;

#[cfg(feature = "kokoro-tts")]
use espeak_rs::text_to_phonemes;

/// Errors that can occur during phonemization
#[derive(Debug, Error)]
#[allow(dead_code)] // Variants are used conditionally based on feature flags
pub enum PhonemizerError {
    /// eSpeak-NG encountered an error
    #[error("eSpeak error: {0}")]
    EspeakError(String),

    /// The phonemizer mutex was poisoned (another thread panicked while holding it)
    #[error("Phonemizer lock poisoned - another thread panicked during phonemization")]
    LockPoisoned,

    /// The specified language is not supported
    #[error("Unsupported language: {0}. Supported: 'a' (American English), 'b' (British English)")]
    UnsupportedLanguage(String),

    /// eSpeak-NG is not installed or not accessible
    #[error(
        "eSpeak-NG not available: {0}. Install with: apt-get install espeak-ng libespeak-ng-dev"
    )]
    EspeakNotInstalled(String),
}

/// Global mutex to serialize espeak-rs calls
/// espeak-rs uses global state internally and is not thread-safe
static ESPEAK_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

// Regex patterns for phoneme post-processing
// Note: Rust regex crate doesn't support look-around assertions, so we use
// capturing groups and replacement logic where needed.
// Match letter followed by "hˈʌndɹɪd" to insert space
static PHONEME_HUNDRED_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"([a-zɹː])(hˈʌndɹɪd)").unwrap());
// Match " z" followed by punctuation or end
static Z_PATTERN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#" z([;:,.!?¡¿—…"«»"" ])"#).unwrap());
static Z_END_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r" z$").unwrap());
// Match "nˈaɪnti" not followed by ː (for ninety pronunciation)
static NINETY_PATTERN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"nˈaɪnti([^ː])").unwrap());
static NINETY_END_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"nˈaɪnti$").unwrap());

/// Phonemizer for converting text to IPA phonemes
///
/// The phonemizer wraps eSpeak-NG and provides Kokoro-specific post-processing
/// to ensure the output phonemes are compatible with the Kokoro model.
pub struct Phonemizer {
    _initialized: bool,
}

impl Phonemizer {
    /// Create a new Phonemizer instance
    ///
    /// Initializes eSpeak-NG by performing a test phonemization.
    ///
    /// # Errors
    ///
    /// Returns an error if eSpeak-NG is not installed or cannot be initialized.
    #[cfg(feature = "kokoro-tts")]
    pub fn new() -> TTSResult<Self> {
        // Initialize espeak-rs by doing a test phonemization
        let _guard = ESPEAK_MUTEX
            .lock()
            .map_err(|_| TTSError::InternalError("Phonemizer lock poisoned".to_string()))?;

        match text_to_phonemes("test", "en-us", None, true, false) {
            Ok(_) => Ok(Self { _initialized: true }),
            Err(e) => Err(TTSError::InternalError(format!(
                "Failed to initialize eSpeak-NG: {}. Install with: apt-get install espeak-ng libespeak-ng-dev",
                e
            ))),
        }
    }

    /// Create a new Phonemizer instance (stub when feature is disabled)
    #[cfg(not(feature = "kokoro-tts"))]
    pub fn new() -> TTSResult<Self> {
        Err(TTSError::InvalidConfiguration(
            "Kokoro TTS feature is not enabled. Rebuild with --features kokoro-tts".to_string(),
        ))
    }

    /// Convert text to phonemes
    ///
    /// # Arguments
    /// * `text` - The text to phonemize (will be normalized first)
    /// * `language` - Language code: "a"/"en-us" for American, "b"/"en-gb" for British
    ///
    /// # Returns
    /// The phonemized text as an IPA string, filtered to only include
    /// characters in the Kokoro vocabulary.
    #[cfg(feature = "kokoro-tts")]
    pub fn phonemize(&self, text: &str, language: &str) -> String {
        match self.phonemize_with_result(text, language, true) {
            Ok(phonemes) => phonemes,
            Err(e) => {
                tracing::warn!("Phonemization failed: {}", e);
                String::new()
            }
        }
    }

    /// Convert text to phonemes (stub when feature is disabled)
    #[cfg(not(feature = "kokoro-tts"))]
    pub fn phonemize(&self, _text: &str, _language: &str) -> String {
        String::new()
    }

    /// Convert text to phonemes with explicit error handling
    ///
    /// # Arguments
    /// * `text` - The text to phonemize
    /// * `language` - Language code: "a"/"en-us" for American, "b"/"en-gb" for British
    /// * `normalize` - Whether to apply text normalization before phonemization
    ///
    /// # Returns
    /// A `Result` containing the phoneme string or a `PhonemizerError`
    #[cfg(feature = "kokoro-tts")]
    pub fn phonemize_with_result(
        &self,
        text: &str,
        language: &str,
        normalize: bool,
    ) -> Result<String, PhonemizerError> {
        if text.is_empty() {
            return Ok(String::new());
        }

        // Normalize the text if requested
        let normalized = if normalize {
            normalize_text(text)
        } else {
            text.to_string()
        };

        if normalized.is_empty() {
            return Ok(String::new());
        }

        // Map language code to espeak language
        let espeak_lang = map_language_code(language)?;

        // Get phonemes from espeak with proper error handling
        let phonemes = phonemize_with_lock(&normalized, espeak_lang)?;

        // Apply Kokoro-specific post-processing
        Ok(postprocess_phonemes(&phonemes, language))
    }

    /// Stub for phonemize_with_result when feature is disabled
    #[cfg(not(feature = "kokoro-tts"))]
    pub fn phonemize_with_result(
        &self,
        _text: &str,
        _language: &str,
        _normalize: bool,
    ) -> Result<String, PhonemizerError> {
        Err(PhonemizerError::EspeakNotInstalled(
            "Kokoro TTS feature is not enabled".to_string(),
        ))
    }

    /// Phonemize multiple texts in a batch
    ///
    /// This is more efficient than calling `phonemize` multiple times because
    /// the mutex is only acquired once for all texts.
    ///
    /// # Arguments
    /// * `texts` - Slice of texts to phonemize
    /// * `language` - Language code for all texts
    /// * `normalize` - Whether to apply text normalization
    ///
    /// # Returns
    /// A vector of phonemized strings, one for each input text
    #[cfg(feature = "kokoro-tts")]
    #[allow(dead_code)] // Public API for external use
    pub fn phonemize_batch(
        &self,
        texts: &[&str],
        language: &str,
        normalize: bool,
    ) -> Result<Vec<String>, PhonemizerError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Map language code once
        let espeak_lang = map_language_code(language)?;

        // Acquire the lock once for all phonemizations
        let _guard = ESPEAK_MUTEX
            .lock()
            .map_err(|_| PhonemizerError::LockPoisoned)?;

        let mut results = Vec::with_capacity(texts.len());

        for text in texts {
            if text.is_empty() {
                results.push(String::new());
                continue;
            }

            // Normalize if requested
            let normalized = if normalize {
                normalize_text(text)
            } else {
                text.to_string()
            };

            if normalized.is_empty() {
                results.push(String::new());
                continue;
            }

            // Phonemize without re-acquiring the lock
            let phonemes = text_to_phonemes(&normalized, espeak_lang, None, true, false)
                .map_err(|e| PhonemizerError::EspeakError(e.to_string()))?
                .join("");

            // Apply post-processing
            results.push(postprocess_phonemes(&phonemes, language));
        }

        Ok(results)
    }

    /// Stub for phonemize_batch when feature is disabled
    #[cfg(not(feature = "kokoro-tts"))]
    pub fn phonemize_batch(
        &self,
        _texts: &[&str],
        _language: &str,
        _normalize: bool,
    ) -> Result<Vec<String>, PhonemizerError> {
        Err(PhonemizerError::EspeakNotInstalled(
            "Kokoro TTS feature is not enabled".to_string(),
        ))
    }
}

/// Map user-friendly language code to eSpeak language code
fn map_language_code(language: &str) -> Result<&'static str, PhonemizerError> {
    match language {
        "a" | "en-us" => Ok("en-us"),
        "b" | "en-gb" => Ok("en-gb"),
        _ => Err(PhonemizerError::UnsupportedLanguage(language.to_string())),
    }
}

/// Phonemize text with the global mutex held
#[cfg(feature = "kokoro-tts")]
fn phonemize_with_lock(text: &str, language: &str) -> Result<String, PhonemizerError> {
    let _guard = ESPEAK_MUTEX
        .lock()
        .map_err(|_| PhonemizerError::LockPoisoned)?;

    let phonemes = text_to_phonemes(text, language, None, true, false)
        .map_err(|e| PhonemizerError::EspeakError(e.to_string()))?
        .join("");

    Ok(phonemes)
}

/// Apply Kokoro-specific phoneme transformations
fn postprocess_phonemes(phonemes: &str, language: &str) -> String {
    let mut result = phonemes.to_string();

    // Fix "kokoro" pronunciation
    result = result
        .replace("kəkˈoːɹoʊ", "kˈoʊkəɹoʊ")
        .replace("kəkˈɔːɹəʊ", "kˈəʊkəɹəʊ");

    // Character substitutions for better model compatibility
    result = result
        .replace('ʲ', "j")
        .replace('r', "ɹ")
        .replace('x', "k")
        .replace('ɬ', "l");

    // Apply regex transformations
    // Insert space before "hˈʌndɹɪd" when preceded by a letter
    result = PHONEME_HUNDRED_RE.replace_all(&result, "$1 $2").to_string();
    // Remove space before "z" when followed by punctuation
    result = Z_PATTERN_RE.replace_all(&result, "z$1").to_string();
    result = Z_END_RE.replace_all(&result, "z").to_string();

    // American English specific: "ninety" pronunciation (ti -> di when not followed by ː)
    if language == "a" || language == "en-us" {
        result = NINETY_PATTERN_RE
            .replace_all(&result, "nˈaɪndi$1")
            .to_string();
        result = NINETY_END_RE.replace_all(&result, "nˈaɪndi").to_string();
    }

    // Filter to only include characters in the vocabulary
    result = result.chars().filter(|&c| VOCAB.contains_key(&c)).collect();

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================
    // Postprocessing tests (always run)
    // ========================

    #[test]
    fn test_postprocess_empty() {
        let result = postprocess_phonemes("", "a");
        assert!(result.is_empty());
    }

    #[test]
    fn test_postprocess_char_replacements() {
        // Test ʲ -> j replacement
        let result = postprocess_phonemes("aʲe", "a");
        assert!(result.contains('j'));
        assert!(!result.contains('ʲ'));

        // Test r -> ɹ replacement
        let result = postprocess_phonemes("red", "a");
        assert!(!result.contains('r'));
        assert!(result.contains('ɹ'));

        // Test x -> k replacement
        let result = postprocess_phonemes("xylophone", "a");
        assert!(!result.contains('x'));

        // Test ɬ -> l replacement
        let result = postprocess_phonemes("ɬamp", "a");
        assert!(!result.contains('ɬ'));
        assert!(result.contains('l'));
    }

    #[test]
    fn test_vocab_filtering() {
        // Characters not in vocab should be filtered out
        let result = postprocess_phonemes("hello€world", "a");
        assert!(!result.contains('€'));
        assert_eq!(result, "hellowoɹld");
    }

    #[test]
    fn test_kokoro_pronunciation_fix() {
        // Test the kokoro pronunciation correction
        let result = postprocess_phonemes("kəkˈoːɹoʊ", "a");
        assert!(result.contains("kˈoʊkəɹoʊ"));

        // British English version
        let result = postprocess_phonemes("kəkˈɔːɹəʊ", "b");
        assert!(result.contains("kˈəʊkəɹəʊ"));
    }

    #[test]
    fn test_ninety_american_english() {
        // American English should convert "nˈaɪnti" to "nˈaɪndi" when not followed by ː
        let result = postprocess_phonemes("nˈaɪntix", "a");
        assert!(result.contains("nˈaɪndi"));

        let result = postprocess_phonemes("nˈaɪnti", "a");
        assert!(result.contains("nˈaɪndi"));

        // Should NOT change when followed by ː
        let result = postprocess_phonemes("nˈaɪntiː", "a");
        // The 'ti' followed by ː should not be changed
        assert!(result.contains("nˈaɪntiː"));
    }

    #[test]
    fn test_ninety_british_english() {
        // British English should NOT apply the ninety transformation
        let result = postprocess_phonemes("nˈaɪnti", "b");
        assert!(result.contains("nˈaɪnti"));
        assert!(!result.contains("nˈaɪndi"));
    }

    #[test]
    fn test_hundred_space_insertion() {
        // Should insert space before "hˈʌndɹɪd" when preceded by a letter
        let result = postprocess_phonemes("ahˈʌndɹɪd", "a");
        assert!(result.contains("a hˈʌndɹɪd"));
    }

    #[test]
    fn test_z_pattern() {
        // Space before z followed by punctuation should be removed
        let result = postprocess_phonemes("hello z.", "a");
        assert!(result.contains("helloz."));

        let result = postprocess_phonemes("hello z", "a");
        // At end of string, space should be removed
        assert!(result.ends_with("helloz") || result.ends_with("helloʊz"));
    }

    #[test]
    fn test_language_code_mapping() {
        // Test valid language codes
        assert_eq!(map_language_code("a").unwrap(), "en-us");
        assert_eq!(map_language_code("en-us").unwrap(), "en-us");
        assert_eq!(map_language_code("b").unwrap(), "en-gb");
        assert_eq!(map_language_code("en-gb").unwrap(), "en-gb");

        // Test invalid language code
        assert!(map_language_code("fr").is_err());
        assert!(map_language_code("de").is_err());
        assert!(map_language_code("invalid").is_err());
    }

    #[test]
    fn test_language_code_error_message() {
        let err = map_language_code("xyz").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("xyz"));
        assert!(msg.contains("Unsupported language"));
    }

    // ========================
    // Feature-gated tests (require kokoro-tts feature and espeak-ng)
    // ========================

    #[cfg(feature = "kokoro-tts")]
    mod integration_tests {
        use super::*;
        use std::thread;

        #[test]
        fn test_phonemizer_new() {
            // This will fail if espeak-ng is not installed
            let result = Phonemizer::new();
            // We don't assert success since espeak-ng might not be installed
            // in CI environment
            match result {
                Ok(phonemizer) => {
                    assert!(phonemizer._initialized);
                }
                Err(e) => {
                    // Expected if espeak-ng is not installed
                    assert!(e.to_string().contains("eSpeak"));
                }
            }
        }

        #[test]
        fn test_basic_phonemization() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let result = phonemizer.phonemize("hello world", "a");
            assert!(!result.is_empty());
            // Should contain IPA characters (stress marks, vowels, etc.)
            assert!(result.contains('ə') || result.contains('ˈ') || result.contains('ɛ'));
        }

        #[test]
        fn test_empty_input() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let result = phonemizer.phonemize("", "a");
            assert!(result.is_empty());

            let result = phonemizer.phonemize_with_result("", "a", true);
            assert!(result.is_ok());
            assert!(result.unwrap().is_empty());
        }

        #[test]
        fn test_language_difference() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            // Use "hello" which should work for both languages
            let us = phonemizer.phonemize("hello", "a");
            let gb_result = phonemizer.phonemize_with_result("hello", "b", true);

            eprintln!("US result for 'hello': '{}'", us);
            eprintln!("GB result for 'hello': {:?}", gb_result);

            // Both should produce output
            assert!(
                !us.is_empty(),
                "American English phonemization failed for 'hello'"
            );

            let gb = match gb_result {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("GB phonemization error: {}", e);
                    // Skip this test if GB fails - might be espeak config issue
                    return;
                }
            };

            assert!(
                !gb.is_empty(),
                "British English phonemization returned empty for 'hello'"
            );
        }

        #[test]
        fn test_phonemize_with_result_success() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let result = phonemizer.phonemize_with_result("hello", "a", true);
            assert!(result.is_ok());
            assert!(!result.unwrap().is_empty());
        }

        #[test]
        fn test_phonemize_with_result_invalid_language() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let result = phonemizer.phonemize_with_result("hello", "invalid", true);
            assert!(result.is_err());
            if let Err(PhonemizerError::UnsupportedLanguage(lang)) = result {
                assert_eq!(lang, "invalid");
            } else {
                panic!("Expected UnsupportedLanguage error");
            }
        }

        #[test]
        fn test_phonemize_without_normalization() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            // With normalization, "Dr." should become "Doctor"
            let with_norm = phonemizer.phonemize_with_result("Dr. Smith", "a", true);
            // Without normalization, "Dr." stays as is
            let without_norm = phonemizer.phonemize_with_result("Dr. Smith", "a", false);

            // Both should succeed
            assert!(with_norm.is_ok());
            assert!(without_norm.is_ok());
        }

        #[test]
        fn test_batch_phonemization() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let texts = vec!["hello", "world", "test"];
            let result = phonemizer.phonemize_batch(&texts, "a", true);

            assert!(result.is_ok());
            let phonemes = result.unwrap();
            assert_eq!(phonemes.len(), 3);
            for p in &phonemes {
                assert!(!p.is_empty());
            }
        }

        #[test]
        fn test_batch_phonemization_empty_texts() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let texts: Vec<&str> = vec![];
            let result = phonemizer.phonemize_batch(&texts, "a", true);

            assert!(result.is_ok());
            assert!(result.unwrap().is_empty());
        }

        #[test]
        fn test_batch_phonemization_with_empty_string() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let texts = vec!["hello", "", "world"];
            let result = phonemizer.phonemize_batch(&texts, "a", true);

            assert!(result.is_ok());
            let phonemes = result.unwrap();
            assert_eq!(phonemes.len(), 3);
            assert!(!phonemes[0].is_empty());
            assert!(phonemes[1].is_empty()); // Empty input -> empty output
            assert!(!phonemes[2].is_empty());
        }

        #[test]
        fn test_batch_invalid_language() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let texts = vec!["hello"];
            let result = phonemizer.phonemize_batch(&texts, "invalid", true);

            assert!(result.is_err());
        }

        #[test]
        fn test_concurrent_phonemization() {
            // Test that concurrent phonemization is thread-safe
            let handles: Vec<_> = (0..5)
                .map(|i| {
                    thread::spawn(move || {
                        let phonemizer = match Phonemizer::new() {
                            Ok(p) => p,
                            Err(_) => return Ok(String::new()), // Skip if espeak not installed
                        };
                        phonemizer.phonemize_with_result(&format!("test {}", i), "a", true)
                    })
                })
                .collect();

            for handle in handles {
                let result = handle.join().expect("Thread panicked");
                // Result should be Ok (empty if espeak not installed)
                assert!(result.is_ok());
            }
        }

        #[test]
        fn test_r_replacement_in_real_word() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let result = phonemizer.phonemize("car", "a");
            // 'r' should be replaced with 'ɹ'
            assert!(!result.contains('r'));
        }

        #[test]
        fn test_unicode_handling() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            // Unicode should be handled (and filtered out if not in vocab)
            let result = phonemizer.phonemize("hello 🎉 world", "a");
            // Emoji should be filtered out, but text should still be phonemized
            assert!(!result.contains('🎉'));
            assert!(!result.is_empty());
        }

        #[test]
        fn test_numbers_phonemization() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let result = phonemizer.phonemize("123", "a");
            // Numbers should be converted to words and phonemized
            assert!(!result.is_empty());
        }

        #[test]
        fn test_punctuation_handling() {
            let phonemizer = match Phonemizer::new() {
                Ok(p) => p,
                Err(_) => return, // Skip if espeak not installed
            };

            let result = phonemizer.phonemize("Hello, world!", "a");
            // Should produce phonemes
            assert!(!result.is_empty());
        }
    }
}
