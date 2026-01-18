//! STT result processing with timing control.
//!
//! Handles STT results with configurable timing, supporting both timeout-based
//! and VAD-based silence detection. When `stt-vad` is compiled, VAD mode is
//! always active. See CLAUDE.md for detailed architecture documentation.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::RwLock;
use tracing::debug;

use crate::core::stt::STTResult;
use crate::core::turn_detect::TurnDetector;
#[cfg(feature = "stt-vad")]
use crate::core::vad::{SilenceTracker, VADEvent};

use super::state::SpeechFinalState;
use super::stt_config::STTProcessingConfig;
use super::turn_detection_tasks;

/// Processor for STT results with timing control
#[derive(Clone)]
pub struct STTResultProcessor {
    config: STTProcessingConfig,
}

impl STTResultProcessor {
    pub fn new(config: STTProcessingConfig) -> Self {
        Self { config }
    }

    /// Process an STT result, handling turn detection and speech_final events.
    pub async fn process_result(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        _turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) -> Option<STTResult> {
        // Fast synchronous checks - no awaits
        if !self.should_deliver_result(&result) {
            return None;
        }

        let now_ms = self.get_current_time_ms();

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
    #[cfg(feature = "stt-vad")]
    pub fn process_vad_event(
        &self,
        event: VADEvent,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        silence_tracker: Arc<SilenceTracker>,
        vad_audio_buffer: Arc<SyncRwLock<Vec<i16>>>,
    ) {
        super::vad_processor::process_vad_event(
            &self.config,
            event,
            speech_final_state,
            turn_detector,
            silence_tracker,
            vad_audio_buffer,
        );
    }

    /// Check if VAD-based silence detection is enabled.
    #[cfg(feature = "stt-vad")]
    pub fn is_vad_enabled(&self) -> bool {
        super::vad_processor::is_vad_enabled()
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
        // Update text buffer and cancel any existing task (person still talking)
        let (buffered_text, is_new_segment) = {
            let mut state = speech_final_state.write();

            // CRITICAL: Cancel old task when new is_final arrives (person still talking)
            if let Some(old_handle) = state.turn_detection_handle.take() {
                debug!(
                    "New is_final arrived - cancelling previous turn detection (person still talking)"
                );
                old_handle.abort();
            }

            state.text_buffer = format!("{}{}", state.text_buffer, result.transcript);

            // Check if this is the first is_final for a new segment
            let is_new = state.segment_start_ms.load(Ordering::Acquire) == 0;

            (state.text_buffer.clone(), is_new)
        };

        // Create and store NEW detection task handle
        let detection_handle = turn_detection_tasks::create_detection_task(
            &self.config,
            result,
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
            let now_ms = self.get_current_time_ms();
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

        // Cancel any pending detection tasks
        self.cancel_detection_task(&mut state);

        // Reset state for next speech segment
        self.reset_speech_state(&mut state);

        Some(result)
    }

    /// Fire a forced speech_final event via callback.
    pub async fn fire_speech_final(
        result: STTResult,
        buffered_text: String,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        detection_method: &str,
    ) {
        turn_detection_tasks::fire_speech_final(
            result,
            buffered_text,
            speech_final_state,
            detection_method,
        )
        .await;
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

    fn cancel_detection_task(&self, state: &mut SpeechFinalState) {
        if let Some(handle) = state.turn_detection_handle.take() {
            debug!("Cancelling pending turn detection task");
            handle.abort();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);
        }

        // Also cancel hard timeout handle if present
        if let Some(handle) = state.hard_timeout_handle.take() {
            debug!("Cancelling pending hard timeout task");
            handle.abort();
        }

        // Cancel VAD turn detection handle if present
        if let Some(handle) = state.vad_turn_detection_handle.take() {
            debug!("Cancelling pending VAD turn detection task");
            handle.abort();
        }

        // Reset VAD state
        state.vad_turn_end_detected.store(false, Ordering::Release);
    }

    fn reset_speech_state(&self, state: &mut SpeechFinalState) {
        state.text_buffer.clear();
        state.last_forced_text.clear();
        state
            .waiting_for_speech_final
            .store(false, Ordering::Release);
        state
            .turn_detection_last_fired_ms
            .store(0, Ordering::Release);
        state.segment_start_ms.store(0, Ordering::Release);
        state.hard_timeout_deadline_ms.store(0, Ordering::Release);

        // Cancel and clear hard timeout handle if present
        if let Some(handle) = state.hard_timeout_handle.take() {
            handle.abort();
        }

        // Reset VAD state
        state.reset_vad_state();
    }

    fn get_current_time_ms(&self) -> usize {
        Self::get_current_time_ms_static()
    }

    fn get_current_time_ms_static() -> usize {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as usize
    }
}

impl Default for STTResultProcessor {
    fn default() -> Self {
        Self::new(STTProcessingConfig::default())
    }
}
