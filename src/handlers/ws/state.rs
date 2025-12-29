//! WebSocket connection state management
//!
//! This module handles the state management for WebSocket connections,
//! optimized for low latency with appropriate use of RwLock and atomic types.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::RwLock;

use crate::{
    auth::Auth,
    core::voice_manager::VoiceManager,
    livekit::{LiveKitClient, operations::OperationQueue},
};

/// WebSocket connection state optimized for low latency
///
/// Uses RwLock for state that changes rarely but is read frequently:
/// - AtomicBool for audio_enabled flag - no locks needed for hot path
/// - RwLock for managers - fast reads, rare writes (only during config)
/// - Optimized for the common case: many reads, few writes
pub struct ConnectionState {
    pub voice_manager: Option<Arc<VoiceManager>>,
    pub livekit_client: Option<Arc<RwLock<LiveKitClient>>>,
    /// Operation queue for non-blocking LiveKit operations
    pub livekit_operation_queue: Option<OperationQueue>,
    /// Whether audio processing (STT/TTS) is enabled for this connection
    pub audio_enabled: AtomicBool,
    /// Unique identifier for this WebSocket session
    pub stream_id: Option<String>,
    /// LiveKit room name for cleanup operations
    pub livekit_room_name: Option<String>,
    /// Local LiveKit participant identity (Sayna agent)
    pub livekit_local_identity: Option<String>,
    /// Recording egress ID for cleanup operations
    pub recording_egress_id: Option<String>,
    /// Auth context for this connection (used for room name normalization)
    pub auth: Auth,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionState {
    pub fn new() -> Self {
        Self {
            voice_manager: None,
            livekit_client: None,
            livekit_operation_queue: None,
            audio_enabled: AtomicBool::new(false),
            stream_id: None,
            livekit_room_name: None,
            livekit_local_identity: None,
            recording_egress_id: None,
            auth: Auth::empty(),
        }
    }

    /// Create a new ConnectionState with the given Auth context
    pub fn with_auth(auth: Auth) -> Self {
        Self {
            voice_manager: None,
            livekit_client: None,
            livekit_operation_queue: None,
            audio_enabled: AtomicBool::new(false),
            stream_id: None,
            livekit_room_name: None,
            livekit_local_identity: None,
            recording_egress_id: None,
            auth,
        }
    }

    /// Check if audio processing is enabled
    pub fn is_audio_enabled(&self) -> bool {
        self.audio_enabled.load(Ordering::Relaxed)
    }

    /// Set audio enabled state
    pub fn set_audio_enabled(&self, enabled: bool) {
        self.audio_enabled.store(enabled, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_state_new() {
        let state = ConnectionState::new();
        assert!(state.stream_id.is_none());
        assert!(state.voice_manager.is_none());
        assert!(state.livekit_client.is_none());
    }

    #[test]
    fn test_connection_state_stream_id_storage() {
        let mut state = ConnectionState::new();
        assert!(state.stream_id.is_none());

        state.stream_id = Some("test-stream-123".to_string());
        assert_eq!(state.stream_id, Some("test-stream-123".to_string()));
    }

    #[test]
    fn test_connection_state_stream_id_with_uuid() {
        let mut state = ConnectionState::new();

        let uuid = uuid::Uuid::new_v4().to_string();
        state.stream_id = Some(uuid.clone());

        assert_eq!(state.stream_id, Some(uuid));
        // Verify UUID format (36 chars: 8-4-4-4-12)
        assert_eq!(state.stream_id.as_ref().unwrap().len(), 36);
    }
}
