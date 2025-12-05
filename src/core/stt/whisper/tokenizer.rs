//! BPE tokenizer for Whisper output decoding
//!
//! This module wraps the Whisper tokenizer for encoding prompts and decoding
//! generated token sequences. Whisper uses a multilingual BPE tokenizer with
//! special tokens for language, task, and timestamps.
//!
//! ## Token Structure
//!
//! Whisper's multilingual vocabulary has 51,865 tokens:
//! - 0-50256: Base GPT-2 vocabulary (50,257 tokens)
//! - 50257-50863: Special tokens (607 tokens)
//!   - 50257: `<|endoftext|>` - End of text
//!   - 50258: `<|startoftranscript|>` - Start of transcript
//!   - 50259-50357: Language tokens (99 languages)
//!   - 50358: `<|translate|>` - Translation task
//!   - 50359: `<|transcribe|>` - Transcription task
//!   - 50360: `<|startoflm|>` - Language model start
//!   - 50361: `<|startofprev|>` - Previous context start
//!   - 50362: `<|nospeech|>` - No speech detected
//!   - 50363: `<|notimestamps|>` - Disable timestamps
//!   - 50364-51864: Timestamp tokens (1,501 tokens for 0.00s to 30.00s)
//!
//! ## Decoding Flow
//!
//! 1. Filter special tokens (or decode them specially)
//! 2. Decode base vocab tokens using GPT-2 BPE
//! 3. Handle timestamp tokens if present
//! 4. Clean up text artifacts

#![cfg(feature = "whisper-stt")]

use crate::core::stt::STTError;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use tokenizers::Tokenizer;
use tracing::{debug, warn};

/// Special token IDs for Whisper
pub mod tokens {
    /// End of text token (also used as unknown token and BOS)
    pub const EOT: i64 = 50257;
    /// Start of transcript token
    pub const SOT: i64 = 50258;
    /// First language token (English)
    pub const LANGUAGE_BEGIN: i64 = 50259;
    /// Last language token (Sundanese)
    pub const LANGUAGE_END: i64 = 50357;
    /// Translate task token
    pub const TRANSLATE: i64 = 50358;
    /// Transcribe task token
    pub const TRANSCRIBE: i64 = 50359;
    /// Language model start token
    pub const STARTOFLM: i64 = 50360;
    /// Previous context start token
    pub const PREV: i64 = 50361;
    /// No speech detected token
    pub const NOSPEECH: i64 = 50362;
    /// No timestamps token
    pub const NO_TIMESTAMPS: i64 = 50363;
    /// First timestamp token (represents 0.00s)
    pub const TIMESTAMP_BEGIN: i64 = 50364;
    /// Last timestamp token (represents 30.00s)
    /// There are 1501 timestamp tokens (0 to 1500 * 0.02 = 30.00s)
    pub const TIMESTAMP_END: i64 = 50364 + 1500;

    /// Base vocabulary size (GPT-2)
    pub const BASE_VOCAB_SIZE: i64 = 50257;
}

/// String representations of special tokens
pub mod token_strings {
    pub const EOT: &str = "<|endoftext|>";
    pub const SOT: &str = "<|startoftranscript|>";
    pub const TRANSLATE: &str = "<|translate|>";
    pub const TRANSCRIBE: &str = "<|transcribe|>";
    pub const STARTOFLM: &str = "<|startoflm|>";
    pub const PREV: &str = "<|startofprev|>";
    pub const NOSPEECH: &str = "<|nospeech|>";
    pub const NO_TIMESTAMPS: &str = "<|notimestamps|>";
}

/// Language codes supported by Whisper (99 languages in order)
/// The index corresponds to (token_id - 50259)
pub const LANGUAGES: &[&str] = &[
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", // 0-9
    "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", // 10-19
    "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", // 20-29
    "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", // 30-39
    "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", // 40-49
    "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", // 50-59
    "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", // 60-69
    "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", // 70-79
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", // 80-89
    "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su", // 90-98
];

/// Language names for display
pub const LANGUAGE_NAMES: &[&str] = &[
    "english",
    "chinese",
    "german",
    "spanish",
    "russian",
    "korean",
    "french",
    "japanese",
    "portuguese",
    "turkish",
    "polish",
    "catalan",
    "dutch",
    "arabic",
    "swedish",
    "italian",
    "indonesian",
    "hindi",
    "finnish",
    "vietnamese",
    "hebrew",
    "ukrainian",
    "greek",
    "malay",
    "czech",
    "romanian",
    "danish",
    "hungarian",
    "tamil",
    "norwegian",
    "thai",
    "urdu",
    "croatian",
    "bulgarian",
    "lithuanian",
    "latin",
    "maori",
    "malayalam",
    "welsh",
    "slovak",
    "telugu",
    "persian",
    "latvian",
    "bengali",
    "serbian",
    "azerbaijani",
    "slovenian",
    "kannada",
    "estonian",
    "macedonian",
    "breton",
    "basque",
    "icelandic",
    "armenian",
    "nepali",
    "mongolian",
    "bosnian",
    "kazakh",
    "albanian",
    "swahili",
    "galician",
    "marathi",
    "punjabi",
    "sinhala",
    "khmer",
    "shona",
    "yoruba",
    "somali",
    "afrikaans",
    "occitan",
    "georgian",
    "belarusian",
    "tajik",
    "sindhi",
    "gujarati",
    "amharic",
    "yiddish",
    "lao",
    "uzbek",
    "faroese",
    "haitian creole",
    "pashto",
    "turkmen",
    "nynorsk",
    "maltese",
    "sanskrit",
    "luxembourgish",
    "myanmar",
    "tibetan",
    "tagalog",
    "malagasy",
    "assamese",
    "tatar",
    "hawaiian",
    "lingala",
    "hausa",
    "bashkir",
    "javanese",
    "sundanese",
];

/// Whisper transcription task
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    /// Transcribe audio in the source language
    Transcribe,
    /// Translate audio to English
    Translate,
}

impl Task {
    /// Get the token ID for this task
    pub fn token_id(&self) -> i64 {
        match self {
            Task::Transcribe => tokens::TRANSCRIBE,
            Task::Translate => tokens::TRANSLATE,
        }
    }

    /// Get the token string for this task
    pub fn token_string(&self) -> &'static str {
        match self {
            Task::Transcribe => token_strings::TRANSCRIBE,
            Task::Translate => token_strings::TRANSLATE,
        }
    }
}

/// A timestamp marker extracted from decoded output
#[derive(Debug, Clone, PartialEq)]
pub struct Timestamp {
    /// Time in seconds (0.00 to 30.00)
    pub time: f32,
    /// Position in the text (character index)
    pub text_position: usize,
}

/// HuggingFace tokenizer.json structure (partial, only what we need)
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TokenizerJson {
    /// BPE model configuration (reserved for future use)
    model: TokenizerModel,
    /// List of special tokens added to the vocabulary
    added_tokens: Option<Vec<AddedToken>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TokenizerModel {
    /// Vocabulary mapping (token string -> token ID)
    vocab: HashMap<String, i64>,
    /// BPE merge rules
    merges: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AddedToken {
    id: i64,
    content: String,
    special: bool,
}

/// Whisper BPE tokenizer wrapper
///
/// Handles decoding of Whisper model outputs, including:
/// - Base vocabulary (tokens 0-50256)
/// - Special tokens (language, task, control)
/// - Timestamp tokens for temporal alignment
pub struct WhisperTokenizer {
    /// HuggingFace tokenizer loaded from tokenizer.json
    tokenizer: Option<Tokenizer>,
    /// Mapping from token ID to string for special tokens
    special_tokens: HashMap<i64, String>,
    /// Mapping from string to token ID for special tokens
    special_token_ids: HashMap<String, i64>,
    /// Current language token ID
    language_token: i64,
    /// Current task token
    task_token: i64,
}

impl WhisperTokenizer {
    /// Create a new tokenizer with default settings (English, transcribe)
    ///
    /// Note: This creates a tokenizer without the HuggingFace tokenizer loaded.
    /// For full functionality, use `load()` to load from tokenizer.json.
    pub fn new() -> Result<Self, STTError> {
        let mut tokenizer = Self {
            tokenizer: None,
            special_tokens: HashMap::new(),
            special_token_ids: HashMap::new(),
            language_token: tokens::LANGUAGE_BEGIN, // English
            task_token: tokens::TRANSCRIBE,
        };

        // Initialize built-in special tokens
        tokenizer.init_special_tokens();

        Ok(tokenizer)
    }

    /// Load tokenizer from HuggingFace tokenizer.json file
    ///
    /// This loads the actual Whisper tokenizer vocabulary from the model's tokenizer file.
    /// This is required for proper decoding of model outputs.
    ///
    /// # Arguments
    /// * `path` - Path to tokenizer.json
    pub fn load(path: &Path) -> Result<Self, STTError> {
        let mut wrapper = Self::new()?;

        // Load the HuggingFace tokenizer
        if path.exists() {
            let hf_tokenizer = Tokenizer::from_file(path).map_err(|e| {
                STTError::ConfigurationError(format!("Failed to load tokenizer.json: {}", e))
            })?;
            wrapper.tokenizer = Some(hf_tokenizer);
            debug!("Loaded Whisper tokenizer from {:?}", path);

            // Also try to load additional special tokens from JSON
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    if let Err(e) = wrapper.load_from_json(&content) {
                        warn!("Failed to parse tokenizer.json metadata: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Failed to read tokenizer.json for metadata: {}", e);
                }
            }
        } else {
            return Err(STTError::ConfigurationError(format!(
                "Tokenizer file not found: {:?}. Run 'sayna init' to download.",
                path
            )));
        }

        Ok(wrapper)
    }

    /// Initialize built-in special tokens
    fn init_special_tokens(&mut self) {
        // Core special tokens
        self.add_special_token(tokens::EOT, token_strings::EOT);
        self.add_special_token(tokens::SOT, token_strings::SOT);
        self.add_special_token(tokens::TRANSLATE, token_strings::TRANSLATE);
        self.add_special_token(tokens::TRANSCRIBE, token_strings::TRANSCRIBE);
        self.add_special_token(tokens::STARTOFLM, token_strings::STARTOFLM);
        self.add_special_token(tokens::PREV, token_strings::PREV);
        self.add_special_token(tokens::NOSPEECH, token_strings::NOSPEECH);
        self.add_special_token(tokens::NO_TIMESTAMPS, token_strings::NO_TIMESTAMPS);

        // Language tokens
        for (idx, lang) in LANGUAGES.iter().enumerate() {
            let token_id = tokens::LANGUAGE_BEGIN + idx as i64;
            let token_str = format!("<|{}|>", lang);
            self.add_special_token(token_id, &token_str);
        }

        // Timestamp tokens (0.00 to 30.00 in 0.02s increments)
        for i in 0..=1500 {
            let token_id = tokens::TIMESTAMP_BEGIN + i;
            let time = i as f32 * 0.02;
            let token_str = format!("<|{:.2}|>", time);
            self.add_special_token(token_id, &token_str);
        }
    }

    /// Add a special token mapping
    fn add_special_token(&mut self, id: i64, content: &str) {
        self.special_tokens.insert(id, content.to_string());
        self.special_token_ids.insert(content.to_string(), id);
    }

    /// Load additional tokens from tokenizer.json content
    fn load_from_json(&mut self, content: &str) -> Result<(), STTError> {
        let tokenizer_json: TokenizerJson = serde_json::from_str(content).map_err(|e| {
            STTError::ConfigurationError(format!("Failed to parse tokenizer.json: {}", e))
        })?;

        // Add any additional special tokens from added_tokens
        if let Some(added_tokens) = tokenizer_json.added_tokens {
            for token in added_tokens {
                if token.special && !self.special_tokens.contains_key(&token.id) {
                    self.add_special_token(token.id, &token.content);
                }
            }
        }

        Ok(())
    }

    /// Set the language for transcription
    ///
    /// # Arguments
    /// * `language` - Two-letter language code (e.g., "en", "es", "zh")
    pub fn set_language(&mut self, language: &str) -> Result<(), STTError> {
        let lang_idx = LANGUAGES
            .iter()
            .position(|&l| l == language)
            .ok_or_else(|| {
                STTError::ConfigurationError(format!(
                    "Unsupported language: {}. Supported: {:?}",
                    language,
                    LANGUAGES.iter().take(10).collect::<Vec<_>>()
                ))
            })?;
        self.language_token = tokens::LANGUAGE_BEGIN + lang_idx as i64;
        Ok(())
    }

    /// Set the task (transcribe or translate)
    pub fn set_task(&mut self, task: Task) {
        self.task_token = task.token_id();
    }

    /// Get the initial prompt tokens for transcription/translation
    ///
    /// Returns the sequence: [SOT, language, task, NO_TIMESTAMPS]
    pub fn get_prompt_tokens(&self) -> Vec<i64> {
        vec![
            tokens::SOT,
            self.language_token,
            self.task_token,
            tokens::NO_TIMESTAMPS,
        ]
    }

    /// Get the initial prompt tokens with timestamps enabled
    ///
    /// Returns the sequence: [SOT, language, task]
    pub fn get_prompt_tokens_with_timestamps(&self) -> Vec<i64> {
        vec![tokens::SOT, self.language_token, self.task_token]
    }

    /// Decode token IDs to text, filtering out special tokens
    ///
    /// This is the primary decode method for getting clean transcription text.
    ///
    /// # Arguments
    /// * `token_ids` - Token IDs to decode
    ///
    /// # Returns
    /// * Decoded text string with special tokens removed
    pub fn decode(&self, token_ids: &[i64]) -> Result<String, STTError> {
        // Filter out special tokens (anything >= EOT)
        let base_tokens: Vec<u32> = token_ids
            .iter()
            .filter(|&&t| (0..tokens::EOT).contains(&t))
            .map(|&t| t as u32)
            .collect();

        // Return early for empty input (no tokenizer needed)
        if base_tokens.is_empty() {
            return Ok(String::new());
        }

        let tokenizer = self.tokenizer.as_ref().ok_or_else(|| {
            STTError::ConfigurationError(
                "Tokenizer not loaded. Use WhisperTokenizer::load() instead of new()".to_string(),
            )
        })?;

        // Decode using HuggingFace tokenizer
        let text = tokenizer.decode(&base_tokens, true).map_err(|e| {
            STTError::AudioProcessingError(format!("Failed to decode tokens: {}", e))
        })?;

        // Clean up the text
        Ok(self.clean_text(&text))
    }

    /// Decode token IDs with timestamp extraction
    ///
    /// Returns both the decoded text and a list of timestamp markers.
    ///
    /// # Arguments
    /// * `token_ids` - Token IDs to decode
    ///
    /// # Returns
    /// * Tuple of (text, timestamps)
    pub fn decode_with_timestamps(
        &self,
        token_ids: &[i64],
    ) -> Result<(String, Vec<Timestamp>), STTError> {
        let tokenizer = self.tokenizer.as_ref().ok_or_else(|| {
            STTError::ConfigurationError(
                "Tokenizer not loaded. Use WhisperTokenizer::load() instead of new()".to_string(),
            )
        })?;

        let mut text_parts: Vec<String> = Vec::new();
        let mut timestamps: Vec<Timestamp> = Vec::new();
        let mut current_position: usize = 0;
        let mut base_tokens: Vec<u32> = Vec::new();

        for &token_id in token_ids {
            if self.is_timestamp(token_id) {
                // Flush accumulated base tokens
                if !base_tokens.is_empty() {
                    let decoded = tokenizer.decode(&base_tokens, true).map_err(|e| {
                        STTError::AudioProcessingError(format!("Failed to decode tokens: {}", e))
                    })?;
                    let cleaned = self.clean_text(&decoded);
                    current_position += cleaned.len();
                    text_parts.push(cleaned);
                    base_tokens.clear();
                }

                // Record timestamp
                if let Some(time) = self.timestamp_to_seconds(token_id) {
                    timestamps.push(Timestamp {
                        time,
                        text_position: current_position,
                    });
                }
            } else if (0..tokens::EOT).contains(&token_id) {
                // Base vocabulary token
                base_tokens.push(token_id as u32);
            }
            // Skip other special tokens
        }

        // Flush remaining base tokens
        if !base_tokens.is_empty() {
            let decoded = tokenizer.decode(&base_tokens, true).map_err(|e| {
                STTError::AudioProcessingError(format!("Failed to decode tokens: {}", e))
            })?;
            text_parts.push(self.clean_text(&decoded));
        }

        Ok((text_parts.join(""), timestamps))
    }

    /// Decode token IDs without filtering, including special token representations
    ///
    /// Useful for debugging and understanding model output.
    ///
    /// # Arguments
    /// * `token_ids` - Token IDs to decode
    ///
    /// # Returns
    /// * Decoded text including special token strings
    pub fn decode_raw(&self, token_ids: &[i64]) -> Result<String, STTError> {
        let tokenizer = self.tokenizer.as_ref().ok_or_else(|| {
            STTError::ConfigurationError(
                "Tokenizer not loaded. Use WhisperTokenizer::load() instead of new()".to_string(),
            )
        })?;

        let mut parts: Vec<String> = Vec::new();
        let mut base_tokens: Vec<u32> = Vec::new();

        for &token_id in token_ids {
            if token_id >= tokens::EOT {
                // Flush accumulated base tokens first
                if !base_tokens.is_empty() {
                    let decoded = tokenizer.decode(&base_tokens, true).map_err(|e| {
                        STTError::AudioProcessingError(format!("Failed to decode tokens: {}", e))
                    })?;
                    parts.push(decoded);
                    base_tokens.clear();
                }

                // Add special token representation
                if let Some(token_str) = self.special_tokens.get(&token_id) {
                    parts.push(token_str.clone());
                } else {
                    parts.push(format!("<|unknown:{}|>", token_id));
                }
            } else if token_id >= 0 {
                base_tokens.push(token_id as u32);
            }
        }

        // Flush remaining base tokens
        if !base_tokens.is_empty() {
            let decoded = tokenizer.decode(&base_tokens, true).map_err(|e| {
                STTError::AudioProcessingError(format!("Failed to decode tokens: {}", e))
            })?;
            parts.push(decoded);
        }

        Ok(parts.join(""))
    }

    /// Check if a token is a special token (>= EOT)
    pub fn is_special_token(&self, token_id: i64) -> bool {
        token_id >= tokens::EOT
    }

    /// Check if a token is a timestamp token
    pub fn is_timestamp(&self, token_id: i64) -> bool {
        (tokens::TIMESTAMP_BEGIN..=tokens::TIMESTAMP_END).contains(&token_id)
    }

    /// Check if a token is a language token
    pub fn is_language_token(&self, token_id: i64) -> bool {
        (tokens::LANGUAGE_BEGIN..=tokens::LANGUAGE_END).contains(&token_id)
    }

    /// Convert timestamp token to seconds
    ///
    /// # Arguments
    /// * `token_id` - Timestamp token ID
    ///
    /// # Returns
    /// * Time in seconds (0.00 to 30.00), or None if not a timestamp token
    pub fn timestamp_to_seconds(&self, token_id: i64) -> Option<f32> {
        if self.is_timestamp(token_id) {
            Some((token_id - tokens::TIMESTAMP_BEGIN) as f32 * 0.02)
        } else {
            None
        }
    }

    /// Convert seconds to timestamp token ID
    ///
    /// # Arguments
    /// * `seconds` - Time in seconds (clamped to 0.00-30.00)
    ///
    /// # Returns
    /// * Timestamp token ID
    pub fn seconds_to_timestamp(&self, seconds: f32) -> i64 {
        let clamped = seconds.clamp(0.0, 30.0);
        let index = (clamped / 0.02).round() as i64;
        tokens::TIMESTAMP_BEGIN + index.min(1500)
    }

    /// Get language code from a language token
    ///
    /// # Arguments
    /// * `token_id` - Language token ID
    ///
    /// # Returns
    /// * Language code (e.g., "en", "es"), or None if not a language token
    pub fn get_language_from_token(&self, token_id: i64) -> Option<&'static str> {
        if self.is_language_token(token_id) {
            let idx = (token_id - tokens::LANGUAGE_BEGIN) as usize;
            LANGUAGES.get(idx).copied()
        } else {
            None
        }
    }

    /// Get language token ID from language code
    ///
    /// # Arguments
    /// * `lang_code` - Two-letter language code
    ///
    /// # Returns
    /// * Token ID, or None if language not supported
    pub fn language_token_id(&self, lang_code: &str) -> Option<i64> {
        LANGUAGES
            .iter()
            .position(|&l| l == lang_code)
            .map(|idx| tokens::LANGUAGE_BEGIN + idx as i64)
    }

    /// Get the full language name from language code
    ///
    /// # Arguments
    /// * `lang_code` - Two-letter language code
    ///
    /// # Returns
    /// * Full language name, or None if not found
    pub fn get_language_name(&self, lang_code: &str) -> Option<&'static str> {
        LANGUAGES
            .iter()
            .position(|&l| l == lang_code)
            .and_then(|idx| LANGUAGE_NAMES.get(idx).copied())
    }

    /// Get special token string by ID
    pub fn get_special_token_string(&self, token_id: i64) -> Option<&str> {
        self.special_tokens.get(&token_id).map(|s| s.as_str())
    }

    /// Get special token ID by string
    pub fn get_special_token_id(&self, token_str: &str) -> Option<i64> {
        self.special_token_ids.get(token_str).copied()
    }

    /// Check if the token sequence indicates no speech was detected
    pub fn has_no_speech(&self, token_ids: &[i64]) -> bool {
        token_ids.contains(&tokens::NOSPEECH)
    }

    /// Clean up decoded text
    ///
    /// - Trims leading/trailing whitespace
    /// - Normalizes multiple spaces to single space
    /// - Preserves a single leading space if the original had exactly one
    ///   (common in BPE output where words are prefixed with space)
    fn clean_text(&self, text: &str) -> String {
        // Normalize whitespace by splitting and rejoining
        let result: String = text.split_whitespace().collect::<Vec<_>>().join(" ");

        // Preserve single leading space if original had exactly one
        // (common in BPE where tokens like " Hello" have the space as part of the token)
        if text.starts_with(' ') && !text.starts_with("  ") && !result.is_empty() {
            format!(" {}", result)
        } else {
            result
        }
    }

    /// Get the end-of-text token ID
    pub fn eot_token(&self) -> i64 {
        tokens::EOT
    }

    /// Get the start-of-transcript token ID
    pub fn sot_token(&self) -> i64 {
        tokens::SOT
    }

    /// Get the current language token ID
    pub fn current_language_token(&self) -> i64 {
        self.language_token
    }

    /// Get the current task token ID
    pub fn current_task_token(&self) -> i64 {
        self.task_token
    }
}

impl Default for WhisperTokenizer {
    fn default() -> Self {
        Self::new().expect("Failed to create default WhisperTokenizer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_special_token_constants() {
        assert_eq!(tokens::EOT, 50257);
        assert_eq!(tokens::SOT, 50258);
        assert_eq!(tokens::LANGUAGE_BEGIN, 50259);
        assert_eq!(tokens::LANGUAGE_END, 50357);
        assert_eq!(tokens::TRANSLATE, 50358);
        assert_eq!(tokens::TRANSCRIBE, 50359);
        assert_eq!(tokens::NO_TIMESTAMPS, 50363);
        assert_eq!(tokens::TIMESTAMP_BEGIN, 50364);
        assert_eq!(tokens::TIMESTAMP_END, 50364 + 1500);
    }

    #[test]
    fn test_languages_list() {
        assert_eq!(LANGUAGES[0], "en");
        assert_eq!(LANGUAGES[1], "zh");
        assert_eq!(LANGUAGES[2], "de");
        assert_eq!(LANGUAGES.len(), 99);

        // Verify language names match
        assert_eq!(LANGUAGE_NAMES[0], "english");
        assert_eq!(LANGUAGE_NAMES[1], "chinese");
        assert_eq!(LANGUAGE_NAMES.len(), LANGUAGES.len());
    }

    #[test]
    fn test_task_tokens() {
        assert_eq!(Task::Transcribe.token_id(), tokens::TRANSCRIBE);
        assert_eq!(Task::Translate.token_id(), tokens::TRANSLATE);
        assert_eq!(Task::Transcribe.token_string(), "<|transcribe|>");
        assert_eq!(Task::Translate.token_string(), "<|translate|>");
    }

    #[test]
    fn test_tokenizer_creation() {
        let tokenizer = WhisperTokenizer::new().unwrap();
        assert_eq!(tokenizer.current_language_token(), tokens::LANGUAGE_BEGIN);
        assert_eq!(tokenizer.current_task_token(), tokens::TRANSCRIBE);
    }

    #[test]
    fn test_set_language() {
        let mut tokenizer = WhisperTokenizer::new().unwrap();

        // Set to English (default, index 0)
        tokenizer.set_language("en").unwrap();
        assert_eq!(tokenizer.current_language_token(), tokens::LANGUAGE_BEGIN);

        // Set to Spanish (index 3)
        tokenizer.set_language("es").unwrap();
        assert_eq!(
            tokenizer.current_language_token(),
            tokens::LANGUAGE_BEGIN + 3
        );

        // Set to Chinese (index 1)
        tokenizer.set_language("zh").unwrap();
        assert_eq!(
            tokenizer.current_language_token(),
            tokens::LANGUAGE_BEGIN + 1
        );

        // Invalid language should fail
        let result = tokenizer.set_language("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_prompt_tokens() {
        let tokenizer = WhisperTokenizer::new().unwrap();
        let prompt = tokenizer.get_prompt_tokens();

        assert_eq!(prompt.len(), 4);
        assert_eq!(prompt[0], tokens::SOT);
        assert_eq!(prompt[1], tokens::LANGUAGE_BEGIN); // English
        assert_eq!(prompt[2], tokens::TRANSCRIBE);
        assert_eq!(prompt[3], tokens::NO_TIMESTAMPS);
    }

    #[test]
    fn test_get_prompt_tokens_with_timestamps() {
        let tokenizer = WhisperTokenizer::new().unwrap();
        let prompt = tokenizer.get_prompt_tokens_with_timestamps();

        assert_eq!(prompt.len(), 3);
        assert_eq!(prompt[0], tokens::SOT);
        assert_eq!(prompt[1], tokens::LANGUAGE_BEGIN); // English
        assert_eq!(prompt[2], tokens::TRANSCRIBE);
    }

    #[test]
    fn test_is_special_token() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        // Base vocab tokens are not special
        assert!(!tokenizer.is_special_token(0));
        assert!(!tokenizer.is_special_token(1000));
        assert!(!tokenizer.is_special_token(50256));

        // Special tokens (>= EOT)
        assert!(tokenizer.is_special_token(tokens::EOT));
        assert!(tokenizer.is_special_token(tokens::SOT));
        assert!(tokenizer.is_special_token(tokens::TRANSCRIBE));
        assert!(tokenizer.is_special_token(tokens::TIMESTAMP_BEGIN));
    }

    #[test]
    fn test_is_timestamp() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        assert!(tokenizer.is_timestamp(tokens::TIMESTAMP_BEGIN));
        assert!(tokenizer.is_timestamp(tokens::TIMESTAMP_BEGIN + 100));
        assert!(tokenizer.is_timestamp(tokens::TIMESTAMP_END));

        assert!(!tokenizer.is_timestamp(tokens::SOT));
        assert!(!tokenizer.is_timestamp(tokens::EOT));
        assert!(!tokenizer.is_timestamp(0));
    }

    #[test]
    fn test_is_language_token() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        assert!(tokenizer.is_language_token(tokens::LANGUAGE_BEGIN));
        assert!(tokenizer.is_language_token(tokens::LANGUAGE_BEGIN + 50));
        assert!(tokenizer.is_language_token(tokens::LANGUAGE_END));

        assert!(!tokenizer.is_language_token(tokens::SOT));
        assert!(!tokenizer.is_language_token(tokens::TRANSLATE));
    }

    #[test]
    fn test_timestamp_to_seconds() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        // First timestamp = 0.00s
        assert_eq!(
            tokenizer.timestamp_to_seconds(tokens::TIMESTAMP_BEGIN),
            Some(0.0)
        );

        // 50 tokens = 1.00s (50 * 0.02)
        assert_eq!(
            tokenizer.timestamp_to_seconds(tokens::TIMESTAMP_BEGIN + 50),
            Some(1.0)
        );

        // 1500 tokens = 30.00s
        assert_eq!(
            tokenizer.timestamp_to_seconds(tokens::TIMESTAMP_END),
            Some(30.0)
        );

        // Non-timestamp token returns None
        assert_eq!(tokenizer.timestamp_to_seconds(tokens::SOT), None);
    }

    #[test]
    fn test_seconds_to_timestamp() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        assert_eq!(tokenizer.seconds_to_timestamp(0.0), tokens::TIMESTAMP_BEGIN);
        assert_eq!(
            tokenizer.seconds_to_timestamp(1.0),
            tokens::TIMESTAMP_BEGIN + 50
        );
        assert_eq!(tokenizer.seconds_to_timestamp(30.0), tokens::TIMESTAMP_END);

        // Clamping
        assert_eq!(
            tokenizer.seconds_to_timestamp(-1.0),
            tokens::TIMESTAMP_BEGIN
        );
        assert_eq!(tokenizer.seconds_to_timestamp(100.0), tokens::TIMESTAMP_END);
    }

    #[test]
    fn test_get_language_from_token() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        assert_eq!(
            tokenizer.get_language_from_token(tokens::LANGUAGE_BEGIN),
            Some("en")
        );
        assert_eq!(
            tokenizer.get_language_from_token(tokens::LANGUAGE_BEGIN + 1),
            Some("zh")
        );
        assert_eq!(
            tokenizer.get_language_from_token(tokens::LANGUAGE_BEGIN + 3),
            Some("es")
        );

        // Non-language token returns None
        assert_eq!(tokenizer.get_language_from_token(tokens::SOT), None);
    }

    #[test]
    fn test_language_token_id() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        assert_eq!(
            tokenizer.language_token_id("en"),
            Some(tokens::LANGUAGE_BEGIN)
        );
        assert_eq!(
            tokenizer.language_token_id("zh"),
            Some(tokens::LANGUAGE_BEGIN + 1)
        );
        assert_eq!(
            tokenizer.language_token_id("es"),
            Some(tokens::LANGUAGE_BEGIN + 3)
        );
        assert_eq!(tokenizer.language_token_id("invalid"), None);
    }

    #[test]
    fn test_get_language_name() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        assert_eq!(tokenizer.get_language_name("en"), Some("english"));
        assert_eq!(tokenizer.get_language_name("zh"), Some("chinese"));
        assert_eq!(tokenizer.get_language_name("es"), Some("spanish"));
        assert_eq!(tokenizer.get_language_name("invalid"), None);
    }

    #[test]
    fn test_special_token_lookup() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        // ID to string
        assert_eq!(
            tokenizer.get_special_token_string(tokens::EOT),
            Some("<|endoftext|>")
        );
        assert_eq!(
            tokenizer.get_special_token_string(tokens::SOT),
            Some("<|startoftranscript|>")
        );

        // String to ID
        assert_eq!(
            tokenizer.get_special_token_id("<|endoftext|>"),
            Some(tokens::EOT)
        );
        assert_eq!(
            tokenizer.get_special_token_id("<|startoftranscript|>"),
            Some(tokens::SOT)
        );
    }

    #[test]
    fn test_decode_empty() {
        let tokenizer = WhisperTokenizer::new().unwrap();
        let result = tokenizer.decode(&[]).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn test_decode_filters_special_tokens() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        // Only special tokens should result in empty string
        let special_only = vec![tokens::SOT, tokens::TRANSCRIBE, tokens::NO_TIMESTAMPS];
        let result = tokenizer.decode(&special_only).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    #[ignore = "Requires loaded tokenizer from model files"]
    fn test_decode_basic_tokens() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        // These token IDs are from Whisper's vocabulary
        let tokens = vec![15947, 1002]; // "Hello world" tokens
        let result = tokenizer.decode(&tokens).unwrap();
        // Exact output depends on tokenizer
        assert!(!result.is_empty());
    }

    #[test]
    #[ignore = "Requires loaded tokenizer from model files"]
    fn test_decode_raw() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        let token_ids = vec![
            tokens::SOT,
            tokens::LANGUAGE_BEGIN, // English
            tokens::TRANSCRIBE,
        ];
        let result = tokenizer.decode_raw(&token_ids).unwrap();

        assert!(result.contains("<|startoftranscript|>"));
        assert!(result.contains("<|en|>"));
        assert!(result.contains("<|transcribe|>"));
    }

    #[test]
    fn test_has_no_speech() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        let with_nospeech = vec![tokens::SOT, tokens::NOSPEECH, tokens::EOT];
        let without_nospeech = vec![tokens::SOT, 464, 3280, tokens::EOT];

        assert!(tokenizer.has_no_speech(&with_nospeech));
        assert!(!tokenizer.has_no_speech(&without_nospeech));
    }

    #[test]
    fn test_clean_text() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        // Multiple spaces should be normalized
        let text = "Hello   world";
        assert_eq!(tokenizer.clean_text(text), "Hello world");

        // Multiple leading/trailing spaces are trimmed (not preserved)
        let text = "  Hello world  ";
        assert_eq!(tokenizer.clean_text(text), "Hello world");

        // Single leading space is preserved (common in BPE output)
        let text = " Hello world";
        assert_eq!(tokenizer.clean_text(text), " Hello world");

        // Multiple leading spaces are NOT preserved (collapsed to none)
        let text = "   Hello";
        assert_eq!(tokenizer.clean_text(text), "Hello");

        // Empty string
        let text = "";
        assert_eq!(tokenizer.clean_text(text), "");

        // Only whitespace
        let text = "   ";
        assert_eq!(tokenizer.clean_text(text), "");
    }

    #[test]
    fn test_default_tokenizer() {
        let tokenizer = WhisperTokenizer::default();
        assert_eq!(tokenizer.current_language_token(), tokens::LANGUAGE_BEGIN);
        assert_eq!(tokenizer.current_task_token(), tokens::TRANSCRIBE);
    }

    #[test]
    fn test_eot_sot_getters() {
        let tokenizer = WhisperTokenizer::new().unwrap();
        assert_eq!(tokenizer.eot_token(), tokens::EOT);
        assert_eq!(tokenizer.sot_token(), tokens::SOT);
    }

    #[test]
    #[ignore = "Requires loaded tokenizer from model files"]
    fn test_decode_with_timestamps_basic() {
        let tokenizer = WhisperTokenizer::new().unwrap();

        // Just timestamps, no text
        let token_ids = vec![tokens::TIMESTAMP_BEGIN, tokens::TIMESTAMP_BEGIN + 50];
        let (text, timestamps) = tokenizer.decode_with_timestamps(&token_ids).unwrap();

        assert_eq!(text, "");
        assert_eq!(timestamps.len(), 2);
        assert_eq!(timestamps[0].time, 0.0);
        assert_eq!(timestamps[1].time, 1.0);
    }

    #[test]
    fn test_timestamp_struct() {
        let ts = Timestamp {
            time: 1.5,
            text_position: 10,
        };
        assert_eq!(ts.time, 1.5);
        assert_eq!(ts.text_position, 10);
    }
}
