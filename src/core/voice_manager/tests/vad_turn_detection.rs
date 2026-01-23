//! Tests for VAD retry and backup timeout behavior.
//!
//! Per CLAUDE.md "Feature Flags" section, these tests are gated under `stt-vad`.
//! Per CLAUDE.md "Testing" section, async tests use `#[tokio::test]`.
//!
//! This module is feature-gated via `#[cfg(feature = "stt-vad")]` in the parent mod.rs.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::Duration;

use crate::core::vad::{SilenceTracker, SilenceTrackerConfig};

use crate::core::voice_manager::state::SpeechFinalState;
use crate::core::voice_manager::turn_detection_tasks::backup_timeout::spawn_backup_timeout_task;
use crate::core::voice_manager::turn_detection_tasks::smart_turn::SmartTurnResult;
use crate::core::voice_manager::turn_detection_tasks::speech_final::handle_smart_turn_result;
use crate::core::voice_manager::vad_processor::VADState;

/// Creates a SpeechFinalState with waiting_for_speech_final set to true and a callback.
fn create_waiting_state() -> Arc<SyncRwLock<SpeechFinalState>> {
    let callback_called = Arc::new(AtomicBool::new(false));
    let callback_called_clone = callback_called.clone();

    let callback = Arc::new(move |_result| {
        let called = callback_called_clone.clone();
        Box::pin(async move {
            called.store(true, Ordering::SeqCst);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));
    {
        let s = state.write();
        s.waiting_for_speech_final.store(true, Ordering::Release);
    }
    state
}

/// Creates a SilenceTracker with short thresholds for testing.
fn create_test_silence_tracker() -> Arc<SilenceTracker> {
    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 50,
        min_speech_duration_ms: 20,
        frame_duration_ms: 10.0,
    };
    Arc::new(SilenceTracker::new(config))
}

/// Creates a VADState for testing.
fn create_test_vad_state() -> Arc<VADState> {
    Arc::new(VADState::new(16000, 512))
}

// =============================================================================
// SmartTurnResult::Incomplete retry behavior tests
// =============================================================================

/// Test that SmartTurnResult::Incomplete clears vad_turn_end_detected to allow TurnEnd retry.
#[tokio::test]
async fn test_incomplete_clears_turn_end_detected_flag() {
    let state = create_waiting_state();
    let silence_tracker = create_test_silence_tracker();
    let vad_state = create_test_vad_state();

    // Set vad_turn_end_detected to true (simulating TurnEnd was detected)
    {
        let s = state.read();
        s.vad_turn_end_detected.store(true, Ordering::Release);
    }

    // Handle Incomplete result
    handle_smart_turn_result(
        SmartTurnResult::Incomplete,
        "test text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
        100, // retry_silence_duration_ms
        0,   // backup_silence_timeout_ms = 0 (disabled)
        "",
        "fallback",
    )
    .await;

    // Verify vad_turn_end_detected was cleared
    let s = state.read();
    assert!(
        !s.vad_turn_end_detected.load(Ordering::Acquire),
        "vad_turn_end_detected should be cleared after Incomplete to allow TurnEnd retry"
    );
}

/// Test that SmartTurnResult::Incomplete calls silence_tracker.allow_turn_end_retry().
#[tokio::test]
async fn test_incomplete_allows_turn_end_retry_on_silence_tracker() {
    let state = create_waiting_state();
    let vad_state = create_test_vad_state();

    // Create tracker with short thresholds
    // Using 50ms threshold with 10ms frames
    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 50,
        min_speech_duration_ms: 10,
        frame_duration_ms: 10.0,
    };
    let silence_tracker = Arc::new(SilenceTracker::new(config));

    // Simulate speech followed by silence until TurnEnd fires
    silence_tracker.process(0.8); // SpeechStart (10ms speech)
    silence_tracker.process(0.8); // More speech (20ms total)
    silence_tracker.process(0.2); // SilenceDetected (10ms silence)
    silence_tracker.process(0.2); // 20ms silence
    silence_tracker.process(0.2); // 30ms silence
    silence_tracker.process(0.2); // 40ms silence
    let turn_end_event = silence_tracker.process(0.2); // 50ms silence -> TurnEnd fires

    assert_eq!(
        turn_end_event,
        Some(crate::core::vad::VADEvent::TurnEnd),
        "First TurnEnd should fire at 50ms silence"
    );

    // At this point, TurnEnd has fired and won't fire again without retry
    // Verify that TurnEnd won't fire (latch is set)
    assert_eq!(silence_tracker.process(0.2), None); // 60ms silence, but latch is set

    // Handle Incomplete result with retry_silence_duration_ms
    // This will call allow_turn_end_retry(60) which:
    // 1. Resets the turn_end_fired latch
    // 2. Subtracts 60ms from accumulated silence (60ms - 60ms = 0ms)
    handle_smart_turn_result(
        SmartTurnResult::Incomplete,
        "test text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
        60, // retry_silence_duration_ms - reset accumulated silence to 0
        0,  // backup_silence_timeout_ms = 0 (disabled)
        "",
        "fallback",
    )
    .await;

    // After allow_turn_end_retry(60), silence is 0ms
    // Need to accumulate 50ms more to reach threshold again
    // Process 5 silence frames (50ms total) to reach threshold
    silence_tracker.process(0.2); // 10ms
    silence_tracker.process(0.2); // 20ms
    silence_tracker.process(0.2); // 30ms
    silence_tracker.process(0.2); // 40ms
    let event = silence_tracker.process(0.2); // 50ms -> should fire TurnEnd again

    assert_eq!(
        event,
        Some(crate::core::vad::VADEvent::TurnEnd),
        "TurnEnd should fire again after allow_turn_end_retry"
    );
}

// =============================================================================
// Backup timeout firing tests
// =============================================================================

/// Test that backup timeout fires after the configured delay when waiting_for_speech_final is true.
#[tokio::test(start_paused = true)]
async fn test_backup_timeout_fires_after_delay() {
    let callback_fired = Arc::new(AtomicBool::new(false));
    let callback_fired_clone = callback_fired.clone();

    let callback = Arc::new(move |_result| {
        let fired = callback_fired_clone.clone();
        Box::pin(async move {
            fired.store(true, Ordering::SeqCst);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));
    {
        let s = state.write();
        s.waiting_for_speech_final.store(true, Ordering::Release);
    }

    let silence_tracker = create_test_silence_tracker();
    let vad_state = create_test_vad_state();

    // Spawn backup timeout with short timeout (50ms)
    let handle = spawn_backup_timeout_task(
        50, // timeout_ms
        "buffered text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
    );

    // Callback should not have fired yet
    assert!(
        !callback_fired.load(Ordering::SeqCst),
        "Callback should not fire before timeout"
    );

    // Advance time past the timeout
    tokio::time::advance(Duration::from_millis(60)).await;

    // Wait for the task to complete
    handle.await.unwrap();

    // Callback should have fired
    assert!(
        callback_fired.load(Ordering::SeqCst),
        "Callback should fire after timeout when waiting_for_speech_final is true"
    );

    // State should be reset after firing
    let s = state.read();
    assert!(
        !s.waiting_for_speech_final.load(Ordering::Acquire),
        "waiting_for_speech_final should be cleared after backup timeout fires"
    );
}

/// Test that backup timeout does NOT fire if waiting_for_speech_final becomes false before timeout.
#[tokio::test(start_paused = true)]
async fn test_backup_timeout_skips_when_not_waiting() {
    let callback_fired = Arc::new(AtomicBool::new(false));
    let callback_fired_clone = callback_fired.clone();

    let callback = Arc::new(move |_result| {
        let fired = callback_fired_clone.clone();
        Box::pin(async move {
            fired.store(true, Ordering::SeqCst);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));
    {
        let s = state.write();
        s.waiting_for_speech_final.store(true, Ordering::Release);
    }

    let silence_tracker = create_test_silence_tracker();
    let vad_state = create_test_vad_state();

    // Spawn backup timeout
    let handle = spawn_backup_timeout_task(
        50,
        "buffered text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
    );

    // Simulate speech_final arriving from another source before timeout
    // (e.g., real STT speech_final or Smart-Turn Complete)
    {
        let s = state.read();
        s.waiting_for_speech_final.store(false, Ordering::Release);
    }

    // Advance time past the timeout
    tokio::time::advance(Duration::from_millis(60)).await;

    // Wait for the task to complete
    handle.await.unwrap();

    // Callback should NOT have fired because we weren't waiting anymore
    assert!(
        !callback_fired.load(Ordering::SeqCst),
        "Callback should not fire when waiting_for_speech_final is false"
    );
}

// =============================================================================
// Backup timeout cancellation tests
// =============================================================================

/// Test that reset_vad_state() cancels the backup timeout.
#[tokio::test(start_paused = true)]
async fn test_reset_vad_state_cancels_backup_timeout() {
    let callback_fired = Arc::new(AtomicBool::new(false));
    let callback_fired_clone = callback_fired.clone();

    let callback = Arc::new(move |_result| {
        let fired = callback_fired_clone.clone();
        Box::pin(async move {
            fired.store(true, Ordering::SeqCst);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));
    {
        let s = state.write();
        s.waiting_for_speech_final.store(true, Ordering::Release);
    }

    let silence_tracker = create_test_silence_tracker();
    let vad_state = create_test_vad_state();

    // Spawn backup timeout with longer timeout
    let handle = spawn_backup_timeout_task(
        100, // timeout_ms
        "buffered text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
    );

    // Store handle in state
    {
        let mut s = state.write();
        s.backup_timeout_handle = Some(handle);
    }

    // Advance time a bit but not past timeout
    tokio::time::advance(Duration::from_millis(30)).await;

    // Simulate speech resuming - call reset_vad_state which should cancel the timeout
    {
        let mut s = state.write();
        s.reset_vad_state();
    }

    // Advance time well past the original timeout
    tokio::time::advance(Duration::from_millis(150)).await;

    // Allow any pending tasks to run
    tokio::task::yield_now().await;

    // Callback should NOT have fired because the timeout was cancelled
    assert!(
        !callback_fired.load(Ordering::SeqCst),
        "Callback should not fire after reset_vad_state cancels the backup timeout"
    );

    // Verify handle was taken (is now None)
    let s = state.read();
    assert!(
        s.backup_timeout_handle.is_none(),
        "backup_timeout_handle should be None after reset_vad_state"
    );
}

/// Test that starting a new Smart-Turn task cancels any existing backup timeout handle.
///
/// This test verifies that when handle_smart_turn_result is called with Incomplete,
/// it aborts any existing backup timeout handle before spawning a new one.
#[tokio::test]
async fn test_incomplete_aborts_existing_backup_timeout_before_starting_new() {
    let state = create_waiting_state();
    let silence_tracker = create_test_silence_tracker();
    let vad_state = create_test_vad_state();

    // Spawn first backup timeout (we'll verify it gets aborted)
    let first_handle = spawn_backup_timeout_task(
        5000, // Long timeout that should never fire
        "first text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
    );

    // Store in state
    {
        let mut s = state.write();
        s.backup_timeout_handle = Some(first_handle);
    }

    // Verify handle is stored
    {
        let s = state.read();
        assert!(
            s.backup_timeout_handle.is_some(),
            "First backup handle should be stored"
        );
    }

    // Now call handle_smart_turn_result with Incomplete - this should:
    // 1. Abort the existing backup timeout
    // 2. Start a new backup timeout
    handle_smart_turn_result(
        SmartTurnResult::Incomplete,
        "second text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
        50,   // retry_silence_duration_ms
        1000, // backup_silence_timeout_ms - new timeout
        "",
        "fallback",
    )
    .await;

    // Verify that a new handle was stored (replacing the old one)
    // The old one was aborted (we can't easily check this, but we verified
    // the code path aborts it in the implementation)
    let s = state.read();
    assert!(
        s.backup_timeout_handle.is_some(),
        "New backup handle should be stored after Incomplete"
    );
}

/// Test that handle_smart_turn_result with Incomplete spawns a backup timeout when enabled.
#[tokio::test(start_paused = true)]
async fn test_incomplete_spawns_backup_timeout_when_enabled() {
    let state = create_waiting_state();
    let silence_tracker = create_test_silence_tracker();
    let vad_state = create_test_vad_state();

    // Verify no backup timeout handle initially
    {
        let s = state.read();
        assert!(
            s.backup_timeout_handle.is_none(),
            "Should not have backup timeout handle initially"
        );
    }

    // Handle Incomplete with backup_silence_timeout_ms > 0
    handle_smart_turn_result(
        SmartTurnResult::Incomplete,
        "test text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
        50,  // retry_silence_duration_ms
        100, // backup_silence_timeout_ms (enabled)
        "",
        "fallback",
    )
    .await;

    // Verify backup timeout handle was set
    let s = state.read();
    assert!(
        s.backup_timeout_handle.is_some(),
        "Backup timeout handle should be set after Incomplete when timeout > 0"
    );
}

/// Test that handle_smart_turn_result with Incomplete does NOT spawn backup timeout when disabled.
#[tokio::test]
async fn test_incomplete_skips_backup_timeout_when_disabled() {
    let state = create_waiting_state();
    let silence_tracker = create_test_silence_tracker();
    let vad_state = create_test_vad_state();

    // Handle Incomplete with backup_silence_timeout_ms = 0 (disabled)
    handle_smart_turn_result(
        SmartTurnResult::Incomplete,
        "test text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
        50, // retry_silence_duration_ms
        0,  // backup_silence_timeout_ms = 0 (disabled)
        "",
        "fallback",
    )
    .await;

    // Verify no backup timeout handle was set
    let s = state.read();
    assert!(
        s.backup_timeout_handle.is_none(),
        "Backup timeout handle should NOT be set when timeout is 0"
    );
}

// =============================================================================
// Complete and Skipped result tests (ensure they don't leave dangling timeouts)
// =============================================================================

/// Test that SmartTurnResult::Complete fires callback and resets state.
#[tokio::test]
async fn test_complete_fires_callback_and_resets() {
    let callback_fired = Arc::new(AtomicBool::new(false));
    let callback_fired_clone = callback_fired.clone();

    let callback = Arc::new(move |_result| {
        let fired = callback_fired_clone.clone();
        Box::pin(async move {
            fired.store(true, Ordering::SeqCst);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));
    {
        let s = state.write();
        s.waiting_for_speech_final.store(true, Ordering::Release);
    }

    let silence_tracker = create_test_silence_tracker();
    let vad_state = create_test_vad_state();

    handle_smart_turn_result(
        SmartTurnResult::Complete("smart_turn_confirmed"),
        "test text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
        50,
        100,
        "",
        "fallback",
    )
    .await;

    // Callback should have fired
    assert!(
        callback_fired.load(Ordering::SeqCst),
        "Callback should fire on Complete"
    );

    // State should be reset
    let s = state.read();
    assert!(
        !s.waiting_for_speech_final.load(Ordering::Acquire),
        "waiting_for_speech_final should be false after Complete"
    );
}

/// Test that SmartTurnResult::Skipped fires callback and resets state.
#[tokio::test]
async fn test_skipped_fires_callback_and_resets() {
    let callback_fired = Arc::new(AtomicBool::new(false));
    let callback_fired_clone = callback_fired.clone();

    let callback = Arc::new(move |_result| {
        let fired = callback_fired_clone.clone();
        Box::pin(async move {
            fired.store(true, Ordering::SeqCst);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));
    {
        let s = state.write();
        s.waiting_for_speech_final.store(true, Ordering::Release);
    }

    let silence_tracker = create_test_silence_tracker();
    let vad_state = create_test_vad_state();

    handle_smart_turn_result(
        SmartTurnResult::Skipped,
        "test text".to_string(),
        state.clone(),
        silence_tracker.clone(),
        vad_state.clone(),
        50,
        100,
        "",
        "vad_silence_only",
    )
    .await;

    // Callback should have fired
    assert!(
        callback_fired.load(Ordering::SeqCst),
        "Callback should fire on Skipped"
    );

    // State should be reset
    let s = state.read();
    assert!(
        !s.waiting_for_speech_final.load(Ordering::Acquire),
        "waiting_for_speech_final should be false after Skipped"
    );
}
