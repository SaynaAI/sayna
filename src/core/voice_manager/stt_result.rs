//! STT result processing with timing control.
//!
//! Handles STT results with VAD-based silence detection. When `stt-vad` is compiled,
//! VAD mode is always active. All timeout-based fallbacks have been removed.
//!
//! Speech final events are emitted based on feature flag:
//! - When `stt-vad` enabled: VAD + Smart-Turn ONLY (STT provider's is_speech_final is IGNORED)
//! - When `stt-vad` NOT enabled: STT provider's `is_speech_final` ONLY
//!
//! See CLAUDE.md for detailed architecture documentation.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
#[cfg(feature = "stt-vad")]
use tokio::sync::RwLock;
use tracing::debug;
#[cfg(feature = "stt-vad")]
use tracing::info;

use crate::core::stt::STTResult;
#[cfg(feature = "stt-vad")]
use crate::core::turn_detect::TurnDetector;
#[cfg(feature = "stt-vad")]
use crate::core::vad::{SilenceTracker, VADEvent};

use super::state::SpeechFinalState;
use super::stt_config::STTProcessingConfig;
#[cfg(feature = "stt-vad")]
use super::turn_detection_tasks;
#[cfg(feature = "stt-vad")]
use super::utils::MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION;
#[cfg(not(feature = "stt-vad"))]
use super::utils::get_current_time_ms;
#[cfg(feature = "stt-vad")]
use super::vad_processor::VADState;

/// Processor for STT results with timing control
#[derive(Clone)]
pub struct STTResultProcessor {
    config: STTProcessingConfig,

    /// Optional turn detector for smart-turn fallback (feature-gated: `stt-vad`)
    #[cfg(feature = "stt-vad")]
    turn_detector: Option<Arc<RwLock<TurnDetector>>>,

    /// Optional silence tracker for smart-turn fallback (feature-gated: `stt-vad`)
    #[cfg(feature = "stt-vad")]
    silence_tracker: Option<Arc<SilenceTracker>>,

    /// Optional VAD state containing audio buffer for smart-turn fallback (feature-gated: `stt-vad`)
    #[cfg(feature = "stt-vad")]
    vad_state: Option<Arc<VADState>>,
}

impl STTResultProcessor {
    /// Create a new STTResultProcessor with just configuration (no VAD components).
    #[cfg(not(feature = "stt-vad"))]
    pub fn new(config: STTProcessingConfig) -> Self {
        Self { config }
    }

    /// Create a new STTResultProcessor with just configuration (no VAD components).
    #[cfg(feature = "stt-vad")]
    pub fn new(config: STTProcessingConfig) -> Self {
        Self {
            config,
            turn_detector: None,
            silence_tracker: None,
            vad_state: None,
        }
    }

    /// Create a new STTResultProcessor with VAD components for smart-turn fallback.
    #[cfg(feature = "stt-vad")]
    pub fn with_vad_components(
        config: STTProcessingConfig,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        silence_tracker: Arc<SilenceTracker>,
        vad_state: Arc<VADState>,
    ) -> Self {
        Self {
            config,
            turn_detector,
            silence_tracker: Some(silence_tracker),
            vad_state: Some(vad_state),
        }
    }

    /// Process an STT result, handling turn detection and speech_final events.
    pub async fn process_result(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    ) -> Option<STTResult> {
        // Fast synchronous checks - no awaits
        if !self.should_deliver_result(&result) {
            return None;
        }

        // Handle real speech_final from STT provider.
        // When stt-vad is enabled, IGNORE this and let VAD + Smart Turn control turn completion.
        #[cfg(not(feature = "stt-vad"))]
        if result.is_speech_final {
            let now_ms = get_current_time_ms();
            return self.handle_real_speech_final(result, speech_final_state, now_ms);
        }

        // When stt-vad is enabled, override is_speech_final to false so user only
        // receives is_speech_final=true from the VAD + Smart Turn path.
        #[cfg(feature = "stt-vad")]
        let result = if result.is_speech_final {
            debug!(
                "STT provider sent is_speech_final=true, but stt-vad is enabled - overriding to false (VAD controls turn completion)"
            );
            STTResult {
                is_speech_final: false,
                ..result
            }
        } else {
            result
        };

        // Handle is_final (but not speech_final) - spawn turn detection in background
        if result.is_final {
            self.handle_turn_detection(result.clone(), speech_final_state);
        }

        // Always return the original result immediately - no awaits in critical path
        Some(result)
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // VAD-based silence detection methods
    // ─────────────────────────────────────────────────────────────────────────────

    /// Process a VAD event, triggering turn detection on silence.
    ///
    /// This method requires that the processor was initialized with VAD components
    /// via `with_vad_components`. If VAD components are not available, the event
    /// is ignored with a debug log.
    #[cfg(feature = "stt-vad")]
    pub fn process_vad_event(
        &self,
        event: VADEvent,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    ) {
        // Use stored VAD components - these should be Some if initialized via with_vad_components
        let (Some(silence_tracker), Some(vad_state)) = (&self.silence_tracker, &self.vad_state)
        else {
            debug!("VAD event received but VAD components not initialized - ignoring");
            return;
        };

        self.handle_vad_event_internal(
            event,
            speech_final_state,
            silence_tracker.clone(),
            vad_state.clone(),
        );
    }

    /// Check if VAD-based silence detection is enabled.
    ///
    /// When `stt-vad` is compiled, VAD is always active and cannot be disabled at runtime.
    #[cfg(feature = "stt-vad")]
    pub fn is_vad_enabled(&self) -> bool {
        true
    }

    /// Check if VAD-based silence detection is enabled.
    #[cfg(not(feature = "stt-vad"))]
    pub fn is_vad_enabled(&self) -> bool {
        false
    }

    /// Get the configured VAD silence duration threshold in milliseconds.
    pub fn vad_silence_duration_ms(&self) -> u64 {
        self.config.vad_silence_duration_ms
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Core STT processing methods
    // ─────────────────────────────────────────────────────────────────────────────

    fn should_deliver_result(&self, result: &STTResult) -> bool {
        // Skip empty final results that aren't speech_final
        !(result.transcript.trim().is_empty() && result.is_final && !result.is_speech_final)
    }

    /// Handle turn detection by accumulating text and setting waiting flag.
    ///
    /// No timeout tasks are spawned. The actual speech_final emission depends on feature:
    /// - When `stt-vad` enabled: VAD + Smart-Turn path controls turn completion
    /// - When `stt-vad` NOT enabled: STT provider's `is_speech_final` controls turn completion
    fn handle_turn_detection(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    ) {
        // Update text buffer and set waiting flag
        let mut state = speech_final_state.write();
        state.text_buffer.push_str(&result.transcript);
        state
            .waiting_for_speech_final
            .store(true, Ordering::Release);

        debug!(
            "Accumulated text for turn detection: '{}' (waiting for VAD TurnEnd or real speech_final)",
            state.text_buffer
        );
    }

    #[cfg(not(feature = "stt-vad"))]
    fn handle_real_speech_final(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        now_ms: usize,
    ) -> Option<STTResult> {
        let mut state = speech_final_state.write();

        // Check for duplicate within the configured window
        if self.is_duplicate_speech_final(&state, &result.transcript, now_ms) {
            debug!(
                "Ignoring duplicate real speech_final - turn detection fired {}ms ago",
                now_ms.saturating_sub(state.turn_detection_last_fired_ms.load(Ordering::Acquire))
            );
            return None;
        }

        // Cancel pending detection tasks and reset state for next speech segment
        state.reset_for_next_segment();

        Some(result)
    }

    #[cfg(not(feature = "stt-vad"))]
    fn is_duplicate_speech_final(
        &self,
        state: &SpeechFinalState,
        transcript: &str,
        now_ms: usize,
    ) -> bool {
        let last_fired_ms = state.turn_detection_last_fired_ms.load(Ordering::Acquire);

        last_fired_ms > 0
            && now_ms.saturating_sub(last_fired_ms) < self.config.duplicate_window_ms
            && state.last_forced_text == transcript
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // VAD event handling (internal)
    // ─────────────────────────────────────────────────────────────────────────────

    /// Handle a VAD event internally, processing speech start/resume/silence/turn-end.
    #[cfg(feature = "stt-vad")]
    fn handle_vad_event_internal(
        &self,
        event: VADEvent,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        silence_tracker: Arc<SilenceTracker>,
        vad_state: Arc<VADState>,
    ) {
        match event {
            VADEvent::SpeechStart => {
                // Use turn_in_progress to determine if we're mid-turn (not waiting_for_speech_final).
                // This is more accurate because turn_in_progress remains true during brief pauses
                // even after silence_tracker resets, preventing audio buffer loss that would cause
                // the Smart Turn model to produce false positives (fixes Pipecat issue #3094).
                let is_turn_in_progress = {
                    let mut state = speech_final_state.write();
                    let was_in_progress = state.turn_in_progress.load(Ordering::Acquire);
                    // Mark that a turn is now in progress
                    state.turn_in_progress.store(true, Ordering::Release);
                    state.reset_vad_state();
                    was_in_progress
                };

                if is_turn_in_progress {
                    debug!(
                        "VAD: Speech started mid-turn - resetting VAD state but preserving audio buffer for context"
                    );
                } else {
                    debug!(
                        "VAD: New speech started - resetting VAD state and clearing audio buffer"
                    );
                    vad_state.clear_audio_buffer();
                }
            }

            VADEvent::SpeechResumed => {
                debug!(
                    "VAD: Speech resumed - cancelling pending VAD turn detection but preserving audio buffer"
                );
                let mut state = speech_final_state.write();
                state.reset_vad_state();
            }

            VADEvent::SilenceDetected => {
                debug!("VAD: Silence detected - waiting for turn end threshold");
            }

            VADEvent::TurnEnd => {
                self.handle_vad_turn_end(speech_final_state, silence_tracker, vad_state);
            }
        }
    }

    /// Handle VAD TurnEnd event by spawning turn detection.
    #[cfg(feature = "stt-vad")]
    fn handle_vad_turn_end(
        &self,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        silence_tracker: Arc<SilenceTracker>,
        vad_state: Arc<VADState>,
    ) {
        let (audio_samples, buffered_text, should_trigger) = {
            let mut state = speech_final_state.write();

            if state.vad_turn_end_detected.load(Ordering::Acquire) {
                debug!("VAD: TurnEnd already detected for this segment - skipping");
                return;
            }

            if !state.waiting_for_speech_final.load(Ordering::Acquire) {
                debug!("VAD: TurnEnd received but not waiting for speech_final - skipping");
                return;
            }

            let audio_buffer = vad_state.audio_buffer.read();
            if audio_buffer.is_empty() {
                debug!("VAD: TurnEnd received but audio buffer is empty - skipping");
                return;
            }

            if audio_buffer.len() < MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION {
                debug!(
                    "VAD: TurnEnd received but audio buffer too short ({} samples < {} min) - skipping",
                    audio_buffer.len(),
                    MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION
                );
                return;
            }

            // Convert VecDeque to Vec for turn detection inference
            let audio: Vec<i16> = audio_buffer.iter().copied().collect();

            state.vad_turn_end_detected.store(true, Ordering::Release);

            // Cancel any previous VAD turn detection task if still running
            if let Some(old_handle) = state.vad_turn_detection_handle.take() {
                debug!("VAD: Cancelling previous VAD turn detection task");
                old_handle.abort();
            }

            (audio, state.text_buffer.clone(), true)
        };

        if !should_trigger {
            return;
        }

        info!(
            "VAD: TurnEnd after {}ms silence - spawning turn detection with {} audio samples",
            self.config.vad_silence_duration_ms,
            audio_samples.len()
        );

        let handle = turn_detection_tasks::create_vad_turn_detection_task(
            &self.config,
            audio_samples,
            buffered_text,
            speech_final_state.clone(),
            self.turn_detector.clone(),
            silence_tracker,
            vad_state,
        );

        let mut state = speech_final_state.write();
        state.vad_turn_detection_handle = Some(handle);
    }
}

impl Default for STTResultProcessor {
    fn default() -> Self {
        Self::new(STTProcessingConfig::default())
    }
}
