//! WebSocket connection state management
//!
//! This module handles the state management for WebSocket connections,
//! optimized for low latency with appropriate use of RwLock and atomic types.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::RwLock;

use crate::{core::voice_manager::VoiceManager, livekit::LiveKitClient};

/// WebSocket connection state optimized for low latency
///
/// Uses RwLock for state that changes rarely but is read frequently:
/// - AtomicBool for audio_enabled flag - no locks needed for hot path
/// - RwLock for managers - fast reads, rare writes (only during config)
/// - Optimized for the common case: many reads, few writes
pub struct ConnectionState {
    pub voice_manager: Option<Arc<VoiceManager>>,
    pub livekit_client: Option<Arc<RwLock<LiveKitClient>>>,
    /// Whether audio processing (STT/TTS) is enabled for this connection
    pub audio_enabled: AtomicBool,
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
            audio_enabled: AtomicBool::new(false),
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
