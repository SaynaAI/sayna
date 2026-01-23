//! Speech final firing and Smart Turn result handling.
//!
//! Contains functions to fire speech_final events and process the result
//! of Smart Turn detection.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tracing::{debug, info};

use crate::core::stt::STTResult;
use crate::core::vad::SilenceTracker;

use super::super::state::SpeechFinalState;
use super::super::utils::get_current_time_ms;
use super::super::vad_processor::VADState;
use super::backup_timeout::spawn_backup_timeout_task;
use super::smart_turn::SmartTurnResult;

/// Fire a forced speech_final event.
///
/// Updates state and invokes the user callback with an artificial speech_final result.
///
/// IMPORTANT: The callback must be invoked BEFORE calling reset_after_firing(),
/// because reset_after_firing() aborts the vad_turn_detection_handle.
pub async fn fire_speech_final(
    buffered_text: String,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    detection_method: &str,
) {
    let callback_opt = {
        let state = speech_final_state.read();
        if state.waiting_for_speech_final.load(Ordering::Acquire) {
            state.user_callback.clone()
        } else {
            None
        }
    };

    if let Some(callback) = callback_opt {
        let fire_time_ms = get_current_time_ms();

        let forced_result = STTResult {
            transcript: String::new(),
            is_final: true,
            is_speech_final: true,
            confidence: 1.0,
        };

        info!("Forcing speech_final via {}", detection_method);

        callback(forced_result).await;

        {
            let mut state = speech_final_state.write();
            state
                .turn_detection_last_fired_ms
                .store(fire_time_ms, Ordering::Release);
            state.last_forced_text = buffered_text.clone();
            state.reset_after_firing();
        }
    }
}

/// Handle the result of Smart Turn detection.
///
/// - `Complete`: Fire speech_final, reset silence tracker, clear audio buffer
/// - `Incomplete`: Reset VAD turn_end state, allow TurnEnd retry, start backup timeout
/// - `Skipped`: Fire speech_final with fallback method, reset tracker, clear buffer
#[allow(clippy::too_many_arguments)]
pub async fn handle_smart_turn_result(
    smart_turn_result: SmartTurnResult,
    buffered_text: String,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    silence_tracker: Arc<SilenceTracker>,
    vad_state: Arc<VADState>,
    retry_silence_duration_ms: u64,
    backup_silence_timeout_ms: u64,
    method_prefix: &str,
    skipped_fallback_method: &str,
) {
    let should_clear_audio_buffer = match smart_turn_result {
        SmartTurnResult::Complete(detection_method) => {
            let final_method = if method_prefix.is_empty() {
                detection_method
            } else {
                match detection_method {
                    "smart_turn_confirmed" => "vad_smart_turn_confirmed",
                    "smart_turn_error_fallback" => "vad_smart_turn_error_fallback",
                    "smart_turn_timeout_fallback" => "vad_inference_timeout_fallback",
                    "timeout_no_detector" => "vad_silence_only",
                    other => other,
                }
            };

            info!(
                "Smart Turn confirmed turn complete via '{}' - firing speech_final",
                final_method
            );

            fire_speech_final(buffered_text, speech_final_state, final_method).await;
            true
        }
        SmartTurnResult::Incomplete => {
            info!("Smart Turn says turn incomplete - enabling retry and starting backup timeout");

            // Clear vad_turn_end_detected to allow TurnEnd to fire again
            {
                let mut state = speech_final_state.write();
                state.vad_turn_end_detected.store(false, Ordering::Release);

                // Abort any existing backup timeout before starting a new one
                if let Some(handle) = state.backup_timeout_handle.take() {
                    handle.abort();
                }

                // Spawn backup timeout task if enabled (timeout_ms > 0)
                if backup_silence_timeout_ms > 0 {
                    let handle = spawn_backup_timeout_task(
                        backup_silence_timeout_ms,
                        buffered_text,
                        speech_final_state.clone(),
                        silence_tracker.clone(),
                        vad_state.clone(),
                    );
                    state.backup_timeout_handle = Some(handle);
                    debug!(
                        "Started backup timeout task ({}ms)",
                        backup_silence_timeout_ms
                    );
                }
            }

            // Allow SilenceTracker to emit TurnEnd again after additional silence
            silence_tracker.allow_turn_end_retry(retry_silence_duration_ms);

            false
        }
        SmartTurnResult::Skipped => {
            info!(
                "Smart Turn skipped (insufficient audio) - firing speech_final via {}",
                skipped_fallback_method
            );

            fire_speech_final(buffered_text, speech_final_state, skipped_fallback_method).await;
            true
        }
    };

    // Only reset on completion; preserves audio context for brief pauses (Pipecat issue #3094)
    if should_clear_audio_buffer {
        silence_tracker.reset();
        vad_state.clear_audio_buffer();
        debug!("Reset silence tracker and cleared audio buffer after speech_final");
    }
}
