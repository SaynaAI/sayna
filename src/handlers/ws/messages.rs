//! WebSocket message types and routing
//!
//! This module defines all message types for WebSocket communication,
//! including incoming and outgoing messages, unified message structures,
//! and message routing for optimized throughput.

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use super::config::{
    LiveKitWebSocketConfig, STTWebSocketConfig, TTSWebSocketConfig, default_allow_interruption,
    default_audio_enabled,
};

/// WebSocket message types for incoming messages
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
#[allow(clippy::large_enum_variant)]
pub enum IncomingMessage {
    #[serde(rename = "config")]
    Config {
        /// Enable audio processing (STT/TTS). Defaults to true if not specified.
        #[serde(
            default = "default_audio_enabled",
            skip_serializing_if = "Option::is_none"
        )]
        audio: Option<bool>,
        /// STT configuration (required only when audio=true)
        #[serde(skip_serializing_if = "Option::is_none")]
        stt_config: Option<STTWebSocketConfig>,
        /// TTS configuration (required only when audio=true)
        #[serde(skip_serializing_if = "Option::is_none")]
        tts_config: Option<TTSWebSocketConfig>,
        /// Optional LiveKit configuration for real-time audio streaming
        #[serde(skip_serializing_if = "Option::is_none")]
        livekit: Option<LiveKitWebSocketConfig>,
    },
    #[serde(rename = "speak")]
    Speak {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        flush: Option<bool>,
        #[serde(
            default = "default_allow_interruption",
            skip_serializing_if = "Option::is_none"
        )]
        allow_interruption: Option<bool>,
    },
    #[serde(rename = "clear")]
    Clear,
    #[serde(rename = "send_message")]
    SendMessage {
        message: String,
        role: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        topic: Option<String>,
    },
}

/// Unified message structure for all incoming messages from various sources
#[derive(Debug, Serialize, Clone)]
pub struct UnifiedMessage {
    /// Text message content (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Binary data encoded as base64 (optional)  
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// Participant/sender identity
    pub identity: String,
    /// Topic/channel for the message
    pub topic: String,
    /// Room/space identifier
    pub room: String,
    /// Timestamp when the message was received
    pub timestamp: u64,
}

/// Participant disconnection information
#[derive(Debug, Serialize, Clone)]
pub struct ParticipantDisconnectedInfo {
    /// Participant's unique identity
    pub identity: String,
    /// Participant's display name (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Room identifier
    pub room: String,
    /// Timestamp when the disconnection occurred
    pub timestamp: u64,
}

/// WebSocket message types for outgoing messages
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum OutgoingMessage {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "stt_result")]
    STTResult {
        transcript: String,
        is_final: bool,
        is_speech_final: bool,
        confidence: f32,
    },
    #[serde(rename = "message")]
    Message {
        /// Unified message structure containing text/data from various sources
        message: UnifiedMessage,
    },
    #[serde(rename = "participant_disconnected")]
    ParticipantDisconnected {
        /// Information about the participant who disconnected
        participant: ParticipantDisconnectedInfo,
    },
    #[serde(rename = "error")]
    Error { message: String },
}

/// Message routing for optimized throughput
/// Using enum for zero-cost abstraction
pub enum MessageRoute {
    Outgoing(OutgoingMessage),
    Binary(Bytes),
}
