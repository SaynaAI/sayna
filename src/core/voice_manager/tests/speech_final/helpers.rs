//! Shared test helpers for speech_final tests.
//!
//! Per CLAUDE.md "Testing" section, shared helpers reduce duplication
//! and improve test maintainability.

use crate::core::voice_manager::state::SpeechFinalState;
use crate::core::voice_manager::utils::get_current_time_ms;
use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Creates a fresh SpeechFinalState wrapped in Arc<SyncRwLock>.
pub fn new_speech_final_state() -> Arc<SyncRwLock<SpeechFinalState>> {
    Arc::new(SyncRwLock::new(SpeechFinalState::new()))
}

/// Resets the given SpeechFinalState to a new instance.
pub fn reset_speech_final_state(state: &Arc<SyncRwLock<SpeechFinalState>>) {
    let mut guard = state.write();
    *guard = SpeechFinalState::new();
}

/// Simulates VAD turn detection firing by updating state.
pub fn simulate_vad_turn_detection(
    state: &Arc<SyncRwLock<SpeechFinalState>>,
    text: &str,
    time_offset_ms: i64,
) {
    let mut guard = state.write();
    let current_ms = get_current_time_ms() as i64;
    let fire_time_ms = (current_ms + time_offset_ms).max(0) as usize;
    guard
        .turn_detection_last_fired_ms
        .store(fire_time_ms, Ordering::Release);
    guard.last_forced_text = text.to_string();
    guard
        .waiting_for_speech_final
        .store(false, Ordering::Release);
}

/// Test case data for is_final without is_speech_final scenarios.
pub struct IsFinalTestCase {
    pub transcript: &'static str,
    pub confidence: f32,
}

/// Common test cases for is_final without is_speech_final.
pub const IS_FINAL_TEST_CASES: &[IsFinalTestCase] = &[
    IsFinalTestCase {
        transcript: "Hello",
        confidence: 0.9,
    },
    IsFinalTestCase {
        transcript: "Test message",
        confidence: 0.8,
    },
];
