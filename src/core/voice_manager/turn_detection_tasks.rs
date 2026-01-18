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
use super::stt_result::STTResultProcessor;

/// Fallback timer duration in milliseconds.
///
/// After receiving an STT "final" result, if "speech_final" is not received within
/// this timeout, the Smart Turn model is executed to determine turn completion.
#[cfg(feature = "stt-vad")]
pub const SMART_TURN_FALLBACK_MS: u64 = 2800;

/// Get current time in milliseconds since Unix epoch.
fn get_current_time_ms() -> usize {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as usize
}

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

        // Fire speech_final with empty result (we'll use buffered text)
        let forced_result = STTResult {
            transcript: String::new(),
            is_final: true,
            is_speech_final: false,
            confidence: 1.0,
        };

        fire_speech_final(
            forced_result,
            buffered_text,
            speech_final_state,
            "hard_timeout_fallback",
        )
        .await;
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
    _result: STTResult,
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
        let detection_method = "timeout_fallback";
        info!(
            "STT silent for {}ms, text unchanged - firing artificial speech_final",
            stt_wait_ms
        );

        // PHASE 3: Fire artificial speech_final
        let result = STTResult {
            transcript: String::new(),
            is_final: true,
            is_speech_final: false,
            confidence: 1.0,
        };
        fire_speech_final(result, buffered_text, speech_final_state, detection_method).await;
    })
}

/// Fire a forced speech_final event.
///
/// This function updates state and invokes the user callback with an artificial
/// speech_final result. Used by both timeout-based and VAD-based paths.
pub async fn fire_speech_final(
    _result: STTResult,
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
            state
                .turn_detection_last_fired_ms
                .store(fire_time_ms, Ordering::Release);
            state.last_forced_text = buffered_text.clone();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);
            state.turn_detection_handle = None;
            state.text_buffer.clear();

            // Cancel and clear hard timeout handle
            if let Some(handle) = state.hard_timeout_handle.take() {
                handle.abort();
            }

            // Clear segment timing for next utterance
            state.segment_start_ms.store(0, Ordering::Release);
            state.hard_timeout_deadline_ms.store(0, Ordering::Release);

            // Reset VAD state for next utterance
            state.reset_vad_state();
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
pub async fn run_smart_turn_detection(
    inference_timeout_ms: u64,
    audio_samples: &[i16],
    turn_detector: Option<Arc<RwLock<TurnDetector>>>,
) -> SmartTurnResult {
    const MIN_AUDIO_SAMPLES: usize = 8000; // 0.5 seconds at 16kHz

    if audio_samples.len() < MIN_AUDIO_SAMPLES {
        debug!(
            "Smart-turn: Audio buffer too short ({} samples < {} min) - skipping",
            audio_samples.len(),
            MIN_AUDIO_SAMPLES
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

/// Create a smart fallback task that uses the Smart Turn model after a timeout.
///
/// This task implements a 2800ms fallback timer that:
/// 1. Waits for `SMART_TURN_FALLBACK_MS` after an STT "final" result
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
    vad_audio_buffer: Arc<SyncRwLock<Vec<i16>>>,
) -> JoinHandle<()> {
    let inference_timeout_ms = config.turn_detection_inference_timeout_ms;

    tokio::spawn(async move {
        // PHASE 1: Wait for smart fallback timeout (2800ms)
        info!(
            "Smart fallback timer started: waiting {}ms for speech_final or new is_final",
            SMART_TURN_FALLBACK_MS
        );
        tokio::time::sleep(Duration::from_millis(SMART_TURN_FALLBACK_MS)).await;

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
            SMART_TURN_FALLBACK_MS
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
            SMART_TURN_FALLBACK_MS
        );
        let audio_samples = {
            let buffer = vad_audio_buffer.read();
            buffer.clone()
        };

        if audio_samples.is_empty() {
            debug!("Smart fallback: Audio buffer is empty - using timeout fallback");
            // Fall through to timeout-only speech_final
            let result = STTResult {
                transcript: String::new(),
                is_final: true,
                is_speech_final: false,
                confidence: 1.0,
            };
            fire_speech_final(
                result,
                buffered_text,
                speech_final_state,
                "timeout_empty_buffer",
            )
            .await;
            return;
        }

        let smart_turn_result =
            run_smart_turn_detection(inference_timeout_ms, &audio_samples, Some(turn_detector))
                .await;

        match smart_turn_result {
            SmartTurnResult::Complete(detection_method) => {
                info!(
                    "Smart fallback: Smart Turn confirmed turn complete via '{}' - firing speech_final",
                    detection_method
                );
                let result = STTResult {
                    transcript: String::new(),
                    is_final: true,
                    is_speech_final: false,
                    confidence: 1.0,
                };
                STTResultProcessor::fire_speech_final(
                    result,
                    buffered_text,
                    speech_final_state,
                    detection_method,
                )
                .await;

                // Reset silence tracker and clear audio buffer
                silence_tracker.reset();
                {
                    let mut buffer = vad_audio_buffer.write();
                    buffer.clear();
                }
                debug!(
                    "Smart fallback: Reset silence tracker and cleared audio buffer after speech_final"
                );
            }
            SmartTurnResult::Incomplete => {
                // Smart Turn says incomplete - reset VAD state and wait for more input
                info!(
                    "Smart fallback: Smart Turn says turn incomplete - resetting VAD state for next attempt"
                );
                {
                    let state = speech_final_state.write();
                    state.vad_turn_end_detected.store(false, Ordering::Release);
                }
                silence_tracker.reset();
            }
            SmartTurnResult::Skipped => {
                // Could not run Smart Turn (e.g., buffer too short) - use timeout fallback
                info!(
                    "Smart fallback: Smart Turn skipped (insufficient audio) - firing speech_final via timeout_fallback"
                );
                let result = STTResult {
                    transcript: String::new(),
                    is_final: true,
                    is_speech_final: false,
                    confidence: 1.0,
                };
                fire_speech_final(
                    result,
                    buffered_text,
                    speech_final_state,
                    "timeout_fallback",
                )
                .await;
            }
        }
    })
}
