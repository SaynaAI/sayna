//! Turn detection task spawning logic.
//!
//! This module contains async task creation for VAD-triggered turn detection.
//! When `stt-vad` is enabled, this is the SOLE source of `is_speech_final=true` events.

#[cfg(feature = "stt-vad")]
mod smart_turn;
#[cfg(feature = "stt-vad")]
mod speech_final;

#[cfg(feature = "stt-vad")]
use parking_lot::RwLock as SyncRwLock;
#[cfg(feature = "stt-vad")]
use std::sync::Arc;
#[cfg(feature = "stt-vad")]
use std::sync::atomic::Ordering;
#[cfg(feature = "stt-vad")]
use tokio::sync::RwLock;
#[cfg(feature = "stt-vad")]
use tokio::task::JoinHandle;
#[cfg(feature = "stt-vad")]
use tracing::debug;

#[cfg(feature = "stt-vad")]
use crate::core::turn_detect::TurnDetector;
#[cfg(feature = "stt-vad")]
use crate::core::vad::SilenceTracker;

#[cfg(feature = "stt-vad")]
use super::state::SpeechFinalState;
#[cfg(feature = "stt-vad")]
use super::stt_config::STTProcessingConfig;
#[cfg(feature = "stt-vad")]
use super::vad_processor::VADState;

/// Create a VAD-triggered turn detection task.
///
/// Spawned when VAD detects sufficient silence (TurnEnd event). Runs the
/// Smart Turn model on accumulated audio to confirm turn completion.
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

        let smart_turn_result = smart_turn::run_smart_turn_detection(
            inference_timeout_ms,
            &audio_samples,
            turn_detector,
        )
        .await;

        speech_final::handle_smart_turn_result(
            smart_turn_result,
            buffered_text,
            speech_final_state,
            silence_tracker,
            vad_state,
            "vad_",
            "vad_silence_only",
        )
        .await;
    })
}
