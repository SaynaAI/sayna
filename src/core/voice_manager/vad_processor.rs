//! VAD-based audio processing for silence detection.
//!
//! This module provides audio chunk processing through VAD for silence detection.
//! It is feature-gated under `stt-vad`. VAD event handling logic has been moved
//! to `STTResultProcessor` for better cohesion.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tracing::warn;

use crate::core::vad::{SilenceTracker, SileroVAD, VADEvent};

use super::errors::{VoiceManagerError, VoiceManagerResult};
use super::utils::MAX_VAD_AUDIO_SAMPLES;

/// Encapsulates VAD processing buffers and state per VoiceManager instance.
///
/// This struct groups together:
/// - `audio_buffer`: Accumulates ALL audio samples during an utterance for turn detection
/// - `frame_buffer`: Temporary buffer for VAD frame processing
/// - `buffer_limit_warned`: Per-instance flag for logging buffer limit warning once
pub struct VADState {
    /// Buffer for accumulating audio samples for turn detection.
    ///
    /// Per smart-turn best practices: "run Smart Turn on the entire recording of the user's turn"
    /// This buffer accumulates ALL samples during an utterance so that when silence is detected,
    /// the turn detection model can analyze the complete context (up to 8 seconds).
    pub audio_buffer: SyncRwLock<Vec<i16>>,

    /// Temporary buffer for VAD frame processing.
    ///
    /// This buffer accumulates partial frames between `receive_audio` calls. VAD requires
    /// complete frames (e.g., 512 samples at 16kHz = 32ms) for inference. Since audio
    /// may arrive in arbitrary chunk sizes, we accumulate samples here and drain
    /// complete frames for VAD processing.
    pub frame_buffer: SyncRwLock<Vec<i16>>,

    /// Per-instance flag to log VAD buffer limit warning only once.
    buffer_limit_warned: AtomicBool,
}

impl VADState {
    /// Create a new VADState with pre-allocated buffers.
    ///
    /// - `audio_buffer_capacity`: Capacity for the audio buffer (default: 8 seconds at 16kHz)
    /// - `frame_buffer_capacity`: Capacity for the frame buffer (default: 512 samples)
    pub fn new(audio_buffer_capacity: usize, frame_buffer_capacity: usize) -> Self {
        Self {
            audio_buffer: SyncRwLock::new(Vec::with_capacity(audio_buffer_capacity)),
            frame_buffer: SyncRwLock::new(Vec::with_capacity(frame_buffer_capacity)),
            buffer_limit_warned: AtomicBool::new(false),
        }
    }

    /// Clear both buffers and reset the warning flag.
    pub fn reset(&self) {
        {
            let mut buffer = self.audio_buffer.write();
            buffer.clear();
        }
        {
            let mut buffer = self.frame_buffer.write();
            buffer.clear();
        }
        self.buffer_limit_warned.store(false, Ordering::Relaxed);
    }

    /// Clear just the audio buffer (used when new speech starts).
    pub fn clear_audio_buffer(&self) {
        let mut buffer = self.audio_buffer.write();
        buffer.clear();
    }
}

/// Process an audio chunk through VAD for silence detection.
///
/// This function converts raw PCM bytes to i16 samples and:
/// 1. Accumulates ALL samples in `vad_state.audio_buffer` for turn detection
/// 2. Processes complete frames through VAD using `vad_state.frame_buffer`
///
/// Returns a vector of VAD events generated from processing the audio.
pub async fn process_audio_chunk(
    audio: &[u8],
    vad: &Arc<RwLock<SileroVAD>>,
    vad_state: &Arc<VADState>,
    silence_tracker: &Arc<SilenceTracker>,
) -> VoiceManagerResult<Vec<VADEvent>> {
    // Convert bytes to i16 samples (assuming 16-bit PCM little-endian)
    let samples: Vec<i16> = audio
        .chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();

    // Get VAD frame size
    let frame_size = {
        let vad_guard = vad.read().await;
        vad_guard.frame_size()
    };

    // Accumulate samples in audio_buffer for turn detection (never drained here)
    {
        let mut audio_buffer = vad_state.audio_buffer.write();
        if audio_buffer.len() < MAX_VAD_AUDIO_SAMPLES {
            let remaining_capacity = MAX_VAD_AUDIO_SAMPLES - audio_buffer.len();
            let samples_to_add = samples.len().min(remaining_capacity);
            audio_buffer.extend_from_slice(&samples[..samples_to_add]);

            if samples_to_add < samples.len()
                && !vad_state.buffer_limit_warned.swap(true, Ordering::Relaxed)
            {
                warn!(
                    "VAD audio buffer reached limit of {} samples (30 seconds). \
                     New audio will not be buffered for turn detection until buffer is cleared.",
                    MAX_VAD_AUDIO_SAMPLES
                );
            }
        }
    }

    // Extract complete frames from frame_buffer for VAD processing
    let frames_to_process: Vec<Vec<i16>> = {
        let mut frame_buffer = vad_state.frame_buffer.write();
        frame_buffer.extend_from_slice(&samples);

        let mut frames = Vec::new();
        while frame_buffer.len() >= frame_size {
            let frame: Vec<i16> = frame_buffer.drain(..frame_size).collect();
            frames.push(frame);
        }
        frames
    }; // Frame buffer lock released here

    // Collect events from processing frames
    let mut events = Vec::new();

    // Process frames through VAD (without holding buffer lock)
    for frame in frames_to_process {
        let speech_prob = {
            let vad_guard = vad.write().await;
            vad_guard.process_audio(&frame).await.map_err(|e| {
                VoiceManagerError::InitializationError(format!("VAD processing error: {}", e))
            })?
        };

        // Feed to silence tracker and collect events
        if let Some(event) = silence_tracker.process(speech_prob) {
            events.push(event);
        }
    }

    Ok(events)
}
