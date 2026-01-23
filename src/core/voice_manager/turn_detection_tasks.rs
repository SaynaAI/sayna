//! Turn detection task spawning logic
//!
//! This module contains async task creation for VAD-triggered turn detection.
//! When `stt-vad` is enabled, this is the SOLE source of `is_speech_final=true` events.
//! STT provider's `is_speech_final` flag is ignored and overridden to false.

#[cfg(feature = "stt-vad")]
use parking_lot::RwLock as SyncRwLock;
#[cfg(feature = "stt-vad")]
use std::sync::Arc;
#[cfg(feature = "stt-vad")]
use std::sync::atomic::Ordering;
#[cfg(feature = "stt-vad")]
use tokio::task::JoinHandle;
#[cfg(feature = "stt-vad")]
use tokio::time::Duration;
#[cfg(feature = "stt-vad")]
use tracing::{debug, info};

#[cfg(feature = "stt-vad")]
use crate::core::stt::STTResult;
#[cfg(feature = "stt-vad")]
use crate::core::turn_detect::TurnDetector;
#[cfg(feature = "stt-vad")]
use crate::core::vad::SilenceTracker;
#[cfg(feature = "stt-vad")]
use tokio::sync::RwLock;

#[cfg(feature = "stt-vad")]
use super::state::SpeechFinalState;
#[cfg(feature = "stt-vad")]
use super::stt_config::STTProcessingConfig;
#[cfg(feature = "stt-vad")]
use super::utils::MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION;
#[cfg(feature = "stt-vad")]
use super::utils::get_current_time_ms;
#[cfg(feature = "stt-vad")]
use super::vad_processor::VADState;

/// Fire a forced speech_final event.
///
/// This function updates state and invokes the user callback with an artificial
/// speech_final result. Used by the VAD-triggered turn detection path.
///
/// IMPORTANT: The callback must be invoked BEFORE calling reset_after_firing(),
/// because reset_after_firing() aborts the vad_turn_detection_handle - which is
/// the current task. If we abort before the await, the callback never executes.
#[cfg(feature = "stt-vad")]
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

        // Create the forced result
        let forced_result = STTResult {
            transcript: String::new(),
            is_final: true,
            is_speech_final: true,
            confidence: 1.0,
        };

        info!("Forcing speech_final via {}", detection_method);

        // CRITICAL: Call the callback FIRST, before resetting state.
        // reset_after_firing() calls reset_vad_state() which aborts the current task's handle.
        // If we abort before the await, the callback invocation is cancelled.
        callback(forced_result).await;

        // Now update state AFTER the callback has completed
        {
            let mut state = speech_final_state.write();

            // Set stats for duplicate detection
            state
                .turn_detection_last_fired_ms
                .store(fire_time_ms, Ordering::Release);
            state.last_forced_text = buffered_text.clone();

            // Reset all other state (preserves last_fired_ms and last_forced_text)
            // This will abort vad_turn_detection_handle, but the task is already finishing
            state.reset_after_firing();
        }
    }
}

/// Result of running Smart Turn detection.
#[cfg(feature = "stt-vad")]
pub enum SmartTurnResult {
    /// Turn is complete, fire speech_final with given detection method.
    Complete(&'static str),
    /// Turn is incomplete, reset and wait.
    Incomplete,
    /// Turn detection could not run (no detector, error, or timeout).
    Skipped,
}

/// Run Smart Turn detection on the audio buffer.
///
/// This shared function encapsulates the logic for running the Smart Turn model
/// on accumulated audio. It is used by both VAD-triggered events and the
/// smart fallback timer.
///
/// The `inference_timeout_ms` parameter (default 800ms from `SpeechFinalConfig`)
/// wraps the detection call. Because `TurnDetector::is_turn_complete` runs the
/// CPU-bound feature extraction and model inference in `spawn_blocking`, this
/// timeout can fire even when the blocking thread is busy.
///
/// # Returns
/// - `SmartTurnResult::Complete(method)` if turn is complete
/// - `SmartTurnResult::Incomplete` if turn is not complete (should reset and wait)
/// - `SmartTurnResult::Skipped` if detection could not run
#[cfg(feature = "stt-vad")]
async fn run_smart_turn_detection(
    inference_timeout_ms: u64,
    audio_samples: &[i16],
    turn_detector: Option<Arc<RwLock<TurnDetector>>>,
) -> SmartTurnResult {
    if audio_samples.len() < MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION {
        debug!(
            "Smart-turn: Audio buffer too short ({} samples < {} min) - skipping",
            audio_samples.len(),
            MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION
        );
        return SmartTurnResult::Skipped;
    }

    let Some(detector) = turn_detector else {
        info!("Smart-turn: No turn detector - firing based on timeout alone");
        return SmartTurnResult::Complete("timeout_no_detector");
    };

    let sample_count = audio_samples.len();
    let audio_vec = audio_samples.to_vec();

    let turn_result = tokio::time::timeout(Duration::from_millis(inference_timeout_ms), async {
        let detector_guard = detector.read().await;
        detector_guard.is_turn_complete(&audio_vec).await
    })
    .await;

    match turn_result {
        Ok(Ok(true)) => {
            info!(
                "Smart-turn: Turn complete confirmed for {} audio samples",
                sample_count
            );
            SmartTurnResult::Complete("smart_turn_confirmed")
        }
        Ok(Ok(false)) => {
            info!("Smart-turn: Model says turn incomplete - waiting for more input");
            SmartTurnResult::Incomplete
        }
        Ok(Err(e)) => {
            tracing::warn!("Smart-turn detection error: {:?} - firing anyway", e);
            SmartTurnResult::Complete("smart_turn_error_fallback")
        }
        Err(_) => {
            tracing::warn!(
                "Smart-turn detection timeout after {}ms - firing anyway",
                inference_timeout_ms
            );
            SmartTurnResult::Complete("smart_turn_timeout_fallback")
        }
    }
}

/// Handle the result of Smart Turn detection.
///
/// This function consolidates the common logic for handling `SmartTurnResult` from both
/// VAD-triggered events and the smart fallback timer. It handles:
/// - `Complete`: Fire speech_final, reset silence tracker, clear audio buffer
/// - `Incomplete`: Reset VAD turn_end state, reset silence tracker
/// - `Skipped`: Fire speech_final with fallback method, reset tracker, clear buffer
///
/// # Arguments
/// * `smart_turn_result` - The result from `run_smart_turn_detection`
/// * `buffered_text` - The accumulated text for this speech segment
/// * `speech_final_state` - Shared state for speech final tracking
/// * `silence_tracker` - Tracker to reset on completion
/// * `vad_state` - VAD state containing audio buffer to clear on completion
/// * `method_prefix` - Optional prefix for detection method (e.g., "vad_" for VAD-triggered)
/// * `skipped_fallback_method` - Method name to use when SmartTurnResult::Skipped
#[cfg(feature = "stt-vad")]
async fn handle_smart_turn_result(
    smart_turn_result: SmartTurnResult,
    buffered_text: String,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    silence_tracker: Arc<SilenceTracker>,
    vad_state: Arc<VADState>,
    method_prefix: &str,
    skipped_fallback_method: &str,
) {
    // Track whether to clear audio buffer (Complete and Skipped do, Incomplete does not)
    let should_clear_audio_buffer = match smart_turn_result {
        SmartTurnResult::Complete(detection_method) => {
            // Apply prefix if provided
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
            info!("Smart Turn says turn incomplete - resetting VAD state for next attempt");
            {
                let state = speech_final_state.write();
                state.vad_turn_end_detected.store(false, Ordering::Release);
            }
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

    // Only reset silence tracker and clear audio buffer when turn is confirmed complete.
    // IMPORTANT: Do NOT reset on Incomplete - this preserves audio context so the model
    // can distinguish brief pauses from actual turn ends (fixes Pipecat issue #3094).
    if should_clear_audio_buffer {
        silence_tracker.reset();
        vad_state.clear_audio_buffer();
        debug!("Reset silence tracker and cleared audio buffer after speech_final");
    }
}

/// Create a VAD-triggered turn detection task.
///
/// This task is spawned when VAD detects sufficient silence (TurnEnd event).
/// It runs the Smart Turn model on the accumulated audio buffer to confirm
/// whether the speaker has finished their turn.
///
/// # Arguments
/// * `config` - STT processing configuration (used for inference timeout)
/// * `audio_samples` - Accumulated audio samples for turn detection
/// * `buffered_text` - The accumulated text for this speech segment
/// * `speech_final_state` - Shared state for speech final tracking
/// * `turn_detector` - Optional turn detector for Smart Turn analysis
/// * `silence_tracker` - Tracker to reset on completion
/// * `vad_state` - VAD state containing audio buffer to clear on completion
#[cfg(feature = "stt-vad")]
pub fn create_vad_turn_detection_task(
    config: &STTProcessingConfig,
    audio_samples: Vec<i16>,
    buffered_text: String,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    silence_tracker: Arc<SilenceTracker>,
    vad_state: Arc<VADState>,
) -> JoinHandle<()> {
    let inference_timeout_ms = config.turn_detection_inference_timeout_ms;

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

        // Use consolidated handler - "vad_" prefix for VAD-triggered, "vad_silence_only" for skipped
        handle_smart_turn_result(
            smart_turn_result,
            buffered_text,
            speech_final_state,
            silence_tracker,
            vad_state,
            "vad_",             // Prefix for VAD-triggered events
            "vad_silence_only", // Fallback method when skipped
        )
        .await;
    })
}
