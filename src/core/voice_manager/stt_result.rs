//! STT result processing with timing control.
//!
//! Handles STT results with configurable timing, supporting both timeout-based
//! and VAD-based silence detection. When `stt-vad` is compiled, VAD mode is
//! always active. See CLAUDE.md for detailed architecture documentation.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
#[cfg(feature = "stt-vad")]
use tokio::sync::RwLock;
#[cfg(not(feature = "stt-vad"))]
use tracing::debug;
#[cfg(feature = "stt-vad")]
use tracing::{debug, info};

use crate::core::stt::STTResult;
#[cfg(feature = "stt-vad")]
use crate::core::turn_detect::TurnDetector;
#[cfg(feature = "stt-vad")]
use crate::core::vad::{SilenceTracker, VADEvent};

use super::state::SpeechFinalState;
use super::stt_config::STTProcessingConfig;
use super::turn_detection_tasks;
#[cfg(feature = "stt-vad")]
use super::utils::MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION;
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

        let now_ms = get_current_time_ms();

        // Handle real speech_final
        if result.is_speech_final {
            return self.handle_real_speech_final(result, speech_final_state, now_ms);
        }

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

    fn handle_turn_detection(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    ) {
        use tracing::info;

        // Update text buffer and cancel any existing task (person still talking)
        let (buffered_text, is_new_segment) = {
            let mut state = speech_final_state.write();

            // CRITICAL: Cancel old task when new is_final arrives - this implements the "timer reset"
            // The new is_final indicates the person is still talking, so we cancel the old timer
            // and start a fresh 2800ms timer below
            if let Some(old_handle) = state.turn_detection_handle.take() {
                info!(
                    "Smart fallback timer reset: new is_final arrived, cancelling previous timer and starting fresh"
                );
                old_handle.abort();
            }

            state.text_buffer.push_str(&result.transcript);

            // Check if this is the first is_final for a new segment
            let is_new = state.segment_start_ms.load(Ordering::Acquire) == 0;

            (state.text_buffer.clone(), is_new)
        };

        // Create and store NEW detection task handle
        // When stt-vad is enabled and components are available, use smart fallback
        #[cfg(feature = "stt-vad")]
        let detection_handle = {
            if let (Some(turn_detector), Some(silence_tracker), Some(vad_state)) =
                (&self.turn_detector, &self.silence_tracker, &self.vad_state)
            {
                turn_detection_tasks::create_smart_fallback_task(
                    &self.config,
                    buffered_text.clone(),
                    speech_final_state.clone(),
                    turn_detector.clone(),
                    silence_tracker.clone(),
                    vad_state.clone(),
                )
            } else {
                turn_detection_tasks::create_detection_task(
                    &self.config,
                    buffered_text.clone(),
                    speech_final_state.clone(),
                )
            }
        };

        #[cfg(not(feature = "stt-vad"))]
        let detection_handle = turn_detection_tasks::create_detection_task(
            &self.config,
            buffered_text,
            speech_final_state.clone(),
        );

        let mut state = speech_final_state.write();
        state.turn_detection_handle = Some(detection_handle);
        state
            .waiting_for_speech_final
            .store(true, Ordering::Release);

        // Schedule hard-timeout task if this is a new segment
        if is_new_segment {
            let now_ms = get_current_time_ms();
            state.segment_start_ms.store(now_ms, Ordering::Release);

            let deadline_ms = now_ms + self.config.speech_final_hard_timeout_ms as usize;
            state
                .hard_timeout_deadline_ms
                .store(deadline_ms, Ordering::Release);

            debug!(
                "Starting new speech segment - hard timeout will fire in {}ms at {}",
                self.config.speech_final_hard_timeout_ms, deadline_ms
            );

            // Cancel any existing hard timeout task
            if let Some(old_handle) = state.hard_timeout_handle.take() {
                debug!("Cancelling previous hard timeout task");
                old_handle.abort();
            }

            // Spawn hard timeout task
            let hard_timeout_handle = turn_detection_tasks::create_hard_timeout_task(
                &self.config,
                speech_final_state.clone(),
                now_ms,
            );
            state.hard_timeout_handle = Some(hard_timeout_handle);
        }
    }

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
                let is_mid_turn = {
                    let mut state = speech_final_state.write();
                    let is_mid_turn = state.waiting_for_speech_final.load(Ordering::Acquire);
                    state.reset_vad_state();
                    is_mid_turn
                };

                if is_mid_turn {
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

            let audio = audio_buffer.clone();

            state.vad_turn_end_detected.store(true, Ordering::Release);

            if let Some(old_handle) = state.turn_detection_handle.take() {
                debug!("VAD: Cancelling timeout-based turn detection task");
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
