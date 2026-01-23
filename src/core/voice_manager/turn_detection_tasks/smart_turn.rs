//! Smart Turn model inference for turn detection.
//!
//! Contains `SmartTurnResult` and the inference function that runs the
//! Smart Turn model on accumulated audio to confirm turn completion.

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::Duration;
use tracing::{debug, info};

use crate::core::turn_detect::TurnDetector;

use super::super::utils::MIN_AUDIO_SAMPLES_FOR_TURN_DETECTION;

/// Result of running Smart Turn detection.
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
/// Returns `Complete(method)` if turn is complete, `Incomplete` if not,
/// or `Skipped` if detection could not run.
pub async fn run_smart_turn_detection(
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

    let turn_result = tokio::time::timeout(Duration::from_millis(inference_timeout_ms), async {
        let detector_guard = detector.read().await;
        detector_guard.is_turn_complete(audio_samples).await
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
