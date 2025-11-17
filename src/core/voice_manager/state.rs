//! State management for VoiceManager

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use tokio::task::JoinHandle;

use super::callbacks::STTCallback;

/// Internal state for managing speech final timing
/// Uses parking_lot RwLock for faster synchronization and optimized field layout
pub struct SpeechFinalState {
    /// Combined text buffer from STT results
    pub text_buffer: String,
    /// Turn detection task handle
    pub turn_detection_handle: Option<JoinHandle<()>>,
    /// Hard timeout task handle - cancels when real speech_final arrives
    pub hard_timeout_handle: Option<JoinHandle<()>>,
    /// Whether we're currently waiting for speech_final - atomic for lock-free reads
    pub waiting_for_speech_final: AtomicBool,
    /// User callback to call when turn detection completes
    pub user_callback: Option<STTCallback>,
    /// Timestamp (ms since epoch) when turn detection last fired - used to prevent duplicates
    pub turn_detection_last_fired_ms: AtomicUsize,
    /// Last text that was force-finalized by turn detection
    pub last_forced_text: String,
    /// Timestamp (ms since epoch) when first is_final frame of current utterance arrived
    pub segment_start_ms: AtomicUsize,
    /// Hard timeout deadline (ms since epoch) - when the hard timeout will fire
    pub hard_timeout_deadline_ms: AtomicUsize,
}

/// State for managing interruption control
/// Uses atomic types for lock-free access in hot paths
pub struct InterruptionState {
    /// Whether interruptions are currently allowed - atomic for lock-free reads
    pub allow_interruption: AtomicBool,
    /// Time when the current non-interruptible audio will finish playing
    /// Stored as milliseconds since epoch for atomic access
    pub non_interruptible_until_ms: AtomicUsize,
    /// Sample rate of the current TTS audio - atomic for lock-free access
    pub current_sample_rate: AtomicU32,
    /// Whether TTS has completed playing all audio - atomic for lock-free reads
    pub is_completed: AtomicBool,
}

impl InterruptionState {
    /// Check if interruption is currently allowed
    pub fn can_interrupt(&self) -> bool {
        // If allow_interruption is true, we can always interrupt
        if self.allow_interruption.load(Ordering::Acquire) {
            return true;
        }

        // If completed and past the non-interruptible time, we can interrupt
        if self.is_completed.load(Ordering::Acquire) {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as usize;
            let until_ms = self.non_interruptible_until_ms.load(Ordering::Acquire);

            if now_ms > until_ms {
                return true;
            }
        }

        false
    }

    /// Reset interruption state to defaults
    pub fn reset(&self) {
        self.allow_interruption.store(true, Ordering::Release);
        self.non_interruptible_until_ms.store(0, Ordering::Release);
        self.is_completed.store(true, Ordering::Release);
    }
}
