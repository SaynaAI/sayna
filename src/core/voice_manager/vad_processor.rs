//! VAD-based silence detection and turn processing
//!
//! This module provides VAD event processing for STT result handling.
//! It is feature-gated under `stt-vad`.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use crate::core::stt::STTResult;
use crate::core::turn_detect::TurnDetector;
use crate::core::vad::{SilenceTracker, VADEvent};

use super::state::SpeechFinalState;
use super::stt_config::STTProcessingConfig;
use super::stt_result::STTResultProcessor;
use super::turn_detection_tasks::{SmartTurnResult, run_smart_turn_detection};

/// Process a VAD event for silence detection.
///
/// This function should be called for every VAD event from the SilenceTracker.
/// When VAD detects silence exceeding the threshold (TurnEnd event), it triggers
/// turn detection on the accumulated text.
pub fn process_vad_event(
    config: &STTProcessingConfig,
    event: VADEvent,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    silence_tracker: Arc<SilenceTracker>,
    vad_audio_buffer: Arc<SyncRwLock<Vec<i16>>>,
) {
    match event {
        VADEvent::SpeechStart => {
            let state = speech_final_state.read();
            let is_mid_turn = state.waiting_for_speech_final.load(Ordering::Acquire);
            drop(state);

            if is_mid_turn {
                debug!(
                    "VAD: Speech started mid-turn - resetting VAD state but preserving audio buffer for context"
                );
                let mut state = speech_final_state.write();
                state.reset_vad_state();
            } else {
                debug!("VAD: New speech started - resetting VAD state and clearing audio buffer");
                let mut state = speech_final_state.write();
                state.reset_vad_state();
                let mut buffer = vad_audio_buffer.write();
                buffer.clear();
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
            handle_vad_turn_end(
                config,
                speech_final_state,
                turn_detector,
                silence_tracker,
                vad_audio_buffer,
            );
        }
    }
}

/// Handle VAD TurnEnd event by spawning turn detection.
fn handle_vad_turn_end(
    config: &STTProcessingConfig,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    silence_tracker: Arc<SilenceTracker>,
    vad_audio_buffer: Arc<SyncRwLock<Vec<i16>>>,
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

        let audio_buffer = vad_audio_buffer.read();
        if audio_buffer.is_empty() {
            debug!("VAD: TurnEnd received but audio buffer is empty - skipping");
            return;
        }

        const MIN_AUDIO_SAMPLES: usize = 8000; // 0.5 seconds at 16kHz
        if audio_buffer.len() < MIN_AUDIO_SAMPLES {
            debug!(
                "VAD: TurnEnd received but audio buffer too short ({} samples < {} min) - skipping",
                audio_buffer.len(),
                MIN_AUDIO_SAMPLES
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
        config.vad_silence_duration_ms,
        audio_samples.len()
    );

    let handle = spawn_vad_turn_detection_audio(
        config.turn_detection_inference_timeout_ms,
        audio_samples,
        buffered_text,
        speech_final_state.clone(),
        turn_detector,
        silence_tracker,
        vad_audio_buffer,
    );

    let mut state = speech_final_state.write();
    state.vad_turn_detection_handle = Some(handle);
}

/// Spawn turn detection task with audio input.
fn spawn_vad_turn_detection_audio(
    inference_timeout_ms: u64,
    audio_samples: Vec<i16>,
    buffered_text: String,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    silence_tracker: Arc<SilenceTracker>,
    vad_audio_buffer: Arc<SyncRwLock<Vec<i16>>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let should_continue = {
            let state = speech_final_state.read();
            state.waiting_for_speech_final.load(Ordering::Acquire)
                && state.vad_turn_end_detected.load(Ordering::Acquire)
        };

        if !should_continue {
            debug!("VAD turn detection cancelled - speech resumed or already handled");
            return;
        }

        // Use the shared run_smart_turn_detection function
        let smart_turn_result =
            run_smart_turn_detection(inference_timeout_ms, &audio_samples, turn_detector).await;

        match smart_turn_result {
            SmartTurnResult::Complete(detection_method) => {
                // Prefix detection method with "vad_" to indicate VAD-triggered
                let vad_method = match detection_method {
                    "smart_turn_confirmed" => "vad_smart_turn_confirmed",
                    "smart_turn_error_fallback" => "vad_smart_turn_error_fallback",
                    "smart_turn_timeout_fallback" => "vad_inference_timeout_fallback",
                    "timeout_no_detector" => "vad_silence_only",
                    other => other,
                };

                let result = STTResult {
                    transcript: String::new(),
                    is_final: true,
                    is_speech_final: true,
                    confidence: 1.0,
                };

                STTResultProcessor::fire_speech_final(
                    result,
                    buffered_text,
                    speech_final_state,
                    vad_method,
                )
                .await;

                silence_tracker.reset();
                {
                    let mut buffer = vad_audio_buffer.write();
                    buffer.clear();
                }
                info!("VAD: Reset silence tracker and cleared audio buffer after speech_final");
            }
            SmartTurnResult::Incomplete => {
                info!("VAD: Smart-turn says incomplete - waiting for more input");
                {
                    let state = speech_final_state.write();
                    state.vad_turn_end_detected.store(false, Ordering::Release);
                }
                silence_tracker.reset();
            }
            SmartTurnResult::Skipped => {
                // Should not happen for VAD path since we already check buffer length
                // in handle_vad_turn_end, but handle gracefully
                info!("VAD: Smart-turn skipped (buffer too short) - firing based on silence alone");
                let result = STTResult {
                    transcript: String::new(),
                    is_final: true,
                    is_speech_final: true,
                    confidence: 1.0,
                };

                STTResultProcessor::fire_speech_final(
                    result,
                    buffered_text,
                    speech_final_state,
                    "vad_silence_only",
                )
                .await;

                silence_tracker.reset();
                {
                    let mut buffer = vad_audio_buffer.write();
                    buffer.clear();
                }
                info!("VAD: Reset silence tracker and cleared audio buffer after speech_final");
            }
        }
    })
}

/// Check if VAD-based silence detection is enabled.
///
/// When `stt-vad` is compiled, VAD is always active and cannot be disabled at runtime.
pub fn is_vad_enabled() -> bool {
    true
}
