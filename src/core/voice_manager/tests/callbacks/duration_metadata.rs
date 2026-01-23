//! Tests for duration metadata handling in TTS callbacks.
//!
//! Per CLAUDE.md "Testing" section, unit tests validate component behavior.
//! Per CLAUDE.md "Core Components - VoiceManager" section, the VoiceManager
//! handles callbacks for STT results and audio output.

use std::sync::atomic::Ordering;

use crate::core::tts::AudioCallback;

use super::helpers::{
    audio_with_duration, interruption_state_disallowed, pcm_audio, tts_callback_with_state,
};

/// Test that duration_ms metadata is preferred over PCM calculation.
///
/// This validates that provider-supplied duration metadata is used when available.
#[tokio::test]
async fn test_duration_ms_metadata_preferred() {
    let state = interruption_state_disallowed(1000, 24000);
    let callback = tts_callback_with_state(state.clone());

    // Audio with explicit duration_ms that differs from what PCM calculation would yield.
    // PCM calc for 4800 bytes @ 24000Hz = 100ms, but we provide 250ms via metadata.
    let audio_with_metadata = audio_with_duration(4800, 24000, "linear16", 250);

    callback.on_audio(audio_with_metadata).await;

    // Should use the metadata value (250), not the computed value (100)
    let final_value = state.non_interruptible_until_ms.load(Ordering::Acquire);
    assert_eq!(
        final_value, 1250,
        "non_interruptible_until_ms should use duration_ms metadata (1000 + 250 = 1250)"
    );
}

/// Test that duration_ms metadata is used for non-PCM formats.
///
/// This validates that non-PCM audio (e.g., mp3, opus) with duration_ms metadata
/// correctly updates interruption timing.
#[tokio::test]
async fn test_duration_ms_for_non_pcm_formats() {
    let state = interruption_state_disallowed(0, 24000);
    let callback = tts_callback_with_state(state.clone());

    // MP3 format with provider-supplied duration
    let mp3_audio = audio_with_duration(1000, 44100, "mp3", 500);

    callback.on_audio(mp3_audio).await;

    let final_value = state.non_interruptible_until_ms.load(Ordering::Acquire);
    assert_eq!(
        final_value, 500,
        "non_interruptible_until_ms should use duration_ms for non-PCM formats"
    );
}

/// Test that PCM integer math correctly calculates duration.
///
/// This ensures integer-based duration calculation for PCM formats is accurate.
#[tokio::test]
async fn test_pcm_integer_math_duration() {
    let state = interruption_state_disallowed(0, 16000);
    let callback = tts_callback_with_state(state.clone());

    // For 16-bit mono at 16000 Hz:
    // duration_ms = (bytes * 1000) / (sample_rate * 2)
    // 3200 bytes: (3200 * 1000) / (16000 * 2) = 3200000 / 32000 = 100ms
    let audio_pcm = pcm_audio(3200, 16000, "pcm");

    callback.on_audio(audio_pcm).await;

    let final_value = state.non_interruptible_until_ms.load(Ordering::Acquire);
    assert_eq!(
        final_value, 100,
        "PCM integer math should compute 100ms for 3200 bytes at 16kHz"
    );
}

/// Test that unknown formats without duration_ms do not update timing.
///
/// Non-PCM formats without provider-supplied duration cannot have their
/// duration reliably computed, so the timing should not be updated.
#[tokio::test]
async fn test_unknown_format_without_duration_skipped() {
    let state = interruption_state_disallowed(1000, 24000);
    let callback = tts_callback_with_state(state.clone());

    // Unknown format without duration_ms - cannot compute duration
    let unknown_format_audio = pcm_audio(5000, 44100, "opus");

    callback.on_audio(unknown_format_audio).await;

    // Value should remain unchanged since format is unknown and no duration_ms
    let final_value = state.non_interruptible_until_ms.load(Ordering::Acquire);
    assert_eq!(
        final_value, 1000,
        "non_interruptible_until_ms should not change for unknown format without duration_ms"
    );
}
