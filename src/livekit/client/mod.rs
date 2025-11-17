//! LiveKit client implementation for real-time audio streaming and publishing.
//!
//! This module exposes the `LiveKitClient` type, which orchestrates room
//! connectivity, audio publishing, data messaging, and event handling. The
//! implementation is split across focused submodules to keep the codebase
//! maintainable while preserving the original behaviour.

mod audio;
mod callbacks;
mod connection;
mod events;
mod messaging;
mod operation_worker;

#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use livekit::prelude::{LocalTrackPublication, Room, RoomEvent};
use livekit::track::LocalAudioTrack;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use tokio::sync::{Mutex, mpsc};
use tracing::warn;

pub(crate) use super::operations::{LiveKitOperation, OperationQueue, QueueStats};
pub(crate) use super::types::LiveKitConfig;

/// Callback type for handling incoming audio chunks.
pub type AudioCallback = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

/// Callback type for handling incoming data messages.
pub type DataCallback = Arc<dyn Fn(DataMessage) + Send + Sync>;

/// Callback type for handling participant disconnection events.
pub type ParticipantDisconnectCallback = Arc<dyn Fn(ParticipantDisconnectEvent) + Send + Sync>;

/// LiveKit data message structure.
#[derive(Debug, Clone)]
pub struct DataMessage {
    /// The participant identity who sent the data.
    pub participant_identity: String,
    /// The raw data payload.
    pub data: Vec<u8>,
    /// Optional topic/channel for the data.
    pub topic: Option<String>,
    /// Timestamp when the data was received.
    pub timestamp: u64,
}

/// LiveKit participant disconnect event structure.
#[derive(Debug, Clone)]
pub struct ParticipantDisconnectEvent {
    /// The participant identity who disconnected.
    pub participant_identity: String,
    /// The participant's display name (if available).
    pub participant_name: Option<String>,
    /// Room identifier.
    pub room_name: String,
    /// Timestamp when the disconnection occurred.
    pub timestamp: u64,
}

/// Reliable data channel threshold (200 MB) to handle bursty payloads without premature backpressure.
pub(crate) const RELIABLE_BUFFER_THRESHOLD_BYTES: u64 = 200 * 1024 * 1024;

/// Maximum audio queue size based on duration (2 seconds worth of frames at 100 frames/sec)
pub(crate) const MAX_AUDIO_QUEUE_SIZE: usize = 200;

/// LiveKit client for handling audio streaming and publishing.
///
/// Optimized for low latency with queue-based operations to eliminate lock contention.
pub struct LiveKitClient {
    pub(crate) config: LiveKitConfig,
    pub(crate) room: Arc<Mutex<Option<Room>>>,
    pub(crate) room_events: Option<mpsc::UnboundedReceiver<RoomEvent>>,
    pub(crate) audio_callback: Option<AudioCallback>,
    pub(crate) data_callback: Option<DataCallback>,
    pub(crate) participant_disconnect_callback: Option<ParticipantDisconnectCallback>,
    pub(crate) active_streams: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    pub(crate) is_connected: Arc<Mutex<bool>>,
    // Atomic flags for fast status queries
    pub(crate) is_connected_atomic: Arc<AtomicBool>,
    pub(crate) has_audio_source_atomic: Arc<AtomicBool>,
    pub(crate) audio_source: Arc<Mutex<Option<Arc<NativeAudioSource>>>>,
    pub(crate) local_audio_track: Option<LocalAudioTrack>,
    pub(crate) local_track_publication: Arc<Mutex<Option<LocalTrackPublication>>>,
    pub(crate) _audio_buffer_pool: Arc<Mutex<Vec<Vec<u8>>>>,
    pub(crate) audio_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    pub(crate) operation_queue: Option<OperationQueue>,
    pub(crate) operation_worker_handle: Option<tokio::task::JoinHandle<()>>,
    pub(crate) stats: Arc<Mutex<QueueStats>>,
}

impl LiveKitClient {
    /// Get a buffer from the pool or create a new one if pool is empty
    pub(crate) fn get_buffer_from_pool(
        pool: &Arc<Mutex<Vec<Vec<u8>>>>,
        capacity: usize,
    ) -> Vec<u8> {
        if let Ok(mut pool_guard) = pool.try_lock()
            && let Some(mut buffer) = pool_guard.pop()
        {
            buffer.clear();
            buffer.reserve(capacity.saturating_sub(buffer.capacity()));
            return buffer;
        }
        Vec::with_capacity(capacity)
    }

    /// Return a buffer to the pool for reuse
    #[cfg(feature = "noise-filter")]
    pub(crate) fn return_buffer_to_pool(pool: &Arc<Mutex<Vec<Vec<u8>>>>, mut buffer: Vec<u8>) {
        const MAX_POOL_SIZE: usize = 8;
        const MAX_BUFFER_SIZE: usize = 48000; // ~500ms at 48kHz stereo

        if buffer.capacity() <= MAX_BUFFER_SIZE {
            buffer.clear();
            if let Ok(mut pool_guard) = pool.try_lock()
                && pool_guard.len() < MAX_POOL_SIZE
            {
                pool_guard.push(buffer);
            }
        }
    }

    /// Create a new LiveKit client with the given configuration.
    pub fn new(config: LiveKitConfig) -> Self {
        let buffer_capacity =
            ((config.sample_rate as usize * config.channels as usize * 2) / 100).max(4096);

        Self {
            config,
            room: Arc::new(Mutex::new(None)),
            room_events: None,
            audio_callback: None,
            data_callback: None,
            participant_disconnect_callback: None,
            active_streams: Arc::new(Mutex::new(Vec::new())),
            is_connected: Arc::new(Mutex::new(false)),
            is_connected_atomic: Arc::new(AtomicBool::new(false)),
            has_audio_source_atomic: Arc::new(AtomicBool::new(false)),
            audio_source: Arc::new(Mutex::new(None)),
            local_audio_track: None,
            local_track_publication: Arc::new(Mutex::new(None)),
            _audio_buffer_pool: Arc::new(Mutex::new(
                (0..4)
                    .map(|_| Vec::with_capacity(buffer_capacity))
                    .collect(),
            )),
            audio_queue: Arc::new(Mutex::new(VecDeque::new())),
            operation_queue: None,
            operation_worker_handle: None,
            stats: Arc::new(Mutex::new(QueueStats::default())),
        }
    }

    /// Get a reference to the operation queue for non-blocking operations.
    pub fn get_operation_queue(&self) -> Option<OperationQueue> {
        self.operation_queue.clone()
    }

    // Test-only accessors to avoid direct field access in tests
    #[cfg(test)]
    pub(crate) async fn has_local_track_publication(&self) -> bool {
        self.local_track_publication.lock().await.is_some()
    }

    #[cfg(test)]
    pub(crate) async fn get_audio_queue_len(&self) -> usize {
        self.audio_queue.lock().await.len()
    }

    #[cfg(test)]
    pub(crate) async fn set_connected(&self, connected: bool) {
        use std::sync::atomic::Ordering;
        *self.is_connected.lock().await = connected;
        self.is_connected_atomic.store(connected, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn has_local_audio_track(&self) -> bool {
        self.local_audio_track.is_some()
    }

    #[cfg(test)]
    pub(crate) async fn has_room(&self) -> bool {
        self.room.lock().await.is_some()
    }

    #[cfg(test)]
    pub(crate) fn has_room_events(&self) -> bool {
        self.room_events.is_some()
    }
}

impl Drop for LiveKitClient {
    fn drop(&mut self) {
        if self.operation_worker_handle.is_some() {
            warn!("LiveKitClient dropped without explicit disconnect call");
        }
    }
}
