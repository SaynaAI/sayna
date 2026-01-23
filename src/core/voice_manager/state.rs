//! State management for VoiceManager

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use tokio::task::JoinHandle;

use super::callbacks::STTCallback;
use super::utils::get_current_time_ms;

/// Internal state for managing speech final timing
///
/// This simplified state removes all timeout-based task handles. Speech final events
/// are ONLY emitted when:
/// 1. STT provider sends a real `is_speech_final=true` event
/// 2. VAD detects silence -> Smart-Turn confirms turn complete (when `stt-vad` enabled)
///
/// Uses parking_lot RwLock for faster synchronization and optimized field layout
pub struct SpeechFinalState {
    /// Combined text buffer from STT results
    pub text_buffer: String,
    /// Whether we're currently waiting for speech_final - atomic for lock-free reads
    pub waiting_for_speech_final: AtomicBool,
    /// User callback to call when turn detection completes
    pub user_callback: Option<STTCallback>,
    /// Timestamp (ms since epoch) when turn detection last fired - used to prevent duplicates
    pub turn_detection_last_fired_ms: AtomicUsize,
    /// Last text that was force-finalized by turn detection
    pub last_forced_text: String,

    // ─────────────────────────────────────────────────────────────────────────
    // VAD-based silence tracking state
    // ─────────────────────────────────────────────────────────────────────────
    /// Whether VAD has detected the TurnEnd event for the current segment.
    ///
    /// This is set to true when the SilenceTracker emits VADEvent::TurnEnd,
    /// indicating that silence has exceeded the configured threshold after speech.
    /// Used to trigger turn detection and prevent duplicate firings.
    pub vad_turn_end_detected: AtomicBool,

    /// Task handle for VAD-triggered turn detection.
    ///
    /// When VAD detects sufficient silence (TurnEnd event), we spawn a task
    /// to run turn detection on the accumulated text. This handle allows
    /// cancellation if new speech arrives.
    pub vad_turn_detection_handle: Option<JoinHandle<()>>,

    /// Whether a speaking turn is currently in progress.
    ///
    /// This flag is set to true on the first SpeechStart of a new turn and
    /// remains true until the turn is confirmed complete (Smart-Turn returns true
    /// or real speech_final is received). This prevents audio buffer clearing
    /// during brief pauses mid-turn, which would cause the model to lose context
    /// and trigger false positives (fixes Pipecat issue #3094).
    pub turn_in_progress: AtomicBool,

    /// Task handle for backup silence timeout.
    ///
    /// When Smart-Turn returns Incomplete, a backup timeout is started. If the
    /// user remains silent for this duration, a speech_final is forced. This
    /// handle allows cancellation if speech resumes or Smart-Turn runs again.
    pub backup_timeout_handle: Option<JoinHandle<()>>,
}

impl SpeechFinalState {
    /// Create a new SpeechFinalState with default values.
    pub fn new() -> Self {
        Self {
            text_buffer: String::new(),
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: None,
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::new(),
            vad_turn_end_detected: AtomicBool::new(false),
            vad_turn_detection_handle: None,
            turn_in_progress: AtomicBool::new(false),
            backup_timeout_handle: None,
        }
    }

    /// Create a new SpeechFinalState with a user callback.
    pub fn with_callback(callback: STTCallback) -> Self {
        Self {
            user_callback: Some(callback),
            ..Self::new()
        }
    }

    /// Reset VAD-specific state for a new speech segment.
    ///
    /// Call this when speech resumes (VADEvent::SpeechStart or SpeechResumed)
    /// to clear the VAD turn detection state and cancel any pending backup timeout.
    pub fn reset_vad_state(&mut self) {
        self.vad_turn_end_detected.store(false, Ordering::Release);
        if let Some(handle) = self.vad_turn_detection_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.backup_timeout_handle.take() {
            handle.abort();
        }
    }

    /// Internal helper for resetting state.
    ///
    /// If `keep_fired_stats` is true, preserves `last_forced_text` and
    /// `turn_detection_last_fired_ms` (used after firing to prevent duplicates).
    fn reset_internal(&mut self, keep_fired_stats: bool) {
        // Reset VAD state
        self.reset_vad_state();

        // Clear text buffer
        self.text_buffer.clear();

        // Reset timing state
        self.waiting_for_speech_final
            .store(false, Ordering::Release);

        // Reset turn-in-progress flag - turn is complete
        self.turn_in_progress.store(false, Ordering::Release);

        // Conditionally clear fired stats
        if !keep_fired_stats {
            self.last_forced_text.clear();
            self.turn_detection_last_fired_ms
                .store(0, Ordering::Release);
        }
    }

    /// Reset all state for a new conversation segment.
    ///
    /// Call this after a speech_final has been delivered to prepare for the
    /// next utterance.
    pub fn reset_for_next_segment(&mut self) {
        self.reset_internal(false);
    }

    /// Reset state after firing a forced speech_final.
    ///
    /// This preserves `last_forced_text` and `turn_detection_last_fired_ms`
    /// to enable duplicate detection for subsequent events.
    pub fn reset_after_firing(&mut self) {
        self.reset_internal(true);
    }
}

impl Default for SpeechFinalState {
    fn default() -> Self {
        Self::new()
    }
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
            let now_ms = get_current_time_ms();
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
