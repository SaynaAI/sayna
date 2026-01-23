//! VAD-based audio processing for silence detection.
//!
//! This module provides audio chunk processing through VAD for silence detection.
//! It is feature-gated under `stt-vad`. VAD event handling logic has been moved
//! to `STTResultProcessor` for better cohesion.

use parking_lot::RwLock as SyncRwLock;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::debug;

use crate::core::vad::{SilenceTracker, SileroVAD, VADEvent};

use super::errors::{VoiceManagerError, VoiceManagerResult};
use super::utils::MAX_VAD_AUDIO_SAMPLES;

/// Encapsulates VAD processing buffers and state per VoiceManager instance.
///
/// This struct groups together:
/// - `audio_buffer`: Ring buffer accumulating recent audio samples for turn detection
/// - `frame_buffer`: Temporary buffer for VAD frame processing
/// - `buffer_limit_warned`: Per-instance flag for logging buffer limit warning once
pub struct VADState {
    /// Ring buffer for accumulating audio samples for turn detection.
    ///
    /// Per smart-turn best practices: "run Smart Turn on the entire recording of the user's turn"
    /// This buffer keeps the most recent `MAX_VAD_AUDIO_SAMPLES` (30 seconds at 16kHz).
    /// When new audio would exceed the limit, oldest samples are dropped to make room.
    /// This ensures turn detection always has access to the most recent audio context.
    pub audio_buffer: SyncRwLock<VecDeque<i16>>,

    /// Temporary buffer for VAD frame processing.
    ///
    /// This buffer accumulates partial frames between `receive_audio` calls. VAD requires
    /// complete frames (e.g., 512 samples at 16kHz = 32ms) for inference. Since audio
    /// may arrive in arbitrary chunk sizes, we accumulate samples here and drain
    /// complete frames for VAD processing.
    pub frame_buffer: SyncRwLock<Vec<i16>>,

    /// Per-instance flag to log VAD buffer limit warning only once.
    buffer_limit_warned: AtomicBool,

    /// Test-only flag to force `process_audio_chunk` to return an error.
    /// Used for testing that STT continues to receive audio when VAD fails.
    #[cfg(test)]
    force_error: AtomicBool,
}

impl VADState {
    /// Create a new VADState with pre-allocated buffers.
    ///
    /// - `audio_buffer_capacity`: Capacity for the ring buffer (default: 8 seconds at 16kHz)
    /// - `frame_buffer_capacity`: Capacity for the frame buffer (default: 512 samples)
    pub fn new(audio_buffer_capacity: usize, frame_buffer_capacity: usize) -> Self {
        Self {
            audio_buffer: SyncRwLock::new(VecDeque::with_capacity(audio_buffer_capacity)),
            frame_buffer: SyncRwLock::new(Vec::with_capacity(frame_buffer_capacity)),
            buffer_limit_warned: AtomicBool::new(false),
            #[cfg(test)]
            force_error: AtomicBool::new(false),
        }
    }

    /// Clear both buffers and reset the warning flag.
    pub fn reset(&self) {
        let (audio_len, frame_len) = {
            let mut audio_buffer = self.audio_buffer.write();
            let mut frame_buffer = self.frame_buffer.write();
            let audio_len = audio_buffer.len();
            let frame_len = frame_buffer.len();
            audio_buffer.clear();
            frame_buffer.clear();
            (audio_len, frame_len)
        };
        self.buffer_limit_warned.store(false, Ordering::Relaxed);
        debug!(
            audio_samples_cleared = audio_len,
            frame_samples_cleared = frame_len,
            "VAD state reset: cleared audio and frame buffers"
        );
    }

    /// Clear just the audio buffer (used when new speech starts).
    pub fn clear_audio_buffer(&self) {
        let mut buffer = self.audio_buffer.write();
        let samples_cleared = buffer.len();
        buffer.clear();
        debug!(
            samples_cleared = samples_cleared,
            "VAD audio buffer cleared"
        );
    }

    /// Test-only: Set flag to force `process_audio_chunk` to return an error.
    #[cfg(test)]
    pub fn set_force_error(&self, force: bool) {
        self.force_error.store(force, Ordering::SeqCst);
    }

    /// Test-only: Check if error is forced.
    #[cfg(test)]
    pub fn is_force_error(&self) -> bool {
        self.force_error.load(Ordering::SeqCst)
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
    // Test-only: Return error immediately if force_error flag is set
    #[cfg(test)]
    if vad_state.is_force_error() {
        return Err(VoiceManagerError::InitializationError(
            "Forced VAD error for testing".to_string(),
        ));
    }

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

    // Accumulate samples in ring buffer for turn detection.
    // Handle oversized chunks defensively to prevent panics.
    {
        let mut audio_buffer = vad_state.audio_buffer.write();

        // Handle oversized chunks: if a single chunk exceeds MAX, keep only the tail
        if samples.len() >= MAX_VAD_AUDIO_SAMPLES {
            // Log this unusual condition (likely indicates upstream batching/buffering)
            if !vad_state.buffer_limit_warned.swap(true, Ordering::Relaxed) {
                debug!(
                    "Received oversized audio chunk ({} samples > {} max). \
                     Keeping only the most recent {} samples for turn detection.",
                    samples.len(),
                    MAX_VAD_AUDIO_SAMPLES,
                    MAX_VAD_AUDIO_SAMPLES
                );
            }
            audio_buffer.clear();
            // Keep only the last MAX_VAD_AUDIO_SAMPLES samples
            let start_idx = samples.len() - MAX_VAD_AUDIO_SAMPLES;
            audio_buffer.extend(samples[start_idx..].iter().copied());
        } else {
            // Normal case: make room for new samples if needed
            let new_total = audio_buffer.len() + samples.len();
            if new_total > MAX_VAD_AUDIO_SAMPLES {
                let samples_to_drop = new_total - MAX_VAD_AUDIO_SAMPLES;
                // Clamp to actual buffer length (defensive)
                let drop_count = samples_to_drop.min(audio_buffer.len());
                audio_buffer.drain(..drop_count);

                if !vad_state.buffer_limit_warned.swap(true, Ordering::Relaxed) {
                    debug!(
                        "VAD audio ring buffer at capacity ({} samples / 30 seconds). \
                         Dropping oldest samples to keep most recent audio for turn detection.",
                        MAX_VAD_AUDIO_SAMPLES
                    );
                }
            }
            // Add all new samples (buffer now has room)
            audio_buffer.extend(samples.iter().copied());
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

    // Get model reference once (brief lock acquisition, no await while holding)
    let vad_model = {
        let vad_guard = vad.read().await;
        vad_guard.get_model()
    }; // Lock released here, before any inference awaits

    // Process frames through VAD using the pre-cloned model reference.
    // This pattern avoids holding the async RwLock across inference awaits,
    // which would otherwise block other async tasks from accessing the VAD.
    for frame in frames_to_process {
        let inference_start = Instant::now();
        let frame_len = frame.len();
        let speech_prob = SileroVAD::process_audio_with_model(Arc::clone(&vad_model), &frame)
            .await
            .map_err(|e| {
                VoiceManagerError::InitializationError(format!("VAD processing error: {}", e))
            })?;
        let inference_duration = inference_start.elapsed();

        // Log VAD inference timing and buffer state
        let current_buffer_len = vad_state.audio_buffer.read().len();
        debug!(
            frame_samples = frame_len,
            inference_ms = inference_duration.as_secs_f64() * 1000.0,
            speech_prob = speech_prob,
            buffer_samples = current_buffer_len,
            buffer_max = MAX_VAD_AUDIO_SAMPLES,
            "VAD inference completed"
        );

        // Feed to silence tracker and collect events
        if let Some(event) = silence_tracker.process(speech_prob) {
            events.push(event);
        }
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that `set_force_error` and `is_force_error` work correctly.
    #[test]
    fn test_force_error_flag() {
        let vad_state = VADState::new(1024, 512);

        // Initially false
        assert!(!vad_state.is_force_error());

        // Set to true
        vad_state.set_force_error(true);
        assert!(vad_state.is_force_error());

        // Set back to false
        vad_state.set_force_error(false);
        assert!(!vad_state.is_force_error());
    }

    /// Test that oversized audio chunks do not panic and are handled correctly.
    #[test]
    fn test_oversized_chunk_does_not_panic() {
        // Create VADState with small initial capacity
        let vad_state = VADState::new(1024, 512);

        // Simulate adding some initial data
        {
            let mut buffer = vad_state.audio_buffer.write();
            buffer.extend(std::iter::repeat(0i16).take(10_000));
        }

        // Create oversized chunk (larger than MAX)
        let oversized_samples: Vec<i16> = (0..MAX_VAD_AUDIO_SAMPLES + 50_000)
            .map(|i| (i % 1000) as i16)
            .collect();

        // Simulate the ring buffer logic for oversized chunks - this should NOT panic
        {
            let mut audio_buffer = vad_state.audio_buffer.write();

            if oversized_samples.len() >= MAX_VAD_AUDIO_SAMPLES {
                audio_buffer.clear();
                let start_idx = oversized_samples.len() - MAX_VAD_AUDIO_SAMPLES;
                audio_buffer.extend(oversized_samples[start_idx..].iter().copied());
            }
        }

        // Verify buffer contains exactly MAX_VAD_AUDIO_SAMPLES
        let buffer = vad_state.audio_buffer.read();
        assert_eq!(buffer.len(), MAX_VAD_AUDIO_SAMPLES);

        // Verify the content is from the tail of the oversized samples
        assert_eq!(buffer[0], (50_000 % 1000) as i16);
    }

    /// Test that normal chunks work correctly with clamped drop_count.
    #[test]
    fn test_normal_chunk_with_clamping() {
        let vad_state = VADState::new(1024, 512);

        // Fill buffer close to capacity
        let fill_count = MAX_VAD_AUDIO_SAMPLES - 1000;
        {
            let mut buffer = vad_state.audio_buffer.write();
            buffer.extend(std::iter::repeat(1i16).take(fill_count));
        }

        // Add a chunk that would require dropping samples
        let new_samples: Vec<i16> = (0..5000).map(|i| (i + 100) as i16).collect();
        {
            let mut audio_buffer = vad_state.audio_buffer.write();

            let new_total = audio_buffer.len() + new_samples.len();
            if new_total > MAX_VAD_AUDIO_SAMPLES {
                let samples_to_drop = new_total - MAX_VAD_AUDIO_SAMPLES;
                let drop_count = samples_to_drop.min(audio_buffer.len());
                audio_buffer.drain(..drop_count);
            }
            audio_buffer.extend(new_samples.iter().copied());
        }

        // Verify buffer is at exactly MAX capacity
        let buffer = vad_state.audio_buffer.read();
        assert_eq!(buffer.len(), MAX_VAD_AUDIO_SAMPLES);
    }
}
