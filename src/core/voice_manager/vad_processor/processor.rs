//! VAD audio processing for silence detection.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::debug;

use crate::core::vad::{SilenceTracker, SileroVAD, VADEvent};

use super::super::errors::{VoiceManagerError, VoiceManagerResult};
use super::super::utils::MAX_VAD_AUDIO_SAMPLES;
use super::state::VADState;

/// Process an audio chunk through VAD for silence detection.
///
/// Converts raw PCM bytes to i16 samples and:
/// 1. Accumulates samples in the ring buffer for turn detection
/// 2. Processes complete frames through VAD
///
/// Returns VAD events generated from processing.
pub async fn process_audio_chunk(
    audio: &[u8],
    vad: &Arc<RwLock<SileroVAD>>,
    vad_state: &Arc<VADState>,
    silence_tracker: &Arc<SilenceTracker>,
) -> VoiceManagerResult<Vec<VADEvent>> {
    #[cfg(test)]
    if vad_state.is_force_error() {
        return Err(VoiceManagerError::InitializationError(
            "Forced VAD error for testing".to_string(),
        ));
    }

    let samples: Cow<'_, [i16]> = bytes_to_samples(audio);

    let frame_size = {
        let vad_guard = vad.read().await;
        vad_guard.frame_size()
    };

    accumulate_samples(vad_state, &samples);

    let (frames_data, num_frames) = drain_complete_frames(vad_state, &samples, frame_size);

    let mut events = Vec::new();

    // Clone model reference once to avoid holding async RwLock across inference awaits.
    let vad_model = {
        let vad_guard = vad.read().await;
        vad_guard.get_model()
    };

    for i in 0..num_frames {
        let frame_start = i * frame_size;
        let frame = &frames_data[frame_start..frame_start + frame_size];

        let inference_start = Instant::now();
        let speech_prob = SileroVAD::process_audio_with_model(Arc::clone(&vad_model), frame)
            .await
            .map_err(|e| {
                VoiceManagerError::InitializationError(format!("VAD processing error: {}", e))
            })?;
        let inference_duration = inference_start.elapsed();

        let current_buffer_len = vad_state.audio_buffer.read().len();
        debug!(
            frame_samples = frame_size,
            inference_ms = inference_duration.as_secs_f64() * 1000.0,
            speech_prob = speech_prob,
            buffer_samples = current_buffer_len,
            buffer_max = MAX_VAD_AUDIO_SAMPLES,
            "VAD inference completed"
        );

        if let Some(event) = silence_tracker.process(speech_prob) {
            events.push(event);
        }
    }

    Ok(events)
}

/// Accumulate audio samples into the ring buffer, dropping oldest when full.
fn accumulate_samples(vad_state: &Arc<VADState>, samples: &[i16]) {
    let mut audio_buffer = vad_state.audio_buffer.write();

    if samples.len() >= MAX_VAD_AUDIO_SAMPLES {
        if !vad_state.swap_buffer_limit_warned() {
            debug!(
                "Received oversized audio chunk ({} samples > {} max). \
                 Keeping only the most recent {} samples for turn detection.",
                samples.len(),
                MAX_VAD_AUDIO_SAMPLES,
                MAX_VAD_AUDIO_SAMPLES
            );
        }
        audio_buffer.clear();
        let start_idx = samples.len() - MAX_VAD_AUDIO_SAMPLES;
        audio_buffer.extend(samples[start_idx..].iter().copied());
    } else {
        let new_total = audio_buffer.len() + samples.len();
        if new_total > MAX_VAD_AUDIO_SAMPLES {
            let samples_to_drop = new_total - MAX_VAD_AUDIO_SAMPLES;
            let drop_count = samples_to_drop.min(audio_buffer.len());
            audio_buffer.drain(..drop_count);

            if !vad_state.swap_buffer_limit_warned() {
                debug!(
                    "VAD audio ring buffer at capacity ({} samples / 30 seconds). \
                     Dropping oldest samples to keep most recent audio for turn detection.",
                    MAX_VAD_AUDIO_SAMPLES
                );
            }
        }
        audio_buffer.extend(samples.iter().copied());
    }
}

/// Drain complete frames from the frame buffer into a contiguous buffer.
fn drain_complete_frames(
    vad_state: &Arc<VADState>,
    samples: &[i16],
    frame_size: usize,
) -> (Vec<i16>, usize) {
    let mut frame_buffer = vad_state.frame_buffer.write();
    frame_buffer.extend_from_slice(samples);

    let num_complete_frames = frame_buffer.len() / frame_size;
    let samples_to_drain = num_complete_frames * frame_size;

    let frames_data: Vec<i16> = frame_buffer.drain(..samples_to_drain).collect();
    (frames_data, num_complete_frames)
}

/// Convert raw PCM bytes to i16 samples.
///
/// Uses `align_to` for zero-allocation when input is properly aligned,
/// falling back to allocation-based conversion for unaligned data.
/// Trailing bytes (if odd length) are ignored, matching `chunks_exact(2)` behavior.
#[inline]
pub(crate) fn bytes_to_samples(audio: &[u8]) -> Cow<'_, [i16]> {
    // SAFETY: align_to checks alignment and returns (prefix, aligned, suffix).
    // When prefix is empty, the slice is properly aligned for i16 access.
    // We use the aligned portion directly without allocation.
    let (prefix, aligned, _suffix) = unsafe { audio.align_to::<i16>() };

    if prefix.is_empty() {
        // Audio is aligned - use directly without allocation.
        // Note: suffix contains any trailing byte (odd-length input), which is ignored
        // to match the original chunks_exact(2) behavior.
        Cow::Borrowed(aligned)
    } else {
        // Audio is unaligned - fall back to allocation-based conversion.
        // This handles cases where the byte slice doesn't start at an i16 boundary.
        let samples: Vec<i16> = audio
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        Cow::Owned(samples)
    }
}
