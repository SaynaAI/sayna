//! WebSocket error types and handling
//!
//! This module defines custom error types for WebSocket operations,
//! providing better error context and type safety.

use thiserror::Error;

/// WebSocket handler error types
#[derive(Debug, Error)]
pub enum WebSocketError {
    /// Audio processing is disabled but audio operation was requested
    #[error("Audio processing is disabled. Send config message with audio=true first.")]
    AudioDisabled,

    /// Voice manager is not configured
    #[error("Voice manager not configured. Send config message with audio=true first.")]
    VoiceManagerNotConfigured,

    /// LiveKit client is not configured
    #[error("LiveKit client not configured. Send config message with livekit configuration first.")]
    LiveKitNotConfigured,

    /// STT configuration is missing
    #[error("STT configuration is required when audio=true")]
    STTConfigMissing,

    /// TTS configuration is missing
    #[error("TTS configuration is required when audio=true")]
    TTSConfigMissing,

    /// Failed to get API key
    #[error("Failed to get API key: {0}")]
    APIKeyError(String),

    /// Voice manager creation failed
    #[error("Failed to create voice manager: {0}")]
    VoiceManagerCreation(String),

    /// Failed to start voice manager
    #[error("Failed to start voice manager: {0}")]
    VoiceManagerStart(String),

    /// Failed to set up callback
    #[error("Failed to set up {callback_type} callback: {error}")]
    CallbackSetup {
        callback_type: String,
        error: String,
    },

    /// Timeout waiting for providers
    #[error("Timeout waiting for voice providers to be ready")]
    ProviderTimeout,

    /// Failed to connect to LiveKit
    #[error("Failed to connect to LiveKit room: {0}")]
    LiveKitConnection(String),

    /// Failed to process audio
    #[error("Failed to process audio: {0}")]
    AudioProcessing(String),

    /// Failed to synthesize speech
    #[error("Failed to synthesize speech: {0}")]
    SpeechSynthesis(String),

    /// Failed to clear TTS
    #[error("Failed to clear TTS provider: {0}")]
    TTSClear(String),

    /// Failed to send message via LiveKit
    #[error("Failed to send message via LiveKit: {0}")]
    LiveKitSend(String),

    /// Invalid message format
    #[error("Invalid message format: {0}")]
    InvalidMessage(String),

    /// WebSocket error
    #[error("WebSocket error: {0}")]
    WebSocket(String),
}

impl WebSocketError {
    /// Convert error to outgoing message format
    pub fn to_message(&self) -> String {
        self.to_string()
    }
}

/// Result type for WebSocket operations
pub type WebSocketResult<T> = Result<T, WebSocketError>;
