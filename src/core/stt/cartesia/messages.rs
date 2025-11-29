//! WebSocket message types for Cartesia STT WebSocket API.
//!
//! This module contains all message types for communication with the
//! Cartesia WebSocket API, including:
//!
//! - **Outgoing messages**: Messages sent from client to server
//!   - Audio data (sent as raw binary WebSocket frames, NOT base64 encoded)
//!   - [`CartesiaCommand::Finalize`]: Signal end of audio stream (flushes buffer)
//!   - [`CartesiaCommand::Done`]: Signal end of session (closes connection)
//!
//! - **Incoming messages**: Messages received from server
//!   - [`CartesiaMessage::Transcript`]: Transcription results (interim or final)
//!   - [`CartesiaMessage::FlushDone`]: Acknowledgment of finalize command
//!   - [`CartesiaMessage::Done`]: Acknowledgment of done command
//!   - [`CartesiaMessage::Error`]: Error responses
//!
//! # Audio Format
//!
//! Audio is sent as raw binary WebSocket frames:
//! - **Encoding**: PCM signed 16-bit little-endian (pcm_s16le)
//! - **NOT base64 encoded** - direct binary transmission
//! - **Chunk size**: Typically 100-200ms of audio per frame

use crate::core::stt::STTResult;
use serde::{Deserialize, Serialize};

// =============================================================================
// Outgoing Messages (Client to Server)
// =============================================================================

/// Commands that can be sent to Cartesia as text WebSocket messages.
///
/// Note: Audio data is sent as raw binary frames, not through this enum.
///
/// # Example
///
/// ```rust
/// use sayna::core::stt::cartesia::CartesiaCommand;
///
/// // Serialize to JSON string for sending
/// let finalize = CartesiaCommand::Finalize;
/// let json = serde_json::to_string(&finalize).unwrap();
/// assert_eq!(json, "\"finalize\"");
/// ```
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CartesiaCommand {
    /// Signal end of audio stream.
    ///
    /// The server will flush any buffered audio and respond with `flush_done`.
    Finalize,

    /// Signal end of session.
    ///
    /// The server will flush remaining audio, respond with `done`, and close the connection.
    Done,
}

// =============================================================================
// Incoming Messages (Server to Client)
// =============================================================================

/// Word-level timestamp information from Cartesia.
///
/// Provides timing details for each transcribed word.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct CartesiaWord {
    /// The transcribed word text
    pub word: String,
    /// Start time in seconds from beginning of audio
    pub start: f64,
    /// End time in seconds from beginning of audio
    pub end: f64,
}

/// Transcript data from Cartesia.
///
/// Contains the transcribed text along with metadata about finality
/// and optional word-level timestamps.
#[derive(Debug, Clone, Deserialize)]
pub struct CartesiaTranscript {
    /// Transcribed text
    pub text: String,
    /// Whether this is a final result (`true`) or interim (`false`)
    pub is_final: bool,
    /// Word-level timestamps (optional, may not be present)
    #[serde(default)]
    pub words: Option<Vec<CartesiaWord>>,
}

impl CartesiaTranscript {
    /// Convert this transcript to an STTResult.
    ///
    /// # Mapping
    ///
    /// | Cartesia Field | STTResult Field | Notes |
    /// |----------------|-----------------|-------|
    /// | `text` | `transcript` | Direct mapping |
    /// | `is_final` | `is_final` | Direct mapping |
    /// | `is_final` | `is_speech_final` | Set to `true` when `is_final=true` |
    /// | - | `confidence` | `1.0` for final, `0.0` for interim |
    ///
    /// Note: Cartesia does not provide confidence scores.
    pub fn to_stt_result(&self) -> STTResult {
        STTResult::new(
            self.text.clone(),
            self.is_final,
            self.is_final, // is_speech_final matches is_final
            if self.is_final { 1.0 } else { 0.0 },
        )
    }
}

impl From<&CartesiaTranscript> for STTResult {
    fn from(transcript: &CartesiaTranscript) -> Self {
        transcript.to_stt_result()
    }
}

impl From<CartesiaTranscript> for STTResult {
    fn from(transcript: CartesiaTranscript) -> Self {
        transcript.to_stt_result()
    }
}

/// Error response from Cartesia.
///
/// Contains error details when something goes wrong.
#[derive(Debug, Clone, Deserialize)]
pub struct CartesiaSTTError {
    /// Error message description
    pub message: String,
}

// =============================================================================
// Message Enum and Parsing
// =============================================================================

/// Enum for all possible WebSocket messages from Cartesia.
///
/// Use [`CartesiaMessage::parse()`] to deserialize incoming WebSocket messages.
///
/// # Message Types
///
/// | JSON `type` | Variant | Description |
/// |-------------|---------|-------------|
/// | `"transcript"` | `Transcript` | Transcription result (interim or final) |
/// | `"flush_done"` | `FlushDone` | Acknowledgment of finalize command |
/// | `"done"` | `Done` | Acknowledgment of done command |
/// | `"error"` | `Error` | Error from the server |
#[derive(Debug, Clone)]
pub enum CartesiaMessage {
    /// Transcription result (may be interim or final).
    ///
    /// Check `is_final` field to determine if this is a final result.
    Transcript(CartesiaTranscript),

    /// Acknowledgment that the finalize command was processed.
    ///
    /// Indicates all buffered audio has been transcribed.
    FlushDone,

    /// Acknowledgment that the done command was processed.
    ///
    /// Indicates the session is complete and the connection will close.
    Done,

    /// Error from the server.
    Error(CartesiaSTTError),

    /// Unknown message type (for forward compatibility).
    Unknown(String),
}

impl CartesiaMessage {
    /// Parse a WebSocket text message into the appropriate type.
    ///
    /// # Arguments
    /// * `text` - Raw JSON text from WebSocket message
    ///
    /// # Returns
    /// * `Result<Self, serde_json::Error>` - Parsed message or parse error
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::stt::cartesia::CartesiaMessage;
    ///
    /// let json = r#"{"type": "transcript", "text": "Hello", "is_final": true}"#;
    /// let msg = CartesiaMessage::parse(json).unwrap();
    /// assert!(msg.is_final_transcript());
    /// ```
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        // First, peek at the type field
        #[derive(Deserialize)]
        struct MessageTypePeek {
            #[serde(rename = "type")]
            message_type: String,
        }

        let peek: MessageTypePeek = serde_json::from_str(text)?;

        match peek.message_type.as_str() {
            "transcript" => {
                // Deserialize the full transcript message
                #[derive(Deserialize)]
                struct TranscriptMessage {
                    text: String,
                    is_final: bool,
                    #[serde(default)]
                    words: Option<Vec<CartesiaWord>>,
                }

                let msg: TranscriptMessage = serde_json::from_str(text)?;
                Ok(CartesiaMessage::Transcript(CartesiaTranscript {
                    text: msg.text,
                    is_final: msg.is_final,
                    words: msg.words,
                }))
            }
            "flush_done" => Ok(CartesiaMessage::FlushDone),
            "done" => Ok(CartesiaMessage::Done),
            "error" => {
                #[derive(Deserialize)]
                struct ErrorMessage {
                    message: String,
                }

                let msg: ErrorMessage = serde_json::from_str(text)?;
                Ok(CartesiaMessage::Error(CartesiaSTTError {
                    message: msg.message,
                }))
            }
            _ => Ok(CartesiaMessage::Unknown(text.to_string())),
        }
    }

    /// Check if this message is a transcript (interim or final).
    #[inline]
    pub fn is_transcript(&self) -> bool {
        matches!(self, CartesiaMessage::Transcript(_))
    }

    /// Check if this message is a final transcript.
    #[inline]
    pub fn is_final_transcript(&self) -> bool {
        matches!(
            self,
            CartesiaMessage::Transcript(t) if t.is_final
        )
    }

    /// Check if this message represents an error.
    #[inline]
    pub fn is_error(&self) -> bool {
        matches!(self, CartesiaMessage::Error(_))
    }

    /// Convert this message to an STTResult if it's a transcript.
    ///
    /// Returns `Some(STTResult)` for transcript messages, `None` otherwise.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::stt::cartesia::CartesiaMessage;
    ///
    /// let json = r#"{"type": "transcript", "text": "Hello world", "is_final": true}"#;
    /// let msg = CartesiaMessage::parse(json).unwrap();
    ///
    /// if let Some(result) = msg.to_stt_result() {
    ///     assert_eq!(result.transcript, "Hello world");
    ///     assert!(result.is_final);
    /// }
    /// ```
    pub fn to_stt_result(&self) -> Option<STTResult> {
        match self {
            CartesiaMessage::Transcript(t) => Some(t.to_stt_result()),
            _ => None,
        }
    }

    /// Get the transcript data if this is a transcript message.
    #[inline]
    pub fn as_transcript(&self) -> Option<&CartesiaTranscript> {
        match self {
            CartesiaMessage::Transcript(t) => Some(t),
            _ => None,
        }
    }

    /// Get the error if this is an error message.
    #[inline]
    pub fn as_error(&self) -> Option<&CartesiaSTTError> {
        match self {
            CartesiaMessage::Error(e) => Some(e),
            _ => None,
        }
    }
}

// =============================================================================
// Legacy Re-exports (for backwards compatibility with mod.rs exports)
// =============================================================================

/// Ready message from Cartesia.
///
/// Note: The Cartesia STT WebSocket API does not actually send a "ready" message.
/// This is kept for API compatibility but may be removed in future versions.
#[derive(Debug, Clone, Deserialize)]
pub struct ReadyMessage {
    /// Message type identifier
    #[serde(rename = "type")]
    pub message_type: String,
}

/// Partial transcript type alias for backwards compatibility.
pub type PartialTranscript = CartesiaTranscript;

/// Final transcript type alias for backwards compatibility.
pub type FinalTranscript = CartesiaTranscript;

/// Done message for backwards compatibility with outgoing message format.
#[derive(Debug, Clone, Serialize)]
pub struct DoneMessage {
    /// Message type identifier (always "done")
    #[serde(rename = "type")]
    pub message_type: &'static str,
}

impl Default for DoneMessage {
    fn default() -> Self {
        Self {
            message_type: "done",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // CartesiaCommand Tests
    // =========================================================================

    #[test]
    fn test_command_finalize_serialization() {
        let cmd = CartesiaCommand::Finalize;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, "\"finalize\"");
    }

    #[test]
    fn test_command_done_serialization() {
        let cmd = CartesiaCommand::Done;
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(json, "\"done\"");
    }

    // =========================================================================
    // CartesiaWord Tests
    // =========================================================================

    #[test]
    fn test_word_deserialization() {
        let json = r#"{"word": "Hello", "start": 0.0, "end": 0.5}"#;
        let word: CartesiaWord = serde_json::from_str(json).unwrap();

        assert_eq!(word.word, "Hello");
        assert_eq!(word.start, 0.0);
        assert_eq!(word.end, 0.5);
    }

    #[test]
    fn test_word_array_deserialization() {
        let json = r#"[
            {"word": "Hello", "start": 0.0, "end": 0.5},
            {"word": "world", "start": 0.6, "end": 1.2}
        ]"#;
        let words: Vec<CartesiaWord> = serde_json::from_str(json).unwrap();

        assert_eq!(words.len(), 2);
        assert_eq!(words[0].word, "Hello");
        assert_eq!(words[1].word, "world");
        assert_eq!(words[1].start, 0.6);
    }

    // =========================================================================
    // CartesiaTranscript Tests
    // =========================================================================

    #[test]
    fn test_transcript_without_words() {
        let json = r#"{"text": "Hello world", "is_final": true}"#;
        let transcript: CartesiaTranscript = serde_json::from_str(json).unwrap();

        assert_eq!(transcript.text, "Hello world");
        assert!(transcript.is_final);
        assert!(transcript.words.is_none());
    }

    #[test]
    fn test_transcript_with_words() {
        let json = r#"{
            "text": "Hello world",
            "is_final": true,
            "words": [
                {"word": "Hello", "start": 0.0, "end": 0.5},
                {"word": "world", "start": 0.6, "end": 1.2}
            ]
        }"#;
        let transcript: CartesiaTranscript = serde_json::from_str(json).unwrap();

        assert_eq!(transcript.text, "Hello world");
        assert!(transcript.is_final);
        assert!(transcript.words.is_some());

        let words = transcript.words.unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].word, "Hello");
        assert_eq!(words[1].end, 1.2);
    }

    #[test]
    fn test_transcript_interim_result() {
        let json = r#"{"text": "Hel", "is_final": false}"#;
        let transcript: CartesiaTranscript = serde_json::from_str(json).unwrap();

        assert_eq!(transcript.text, "Hel");
        assert!(!transcript.is_final);
    }

    #[test]
    fn test_transcript_to_stt_result_final() {
        let transcript = CartesiaTranscript {
            text: "Hello world".to_string(),
            is_final: true,
            words: None,
        };

        let result = transcript.to_stt_result();

        assert_eq!(result.transcript, "Hello world");
        assert!(result.is_final);
        assert!(result.is_speech_final);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn test_transcript_to_stt_result_interim() {
        let transcript = CartesiaTranscript {
            text: "Hel".to_string(),
            is_final: false,
            words: None,
        };

        let result = transcript.to_stt_result();

        assert_eq!(result.transcript, "Hel");
        assert!(!result.is_final);
        assert!(!result.is_speech_final);
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn test_transcript_from_trait() {
        let transcript = CartesiaTranscript {
            text: "Test".to_string(),
            is_final: true,
            words: None,
        };

        let result: STTResult = (&transcript).into();
        assert_eq!(result.transcript, "Test");
        assert!(result.is_final);

        let result: STTResult = transcript.into();
        assert_eq!(result.transcript, "Test");
    }

    // =========================================================================
    // CartesiaMessage Parse Tests
    // =========================================================================

    #[test]
    fn test_parse_transcript_message() {
        let json = r#"{"type": "transcript", "text": "Hello world", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        assert!(msg.is_transcript());
        assert!(msg.is_final_transcript());

        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.text, "Hello world");
        assert!(transcript.is_final);
    }

    #[test]
    fn test_parse_transcript_with_words() {
        let json = r#"{
            "type": "transcript",
            "text": "Hello world",
            "is_final": true,
            "words": [
                {"word": "Hello", "start": 0.0, "end": 0.5},
                {"word": "world", "start": 0.6, "end": 1.2}
            ]
        }"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert!(transcript.words.is_some());
        assert_eq!(transcript.words.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_parse_interim_transcript() {
        let json = r#"{"type": "transcript", "text": "Hel", "is_final": false}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        assert!(msg.is_transcript());
        assert!(!msg.is_final_transcript());

        let transcript = msg.as_transcript().unwrap();
        assert!(!transcript.is_final);
    }

    #[test]
    fn test_parse_flush_done_message() {
        let json = r#"{"type": "flush_done"}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        assert!(matches!(msg, CartesiaMessage::FlushDone));
        assert!(!msg.is_transcript());
        assert!(!msg.is_error());
    }

    #[test]
    fn test_parse_done_message() {
        let json = r#"{"type": "done"}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        assert!(matches!(msg, CartesiaMessage::Done));
    }

    #[test]
    fn test_parse_error_message() {
        let json = r#"{"type": "error", "message": "Invalid audio format"}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        assert!(msg.is_error());
        assert!(!msg.is_transcript());

        let error = msg.as_error().unwrap();
        assert_eq!(error.message, "Invalid audio format");
    }

    #[test]
    fn test_parse_unknown_message() {
        let json = r#"{"type": "unknown_type", "data": "something"}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        assert!(matches!(msg, CartesiaMessage::Unknown(_)));
    }

    // =========================================================================
    // CartesiaMessage Helper Method Tests
    // =========================================================================

    #[test]
    fn test_to_stt_result_transcript() {
        let json = r#"{"type": "transcript", "text": "Hello", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let result = msg.to_stt_result();
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.transcript, "Hello");
        assert!(result.is_final);
    }

    #[test]
    fn test_to_stt_result_non_transcript() {
        let json = r#"{"type": "flush_done"}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        assert!(msg.to_stt_result().is_none());
    }

    // =========================================================================
    // Legacy Type Tests
    // =========================================================================

    #[test]
    fn test_done_message_default() {
        let msg = DoneMessage::default();
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"done"}"#);
    }

    // =========================================================================
    // Edge Case Tests
    // =========================================================================

    #[test]
    fn test_empty_transcript_text() {
        let json = r#"{"type": "transcript", "text": "", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.text, "");
        assert!(transcript.is_final);
    }

    #[test]
    fn test_transcript_with_empty_words_array() {
        let json = r#"{"type": "transcript", "text": "Hello", "is_final": true, "words": []}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert!(transcript.words.is_some());
        assert!(transcript.words.as_ref().unwrap().is_empty());
    }

    #[test]
    fn test_transcript_with_unicode() {
        let json = r#"{"type": "transcript", "text": "こんにちは世界", "is_final": true}"#;
        let msg = CartesiaMessage::parse(json).unwrap();

        let transcript = msg.as_transcript().unwrap();
        assert_eq!(transcript.text, "こんにちは世界");
    }

    #[test]
    fn test_word_with_fractional_timestamps() {
        let json = r#"{"word": "test", "start": 1.234567, "end": 2.345678}"#;
        let word: CartesiaWord = serde_json::from_str(json).unwrap();

        assert!((word.start - 1.234567).abs() < 0.0001);
        assert!((word.end - 2.345678).abs() < 0.0001);
    }
}
