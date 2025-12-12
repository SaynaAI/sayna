use serde::{Deserialize, Serialize};

/// LiveKit configuration for connecting to a room
#[derive(Debug, Clone)]
pub struct LiveKitConfig {
    pub url: String,
    pub token: String,
    /// Room name (extracted from token or provided separately)
    pub room_name: String,
    /// Sample rate for audio publishing (from TTS config)
    pub sample_rate: u32,
    /// Number of audio channels for publishing (typically 1 for mono)
    pub channels: u16,
    /// Enable noise filtering on incoming audio (default: enabled when the `noise-filter`
    /// feature is compiled in). Set to `false` to reduce latency when filtering is
    /// available.
    pub enable_noise_filter: bool,
    /// List of participant identities to listen to for audio tracks and data messages.
    ///
    /// If empty, all participants' audio and data will be processed.
    /// If populated, only participants in this list will be processed.
    pub listen_participants: Vec<String>,
}

/// LiveKit connection status
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Failed,
}

/// Audio frame information received from LiveKit
#[derive(Debug, Clone)]
pub struct AudioFrameInfo {
    pub data: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u8,
    pub participant_identity: String,
    pub timestamp: u64,
}

/// LiveKit room information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInfo {
    pub name: String,
    pub participants_count: usize,
    pub audio_tracks_count: usize,
    pub video_tracks_count: usize,
}

/// Participant information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantInfo {
    pub identity: String,
    pub name: Option<String>,
    pub is_local: bool,
    pub audio_tracks: Vec<String>,
    pub video_tracks: Vec<String>,
}

/// LiveKit error types specific to this implementation
#[derive(Debug, thiserror::Error)]
pub enum LiveKitError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Not connected to room")]
    NotConnected,

    #[error("Audio callback not set")]
    AudioCallbackNotSet,

    #[error("Room event error: {0}")]
    RoomEventError(String),

    #[error("Audio stream error: {0}")]
    AudioStreamError(String),

    #[error("SIP transfer request timeout (transfer likely succeeded)")]
    SIPTransferRequestTimeout,
}
