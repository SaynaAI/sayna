//! VAD state management for audio buffering.

use parking_lot::RwLock as SyncRwLock;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::debug;

/// Encapsulates VAD processing buffers and state per VoiceManager instance.
pub struct VADState {
    /// Ring buffer for recent audio samples (max 30 seconds at 16kHz).
    /// Oldest samples are dropped when new audio would exceed the limit.
    pub audio_buffer: SyncRwLock<VecDeque<i16>>,

    /// Accumulates partial frames between `receive_audio` calls.
    /// VAD requires complete frames (e.g., 512 samples) for inference.
    pub frame_buffer: SyncRwLock<Vec<i16>>,

    /// Logs buffer limit warning only once per instance.
    buffer_limit_warned: AtomicBool,

    #[cfg(test)]
    force_error: AtomicBool,
}

impl VADState {
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

    /// Check and swap the buffer limit warning flag (returns previous value).
    pub fn swap_buffer_limit_warned(&self) -> bool {
        self.buffer_limit_warned.swap(true, Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn set_force_error(&self, force: bool) {
        self.force_error.store(force, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn is_force_error(&self) -> bool {
        self.force_error.load(Ordering::SeqCst)
    }
}
