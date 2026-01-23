//! Tests for PCM format variant recognition in TTS callbacks.
//!
//! Per rust.mdc "Testing" section 5.1, table-driven tests improve maintainability
//! by testing multiple scenarios with different inputs in a single test function.
//!
//! Per CLAUDE.md "Testing" section, unit tests use `#[tokio::test]` for async tests.

use std::sync::atomic::Ordering;

use crate::core::tts::AudioCallback;

use super::helpers::{
    PCM_FORMAT_CASES, interruption_state_disallowed, pcm_audio, tts_callback_with_state,
};

/// Test that PCM variant format names are recognized (linear16, pcm, pcm_16).
///
/// This table-driven test verifies all common PCM format string variants
/// are correctly identified and processed for duration calculation.
#[tokio::test]
async fn test_pcm_format_variants() {
    for case in PCM_FORMAT_CASES {
        let state = interruption_state_disallowed(0, case.sample_rate);
        let callback = tts_callback_with_state(state.clone());

        let audio = pcm_audio(case.byte_count, case.sample_rate, case.format);

        callback.on_audio(audio).await;

        let final_value = state.non_interruptible_until_ms.load(Ordering::Acquire);
        assert_eq!(
            final_value, case.expected_duration_ms,
            "Format '{}' should be recognized as PCM and compute {}ms",
            case.format, case.expected_duration_ms
        );
    }
}
