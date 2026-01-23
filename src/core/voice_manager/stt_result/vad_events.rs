//! VAD event handling for STT result processing.
//!
//! This module contains all VAD-specific event handling code that is feature-gated
//! behind `stt-vad`. It extends `STTResultProcessor` with methods for processing
//! VAD events and triggering turn detection based on silence.
//!
//! The separation follows the CLAUDE.md architecture guidance for VoiceManager
//! which notes that "VAD + Turn Detection" is feature-gated under `stt-vad`.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::{debug, info};

use crate::core::vad::{SilenceTracker, VADEvent};

use super::super::state::SpeechFinalState;
use super::super::turn_detection_tasks;
use super::super::utils::MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION;
use super::super::vad_processor::VADState;
use super::processor::STTResultProcessor;

impl STTResultProcessor {
    /// Process a VAD event, triggering turn detection on silence.
    ///
    /// This method requires that the processor was initialized with VAD components
    /// via `with_vad_components`. If VAD components are not available, the event
    /// is ignored with a debug log.
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

    /// Handle a VAD event internally, processing speech start/resume/silence/turn-end.
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
