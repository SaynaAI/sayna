//! Tests for interruption timing updates in TTS callbacks.
//!
//! Per CLAUDE.md "Testing" section, unit tests use `#[tokio::test]` for async tests.

use std::sync::atomic::Ordering;

use crate::core::tts::AudioCallback;

use super::helpers::{
    interruption_state_allowed, interruption_state_disallowed, pcm_audio, tts_callback_with_state,
};

/// Test that multiple audio chunks atomically update non_interruptible_until_ms.
///
/// This regression test verifies that concurrent audio callbacks cannot lose
/// increments due to non-atomic load+store operations. The fix uses fetch_add
/// to ensure monotonic updates.
#[tokio::test]
async fn test_non_interruptible_until_ms_atomic_updates() {
    let state = interruption_state_disallowed(1000, 24000);
    let callback = tts_callback_with_state(state.clone());

    // Each chunk is 4800 bytes at 24000 Hz, 16-bit mono = 100ms duration
    let audio_chunk = pcm_audio(4800, 24000, "linear16");

    // Process 5 audio chunks sequentially
    for _ in 0..5 {
        callback.on_audio(audio_chunk.clone()).await;
    }

    // Expected: 1000 (initial) + 5 * 100 (chunk durations) = 1500
    let final_value = state.non_interruptible_until_ms.load(Ordering::Acquire);
    assert_eq!(
        final_value, 1500,
        "non_interruptible_until_ms should increase by sum of chunk durations"
    );
}

/// Test that non_interruptible_until_ms remains monotonically increasing.
#[tokio::test]
async fn test_non_interruptible_until_ms_monotonic() {
    let state = interruption_state_disallowed(0, 24000);
    let callback = tts_callback_with_state(state.clone());

    let audio_chunk = pcm_audio(4800, 24000, "linear16"); // 100ms per chunk

    let mut prev_value = 0usize;
    for i in 0..10 {
        callback.on_audio(audio_chunk.clone()).await;

        let current_value = state.non_interruptible_until_ms.load(Ordering::Acquire);

        assert!(
            current_value > prev_value,
            "non_interruptible_until_ms should monotonically increase: iteration {}, prev={}, current={}",
            i,
            prev_value,
            current_value
        );

        prev_value = current_value;
    }
}

/// Test that non_interruptible_until_ms is not updated when allow_interruption is true.
#[tokio::test]
async fn test_no_update_when_interruption_allowed() {
    let state = interruption_state_allowed(1000, 24000);
    let callback = tts_callback_with_state(state.clone());

    let audio_chunk = pcm_audio(4800, 24000, "linear16");

    // Process multiple chunks
    for _ in 0..5 {
        callback.on_audio(audio_chunk.clone()).await;
    }

    // Value should remain unchanged since allow_interruption is true
    let final_value = state.non_interruptible_until_ms.load(Ordering::Acquire);
    assert_eq!(
        final_value, 1000,
        "non_interruptible_until_ms should not change when allow_interruption is true"
    );
}
