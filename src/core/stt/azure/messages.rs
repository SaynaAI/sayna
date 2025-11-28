//! WebSocket message types for Microsoft Azure Speech-to-Text API.
//!
//! This module contains all message types for parsing Azure's WebSocket responses,
//! including:
//!
//! - **Recognition Status**: Outcome indicators for recognition attempts
//! - **Speech Phrase**: Final transcription results with confidence scores
//! - **Speech Hypothesis**: Interim (partial) transcription results
//! - **Word Timing**: Per-word timing information in detailed mode
//! - **NBest Results**: Alternative transcriptions ranked by confidence
//!
//! # Message Format
//!
//! Azure Speech-to-Text sends JSON text messages that may be prefixed with headers.
//! The parser handles both formats:
//!
//! 1. Pure JSON: `{"RecognitionStatus": "Success", ...}`
//! 2. Header-prefixed: `path:speech.phrase\r\n\r\n{"RecognitionStatus": ...}`
//!
//! # Example
//!
//! ```rust
//! use sayna::core::stt::azure::messages::AzureMessage;
//!
//! // Header-prefixed format (as sent by Azure)
//! let header_prefixed = "path:speech.hypothesis\r\n\r\n{\"Text\": \"hello\", \"Offset\": 0, \"Duration\": 0}";
//!
//! // Parse the message
//! let msg = AzureMessage::parse(header_prefixed);
//! ```

use serde::Deserialize;

use crate::core::stt::base::STTResult;

// =============================================================================
// Recognition Status
// =============================================================================

/// Azure Speech-to-Text recognition status values.
///
/// Indicates the outcome of a recognition attempt in `speech.phrase` messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecognitionStatus {
    /// Speech was successfully recognized.
    Success,
    /// Audio was processed but no speech was detected.
    NoMatch,
    /// Too much silence at the start before any speech.
    InitialSilenceTimeout,
    /// Unintelligible audio was detected (background noise, multiple speakers).
    BabbleTimeout,
    /// A processing error occurred.
    Error,
    /// The dictation session has ended.
    EndOfDictation,
    /// Unknown status value (for forward compatibility).
    Unknown(String),
}

impl RecognitionStatus {
    /// Check if this status represents a successful recognition.
    #[inline]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    /// Check if this status indicates an error or failure.
    #[inline]
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error | Self::BabbleTimeout)
    }

    /// Check if this status indicates no speech was detected.
    #[inline]
    pub fn is_no_speech(&self) -> bool {
        matches!(self, Self::NoMatch | Self::InitialSilenceTimeout)
    }
}

impl std::str::FromStr for RecognitionStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let status = match s {
            "Success" => Self::Success,
            "NoMatch" => Self::NoMatch,
            "InitialSilenceTimeout" => Self::InitialSilenceTimeout,
            "BabbleTimeout" => Self::BabbleTimeout,
            "Error" => Self::Error,
            "EndOfDictation" => Self::EndOfDictation,
            _ => Self::Unknown(s.to_string()),
        };
        Ok(status)
    }
}

impl<'de> Deserialize<'de> for RecognitionStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(s.parse().unwrap())
    }
}

// =============================================================================
// Word Timing
// =============================================================================

/// Word-level timing information in detailed recognition results.
///
/// Provided when `word_level_timing` is enabled in `AzureSTTConfig`.
/// All timing values are in 100-nanosecond units from stream beginning.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WordTiming {
    /// The recognized word.
    pub word: String,

    /// Start time in 100-nanosecond units from stream beginning.
    pub offset: u64,

    /// Duration of the word in 100-nanosecond units.
    pub duration: u64,

    /// Per-word confidence score (0.0 to 1.0).
    ///
    /// Only present in some response modes.
    #[serde(default)]
    pub confidence: Option<f64>,
}

impl WordTiming {
    /// Get the start time in seconds.
    #[inline]
    pub fn start_seconds(&self) -> f64 {
        self.offset as f64 / 10_000_000.0
    }

    /// Get the duration in seconds.
    #[inline]
    pub fn duration_seconds(&self) -> f64 {
        self.duration as f64 / 10_000_000.0
    }

    /// Get the end time in seconds.
    #[inline]
    pub fn end_seconds(&self) -> f64 {
        self.start_seconds() + self.duration_seconds()
    }
}

// =============================================================================
// NBest Result
// =============================================================================

/// Alternative recognition result in detailed mode.
///
/// Azure returns an array of `NBest` entries ranked by confidence,
/// with the first entry being the most likely transcription.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NBestResult {
    /// Confidence score from 0.0 to 1.0.
    pub confidence: f64,

    /// Raw recognition without formatting or normalization.
    ///
    /// Example: "one two three four"
    pub lexical: String,

    /// Inverse Text Normalized form with numbers and abbreviations normalized.
    ///
    /// Example: "1234" instead of "one two three four"
    #[serde(rename = "ITN")]
    pub itn: String,

    /// Same as ITN but with profanity masked according to settings.
    #[serde(rename = "MaskedITN")]
    pub masked_itn: String,

    /// Final formatted text intended for display to users.
    ///
    /// Example: "Hello, World!" with punctuation and capitalization
    pub display: String,

    /// Word-level timing information.
    ///
    /// Only present when `word_level_timing` is enabled.
    #[serde(default)]
    pub words: Option<Vec<WordTiming>>,
}

// =============================================================================
// Speech Phrase (Final Results)
// =============================================================================

/// Final recognition result from `speech.phrase` events.
///
/// Contains the complete transcription after Azure has finalized
/// processing of a speech segment.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SpeechPhrase {
    /// Recognition outcome status.
    pub recognition_status: RecognitionStatus,

    /// Start position in the audio stream (100-nanosecond units).
    pub offset: u64,

    /// Duration of the recognized speech (100-nanosecond units).
    pub duration: u64,

    /// Display text in simple format.
    ///
    /// Only present when using `AzureOutputFormat::Simple`.
    #[serde(default)]
    pub display_text: Option<String>,

    /// NBest alternatives in detailed format.
    ///
    /// Only present when using `AzureOutputFormat::Detailed`.
    #[serde(default, rename = "NBest")]
    pub nbest: Option<Vec<NBestResult>>,
}

impl SpeechPhrase {
    /// Get the best transcript text.
    ///
    /// Returns `DisplayText` for simple format, or the first NBest `Display`
    /// for detailed format. Returns `None` if no text is available.
    pub fn transcript(&self) -> Option<&str> {
        // Try simple format first
        if let Some(ref text) = self.display_text {
            return Some(text.as_str());
        }

        // Try detailed format
        if let Some(ref nbest) = self.nbest
            && let Some(first) = nbest.first()
        {
            return Some(first.display.as_str());
        }

        None
    }

    /// Get the confidence score.
    ///
    /// Returns the confidence from the first NBest entry if available,
    /// or 1.0 for simple format (assumed successful).
    pub fn confidence(&self) -> f32 {
        if let Some(ref nbest) = self.nbest
            && let Some(first) = nbest.first()
        {
            return first.confidence as f32;
        }

        // Simple format doesn't include confidence, assume 1.0 for success
        if self.recognition_status.is_success() {
            1.0
        } else {
            0.0
        }
    }

    /// Get the start time in seconds.
    #[inline]
    pub fn start_seconds(&self) -> f64 {
        self.offset as f64 / 10_000_000.0
    }

    /// Get the duration in seconds.
    #[inline]
    pub fn duration_seconds(&self) -> f64 {
        self.duration as f64 / 10_000_000.0
    }

    /// Convert to the standard STTResult format.
    ///
    /// Returns `None` if recognition was not successful (NoMatch, Error, etc.).
    pub fn to_stt_result(&self) -> Option<STTResult> {
        // Only produce results for successful recognition
        if !self.recognition_status.is_success() {
            return None;
        }

        let transcript = self.transcript()?.to_string();
        let confidence = self.confidence();

        Some(STTResult::new(
            transcript, true, // is_final
            true, // is_speech_final
            confidence,
        ))
    }
}

// =============================================================================
// Speech Hypothesis (Interim Results)
// =============================================================================

/// Interim (partial) recognition result from `speech.hypothesis` events.
///
/// Sent during speech recognition before finalization.
/// The text may change as more audio is processed.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SpeechHypothesis {
    /// Current partial transcription text.
    pub text: String,

    /// Start position in the audio stream (100-nanosecond units).
    pub offset: u64,

    /// Duration processed so far (100-nanosecond units).
    pub duration: u64,
}

impl SpeechHypothesis {
    /// Get the start time in seconds.
    #[inline]
    pub fn start_seconds(&self) -> f64 {
        self.offset as f64 / 10_000_000.0
    }

    /// Get the duration in seconds.
    #[inline]
    pub fn duration_seconds(&self) -> f64 {
        self.duration as f64 / 10_000_000.0
    }

    /// Convert to the standard STTResult format.
    pub fn to_stt_result(&self) -> STTResult {
        STTResult::new(
            self.text.clone(),
            false, // is_final
            false, // is_speech_final
            0.0,   // confidence not available for hypotheses
        )
    }
}

// =============================================================================
// Speech Start/End Detection
// =============================================================================

/// Speech start detection event.
///
/// Signals when Azure detects the beginning of speech in the audio stream.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SpeechStartDetected {
    /// Position in the audio stream where speech was detected (100-nanosecond units).
    pub offset: u64,
}

impl SpeechStartDetected {
    /// Get the offset in seconds.
    #[inline]
    pub fn offset_seconds(&self) -> f64 {
        self.offset as f64 / 10_000_000.0
    }
}

/// Speech end detection event.
///
/// Signals when Azure detects the end of speech in the audio stream.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SpeechEndDetected {
    /// Position in the audio stream where speech ended (100-nanosecond units).
    pub offset: u64,
}

impl SpeechEndDetected {
    /// Get the offset in seconds.
    #[inline]
    pub fn offset_seconds(&self) -> f64 {
        self.offset as f64 / 10_000_000.0
    }
}

// =============================================================================
// Main Message Enum
// =============================================================================

/// Unified enum for all Azure Speech-to-Text WebSocket messages.
///
/// Use `AzureMessage::parse()` to deserialize incoming WebSocket messages
/// into the appropriate variant.
#[derive(Debug, Clone)]
pub enum AzureMessage {
    /// Speech was detected in the audio stream.
    SpeechStartDetected(SpeechStartDetected),

    /// Interim transcription result (may change).
    SpeechHypothesis(SpeechHypothesis),

    /// Final transcription result.
    SpeechPhrase(SpeechPhrase),

    /// End of speech segment detected.
    SpeechEndDetected(SpeechEndDetected),

    /// Beginning of a recognition turn.
    TurnStart,

    /// End of a recognition turn.
    TurnEnd,

    /// Unknown or unhandled message type.
    Unknown(String),
}

impl AzureMessage {
    /// Parse a WebSocket message into the appropriate Azure message type.
    ///
    /// Handles both pure JSON and header-prefixed message formats.
    ///
    /// # Arguments
    ///
    /// * `text` - Raw text from WebSocket message
    ///
    /// # Returns
    ///
    /// * `Result<Self, AzureMessageError>` - Parsed message or parse error
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::stt::azure::messages::AzureMessage;
    ///
    /// // Pure JSON format
    /// let json = r#"{"Text": "hello", "Offset": 0, "Duration": 0}"#;
    /// let msg = AzureMessage::parse_with_path("speech.hypothesis", json);
    ///
    /// // Header-prefixed format
    /// let prefixed = "path:speech.phrase\r\n\r\n{\"RecognitionStatus\": \"Success\"}";
    /// let msg = AzureMessage::parse(prefixed);
    /// ```
    pub fn parse(text: &str) -> Result<Self, AzureMessageError> {
        // Check for header-prefixed format (path:xxx followed by blank line)
        if let Some((path, json)) = Self::extract_path_and_body(text) {
            return Self::parse_with_path(&path, json);
        }

        // No path detected, try to infer from JSON content
        Self::parse_unknown_format(text)
    }

    /// Parse a message with a known path/event type.
    ///
    /// # Arguments
    ///
    /// * `path` - The message path (e.g., "speech.phrase", "speech.hypothesis")
    /// * `json` - The JSON body to parse
    pub fn parse_with_path(path: &str, json: &str) -> Result<Self, AzureMessageError> {
        match path {
            "speech.startDetected" => {
                let msg: SpeechStartDetected = serde_json::from_str(json)
                    .map_err(|e| AzureMessageError::ParseError(e.to_string()))?;
                Ok(AzureMessage::SpeechStartDetected(msg))
            }
            "speech.hypothesis" => {
                let msg: SpeechHypothesis = serde_json::from_str(json)
                    .map_err(|e| AzureMessageError::ParseError(e.to_string()))?;
                Ok(AzureMessage::SpeechHypothesis(msg))
            }
            "speech.phrase" => {
                let msg: SpeechPhrase = serde_json::from_str(json)
                    .map_err(|e| AzureMessageError::ParseError(e.to_string()))?;
                Ok(AzureMessage::SpeechPhrase(msg))
            }
            "speech.endDetected" => {
                let msg: SpeechEndDetected = serde_json::from_str(json)
                    .map_err(|e| AzureMessageError::ParseError(e.to_string()))?;
                Ok(AzureMessage::SpeechEndDetected(msg))
            }
            "turn.start" => Ok(AzureMessage::TurnStart),
            "turn.end" => Ok(AzureMessage::TurnEnd),
            _ => Ok(AzureMessage::Unknown(text_preview(json))),
        }
    }

    /// Extract the path and JSON body from a header-prefixed message.
    ///
    /// Azure messages may have the format:
    /// ```text
    /// X-RequestId:abc123
    /// path:speech.phrase
    ///
    /// {"RecognitionStatus": "Success", ...}
    /// ```
    fn extract_path_and_body(text: &str) -> Option<(String, &str)> {
        // Look for "path:" header (case-insensitive)
        let text_lower = text.to_lowercase();
        let path_prefix = "path:";

        let path_pos = text_lower.find(path_prefix)?;
        let path_start = path_pos + path_prefix.len();

        // Find the end of the path line
        let path_end = text[path_start..]
            .find(['\r', '\n'])
            .map(|pos| path_start + pos)
            .unwrap_or(text.len());

        let path = text[path_start..path_end].trim().to_string();

        // Find the JSON body after blank line (double newline)
        let body_start = text[path_end..]
            .find("\r\n\r\n")
            .map(|pos| path_end + pos + 4)
            .or_else(|| text[path_end..].find("\n\n").map(|pos| path_end + pos + 2))?;

        let json = text[body_start..].trim();

        if json.is_empty() || path.is_empty() {
            return None;
        }

        Some((path, json))
    }

    /// Try to parse a message without knowing the path.
    ///
    /// Attempts to infer the message type from JSON content.
    fn parse_unknown_format(text: &str) -> Result<Self, AzureMessageError> {
        // Try to peek at the JSON structure to determine type
        #[derive(Deserialize)]
        struct TypePeek {
            #[serde(rename = "RecognitionStatus")]
            recognition_status: Option<String>,
            #[serde(rename = "Text")]
            text: Option<String>,
            #[serde(rename = "Offset")]
            offset: Option<u64>,
        }

        let peek: TypePeek =
            serde_json::from_str(text).map_err(|e| AzureMessageError::ParseError(e.to_string()))?;

        // Determine message type based on available fields
        if peek.recognition_status.is_some() {
            // Has RecognitionStatus -> SpeechPhrase
            let msg: SpeechPhrase = serde_json::from_str(text)
                .map_err(|e| AzureMessageError::ParseError(e.to_string()))?;
            return Ok(AzureMessage::SpeechPhrase(msg));
        }

        if peek.text.is_some() && peek.offset.is_some() {
            // Has Text and Offset but no RecognitionStatus -> SpeechHypothesis
            let msg: SpeechHypothesis = serde_json::from_str(text)
                .map_err(|e| AzureMessageError::ParseError(e.to_string()))?;
            return Ok(AzureMessage::SpeechHypothesis(msg));
        }

        if peek.offset.is_some() && peek.text.is_none() && peek.recognition_status.is_none() {
            // Only has Offset -> could be start or end detected
            // Try to parse as SpeechStartDetected first
            if let Ok(msg) = serde_json::from_str::<SpeechStartDetected>(text) {
                return Ok(AzureMessage::SpeechStartDetected(msg));
            }
        }

        // Unknown format
        Ok(AzureMessage::Unknown(text_preview(text)))
    }

    /// Check if this message contains a final transcript.
    #[inline]
    pub fn is_final_transcript(&self) -> bool {
        matches!(self, AzureMessage::SpeechPhrase(_))
    }

    /// Check if this message contains an interim transcript.
    #[inline]
    pub fn is_interim_transcript(&self) -> bool {
        matches!(self, AzureMessage::SpeechHypothesis(_))
    }

    /// Convert this message to an STTResult if applicable.
    ///
    /// Returns `Some(STTResult)` for SpeechPhrase (with successful recognition)
    /// and SpeechHypothesis messages. Returns `None` for other message types.
    pub fn to_stt_result(&self) -> Option<STTResult> {
        match self {
            AzureMessage::SpeechPhrase(phrase) => phrase.to_stt_result(),
            AzureMessage::SpeechHypothesis(hypothesis) => Some(hypothesis.to_stt_result()),
            _ => None,
        }
    }
}

// =============================================================================
// Error Type
// =============================================================================

/// Error type for Azure message parsing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AzureMessageError {
    /// JSON parsing error.
    #[error("Failed to parse message: {0}")]
    ParseError(String),

    /// Invalid message format.
    #[error("Invalid message format: {0}")]
    InvalidFormat(String),
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Create a preview of text for logging (truncated if too long).
fn text_preview(text: &str) -> String {
    const MAX_LEN: usize = 100;
    if text.len() <= MAX_LEN {
        text.to_string()
    } else {
        format!("{}...", &text[..MAX_LEN])
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // RecognitionStatus Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_recognition_status_from_str() {
        assert_eq!(
            "Success".parse::<RecognitionStatus>().unwrap(),
            RecognitionStatus::Success
        );
        assert_eq!(
            "NoMatch".parse::<RecognitionStatus>().unwrap(),
            RecognitionStatus::NoMatch
        );
        assert_eq!(
            "InitialSilenceTimeout"
                .parse::<RecognitionStatus>()
                .unwrap(),
            RecognitionStatus::InitialSilenceTimeout
        );
        assert_eq!(
            "BabbleTimeout".parse::<RecognitionStatus>().unwrap(),
            RecognitionStatus::BabbleTimeout
        );
        assert_eq!(
            "Error".parse::<RecognitionStatus>().unwrap(),
            RecognitionStatus::Error
        );
        assert_eq!(
            "EndOfDictation".parse::<RecognitionStatus>().unwrap(),
            RecognitionStatus::EndOfDictation
        );
    }

    #[test]
    fn test_recognition_status_unknown() {
        let status = "SomeNewStatus".parse::<RecognitionStatus>().unwrap();
        assert!(matches!(status, RecognitionStatus::Unknown(_)));
        if let RecognitionStatus::Unknown(s) = status {
            assert_eq!(s, "SomeNewStatus");
        }
    }

    #[test]
    fn test_recognition_status_is_success() {
        assert!(RecognitionStatus::Success.is_success());
        assert!(!RecognitionStatus::NoMatch.is_success());
        assert!(!RecognitionStatus::Error.is_success());
    }

    #[test]
    fn test_recognition_status_is_error() {
        assert!(RecognitionStatus::Error.is_error());
        assert!(RecognitionStatus::BabbleTimeout.is_error());
        assert!(!RecognitionStatus::Success.is_error());
        assert!(!RecognitionStatus::NoMatch.is_error());
    }

    #[test]
    fn test_recognition_status_is_no_speech() {
        assert!(RecognitionStatus::NoMatch.is_no_speech());
        assert!(RecognitionStatus::InitialSilenceTimeout.is_no_speech());
        assert!(!RecognitionStatus::Success.is_no_speech());
        assert!(!RecognitionStatus::Error.is_no_speech());
    }

    // -------------------------------------------------------------------------
    // WordTiming Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_word_timing_deserialize() {
        let json = r#"{"Word": "hello", "Offset": 5000000, "Duration": 2500000}"#;
        let word: WordTiming = serde_json::from_str(json).unwrap();

        assert_eq!(word.word, "hello");
        assert_eq!(word.offset, 5000000);
        assert_eq!(word.duration, 2500000);
        assert!(word.confidence.is_none());
    }

    #[test]
    fn test_word_timing_with_confidence() {
        let json =
            r#"{"Word": "world", "Offset": 10000000, "Duration": 3000000, "Confidence": 0.95}"#;
        let word: WordTiming = serde_json::from_str(json).unwrap();

        assert_eq!(word.word, "world");
        assert_eq!(word.confidence, Some(0.95));
    }

    #[test]
    fn test_word_timing_seconds_conversion() {
        let word = WordTiming {
            word: "test".to_string(),
            offset: 10_000_000,  // 1 second
            duration: 5_000_000, // 0.5 seconds
            confidence: None,
        };

        assert!((word.start_seconds() - 1.0).abs() < 0.001);
        assert!((word.duration_seconds() - 0.5).abs() < 0.001);
        assert!((word.end_seconds() - 1.5).abs() < 0.001);
    }

    // -------------------------------------------------------------------------
    // NBestResult Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_nbest_result_deserialize() {
        let json = r#"{
            "Confidence": 0.95,
            "Lexical": "hello world",
            "ITN": "hello world",
            "MaskedITN": "hello world",
            "Display": "Hello world."
        }"#;

        let nbest: NBestResult = serde_json::from_str(json).unwrap();

        assert!((nbest.confidence - 0.95).abs() < 0.001);
        assert_eq!(nbest.lexical, "hello world");
        assert_eq!(nbest.itn, "hello world");
        assert_eq!(nbest.masked_itn, "hello world");
        assert_eq!(nbest.display, "Hello world.");
        assert!(nbest.words.is_none());
    }

    #[test]
    fn test_nbest_result_with_words() {
        let json = r#"{
            "Confidence": 0.92,
            "Lexical": "hello",
            "ITN": "hello",
            "MaskedITN": "hello",
            "Display": "Hello.",
            "Words": [
                {"Word": "Hello", "Offset": 5000000, "Duration": 2500000}
            ]
        }"#;

        let nbest: NBestResult = serde_json::from_str(json).unwrap();

        assert!(nbest.words.is_some());
        let words = nbest.words.unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].word, "Hello");
    }

    // -------------------------------------------------------------------------
    // SpeechPhrase Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_speech_phrase_simple_format() {
        let json = r#"{
            "RecognitionStatus": "Success",
            "Offset": 5000000,
            "Duration": 28500000,
            "DisplayText": "Hello world."
        }"#;

        let phrase: SpeechPhrase = serde_json::from_str(json).unwrap();

        assert_eq!(phrase.recognition_status, RecognitionStatus::Success);
        assert_eq!(phrase.offset, 5000000);
        assert_eq!(phrase.duration, 28500000);
        assert_eq!(phrase.display_text, Some("Hello world.".to_string()));
        assert!(phrase.nbest.is_none());
    }

    #[test]
    fn test_speech_phrase_detailed_format() {
        let json = r#"{
            "RecognitionStatus": "Success",
            "Offset": 5000000,
            "Duration": 28500000,
            "NBest": [{
                "Confidence": 0.95,
                "Lexical": "hello world",
                "ITN": "hello world",
                "MaskedITN": "hello world",
                "Display": "Hello world."
            }]
        }"#;

        let phrase: SpeechPhrase = serde_json::from_str(json).unwrap();

        assert_eq!(phrase.recognition_status, RecognitionStatus::Success);
        assert!(phrase.display_text.is_none());
        assert!(phrase.nbest.is_some());

        let nbest = phrase.nbest.as_ref().unwrap();
        assert_eq!(nbest.len(), 1);
        assert_eq!(nbest[0].display, "Hello world.");
    }

    #[test]
    fn test_speech_phrase_transcript() {
        // Simple format
        let simple_json = r#"{
            "RecognitionStatus": "Success",
            "Offset": 0,
            "Duration": 0,
            "DisplayText": "Simple text"
        }"#;
        let simple: SpeechPhrase = serde_json::from_str(simple_json).unwrap();
        assert_eq!(simple.transcript(), Some("Simple text"));

        // Detailed format
        let detailed_json = r#"{
            "RecognitionStatus": "Success",
            "Offset": 0,
            "Duration": 0,
            "NBest": [{"Confidence": 0.9, "Lexical": "x", "ITN": "x", "MaskedITN": "x", "Display": "Detailed text"}]
        }"#;
        let detailed: SpeechPhrase = serde_json::from_str(detailed_json).unwrap();
        assert_eq!(detailed.transcript(), Some("Detailed text"));
    }

    #[test]
    fn test_speech_phrase_confidence() {
        // Simple format - should return 1.0 for success
        let simple: SpeechPhrase = serde_json::from_str(
            r#"{
            "RecognitionStatus": "Success",
            "Offset": 0,
            "Duration": 0,
            "DisplayText": "Text"
        }"#,
        )
        .unwrap();
        assert!((simple.confidence() - 1.0).abs() < 0.001);

        // Detailed format - should return NBest confidence
        let detailed: SpeechPhrase = serde_json::from_str(r#"{
            "RecognitionStatus": "Success",
            "Offset": 0,
            "Duration": 0,
            "NBest": [{"Confidence": 0.85, "Lexical": "x", "ITN": "x", "MaskedITN": "x", "Display": "x"}]
        }"#)
        .unwrap();
        assert!((detailed.confidence() - 0.85).abs() < 0.001);

        // NoMatch - should return 0.0
        let no_match: SpeechPhrase = serde_json::from_str(
            r#"{
            "RecognitionStatus": "NoMatch",
            "Offset": 0,
            "Duration": 0
        }"#,
        )
        .unwrap();
        assert!((no_match.confidence() - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_speech_phrase_to_stt_result_success() {
        let phrase: SpeechPhrase = serde_json::from_str(
            r#"{
            "RecognitionStatus": "Success",
            "Offset": 0,
            "Duration": 0,
            "DisplayText": "Hello world."
        }"#,
        )
        .unwrap();

        let result = phrase.to_stt_result();
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.transcript, "Hello world.");
        assert!(result.is_final);
        assert!(result.is_speech_final);
        assert!((result.confidence - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_speech_phrase_to_stt_result_no_match() {
        let phrase: SpeechPhrase = serde_json::from_str(
            r#"{
            "RecognitionStatus": "NoMatch",
            "Offset": 0,
            "Duration": 0
        }"#,
        )
        .unwrap();

        let result = phrase.to_stt_result();
        assert!(result.is_none());
    }

    #[test]
    fn test_speech_phrase_to_stt_result_error() {
        let phrase: SpeechPhrase = serde_json::from_str(
            r#"{
            "RecognitionStatus": "Error",
            "Offset": 0,
            "Duration": 0
        }"#,
        )
        .unwrap();

        let result = phrase.to_stt_result();
        assert!(result.is_none());
    }

    // -------------------------------------------------------------------------
    // SpeechHypothesis Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_speech_hypothesis_deserialize() {
        let json = r#"{"Text": "hello", "Offset": 5000000, "Duration": 2500000}"#;
        let hypothesis: SpeechHypothesis = serde_json::from_str(json).unwrap();

        assert_eq!(hypothesis.text, "hello");
        assert_eq!(hypothesis.offset, 5000000);
        assert_eq!(hypothesis.duration, 2500000);
    }

    #[test]
    fn test_speech_hypothesis_to_stt_result() {
        let hypothesis = SpeechHypothesis {
            text: "partial text".to_string(),
            offset: 0,
            duration: 0,
        };

        let result = hypothesis.to_stt_result();
        assert_eq!(result.transcript, "partial text");
        assert!(!result.is_final);
        assert!(!result.is_speech_final);
        assert!((result.confidence - 0.0).abs() < 0.001);
    }

    // -------------------------------------------------------------------------
    // SpeechStartDetected/EndDetected Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_speech_start_detected_deserialize() {
        let json = r#"{"Offset": 10000000}"#;
        let msg: SpeechStartDetected = serde_json::from_str(json).unwrap();

        assert_eq!(msg.offset, 10000000);
        assert!((msg.offset_seconds() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_speech_end_detected_deserialize() {
        let json = r#"{"Offset": 50000000}"#;
        let msg: SpeechEndDetected = serde_json::from_str(json).unwrap();

        assert_eq!(msg.offset, 50000000);
        assert!((msg.offset_seconds() - 5.0).abs() < 0.001);
    }

    // -------------------------------------------------------------------------
    // AzureMessage Parsing Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_azure_message_parse_hypothesis_with_path() {
        let json = r#"{"Text": "hello world", "Offset": 5000000, "Duration": 2500000}"#;
        let msg = AzureMessage::parse_with_path("speech.hypothesis", json).unwrap();

        assert!(matches!(msg, AzureMessage::SpeechHypothesis(_)));
        if let AzureMessage::SpeechHypothesis(h) = msg {
            assert_eq!(h.text, "hello world");
        }
    }

    #[test]
    fn test_azure_message_parse_phrase_with_path() {
        let json = r#"{
            "RecognitionStatus": "Success",
            "Offset": 5000000,
            "Duration": 28500000,
            "DisplayText": "Hello world."
        }"#;
        let msg = AzureMessage::parse_with_path("speech.phrase", json).unwrap();

        assert!(matches!(msg, AzureMessage::SpeechPhrase(_)));
        if let AzureMessage::SpeechPhrase(p) = msg {
            assert_eq!(p.recognition_status, RecognitionStatus::Success);
            assert_eq!(p.transcript(), Some("Hello world."));
        }
    }

    #[test]
    fn test_azure_message_parse_turn_events() {
        assert!(matches!(
            AzureMessage::parse_with_path("turn.start", "{}").unwrap(),
            AzureMessage::TurnStart
        ));
        assert!(matches!(
            AzureMessage::parse_with_path("turn.end", "{}").unwrap(),
            AzureMessage::TurnEnd
        ));
    }

    #[test]
    fn test_azure_message_parse_header_prefixed() {
        let text = "X-RequestId:abc123\r\npath:speech.hypothesis\r\n\r\n{\"Text\": \"hello\", \"Offset\": 0, \"Duration\": 0}";
        let msg = AzureMessage::parse(text).unwrap();

        assert!(matches!(msg, AzureMessage::SpeechHypothesis(_)));
    }

    #[test]
    fn test_azure_message_parse_unix_newlines() {
        let text = "path:speech.phrase\n\n{\"RecognitionStatus\": \"Success\", \"Offset\": 0, \"Duration\": 0, \"DisplayText\": \"test\"}";
        let msg = AzureMessage::parse(text).unwrap();

        assert!(matches!(msg, AzureMessage::SpeechPhrase(_)));
    }

    #[test]
    fn test_azure_message_parse_pure_json_phrase() {
        let json = r#"{"RecognitionStatus": "Success", "Offset": 0, "Duration": 0, "DisplayText": "test"}"#;
        let msg = AzureMessage::parse(json).unwrap();

        assert!(matches!(msg, AzureMessage::SpeechPhrase(_)));
    }

    #[test]
    fn test_azure_message_parse_pure_json_hypothesis() {
        let json = r#"{"Text": "partial", "Offset": 1000000, "Duration": 500000}"#;
        let msg = AzureMessage::parse(json).unwrap();

        assert!(matches!(msg, AzureMessage::SpeechHypothesis(_)));
    }

    #[test]
    fn test_azure_message_to_stt_result() {
        // Phrase message
        let phrase_json = r#"{"RecognitionStatus": "Success", "Offset": 0, "Duration": 0, "DisplayText": "Final."}"#;
        let phrase_msg = AzureMessage::parse(phrase_json).unwrap();
        let result = phrase_msg.to_stt_result();
        assert!(result.is_some());
        assert!(result.unwrap().is_final);

        // Hypothesis message
        let hyp_json = r#"{"Text": "Partial", "Offset": 0, "Duration": 0}"#;
        let hyp_msg = AzureMessage::parse(hyp_json).unwrap();
        let result = hyp_msg.to_stt_result();
        assert!(result.is_some());
        assert!(!result.unwrap().is_final);

        // TurnStart has no result
        let turn_msg = AzureMessage::TurnStart;
        assert!(turn_msg.to_stt_result().is_none());
    }

    #[test]
    fn test_azure_message_is_final() {
        let phrase = AzureMessage::SpeechPhrase(SpeechPhrase {
            recognition_status: RecognitionStatus::Success,
            offset: 0,
            duration: 0,
            display_text: Some("test".to_string()),
            nbest: None,
        });
        assert!(phrase.is_final_transcript());
        assert!(!phrase.is_interim_transcript());

        let hypothesis = AzureMessage::SpeechHypothesis(SpeechHypothesis {
            text: "test".to_string(),
            offset: 0,
            duration: 0,
        });
        assert!(!hypothesis.is_final_transcript());
        assert!(hypothesis.is_interim_transcript());
    }

    #[test]
    fn test_azure_message_unknown_path() {
        let msg = AzureMessage::parse_with_path("some.unknown.path", "{}").unwrap();
        assert!(matches!(msg, AzureMessage::Unknown(_)));
    }

    #[test]
    fn test_azure_message_error_invalid_json() {
        let result = AzureMessage::parse("not valid json");
        assert!(result.is_err());
    }

    // -------------------------------------------------------------------------
    // Edge Cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_speech_phrase_empty_nbest() {
        let json = r#"{
            "RecognitionStatus": "Success",
            "Offset": 0,
            "Duration": 0,
            "NBest": []
        }"#;

        let phrase: SpeechPhrase = serde_json::from_str(json).unwrap();
        assert!(phrase.transcript().is_none());
        assert!((phrase.confidence() - 1.0).abs() < 0.001); // Fallback to 1.0 for success
    }

    #[test]
    fn test_word_timing_zero_values() {
        let word = WordTiming {
            word: "".to_string(),
            offset: 0,
            duration: 0,
            confidence: Some(0.0),
        };

        assert!((word.start_seconds() - 0.0).abs() < 0.001);
        assert!((word.duration_seconds() - 0.0).abs() < 0.001);
        assert!((word.end_seconds() - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_detailed_format_multiple_nbest() {
        let json = r#"{
            "RecognitionStatus": "Success",
            "Offset": 0,
            "Duration": 0,
            "NBest": [
                {"Confidence": 0.95, "Lexical": "best", "ITN": "best", "MaskedITN": "best", "Display": "Best."},
                {"Confidence": 0.80, "Lexical": "rest", "ITN": "rest", "MaskedITN": "rest", "Display": "Rest."}
            ]
        }"#;

        let phrase: SpeechPhrase = serde_json::from_str(json).unwrap();

        // Should return first (best) result
        assert_eq!(phrase.transcript(), Some("Best."));
        assert!((phrase.confidence() - 0.95).abs() < 0.001);
    }

    // -------------------------------------------------------------------------
    // Additional Edge Case Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_speech_phrase_with_word_timing() {
        let json = r#"{
            "RecognitionStatus": "Success",
            "Offset": 5000000,
            "Duration": 28500000,
            "NBest": [{
                "Confidence": 0.92,
                "Lexical": "hello world",
                "ITN": "hello world",
                "MaskedITN": "hello world",
                "Display": "Hello world.",
                "Words": [
                    {"Word": "Hello", "Offset": 5000000, "Duration": 3000000, "Confidence": 0.95},
                    {"Word": "world", "Offset": 8500000, "Duration": 5000000, "Confidence": 0.90}
                ]
            }]
        }"#;

        let phrase: SpeechPhrase = serde_json::from_str(json).unwrap();
        let nbest = phrase.nbest.as_ref().unwrap();
        let words = nbest[0].words.as_ref().unwrap();

        assert_eq!(words.len(), 2);
        assert_eq!(words[0].word, "Hello");
        assert_eq!(words[1].word, "world");
        assert!((words[0].confidence.unwrap() - 0.95).abs() < 0.001);
        assert!((words[1].start_seconds() - 0.85).abs() < 0.001);
    }

    #[test]
    fn test_speech_phrase_initial_silence_timeout() {
        let json = r#"{
            "RecognitionStatus": "InitialSilenceTimeout",
            "Offset": 0,
            "Duration": 50000000
        }"#;

        let phrase: SpeechPhrase = serde_json::from_str(json).unwrap();
        assert_eq!(
            phrase.recognition_status,
            RecognitionStatus::InitialSilenceTimeout
        );
        assert!(phrase.recognition_status.is_no_speech());
        assert!(!phrase.recognition_status.is_success());
        assert!(phrase.to_stt_result().is_none());
    }

    #[test]
    fn test_speech_phrase_babble_timeout() {
        let json = r#"{
            "RecognitionStatus": "BabbleTimeout",
            "Offset": 0,
            "Duration": 30000000
        }"#;

        let phrase: SpeechPhrase = serde_json::from_str(json).unwrap();
        assert_eq!(phrase.recognition_status, RecognitionStatus::BabbleTimeout);
        assert!(phrase.recognition_status.is_error());
        assert!(phrase.to_stt_result().is_none());
    }

    #[test]
    fn test_speech_phrase_end_of_dictation() {
        let json = r#"{
            "RecognitionStatus": "EndOfDictation",
            "Offset": 0,
            "Duration": 0
        }"#;

        let phrase: SpeechPhrase = serde_json::from_str(json).unwrap();
        assert_eq!(phrase.recognition_status, RecognitionStatus::EndOfDictation);
        assert!(!phrase.recognition_status.is_success());
        assert!(!phrase.recognition_status.is_error());
        assert!(!phrase.recognition_status.is_no_speech());
    }

    #[test]
    fn test_speech_phrase_timing_conversion() {
        let phrase = SpeechPhrase {
            recognition_status: RecognitionStatus::Success,
            offset: 15_000_000,   // 1.5 seconds
            duration: 25_000_000, // 2.5 seconds
            display_text: Some("Test".to_string()),
            nbest: None,
        };

        assert!((phrase.start_seconds() - 1.5).abs() < 0.001);
        assert!((phrase.duration_seconds() - 2.5).abs() < 0.001);
    }

    #[test]
    fn test_speech_hypothesis_timing_conversion() {
        let hypothesis = SpeechHypothesis {
            text: "partial".to_string(),
            offset: 20_000_000,   // 2 seconds
            duration: 10_000_000, // 1 second
        };

        assert!((hypothesis.start_seconds() - 2.0).abs() < 0.001);
        assert!((hypothesis.duration_seconds() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_azure_message_parse_speech_start_detected() {
        let json = r#"{"Offset": 5000000}"#;
        let msg = AzureMessage::parse_with_path("speech.startDetected", json).unwrap();

        assert!(matches!(msg, AzureMessage::SpeechStartDetected(_)));
        if let AzureMessage::SpeechStartDetected(start) = msg {
            assert!((start.offset_seconds() - 0.5).abs() < 0.001);
        }
    }

    #[test]
    fn test_azure_message_parse_speech_end_detected() {
        let json = r#"{"Offset": 30000000}"#;
        let msg = AzureMessage::parse_with_path("speech.endDetected", json).unwrap();

        assert!(matches!(msg, AzureMessage::SpeechEndDetected(_)));
        if let AzureMessage::SpeechEndDetected(end) = msg {
            assert!((end.offset_seconds() - 3.0).abs() < 0.001);
        }
    }

    #[test]
    fn test_azure_message_infer_type_from_content() {
        // Message with just Offset should be inferred as SpeechStartDetected
        let json = r#"{"Offset": 1000000}"#;
        let msg = AzureMessage::parse(json).unwrap();
        assert!(matches!(msg, AzureMessage::SpeechStartDetected(_)));
    }

    #[test]
    fn test_text_preview_truncation() {
        // Test that long unknown messages are truncated in the Unknown variant
        let long_json = format!("{{\"Unknown\": \"{}\"}}", "x".repeat(200));
        let msg = AzureMessage::parse_with_path("unknown.path", &long_json).unwrap();

        if let AzureMessage::Unknown(preview) = msg {
            assert!(preview.len() <= 103); // 100 chars + "..."
            assert!(preview.ends_with("..."));
        } else {
            panic!("Expected Unknown message");
        }
    }

    #[test]
    fn test_nbest_all_text_forms() {
        let json = r#"{
            "Confidence": 0.88,
            "Lexical": "one two three",
            "ITN": "123",
            "MaskedITN": "1 2 3",
            "Display": "One, two, three."
        }"#;

        let nbest: NBestResult = serde_json::from_str(json).unwrap();

        assert_eq!(nbest.lexical, "one two three");
        assert_eq!(nbest.itn, "123");
        assert_eq!(nbest.masked_itn, "1 2 3");
        assert_eq!(nbest.display, "One, two, three.");
        assert!((nbest.confidence - 0.88).abs() < 0.001);
    }

    #[test]
    fn test_recognition_status_unknown_variant() {
        let status: RecognitionStatus = "FutureNewStatus".parse().unwrap();
        assert!(matches!(status, RecognitionStatus::Unknown(_)));
        assert!(!status.is_success());
        assert!(!status.is_error());
        assert!(!status.is_no_speech());
    }

    #[test]
    fn test_speech_phrase_no_text_available() {
        // SpeechPhrase with neither DisplayText nor NBest
        let phrase = SpeechPhrase {
            recognition_status: RecognitionStatus::Success,
            offset: 0,
            duration: 0,
            display_text: None,
            nbest: None,
        };

        assert!(phrase.transcript().is_none());
        // to_stt_result should return None because transcript() returns None
        assert!(phrase.to_stt_result().is_none());
    }

    #[test]
    fn test_azure_message_error_type() {
        let error = AzureMessageError::ParseError("test error".to_string());
        assert!(error.to_string().contains("test error"));

        let format_error = AzureMessageError::InvalidFormat("bad format".to_string());
        assert!(format_error.to_string().contains("bad format"));
    }
}
