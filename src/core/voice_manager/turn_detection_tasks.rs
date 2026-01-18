//! Turn detection task spawning logic
//!
//! This module contains async task creation for turn detection timing control,
//! including timeout-based fallback detection and hard timeout enforcement.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::{debug, info};

use crate::core::stt::STTResult;
#[cfg(feature = "stt-vad")]
use crate::core::turn_detect::TurnDetector;
#[cfg(feature = "stt-vad")]
use crate::core::vad::SilenceTracker;
#[cfg(feature = "stt-vad")]
use tokio::sync::RwLock;

use super::state::SpeechFinalState;
use super::stt_config::STTProcessingConfig;
#[cfg(feature = "stt-vad")]
use super::utils::MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION;
use super::utils::get_current_time_ms;
#[cfg(feature = "stt-vad")]
use super::vad_processor::VADState;

/// Create a hard timeout task that enforces the maximum wait time.
///
/// This task ensures that every speech segment gets a speech_final within
/// `speech_final_hard_timeout_ms`, regardless of whether the STT provider
/// sends speech_final or the turn detector confirms.
pub fn create_hard_timeout_task(
    config: &STTProcessingConfig,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    segment_start_ms: usize,
) -> JoinHandle<()> {
    let hard_timeout_ms = config.speech_final_hard_timeout_ms;

    tokio::spawn(async move {
        // Calculate remaining time until hard timeout
        let now_ms = get_current_time_ms();
        let elapsed_ms = now_ms.saturating_sub(segment_start_ms);
        let remaining_ms = hard_timeout_ms.saturating_sub(elapsed_ms as u64);

        debug!(
            "Hard timeout scheduled: will fire in {}ms (total timeout: {}ms, elapsed: {}ms)",
            remaining_ms, hard_timeout_ms, elapsed_ms
        );

        // Sleep for the remaining time
        tokio::time::sleep(Duration::from_millis(remaining_ms)).await;

        // Check if we should still fire (not cancelled by real speech_final)
        let should_fire = {
            let state = speech_final_state.read();
            state.waiting_for_speech_final.load(Ordering::Acquire)
        };

        if !should_fire {
            debug!("Hard timeout cancelled - speech_final already fired");
            return;
        }

        // Hard timeout has fired - force speech_final
        let total_wait_ms = get_current_time_ms().saturating_sub(segment_start_ms);

        // Get the buffered text before firing
        let buffered_text = {
            let state = speech_final_state.read();
            state.text_buffer.clone()
        };

        tracing::warn!(
            "Hard timeout fired after {}ms - forcing speech_final (no real speech_final or turn detection confirmation received)",
            total_wait_ms
        );

        fire_speech_final(buffered_text, speech_final_state, "hard_timeout_fallback").await;
    })
}

/// Create a detection task that waits for STT provider, then fires as timeout fallback.
///
/// Voice AI Best Practice Logic:
/// 1. Wait for STT provider to send real speech_final (they see the audio stream)
/// 2. If STT is silent and text hasn't changed, fire artificial speech_final
///
/// Note: This path does NOT use the TurnDetector because it doesn't have access
/// to the audio buffer. Turn detection with audio is handled by the VAD path.
/// This timeout-based path serves as a fallback when VAD is not available or hasn't fired.
pub fn create_detection_task(
    config: &STTProcessingConfig,
    buffered_text: String,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
) -> JoinHandle<()> {
    let stt_wait_ms = config.stt_speech_final_wait_ms;

    tokio::spawn(async move {
        // PHASE 1: Wait for STT provider to send real speech_final
        // This is the primary path - we trust the STT provider first
        debug!(
            "Waiting {}ms for real speech_final from STT provider",
            stt_wait_ms
        );
        tokio::time::sleep(Duration::from_millis(stt_wait_ms)).await;

        // Check if we should still fire (not cancelled by real speech_final or new is_final)
        let should_continue = {
            let state = speech_final_state.read();
            state.waiting_for_speech_final.load(Ordering::Acquire)
        };

        if !should_continue {
            debug!("Turn detection cancelled - real speech_final arrived or new is_final");
            return;
        }

        // PHASE 2: STT didn't send speech_final - check if text changed
        // Check if text buffer has changed (new transcripts arrived)
        let current_text = {
            let state = speech_final_state.read();
            state.text_buffer.clone()
        };

        // If text changed, someone is still talking - don't fire
        if current_text != buffered_text {
            info!(
                "Text buffer changed during wait (old: '{}', new: '{}') - person still talking, not firing",
                buffered_text, current_text
            );
            return;
        }

        // Text hasn't changed - fire based on timeout
        // Note: Turn detection with audio is handled by VAD path, not here
        info!(
            "STT silent for {}ms, text unchanged - firing artificial speech_final",
            stt_wait_ms
        );

        // PHASE 3: Fire artificial speech_final
        fire_speech_final(buffered_text, speech_final_state, "timeout_fallback").await;
    })
}

/// Fire a forced speech_final event.
///
/// This function updates state and invokes the user callback with an artificial
/// speech_final result. Used by both timeout-based and VAD-based paths.
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

        // Update state before firing callback
        {
            let mut state = speech_final_state.write();

            // Set stats that reset_after_firing will preserve (for duplicate detection)
            state
                .turn_detection_last_fired_ms
                .store(fire_time_ms, Ordering::Release);
            state.last_forced_text = buffered_text.clone();

            // Reset all other state (preserves last_fired_ms and last_forced_text)
            state.reset_after_firing();
        }

        let forced_result = STTResult {
            transcript: String::new(),
            is_final: true,
            is_speech_final: true,
            confidence: 1.0,
        };

        info!("Forcing speech_final via {}", detection_method);
        callback(forced_result).await;
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

    // Reset silence tracker (always) and optionally clear audio buffer
    silence_tracker.reset();
    if should_clear_audio_buffer {
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

/// Create a smart fallback task that uses the Smart Turn model after a timeout.
///
/// This task implements a configurable fallback timer that:
/// 1. Waits for `stt_speech_final_wait_ms` after an STT "final" result
/// 2. If still waiting for speech_final and text unchanged, runs Smart Turn
/// 3. If Smart Turn confirms end-of-speech, forces speech_final
/// 4. If Smart Turn says incomplete, resets and waits for more input
#[cfg(feature = "stt-vad")]
pub fn create_smart_fallback_task(
    config: &STTProcessingConfig,
    buffered_text: String,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    turn_detector: Arc<RwLock<TurnDetector>>,
    silence_tracker: Arc<SilenceTracker>,
    vad_state: Arc<VADState>,
) -> JoinHandle<()> {
    let inference_timeout_ms = config.turn_detection_inference_timeout_ms;
    let smart_fallback_ms = config.stt_speech_final_wait_ms;

    tokio::spawn(async move {
        // PHASE 1: Wait for smart fallback timeout
        info!(
            "Smart fallback timer started: waiting {}ms for speech_final or new is_final",
            smart_fallback_ms
        );
        tokio::time::sleep(Duration::from_millis(smart_fallback_ms)).await;

        // Check if we should still proceed
        let should_continue = {
            let state = speech_final_state.read();
            state.waiting_for_speech_final.load(Ordering::Acquire)
        };

        if !should_continue {
            debug!("Smart fallback timer cancelled: speech_final already arrived before timeout");
            return;
        }

        // PHASE 2: Timer expired - check if text buffer has changed
        info!(
            "Smart fallback timer expired after {}ms - checking if text changed",
            smart_fallback_ms
        );

        let current_text = {
            let state = speech_final_state.read();
            state.text_buffer.clone()
        };

        if current_text != buffered_text {
            info!(
                "Smart fallback: Text buffer changed during wait (new is_final arrived) - timer effectively reset, not firing"
            );
            return;
        }

        // PHASE 3: Text unchanged after timeout - get audio samples and run Smart Turn
        info!(
            "Smart fallback: No new text after {}ms - executing Smart Turn detection",
            smart_fallback_ms
        );
        let audio_samples = {
            let buffer = vad_state.audio_buffer.read();
            buffer.clone()
        };

        if audio_samples.is_empty() {
            debug!("Smart fallback: Audio buffer is empty - using timeout fallback");
            fire_speech_final(buffered_text, speech_final_state, "timeout_empty_buffer").await;
            return;
        }

        let smart_turn_result =
            run_smart_turn_detection(inference_timeout_ms, &audio_samples, Some(turn_detector))
                .await;

        // Use consolidated handler - no prefix for smart fallback, "timeout_fallback" for skipped
        handle_smart_turn_result(
            smart_turn_result,
            buffered_text,
            speech_final_state,
            silence_tracker,
            vad_state,
            "",                 // No prefix for smart fallback path
            "timeout_fallback", // Fallback method when skipped
        )
        .await;
    })
}
