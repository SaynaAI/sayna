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

    /// SIP transfer failed due to invalid phone number format.
    /// Raised when the provided phone number doesn't match expected E.164 format.
    /// Client should ensure the phone number starts with + followed by country code and number.
    #[error("Invalid phone number: {0}")]
    SIPTransferInvalidPhoneNumber(String),

    /// SIP transfer API call failed.
    /// Raised when the LiveKit SIP transfer request returns an error.
    /// Client may retry the transfer or check the SIP configuration.
    #[error("SIP transfer failed: {0}")]
    SIPTransferFailed(String),

    /// No SIP participant available to transfer.
    /// Raised when attempting a SIP transfer but no SIP participant exists in the room.
    /// Client should ensure a SIP call is active before attempting transfer.
    #[error("No SIP participant found in room to transfer")]
    SIPTransferNoParticipant,

    /// Room name not available for SIP transfer.
    /// Raised when the connection state lacks a room name required for SIP operations.
    /// Client should ensure LiveKit is configured before attempting SIP transfer.
    #[error("Room name not available. Ensure LiveKit is configured.")]
    SIPTransferNoRoomName,

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sip_transfer_invalid_phone_error() {
        let error = WebSocketError::SIPTransferInvalidPhoneNumber("contains letters".to_string());
        let msg = error.to_message();
        assert!(msg.contains("Invalid phone number"));
        assert!(msg.contains("contains letters"));
    }

    #[test]
    fn test_sip_transfer_failed_error() {
        let error = WebSocketError::SIPTransferFailed("connection timeout".to_string());
        let msg = error.to_message();
        assert!(msg.contains("SIP transfer failed"));
        assert!(msg.contains("connection timeout"));
    }

    #[test]
    fn test_sip_transfer_no_participant_error() {
        let error = WebSocketError::SIPTransferNoParticipant;
        let msg = error.to_message();
        assert!(msg.contains("No SIP participant"));
    }

    #[test]
    fn test_sip_transfer_no_room_error() {
        let error = WebSocketError::SIPTransferNoRoomName;
        let msg = error.to_message();
        assert!(msg.contains("Room name not available"));
    }

    #[test]
    fn test_audio_disabled_error() {
        let error = WebSocketError::AudioDisabled;
        let msg = error.to_message();
        assert!(msg.contains("Audio processing is disabled"));
    }

    #[test]
    fn test_voice_manager_not_configured_error() {
        let error = WebSocketError::VoiceManagerNotConfigured;
        let msg = error.to_message();
        assert!(msg.contains("Voice manager not configured"));
    }

    #[test]
    fn test_livekit_not_configured_error() {
        let error = WebSocketError::LiveKitNotConfigured;
        let msg = error.to_message();
        assert!(msg.contains("LiveKit client not configured"));
    }

    #[test]
    fn test_error_display_trait() {
        // Verify that Display implementation matches to_message
        let error = WebSocketError::SIPTransferFailed("test error".to_string());
        assert_eq!(error.to_string(), error.to_message());
    }

    #[test]
    fn test_api_key_error() {
        let error = WebSocketError::APIKeyError("missing key".to_string());
        let msg = error.to_message();
        assert!(msg.contains("Failed to get API key"));
        assert!(msg.contains("missing key"));
    }

    #[test]
    fn test_invalid_message_error() {
        let error = WebSocketError::InvalidMessage("not JSON".to_string());
        let msg = error.to_message();
        assert!(msg.contains("Invalid message format"));
        assert!(msg.contains("not JSON"));
    }
}
