//! Shared test helpers for callbacks tests.
//!
//! Per CLAUDE.md "Testing" section, shared helpers reduce duplication
//! and improve test maintainability.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize};

use crate::core::tts::AudioData;
use crate::core::voice_manager::callbacks::VoiceManagerTTSCallback;
use crate::core::voice_manager::state::InterruptionState;

/// Creates an InterruptionState with allow_interruption=false for timing tests.
/// This is the typical setup for tests that verify non_interruptible_until_ms updates.
pub fn interruption_state_disallowed(
    initial_ms: usize,
    sample_rate: u32,
) -> Arc<InterruptionState> {
    Arc::new(InterruptionState {
        allow_interruption: AtomicBool::new(false),
        non_interruptible_until_ms: AtomicUsize::new(initial_ms),
        current_sample_rate: AtomicU32::new(sample_rate),
        is_completed: AtomicBool::new(false),
    })
}

/// Creates an InterruptionState with allow_interruption=true.
/// Used for tests that verify timing is NOT updated when interruptions are allowed.
pub fn interruption_state_allowed(initial_ms: usize, sample_rate: u32) -> Arc<InterruptionState> {
    Arc::new(InterruptionState {
        allow_interruption: AtomicBool::new(true),
        non_interruptible_until_ms: AtomicUsize::new(initial_ms),
        current_sample_rate: AtomicU32::new(sample_rate),
        is_completed: AtomicBool::new(false),
    })
}

/// Creates a VoiceManagerTTSCallback with only interruption state (no other callbacks).
pub fn tts_callback_with_state(state: Arc<InterruptionState>) -> VoiceManagerTTSCallback {
    VoiceManagerTTSCallback {
        audio_callback: None,
        error_callback: None,
        interruption_state: Some(state),
        complete_callback: None,
    }
}

/// Creates test audio data in PCM format with a specific byte size.
///
/// For 16-bit mono audio at the given sample rate:
/// - bytes_per_sample = 2
/// - channels = 1
/// - duration_ms = (data.len() * 1000) / (sample_rate * 2)
///
/// Example: 4800 bytes at 24000 Hz = 100ms
pub fn pcm_audio(byte_count: usize, sample_rate: u32, format: &str) -> AudioData {
    AudioData {
        data: vec![0u8; byte_count],
        sample_rate,
        format: format.to_string(),
        duration_ms: None,
    }
}

/// Creates test audio data with explicit duration metadata.
/// Used for non-PCM formats or when provider supplies duration.
pub fn audio_with_duration(
    byte_count: usize,
    sample_rate: u32,
    format: &str,
    duration_ms: u32,
) -> AudioData {
    AudioData {
        data: vec![0u8; byte_count],
        sample_rate,
        format: format.to_string(),
        duration_ms: Some(duration_ms),
    }
}

/// Test case for PCM format variants (table-driven testing).
pub struct PcmFormatTestCase {
    pub format: &'static str,
    pub byte_count: usize,
    pub sample_rate: u32,
    pub expected_duration_ms: usize,
}

/// Common PCM format test cases covering various format string variants.
/// Per rust.mdc "Testing" section 5.1, table-driven tests improve maintainability.
pub const PCM_FORMAT_CASES: &[PcmFormatTestCase] = &[
    // 4800 bytes @ 24kHz = 100ms
    PcmFormatTestCase {
        format: "linear16",
        byte_count: 4800,
        sample_rate: 24000,
        expected_duration_ms: 100,
    },
    PcmFormatTestCase {
        format: "pcm",
        byte_count: 4800,
        sample_rate: 24000,
        expected_duration_ms: 100,
    },
    PcmFormatTestCase {
        format: "pcm_16",
        byte_count: 4800,
        sample_rate: 24000,
        expected_duration_ms: 100,
    },
    PcmFormatTestCase {
        format: "LINEAR16",
        byte_count: 4800,
        sample_rate: 24000,
        expected_duration_ms: 100,
    },
    PcmFormatTestCase {
        format: "PCM",
        byte_count: 4800,
        sample_rate: 24000,
        expected_duration_ms: 100,
    },
];
