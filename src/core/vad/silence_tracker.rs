//! Silence duration tracking for VAD-based turn detection.
//!
//! This module provides the `SilenceTracker` component that monitors VAD output
//! and detects when silence exceeds a configurable threshold. It bridges the gap
//! between raw VAD probabilities and turn detection logic.
//!
//! # State Transitions
//!
//! ```text
//! [Initial] ─── speech_prob > threshold ──► [Speaking]
//!     │                                          │
//!     └── speech_prob <= threshold ──► (no event)│
//!                                                │
//! [Speaking] ─── speech_prob <= threshold ──► [Silence Detected]
//!     ▲                                          │
//!     │                                          │
//!     └───── speech_prob > threshold ────────────┘
//!                 (SpeechResumed)
//!
//! [Silence Detected] ─── silence >= threshold ──► [Turn End]
//!                    ─── speech_prob > threshold ──► [Speaking] (SpeechResumed)
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
use tracing::{debug, info};

/// Event emitted by the silence tracker during state transitions.
///
/// These events represent meaningful transitions in the VAD state machine
/// that can be used by higher-level components for turn detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VADEvent {
    /// Speech has started after silence.
    ///
    /// This event is emitted when the VAD transitions from detecting silence
    /// to detecting speech. It indicates the beginning of a potential utterance.
    SpeechStart,

    /// Silence detected (short, not yet exceeding threshold).
    ///
    /// This event is emitted when speech transitions to silence, but the
    /// silence has not yet exceeded the configured threshold. It indicates
    /// a potential pause in speech.
    SilenceDetected,

    /// Speech resumed after brief silence (under threshold).
    ///
    /// This event is emitted when speech resumes before the silence threshold
    /// was exceeded. It indicates a mid-utterance pause rather than a turn end.
    SpeechResumed,

    /// Silence exceeded threshold - turn may be complete.
    ///
    /// This event is emitted when continuous silence has exceeded the configured
    /// threshold after sufficient speech. This is the primary signal that a
    /// speaker's turn has ended.
    TurnEnd,
}

/// Configuration for silence tracking.
///
/// These parameters control when the `SilenceTracker` emits events,
/// particularly the `TurnEnd` event that signals a speaker has finished.
#[derive(Debug, Clone, Copy)]
pub struct SilenceTrackerConfig {
    /// Speech probability threshold (0.0 to 1.0).
    ///
    /// Audio frames with VAD probability above this threshold are considered speech.
    /// Default: 0.5 (standard Silero-VAD recommendation).
    pub threshold: f32,

    /// Minimum silence duration to trigger TurnEnd (ms).
    ///
    /// After detecting speech, this is how long continuous silence must be
    /// observed before considering the speech segment complete.
    /// Default: 300ms (from PLAN requirements).
    pub silence_duration_ms: u64,

    /// Minimum speech duration before checking silence (ms).
    ///
    /// Prevents false triggers on brief pauses or noise spikes. The system won't
    /// emit TurnEnd until at least this much speech has been detected.
    /// Default: 100ms.
    pub min_speech_duration_ms: u64,

    /// Frame duration in milliseconds (based on sample rate).
    ///
    /// This is calculated from the VAD frame size and sample rate:
    /// - 16kHz: 512 samples = 32ms per frame
    /// - 8kHz: 256 samples = 32ms per frame
    ///
    /// Default: 32.0ms.
    pub frame_duration_ms: f32,
}

impl Default for SilenceTrackerConfig {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            // Increased from 200ms to 500ms for longer conversations.
            // Natural speech pauses (200-400ms) shouldn't trigger premature turn detection.
            silence_duration_ms: 500,
            // Increased from 100ms to 250ms to filter out brief filler sounds
            min_speech_duration_ms: 250,
            frame_duration_ms: 32.0, // 512 samples at 16kHz
        }
    }
}

impl SilenceTrackerConfig {
    /// Create config from VAD sample rate and parameters.
    ///
    /// # Arguments
    /// * `sample_rate` - Sample rate in Hz (8000 or 16000)
    /// * `threshold` - Speech probability threshold (0.0 to 1.0)
    /// * `silence_duration_ms` - Minimum silence duration for TurnEnd (ms)
    /// * `min_speech_duration_ms` - Minimum speech duration before tracking silence (ms)
    pub fn from_sample_rate(
        sample_rate: u32,
        threshold: f32,
        silence_duration_ms: u64,
        min_speech_duration_ms: u64,
    ) -> Self {
        let frame_size = if sample_rate == 8000 { 256 } else { 512 };
        let frame_duration_ms = (frame_size as f32 / sample_rate as f32) * 1000.0;

        Self {
            threshold,
            silence_duration_ms,
            min_speech_duration_ms,
            frame_duration_ms,
        }
    }

    /// Create a new config with the specified silence threshold.
    pub fn with_silence_duration_ms(mut self, duration_ms: u64) -> Self {
        self.silence_duration_ms = duration_ms;
        self
    }

    /// Create a new config with the specified speech threshold.
    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }

    /// Create a new config with the specified minimum speech duration.
    pub fn with_min_speech_duration_ms(mut self, duration_ms: u64) -> Self {
        self.min_speech_duration_ms = duration_ms;
        self
    }
}

/// Tracks silence duration from VAD output for turn detection.
///
/// This component monitors frame-by-frame VAD probabilities and determines
/// when meaningful state transitions occur. It is thread-safe and uses
/// atomics for lock-free access to most state fields.
///
/// # Usage
///
/// ```ignore
/// let config = SilenceTrackerConfig::default();
/// let tracker = SilenceTracker::new(config);
///
/// // Process each VAD frame
/// for speech_prob in vad_results {
///     if let Some(event) = tracker.process(speech_prob) {
///         match event {
///             VADEvent::SpeechStart => { /* User started speaking */ }
///             VADEvent::TurnEnd => { /* User finished speaking */ }
///             _ => {}
///         }
///     }
/// }
///
/// // Reset for new conversation
/// tracker.reset();
/// ```
pub struct SilenceTracker {
    config: SilenceTrackerConfig,

    /// Whether we're currently in a speech segment.
    is_speaking: AtomicBool,

    /// Accumulated speech duration in this segment (ms).
    speech_duration_ms: AtomicU64,

    /// Accumulated silence duration since last speech (ms).
    silence_duration_ms: AtomicU64,

    /// Whether TurnEnd has already been fired for this silence period.
    /// Prevents duplicate TurnEnd events.
    turn_end_fired: AtomicBool,

    /// Timestamp when speech segment started.
    /// Uses RwLock since Instant cannot be stored in atomics.
    speech_start_time: parking_lot::RwLock<Option<Instant>>,
}

impl SilenceTracker {
    /// Create a new silence tracker with the given configuration.
    pub fn new(config: SilenceTrackerConfig) -> Self {
        Self {
            config,
            is_speaking: AtomicBool::new(false),
            speech_duration_ms: AtomicU64::new(0),
            silence_duration_ms: AtomicU64::new(0),
            turn_end_fired: AtomicBool::new(false),
            speech_start_time: parking_lot::RwLock::new(None),
        }
    }

    /// Process a VAD probability and return any triggered event.
    ///
    /// This method implements the core state machine for silence tracking.
    /// It should be called once per VAD frame with the speech probability.
    ///
    /// # Arguments
    /// * `speech_prob` - Speech probability from VAD (0.0 to 1.0)
    ///
    /// # Returns
    /// * `Some(VADEvent)` - If a state transition occurred
    /// * `None` - If no significant transition occurred
    pub fn process(&self, speech_prob: f32) -> Option<VADEvent> {
        let is_speech = speech_prob > self.config.threshold;
        let was_speaking = self.is_speaking.load(Ordering::Acquire);
        let frame_duration = self.config.frame_duration_ms as u64;

        if is_speech {
            self.process_speech_frame(was_speaking, frame_duration)
        } else {
            self.process_silence_frame(was_speaking, frame_duration)
        }
    }

    /// Process a frame that contains speech.
    fn process_speech_frame(&self, was_speaking: bool, frame_duration: u64) -> Option<VADEvent> {
        // Update state for speech
        self.is_speaking.store(true, Ordering::Release);
        self.speech_duration_ms
            .fetch_add(frame_duration, Ordering::Relaxed);
        self.turn_end_fired.store(false, Ordering::Release);

        let silence_before = self.silence_duration_ms.swap(0, Ordering::AcqRel);

        // Check if this is a new speech segment or resuming speech
        // We're resuming if we had previous speech and accumulated silence
        let had_previous_speech = self.speech_duration_ms.load(Ordering::Acquire) > frame_duration;

        if !was_speaking && had_previous_speech && silence_before > 0 {
            // Had speech, then silence, now speaking again = resume
            debug!("VAD: Speech resumed after {}ms silence", silence_before);
            return Some(VADEvent::SpeechResumed);
        } else if !was_speaking && !had_previous_speech {
            // Brand new speech segment
            *self.speech_start_time.write() = Some(Instant::now());
            debug!("VAD: Speech started");
            return Some(VADEvent::SpeechStart);
        } else if was_speaking && silence_before > 0 {
            // Was speaking, had brief silence (within same frame timing), now speaking again
            debug!("VAD: Speech resumed after {}ms silence", silence_before);
            return Some(VADEvent::SpeechResumed);
        }

        None
    }

    /// Process a frame that contains silence.
    fn process_silence_frame(&self, was_speaking: bool, frame_duration: u64) -> Option<VADEvent> {
        // Update silence duration
        let current_silence = self
            .silence_duration_ms
            .fetch_add(frame_duration, Ordering::AcqRel);
        let new_silence = current_silence + frame_duration;

        if was_speaking {
            // Just transitioned from speech to silence
            // Mark as no longer speaking
            self.is_speaking.store(false, Ordering::Release);

            let speech_duration = self.speech_duration_ms.load(Ordering::Acquire);
            let min_speech = self.config.min_speech_duration_ms;

            if speech_duration < min_speech {
                // Not enough speech yet, don't emit silence event
                debug!(
                    "VAD: Ignoring silence (speech_duration={}ms < min={}ms)",
                    speech_duration, min_speech
                );
                return None;
            }

            debug!(
                "VAD: Silence detected after {}ms of speech",
                speech_duration
            );
            return Some(VADEvent::SilenceDetected);
        }

        // Check if silence threshold exceeded for TurnEnd
        self.check_turn_end(new_silence)
    }

    /// Check if silence duration has exceeded threshold and emit TurnEnd if so.
    fn check_turn_end(&self, new_silence: u64) -> Option<VADEvent> {
        let already_fired = self.turn_end_fired.load(Ordering::Acquire);

        if already_fired || new_silence < self.config.silence_duration_ms {
            return None;
        }

        // Check minimum speech duration requirement
        let speech_duration = self.speech_duration_ms.load(Ordering::Acquire);
        if speech_duration < self.config.min_speech_duration_ms {
            return None;
        }

        // Fire TurnEnd
        self.turn_end_fired.store(true, Ordering::Release);
        info!(
            "VAD: Turn end detected after {}ms silence (speech_duration={}ms)",
            new_silence, speech_duration
        );
        Some(VADEvent::TurnEnd)
    }

    /// Reset the tracker state.
    ///
    /// Call this when starting a new conversation or after handling a turn end.
    /// This clears all accumulated speech and silence durations.
    pub fn reset(&self) {
        self.is_speaking.store(false, Ordering::Release);
        self.speech_duration_ms.store(0, Ordering::Release);
        self.silence_duration_ms.store(0, Ordering::Release);
        self.turn_end_fired.store(false, Ordering::Release);
        *self.speech_start_time.write() = None;
        debug!("SilenceTracker state reset");
    }

    /// Get current silence duration in milliseconds.
    pub fn current_silence_ms(&self) -> u64 {
        self.silence_duration_ms.load(Ordering::Acquire)
    }

    /// Get current speech duration in milliseconds.
    pub fn current_speech_ms(&self) -> u64 {
        self.speech_duration_ms.load(Ordering::Acquire)
    }

    /// Check if currently detecting speech.
    pub fn is_speaking(&self) -> bool {
        self.is_speaking.load(Ordering::Acquire)
    }

    /// Check if speech has been detected in the current segment.
    pub fn has_speech(&self) -> bool {
        self.speech_duration_ms.load(Ordering::Acquire) > 0
    }

    /// Get the elapsed time since speech started, if applicable.
    pub fn speech_elapsed(&self) -> Option<std::time::Duration> {
        self.speech_start_time
            .read()
            .map(|start| Instant::now().duration_since(start))
    }

    /// Get the configured silence duration threshold.
    pub fn silence_threshold_ms(&self) -> u64 {
        self.config.silence_duration_ms
    }

    /// Get the configured speech probability threshold.
    pub fn speech_threshold(&self) -> f32 {
        self.config.threshold
    }

    /// Get the current configuration.
    pub fn config(&self) -> &SilenceTrackerConfig {
        &self.config
    }

    /// Allow TurnEnd to fire again after Smart-Turn returns Incomplete.
    ///
    /// This method resets the TurnEnd latch and reduces accumulated silence
    /// to enforce a retry delay. After calling this, TurnEnd can fire again
    /// once additional silence accumulates past the threshold.
    ///
    /// # Arguments
    /// * `additional_silence_ms` - Amount to subtract from accumulated silence (ms).
    ///   This enforces a minimum wait before the next TurnEnd can fire.
    pub fn allow_turn_end_retry(&self, additional_silence_ms: u64) {
        // Reset the TurnEnd latch so it can fire again
        self.turn_end_fired.store(false, Ordering::Release);

        // Reduce accumulated silence to enforce retry delay
        let previous_silence = self
            .silence_duration_ms
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(current.saturating_sub(additional_silence_ms))
            })
            .unwrap_or(0);

        let new_silence = previous_silence.saturating_sub(additional_silence_ms);

        debug!(
            "SilenceTracker: allow_turn_end_retry called, silence adjusted from {}ms to {}ms (subtracted {}ms)",
            previous_silence, new_silence, additional_silence_ms
        );
    }
}

impl Default for SilenceTracker {
    fn default() -> Self {
        Self::new(SilenceTrackerConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SilenceTrackerConfig::default();
        assert_eq!(config.threshold, 0.5);
        // Increased to 500ms for longer conversations (was 200ms)
        assert_eq!(config.silence_duration_ms, 500);
        assert_eq!(config.min_speech_duration_ms, 250); // Increased to filter filler sounds
        assert_eq!(config.frame_duration_ms, 32.0);
    }

    #[test]
    fn test_config_from_sample_rate() {
        let config_16k = SilenceTrackerConfig::from_sample_rate(16000, 0.6, 250, 150);
        assert_eq!(config_16k.threshold, 0.6);
        assert_eq!(config_16k.silence_duration_ms, 250);
        assert_eq!(config_16k.min_speech_duration_ms, 150);
        assert_eq!(config_16k.frame_duration_ms, 32.0); // 512/16000 * 1000

        let config_8k = SilenceTrackerConfig::from_sample_rate(8000, 0.5, 300, 100);
        assert_eq!(config_8k.frame_duration_ms, 32.0); // 256/8000 * 1000
    }

    #[test]
    fn test_config_builder_methods() {
        let config = SilenceTrackerConfig::default()
            .with_threshold(0.7)
            .with_silence_duration_ms(500)
            .with_min_speech_duration_ms(200);

        assert_eq!(config.threshold, 0.7);
        assert_eq!(config.silence_duration_ms, 500);
        assert_eq!(config.min_speech_duration_ms, 200);
    }

    #[test]
    fn test_no_speech_no_events() {
        let tracker = SilenceTracker::default();

        // Silence without prior speech should not emit events
        assert_eq!(tracker.process(0.2), None);
        assert_eq!(tracker.process(0.3), None);
        assert_eq!(tracker.process(0.1), None);

        assert!(!tracker.has_speech());
        assert!(!tracker.is_speaking());
    }

    #[test]
    fn test_speech_start_event() {
        let tracker = SilenceTracker::default();

        // Speech should trigger SpeechStart
        let event = tracker.process(0.8);
        assert_eq!(event, Some(VADEvent::SpeechStart));
        assert!(tracker.is_speaking());
        assert!(tracker.has_speech());
    }

    #[test]
    fn test_continued_speech_no_event() {
        let tracker = SilenceTracker::default();

        // First speech frame
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));

        // Continued speech should not emit events
        assert_eq!(tracker.process(0.9), None);
        assert_eq!(tracker.process(0.7), None);
        assert_eq!(tracker.process(0.6), None);
    }

    #[test]
    fn test_silence_detected_after_sufficient_speech() {
        // Config with very short min_speech_duration for testing
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 300,
            min_speech_duration_ms: 32, // One frame
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Start speaking
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));

        // Transition to silence after sufficient speech
        let event = tracker.process(0.2);
        assert_eq!(event, Some(VADEvent::SilenceDetected));
    }

    #[test]
    fn test_silence_ignored_without_sufficient_speech() {
        // Config with high min_speech_duration
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 300,
            min_speech_duration_ms: 200, // Need ~6 frames
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Start speaking (only 1 frame = 32ms)
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));

        // Transition to silence - should be ignored due to insufficient speech
        assert_eq!(tracker.process(0.2), None);
    }

    #[test]
    fn test_speech_resumed_event() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 300,
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Start speaking
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));

        // Brief silence
        assert_eq!(tracker.process(0.2), Some(VADEvent::SilenceDetected));

        // Resume speaking before turn end threshold
        let event = tracker.process(0.8);
        assert_eq!(event, Some(VADEvent::SpeechResumed));
    }

    #[test]
    fn test_turn_end_after_silence_threshold() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64, // 2 frames
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Start speaking
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));

        // First silence frame
        assert_eq!(tracker.process(0.2), Some(VADEvent::SilenceDetected));

        // Second silence frame - should trigger TurnEnd (64ms >= 64ms threshold)
        let event = tracker.process(0.2);
        assert_eq!(event, Some(VADEvent::TurnEnd));
    }

    #[test]
    fn test_turn_end_fires_only_once() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64,
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Start speaking
        tracker.process(0.8);

        // Silence until TurnEnd
        tracker.process(0.2); // SilenceDetected
        assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd));

        // More silence should not trigger another TurnEnd
        assert_eq!(tracker.process(0.2), None);
        assert_eq!(tracker.process(0.2), None);
        assert_eq!(tracker.process(0.2), None);
    }

    #[test]
    fn test_reset_clears_state() {
        let tracker = SilenceTracker::default();

        // Accumulate some state
        tracker.process(0.8); // Speech
        tracker.process(0.9);
        tracker.process(0.2); // Silence

        assert!(tracker.has_speech());
        assert!(tracker.current_speech_ms() > 0);
        assert!(tracker.current_silence_ms() > 0);

        // Reset
        tracker.reset();

        assert!(!tracker.has_speech());
        assert!(!tracker.is_speaking());
        assert_eq!(tracker.current_speech_ms(), 0);
        assert_eq!(tracker.current_silence_ms(), 0);
    }

    #[test]
    fn test_speech_elapsed_duration() {
        let tracker = SilenceTracker::default();

        // No speech yet
        assert!(tracker.speech_elapsed().is_none());

        // Start speaking
        tracker.process(0.8);

        // Should have elapsed time
        let elapsed = tracker.speech_elapsed();
        assert!(elapsed.is_some());

        // After reset, should be None again
        tracker.reset();
        assert!(tracker.speech_elapsed().is_none());
    }

    #[test]
    fn test_getters() {
        let config = SilenceTrackerConfig {
            threshold: 0.6,
            silence_duration_ms: 250,
            min_speech_duration_ms: 150,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        assert_eq!(tracker.silence_threshold_ms(), 250);
        assert_eq!(tracker.speech_threshold(), 0.6);
        assert_eq!(tracker.config().min_speech_duration_ms, 150);
    }

    #[test]
    fn test_complete_turn_cycle() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 96,    // 3 frames
            min_speech_duration_ms: 64, // 2 frames
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Full cycle: silence -> speech -> pause -> resume -> silence -> turn end
        assert_eq!(tracker.process(0.2), None); // Initial silence (no speech yet)
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));
        assert_eq!(tracker.process(0.9), None); // Continued speech
        assert_eq!(tracker.process(0.7), None); // Continued speech (now 64ms)
        assert_eq!(tracker.process(0.3), Some(VADEvent::SilenceDetected)); // Brief pause
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechResumed)); // Resume
        assert_eq!(tracker.process(0.9), None); // More speech
        assert_eq!(tracker.process(0.2), Some(VADEvent::SilenceDetected)); // Silence starts
        assert_eq!(tracker.process(0.3), None); // More silence
        assert_eq!(tracker.process(0.1), Some(VADEvent::TurnEnd)); // 96ms silence reached

        // After turn end, new speech should work
        tracker.reset();
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));
    }

    #[test]
    fn test_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let tracker = Arc::new(SilenceTracker::default());

        // Spawn multiple threads reading state
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let tracker = Arc::clone(&tracker);
                thread::spawn(move || {
                    for _ in 0..100 {
                        let _ = tracker.is_speaking();
                        let _ = tracker.has_speech();
                        let _ = tracker.current_silence_ms();
                        let _ = tracker.current_speech_ms();
                    }
                })
            })
            .collect();

        // Process some frames in main thread
        for i in 0..100 {
            let prob = if i % 3 == 0 { 0.8 } else { 0.3 };
            let _ = tracker.process(prob);
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_silence_tracker_default_initial_state() {
        let tracker = SilenceTracker::default();
        assert!(!tracker.is_speaking());
        assert_eq!(tracker.current_silence_ms(), 0);
        assert_eq!(tracker.current_speech_ms(), 0);
        assert!(!tracker.has_speech());
        assert!(tracker.speech_elapsed().is_none());
    }

    #[test]
    fn test_silence_accumulation_toward_turn_end() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 300,
            min_speech_duration_ms: 50,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Start with speech (need at least 2 frames for 64ms > 50ms min)
        tracker.process(0.8);
        tracker.process(0.8);

        // First silence frame
        let event = tracker.process(0.2);
        assert_eq!(event, Some(VADEvent::SilenceDetected));
        assert_eq!(tracker.current_silence_ms(), 32);

        // Accumulate more silence frames without TurnEnd (need ~10 frames for 300ms)
        for i in 1..8 {
            let event = tracker.process(0.2);
            // Should not trigger TurnEnd until 300ms
            assert_eq!(event, None, "Frame {} should not trigger TurnEnd", i);
            assert_eq!(tracker.current_silence_ms(), 32 * (i + 1) as u64);
        }

        // 9th frame reaches 288ms, 10th should trigger TurnEnd at 320ms >= 300ms
        assert_eq!(tracker.process(0.2), None); // 288ms
        let event = tracker.process(0.2); // 320ms
        assert_eq!(event, Some(VADEvent::TurnEnd));
    }

    #[test]
    fn test_turn_end_requires_min_speech_duration() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 100,    // Short silence threshold
            min_speech_duration_ms: 200, // Longer min speech requirement
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Very brief speech (only 32ms, less than 200ms required)
        tracker.process(0.8);

        // Long silence - should NOT trigger TurnEnd because min_speech not met
        for _ in 0..10 {
            let event = tracker.process(0.2);
            // Should not trigger TurnEnd since we haven't met min_speech_duration
            assert_ne!(event, Some(VADEvent::TurnEnd));
        }

        // Even with lots of silence, no TurnEnd without sufficient speech
        assert!(tracker.current_silence_ms() >= 300);
        assert!(tracker.current_speech_ms() < 200);
    }

    #[test]
    fn test_boundary_threshold_values() {
        // Test exactly at threshold
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64,
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Exactly at threshold (0.5) should be considered silence (prob > threshold for speech)
        assert_eq!(tracker.process(0.5), None);
        assert!(!tracker.is_speaking());

        // Just above threshold should be speech
        assert_eq!(tracker.process(0.51), Some(VADEvent::SpeechStart));
        assert!(tracker.is_speaking());
    }

    #[test]
    fn test_speech_after_turn_end() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64,
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Complete first turn
        tracker.process(0.8); // SpeechStart
        tracker.process(0.2); // SilenceDetected
        tracker.process(0.2); // TurnEnd

        // New speech after TurnEnd without reset - should trigger SpeechResumed
        // since we had previous speech in this segment
        let event = tracker.process(0.8);
        assert_eq!(event, Some(VADEvent::SpeechResumed));

        // After explicit reset, new speech should trigger SpeechStart
        tracker.reset();
        let event = tracker.process(0.8);
        assert_eq!(event, Some(VADEvent::SpeechStart));
    }

    #[test]
    fn test_rapid_speech_silence_transitions() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 128, // 4 frames
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Start speaking
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));

        // Rapid alternations - shouldn't trigger TurnEnd
        for _ in 0..5 {
            tracker.process(0.2); // Brief silence
            tracker.process(0.8); // Speech resumes
        }

        // Should still be in speaking state (no TurnEnd triggered)
        // and not have enough accumulated silence
        assert!(tracker.current_silence_ms() < 128);
    }

    #[test]
    fn test_multiple_consecutive_resets() {
        let tracker = SilenceTracker::default();

        // Build up state
        tracker.process(0.8);
        tracker.process(0.8);
        tracker.process(0.2);

        // Multiple resets should be idempotent
        tracker.reset();
        tracker.reset();
        tracker.reset();

        assert!(!tracker.is_speaking());
        assert_eq!(tracker.current_silence_ms(), 0);
        assert_eq!(tracker.current_speech_ms(), 0);
        assert!(!tracker.has_speech());
    }

    #[test]
    fn test_exact_threshold_silence_duration() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64, // Exactly 2 frames
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Speech
        tracker.process(0.8);

        // First silence frame (32ms) - SilenceDetected
        assert_eq!(tracker.process(0.2), Some(VADEvent::SilenceDetected));
        assert!(tracker.current_silence_ms() < 64);

        // Second silence frame (64ms) - should trigger TurnEnd at exactly threshold
        let event = tracker.process(0.2);
        assert_eq!(event, Some(VADEvent::TurnEnd));
        assert!(tracker.current_silence_ms() >= 64);
    }

    #[test]
    fn test_allow_turn_end_retry_resets_latch_and_reduces_silence() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64, // 2 frames
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Build up speech and trigger TurnEnd
        tracker.process(0.8); // SpeechStart
        tracker.process(0.2); // SilenceDetected (32ms silence)
        assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd)); // 64ms silence

        // TurnEnd has fired, so more silence shouldn't trigger it again
        assert_eq!(tracker.process(0.2), None); // 96ms silence
        let silence_before = tracker.current_silence_ms();
        assert_eq!(silence_before, 96);

        // Call allow_turn_end_retry to reset the latch and reduce silence
        tracker.allow_turn_end_retry(50);

        // Verify silence was reduced
        let silence_after = tracker.current_silence_ms();
        assert_eq!(silence_after, 46); // 96 - 50 = 46
    }

    #[test]
    fn test_allow_turn_end_retry_enables_turn_end_to_fire_again() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64, // 2 frames
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Build up speech and trigger first TurnEnd
        tracker.process(0.8); // SpeechStart
        tracker.process(0.2); // SilenceDetected (32ms)
        assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd)); // 64ms

        // More silence should NOT trigger TurnEnd again
        assert_eq!(tracker.process(0.2), None); // 96ms
        assert_eq!(tracker.process(0.2), None); // 128ms

        // Allow retry with reduction that puts us below threshold
        // Current silence is 128ms, reduce by 100ms to get 28ms
        tracker.allow_turn_end_retry(100);
        assert_eq!(tracker.current_silence_ms(), 28);

        // Now accumulate silence until threshold is reached again
        assert_eq!(tracker.process(0.2), None); // 60ms (still below 64ms)
        // Next frame should trigger TurnEnd (60 + 32 = 92ms >= 64ms)
        assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd));
    }

    #[test]
    fn test_allow_turn_end_retry_saturating_sub() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64,
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Build up a small amount of silence
        tracker.process(0.8); // SpeechStart
        tracker.process(0.2); // 32ms silence

        // Try to subtract more than accumulated silence
        tracker.allow_turn_end_retry(1000);

        // Should saturate at 0, not underflow
        assert_eq!(tracker.current_silence_ms(), 0);
    }

    #[test]
    fn test_allow_turn_end_retry_multiple_retries() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64, // 2 frames
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Build up speech and trigger first TurnEnd
        tracker.process(0.8); // SpeechStart
        tracker.process(0.2); // SilenceDetected
        assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd)); // First TurnEnd

        // Allow retry and accumulate to second TurnEnd
        tracker.allow_turn_end_retry(64); // Reset silence to 0
        assert_eq!(tracker.current_silence_ms(), 0);
        tracker.process(0.2); // 32ms
        assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd)); // Second TurnEnd

        // Allow retry and accumulate to third TurnEnd
        tracker.allow_turn_end_retry(64);
        tracker.process(0.2); // 32ms
        assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd)); // Third TurnEnd
    }
}
