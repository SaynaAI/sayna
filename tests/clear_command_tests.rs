//! Integration tests for the clear command flow
//!
//! This module tests the clear command flow at the VoiceManager and OperationQueue
//! levels using only public APIs. Tests verify:
//! - TTS provider queue clearing
//! - Audio clear callbacks are invoked
//! - Generation-based staleness detection works
//! - Non-interruptible audio is respected
//! - Concurrent operations don't cause deadlocks

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio::time::timeout;

use sayna::core::stt::STTConfig;
use sayna::core::tts::TTSConfig;
use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
use sayna::livekit::operations::{LiveKitOperation, OperationQueue};

/// Helper macro to create VoiceManager with feature-gated VAD parameter
#[cfg(feature = "stt-vad")]
macro_rules! create_voice_manager {
    ($config:expr) => {
        VoiceManager::new($config, None, None)
    };
}

#[cfg(not(feature = "stt-vad"))]
macro_rules! create_voice_manager {
    ($config:expr) => {
        VoiceManager::new($config, None)
    };
}

fn create_test_voice_manager_config() -> VoiceManagerConfig {
    let stt_config = STTConfig {
        provider: "deepgram".to_string(),
        api_key: "test_key".to_string(),
        ..Default::default()
    };
    let tts_config = TTSConfig {
        provider: "deepgram".to_string(),
        api_key: "test_key".to_string(),
        ..Default::default()
    };
    VoiceManagerConfig::new(stt_config, tts_config)
}

// =============================================================================
// Test 1: Basic Clear During TTS - VoiceManager Level
// =============================================================================

#[tokio::test]
async fn test_clear_tts_clears_audio_callback() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    // Track if audio clear callback is called
    let callback_called = Arc::new(AtomicBool::new(false));
    let callback_called_clone = callback_called.clone();

    // Register audio clear callback
    let _ = voice_manager
        .on_audio_clear(move || {
            let called = callback_called_clone.clone();
            Box::pin(async move {
                called.store(true, Ordering::SeqCst);
            })
        })
        .await;

    // Call clear_tts
    let result = voice_manager.clear_tts().await;
    assert!(result.is_ok(), "clear_tts should succeed");

    // Verify callback was called
    assert!(
        callback_called.load(Ordering::SeqCst),
        "Audio clear callback should be invoked when clear_tts is called"
    );
}

#[tokio::test]
async fn test_clear_tts_callback_timing() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    // Register no-op callback to ensure clear doesn't hang
    let _ = voice_manager.on_audio_clear(|| Box::pin(async {})).await;

    // Clear should complete within reasonable time (100ms)
    let result = timeout(Duration::from_millis(100), voice_manager.clear_tts()).await;

    assert!(result.is_ok(), "clear_tts should complete within 100ms");
    assert!(result.unwrap().is_ok(), "clear_tts should succeed");
}

// =============================================================================
// Test 2: Rapid Speak-Clear Sequence
// =============================================================================

#[tokio::test]
async fn test_rapid_clear_sequence() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    // Track callback count
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_clone = callback_count.clone();

    let _ = voice_manager
        .on_audio_clear(move || {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
            })
        })
        .await;

    // Rapid fire clear commands
    for _ in 0..5 {
        let result = voice_manager.clear_tts().await;
        assert!(result.is_ok(), "Each clear_tts should succeed");
    }

    // All callbacks should have been called
    assert_eq!(
        callback_count.load(Ordering::SeqCst),
        5,
        "Clear callback should be called once per clear_tts call"
    );
}

#[tokio::test]
async fn test_speak_then_immediate_clear() {
    let config = create_test_voice_manager_config();
    let voice_manager =
        Arc::new(create_voice_manager!(config).expect("Failed to create VoiceManager"));

    let clear_callback_called = Arc::new(AtomicBool::new(false));
    let clear_callback_clone = clear_callback_called.clone();

    let _ = voice_manager
        .on_audio_clear(move || {
            let called = clear_callback_clone.clone();
            Box::pin(async move {
                called.store(true, Ordering::SeqCst);
            })
        })
        .await;

    // Send speak command with flush=true (will fail due to test API key but queues the text)
    // The important thing is the clear happens right after
    let _ = voice_manager.speak("Test text to speak", true).await;

    // Immediately clear
    let clear_result = voice_manager.clear_tts().await;
    assert!(clear_result.is_ok(), "Clear should succeed");
    assert!(
        clear_callback_called.load(Ordering::SeqCst),
        "Clear callback should be called"
    );
}

// =============================================================================
// Test 3: Clear with Pending Queue Operations
// =============================================================================

#[tokio::test]
async fn test_operation_queue_generation_staleness() {
    let (queue, mut receiver, audio_generation) = OperationQueue::new(100);

    // Queue audio at generation 0
    let (tx1, _rx1) = oneshot::channel();
    queue
        .queue_audio(vec![1, 2, 3, 4], tx1)
        .await
        .expect("Queue should accept audio");

    // Cancel audio (increments generation to 1)
    let new_gen = queue.cancel_audio();
    assert_eq!(new_gen, 1, "Generation should increment to 1");

    // Queue more audio at generation 1
    let (tx2, _rx2) = oneshot::channel();
    queue
        .queue_audio(vec![5, 6, 7, 8], tx2)
        .await
        .expect("Queue should accept audio");

    // Verify first operation has stale generation
    let op1 = receiver
        .recv()
        .await
        .expect("Should receive first operation");
    if let LiveKitOperation::SendAudio { generation, .. } = op1.operation {
        assert_eq!(generation, 0, "First operation should have generation 0");
        // This operation should be skipped by the worker because 0 < 1
        assert!(
            generation < audio_generation.load(Ordering::Acquire),
            "Operation generation should be stale"
        );
    } else {
        panic!("Expected SendAudio operation");
    }

    // Second operation has current generation
    let op2 = receiver
        .recv()
        .await
        .expect("Should receive second operation");
    if let LiveKitOperation::SendAudio { generation, .. } = op2.operation {
        assert_eq!(generation, 1, "Second operation should have generation 1");
        // This operation should be processed because 1 >= 1
        assert!(
            generation >= audio_generation.load(Ordering::Acquire),
            "Operation generation should be current"
        );
    } else {
        panic!("Expected SendAudio operation");
    }
}

#[tokio::test]
async fn test_clear_audio_operation_priority() {
    let (queue, mut receiver, _) = OperationQueue::new(100);

    // Queue a ClearAudio operation
    let (tx, _rx) = oneshot::channel();
    queue
        .queue(LiveKitOperation::ClearAudio { response_tx: tx })
        .await
        .expect("Queue should accept clear operation");

    // Verify it has high priority
    let op = receiver.recv().await.expect("Should receive operation");
    assert_eq!(
        op.priority,
        sayna::livekit::operations::OperationPriority::High,
        "ClearAudio should have High priority"
    );
}

#[tokio::test]
async fn test_cancel_audio_invalidates_pending() {
    let (queue, _, audio_generation) = OperationQueue::new(100);

    // Get initial generation
    let initial_gen = queue.current_audio_generation();
    assert_eq!(initial_gen, 0);

    // Simulate queueing audio operations
    let gen_at_queue = queue.current_audio_generation();
    assert_eq!(gen_at_queue, 0);

    // Cancel (simulating clear command)
    queue.cancel_audio();
    queue.cancel_audio();
    queue.cancel_audio();

    // Generation should have incremented
    let current_gen = queue.current_audio_generation();
    assert_eq!(current_gen, 3);
    assert_eq!(audio_generation.load(Ordering::Acquire), 3);

    // Any operation with generation < 3 is stale
    assert!(gen_at_queue < current_gen);
}

// =============================================================================
// Test 4: Clear with Non-Interruptible Audio
// =============================================================================

#[tokio::test]
async fn test_clear_respects_interruption_state() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    let callback_called = Arc::new(AtomicBool::new(false));
    let callback_clone = callback_called.clone();

    let _ = voice_manager
        .on_audio_clear(move || {
            let called = callback_clone.clone();
            Box::pin(async move {
                called.store(true, Ordering::SeqCst);
            })
        })
        .await;

    // Initially not blocked
    assert!(
        !voice_manager.is_interruption_blocked().await,
        "Initially should not be blocked"
    );

    // Clear should work
    let result = voice_manager.clear_tts().await;
    assert!(result.is_ok());
    assert!(callback_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_interruption_blocked_skips_clear() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    let callback_called = Arc::new(AtomicBool::new(false));
    let callback_clone = callback_called.clone();

    let _ = voice_manager
        .on_audio_clear(move || {
            let called = callback_clone.clone();
            Box::pin(async move {
                called.store(true, Ordering::SeqCst);
            })
        })
        .await;

    // Start non-interruptible speech
    // Note: speak_with_interruption(text, flush, allow_interruption=false) blocks interruption
    let _ = voice_manager
        .speak_with_interruption("Test", true, false)
        .await;

    // Now interruption should be blocked
    assert!(
        voice_manager.is_interruption_blocked().await,
        "Should be blocked during non-interruptible speech"
    );

    // Clear should return Ok but NOT call the callback
    callback_called.store(false, Ordering::SeqCst);
    let result = voice_manager.clear_tts().await;
    assert!(result.is_ok(), "clear_tts returns Ok even when blocked");

    // Callback should NOT have been called because interruption was blocked
    assert!(
        !callback_called.load(Ordering::SeqCst),
        "Callback should not be called when interruption is blocked"
    );
}

// =============================================================================
// Test 5: Concurrent Clear and Audio Operations
// =============================================================================

#[tokio::test]
async fn test_concurrent_clear_operations_no_deadlock() {
    let config = create_test_voice_manager_config();
    let voice_manager =
        Arc::new(create_voice_manager!(config).expect("Failed to create VoiceManager"));

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let _ = voice_manager
        .on_audio_clear(move || {
            let count = counter_clone.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                // Small delay to increase chance of contention
                tokio::time::sleep(Duration::from_millis(5)).await;
            })
        })
        .await;

    // Spawn multiple concurrent clear tasks
    let mut handles = vec![];
    for _ in 0..10 {
        let vm = voice_manager.clone();
        handles.push(tokio::spawn(async move {
            vm.clear_tts().await.expect("Clear should succeed");
        }));
    }

    // Wait for all with timeout to detect deadlocks
    let result = timeout(Duration::from_secs(5), async {
        for handle in handles {
            handle.await.expect("Task should complete");
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "All concurrent clear operations should complete without deadlock"
    );

    // All callbacks should have been called
    assert_eq!(counter.load(Ordering::SeqCst), 10);
}

#[tokio::test]
async fn test_concurrent_queue_operations() {
    let (queue, receiver, audio_generation) = OperationQueue::new(1000);

    // Drop receiver to avoid channel full errors
    drop(receiver);

    let queue = Arc::new(queue);

    // Spawn tasks that queue audio and cancel concurrently
    let mut handles = vec![];

    // Audio queueing tasks
    for _ in 0..5 {
        let q = queue.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..10 {
                let (tx, _rx) = oneshot::channel();
                let _ = q.queue_audio(vec![1, 2, 3, 4], tx).await;
            }
        }));
    }

    // Cancel tasks
    for _ in 0..5 {
        let q = queue.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..10 {
                q.cancel_audio();
            }
        }));
    }

    // Wait for all with timeout
    let result = timeout(Duration::from_secs(5), async {
        for handle in handles {
            handle.await.expect("Task should complete");
        }
    })
    .await;

    assert!(
        result.is_ok(),
        "Concurrent operations should complete without deadlock"
    );

    // Generation should have been incremented 50 times (5 tasks * 10 cancels)
    assert_eq!(audio_generation.load(Ordering::SeqCst), 50);
}

// =============================================================================
// Test 6: Clear Without LiveKit / No Callback Registered
// =============================================================================

#[tokio::test]
async fn test_clear_without_audio_callback() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    // Don't register any callback at all
    // Should complete without error
    let result = timeout(Duration::from_millis(100), voice_manager.clear_tts()).await;

    assert!(result.is_ok(), "Should complete within timeout");
    assert!(result.unwrap().is_ok(), "clear_tts should succeed");
}

#[tokio::test]
async fn test_clear_without_livekit_callback() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    // Don't register any LiveKit callback
    // Clear should still work without errors
    let result = voice_manager.clear_tts().await;
    assert!(
        result.is_ok(),
        "clear_tts should succeed without LiveKit callback"
    );
}

// =============================================================================
// Test 7: Clear Command Integration with Operation Queue
// =============================================================================

#[tokio::test]
async fn test_clear_via_operation_queue() {
    let (queue, mut receiver, _) = OperationQueue::new(100);

    // Queue a clear operation
    let (tx, rx) = oneshot::channel();
    queue
        .queue(LiveKitOperation::ClearAudio { response_tx: tx })
        .await
        .expect("Should queue clear operation");

    // Receive the operation
    let op = receiver.recv().await.expect("Should receive operation");

    // Verify it's a ClearAudio operation
    match op.operation {
        LiveKitOperation::ClearAudio { response_tx } => {
            // Simulate worker handling it
            response_tx.send(Ok(())).expect("Should send response");
        }
        _ => panic!("Expected ClearAudio operation"),
    }

    // Verify response is received
    let result = rx.await.expect("Should receive response");
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_cancel_audio_before_clear_operation() {
    let (queue, mut receiver, _) = OperationQueue::new(100);

    // Queue audio
    let (tx1, _) = oneshot::channel();
    queue.queue_audio(vec![1, 2, 3], tx1).await.unwrap();

    // Cancel audio (this should invalidate pending audio)
    let new_gen = queue.cancel_audio();
    assert_eq!(new_gen, 1);

    // Queue clear operation
    let (tx2, rx2) = oneshot::channel();
    queue
        .queue(LiveKitOperation::ClearAudio { response_tx: tx2 })
        .await
        .unwrap();

    // First operation should be SendAudio with stale generation
    let op1 = receiver.recv().await.unwrap();
    if let LiveKitOperation::SendAudio { generation, .. } = op1.operation {
        assert_eq!(generation, 0, "Should have stale generation");
    } else {
        panic!("Expected SendAudio");
    }

    // Second operation should be ClearAudio
    let op2 = receiver.recv().await.unwrap();
    match op2.operation {
        LiveKitOperation::ClearAudio { response_tx } => {
            response_tx.send(Ok(())).unwrap();
        }
        _ => panic!("Expected ClearAudio"),
    }

    assert!(rx2.await.unwrap().is_ok());
}

// =============================================================================
// Test 8: Clear Command Timing Verification
// =============================================================================

#[tokio::test]
async fn test_clear_tts_timing() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    // Register callback with delay
    let _ = voice_manager
        .on_audio_clear(|| {
            Box::pin(async {
                // Simulate some async work
                tokio::time::sleep(Duration::from_millis(10)).await;
            })
        })
        .await;

    let start = std::time::Instant::now();
    let result = voice_manager.clear_tts().await;
    let elapsed = start.elapsed();

    assert!(result.is_ok());
    // Should complete in reasonable time (callback delay + internal timeout)
    // Internal timeout is 50ms, so total should be < 100ms
    assert!(
        elapsed < Duration::from_millis(150),
        "Clear should complete quickly, took {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_multiple_sequential_clears_timing() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    let _ = voice_manager.on_audio_clear(|| Box::pin(async {})).await;

    let start = std::time::Instant::now();

    // Execute 10 clears sequentially
    for _ in 0..10 {
        voice_manager
            .clear_tts()
            .await
            .expect("Clear should succeed");
    }

    let elapsed = start.elapsed();

    // 10 clears with ~50ms internal timeout each should take < 1 second
    assert!(
        elapsed < Duration::from_secs(1),
        "10 sequential clears should complete in <1s, took {:?}",
        elapsed
    );
}

// =============================================================================
// Test 9: State Consistency After Clear
// =============================================================================

#[tokio::test]
async fn test_voice_manager_state_after_clear() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    let clear_count = Arc::new(AtomicUsize::new(0));
    let count_clone = clear_count.clone();

    let _ = voice_manager
        .on_audio_clear(move || {
            let count = count_clone.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
            })
        })
        .await;

    // Clear and verify state
    voice_manager.clear_tts().await.unwrap();

    // After clear, interruption should not be blocked (state was reset)
    assert!(
        !voice_manager.is_interruption_blocked().await,
        "After clear, interruption should not be blocked"
    );

    assert_eq!(clear_count.load(Ordering::SeqCst), 1);

    // Can clear again
    voice_manager.clear_tts().await.unwrap();
    assert_eq!(clear_count.load(Ordering::SeqCst), 2);
}

// =============================================================================
// Test 10: Error Handling During Clear
// =============================================================================

#[tokio::test]
async fn test_clear_callback_completes_correctly() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    // Register callback that does minimal work
    let _ = voice_manager
        .on_audio_clear(|| {
            Box::pin(async {
                // This is a no-op but tests that async closures work correctly
            })
        })
        .await;

    // Clear should succeed
    let result = voice_manager.clear_tts().await;
    assert!(result.is_ok());
}

// =============================================================================
// Test 11: Operation Queue Edge Cases
// =============================================================================

#[tokio::test]
async fn test_multiple_cancel_audio_calls() {
    let (queue, _, audio_generation) = OperationQueue::new(100);

    // Multiple cancel calls should each increment atomically
    for i in 1..=100 {
        let new_gen = queue.cancel_audio();
        assert_eq!(new_gen, i, "Generation should be {}", i);
    }

    assert_eq!(audio_generation.load(Ordering::Acquire), 100);
    assert_eq!(queue.current_audio_generation(), 100);
}

#[tokio::test]
async fn test_operation_queue_clone_shares_generation() {
    let (queue1, _, audio_generation) = OperationQueue::new(100);
    let queue2 = queue1.clone();

    // Cancel on queue1
    queue1.cancel_audio();
    assert_eq!(queue2.current_audio_generation(), 1);
    assert_eq!(audio_generation.load(Ordering::Acquire), 1);

    // Cancel on queue2
    queue2.cancel_audio();
    assert_eq!(queue1.current_audio_generation(), 2);
    assert_eq!(audio_generation.load(Ordering::Acquire), 2);
}

// =============================================================================
// Test 12: VoiceManager Speak with Interruption
// =============================================================================

#[tokio::test]
async fn test_speak_with_interruption_allows_clear() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    let callback_count = Arc::new(AtomicUsize::new(0));
    let count_clone = callback_count.clone();

    let _ = voice_manager
        .on_audio_clear(move || {
            let count = count_clone.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
            })
        })
        .await;

    // Speak with allow_interruption=true
    let _ = voice_manager
        .speak_with_interruption("Hello world", true, true)
        .await;

    // Should NOT be blocked
    assert!(
        !voice_manager.is_interruption_blocked().await,
        "Should not be blocked with allow_interruption=true"
    );

    // Clear should call the callback
    voice_manager.clear_tts().await.unwrap();
    assert_eq!(
        callback_count.load(Ordering::SeqCst),
        1,
        "Callback should be called"
    );
}

#[tokio::test]
async fn test_clear_after_non_interruptible_resets() {
    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    // Speak with allow_interruption=false
    let _ = voice_manager
        .speak_with_interruption("Test", true, false)
        .await;

    // Initially blocked
    assert!(voice_manager.is_interruption_blocked().await);

    // First clear does nothing but returns Ok
    let result = voice_manager.clear_tts().await;
    assert!(result.is_ok());

    // Still blocked (interruption state not changed by failed clear)
    assert!(voice_manager.is_interruption_blocked().await);
}

// =============================================================================
// Test 13: Integration Test - Complete Clear Flow Simulation
// =============================================================================

#[tokio::test]
async fn test_complete_clear_flow_simulation() {
    // This test simulates the complete clear command flow:
    // 1. Create VoiceManager
    // 2. Register audio clear callback (simulating LiveKit registration)
    // 3. Queue some "audio" via the operation queue
    // 4. Cancel audio via operation queue (simulating clear)
    // 5. Verify pending audio is marked stale
    // 6. Call clear_tts on VoiceManager
    // 7. Verify callback was invoked

    let config = create_test_voice_manager_config();
    let voice_manager = create_voice_manager!(config).expect("Failed to create VoiceManager");

    // Create operation queue (simulating LiveKit integration)
    let (queue, mut receiver, audio_generation) = OperationQueue::new(100);

    // Track callback invocation
    let callback_invoked = Arc::new(AtomicBool::new(false));
    let queue_for_callback = queue.clone();
    let callback_clone = callback_invoked.clone();

    // Register audio clear callback that:
    // 1. Cancels audio operations
    // 2. Queues a ClearAudio operation
    let _ = voice_manager
        .on_audio_clear(move || {
            let q = queue_for_callback.clone();
            let invoked = callback_clone.clone();
            Box::pin(async move {
                invoked.store(true, Ordering::SeqCst);
                // Cancel pending audio
                q.cancel_audio();
                // Queue clear operation
                let (tx, _rx) = oneshot::channel();
                let _ = q
                    .queue(LiveKitOperation::ClearAudio { response_tx: tx })
                    .await;
            })
        })
        .await;

    // Queue some audio operations (simulating TTS sending audio)
    for _ in 0..3 {
        let (tx, _) = oneshot::channel();
        queue.queue_audio(vec![0u8; 100], tx).await.unwrap();
    }

    // Call clear_tts
    voice_manager.clear_tts().await.unwrap();

    // Verify callback was invoked
    assert!(callback_invoked.load(Ordering::SeqCst));

    // Generation should have been incremented by the callback
    assert_eq!(audio_generation.load(Ordering::Acquire), 1);

    // Drain the queue and verify operations
    let mut audio_ops = 0;
    let mut clear_ops = 0;
    while let Ok(Some(op)) = timeout(Duration::from_millis(50), receiver.recv()).await {
        match op.operation {
            LiveKitOperation::SendAudio { generation, .. } => {
                // All audio operations have stale generation (0 < 1)
                assert_eq!(generation, 0, "Audio should have pre-cancel generation");
                audio_ops += 1;
            }
            LiveKitOperation::ClearAudio { .. } => {
                clear_ops += 1;
            }
            _ => {}
        }
    }

    assert_eq!(audio_ops, 3, "Should have 3 audio operations");
    assert_eq!(clear_ops, 1, "Should have 1 clear operation");
}
