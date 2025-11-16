//! Tests for VoiceManager

use crate::core::stt::{STTConfig, STTResult};
use crate::core::tts::TTSConfig;
use crate::core::voice_manager::state::SpeechFinalState;
use crate::core::voice_manager::stt_result::STTResultProcessor;
use crate::core::voice_manager::{VoiceManager, VoiceManagerConfig};
use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::mpsc;

#[tokio::test]
async fn test_voice_manager_creation() {
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
    let config = VoiceManagerConfig::new(stt_config, tts_config);

    let result = VoiceManager::new(config, None);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_voice_manager_config_access() {
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
    let config = VoiceManagerConfig::new(stt_config, tts_config);

    let voice_manager = VoiceManager::new(config, None).unwrap();
    let retrieved_config = voice_manager.get_config();

    assert_eq!(retrieved_config.stt_config.provider, "deepgram");
    assert_eq!(retrieved_config.tts_config.provider, "deepgram");
}

#[tokio::test]
async fn test_voice_manager_callback_registration() {
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
    let config = VoiceManagerConfig::new(stt_config, tts_config);

    let voice_manager = VoiceManager::new(config, None).unwrap();

    // Test STT callback registration
    let stt_result = voice_manager
        .on_stt_result(|result| {
            Box::pin(async move {
                println!("STT Result: {}", result.transcript);
            })
        })
        .await;

    assert!(stt_result.is_ok());

    // Test TTS callback registration
    let tts_result = voice_manager
        .on_tts_audio(|audio_data| {
            Box::pin(async move {
                println!("TTS Audio: {} bytes", audio_data.data.len());
            })
        })
        .await;

    assert!(tts_result.is_ok());
}

#[tokio::test]
async fn test_speech_final_timing_control() {
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
    let config = VoiceManagerConfig::new(stt_config, tts_config);

    let voice_manager = VoiceManager::new(config, None).unwrap();

    // Channel to collect results
    let (tx, _rx) = mpsc::unbounded_channel();
    let result_counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = result_counter.clone();

    // Register callback to collect results
    voice_manager
        .on_stt_result(move |result| {
            let tx = tx.clone();
            let counter = counter_clone.clone();
            Box::pin(async move {
                counter.fetch_add(1, Ordering::Relaxed);
                let _ = tx.send(result);
            })
        })
        .await
        .unwrap();

    // Test Case 1: is_final result should return immediately and start timer
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::with_capacity(1024),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: None,
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::with_capacity(1024),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState {
                text_buffer: String::with_capacity(1024),
                turn_detection_handle: None,
                hard_timeout_handle: None,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
                turn_detection_last_fired_ms: AtomicUsize::new(0),
                last_forced_text: String::with_capacity(1024),
                segment_start_ms: AtomicUsize::new(0),
                hard_timeout_deadline_ms: AtomicUsize::new(0),
            };
        }

        // Send is_final=true without is_speech_final=true
        let result1 = STTResult::new("Hello".to_string(), true, false, 0.9);
        let processor = STTResultProcessor::default();
        let processed = processor
            .process_result(result1, speech_final_state.clone(), None)
            .await;

        // Should return original result immediately
        assert!(processed.is_some());
        let processed_result = processed.unwrap();
        assert_eq!(processed_result.transcript, "Hello");
        assert!(processed_result.is_final);
        assert!(!processed_result.is_speech_final);

        // Timer should be started and state updated
        let state = speech_final_state.read();
        assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(state.text_buffer, "Hello");
        assert!(state.turn_detection_handle.is_some());
    }

    // Test Case 2: Timer should be started for final results and state should be set correctly
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::with_capacity(1024),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: None,
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::with_capacity(1024),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState {
                text_buffer: String::with_capacity(1024),
                turn_detection_handle: None,
                hard_timeout_handle: None,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
                turn_detection_last_fired_ms: AtomicUsize::new(0),
                last_forced_text: String::with_capacity(1024),
                segment_start_ms: AtomicUsize::new(0),
                hard_timeout_deadline_ms: AtomicUsize::new(0),
            };
        }

        // Send is_final=true without is_speech_final=true
        let result1 = STTResult::new("Test message".to_string(), true, false, 0.8);
        let processor = STTResultProcessor::default();
        let processed = processor
            .process_result(result1, speech_final_state.clone(), None)
            .await;

        // Should return the original result immediately
        assert!(processed.is_some());
        let processed_result = processed.unwrap();
        assert_eq!(processed_result.transcript, "Test message");
        assert!(processed_result.is_final);
        assert!(!processed_result.is_speech_final);

        // Check that timer was started and state is correct
        let state = speech_final_state.read();
        assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(state.text_buffer, "Test message");
        assert!(state.turn_detection_handle.is_some());
    }

    // Test Case 3: Real speech_final should cancel timer and reset state
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::with_capacity(1024),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: None,
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::with_capacity(1024),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState {
                text_buffer: String::with_capacity(1024),
                turn_detection_handle: None,
                hard_timeout_handle: None,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
                turn_detection_last_fired_ms: AtomicUsize::new(0),
                last_forced_text: String::with_capacity(1024),
                segment_start_ms: AtomicUsize::new(0),
                hard_timeout_deadline_ms: AtomicUsize::new(0),
            };
        }

        // Send is_final=true result to start timer
        let result1 = STTResult::new("Hello world".to_string(), true, false, 0.9);
        let processor = STTResultProcessor::default();
        let _processed1 = processor
            .process_result(result1, speech_final_state.clone(), None)
            .await;

        // Verify timer was started
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.turn_detection_handle.is_some());
            assert_eq!(state.text_buffer, "Hello world");
        }

        // Send is_speech_final=true (should cancel timer and reset state)
        let result2 = STTResult::new("final result".to_string(), true, true, 0.95);
        let processor2 = STTResultProcessor::default();
        let processed2 = processor2
            .process_result(result2, speech_final_state.clone(), None)
            .await;

        // Should return the original speech_final result
        assert!(processed2.is_some());
        let final_result = processed2.unwrap();
        assert!(final_result.is_speech_final);
        assert!(final_result.is_final);
        assert_eq!(final_result.transcript, "final result");
        assert_eq!(final_result.confidence, 0.95);

        // State should be reset
        let state = speech_final_state.read();
        assert!(!state.waiting_for_speech_final.load(Ordering::Acquire));
        assert!(state.text_buffer.is_empty());
        assert!(state.turn_detection_handle.is_none());
    }

    // Test Case 4: Direct speech_final with no prior timer should return original result
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::with_capacity(1024),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: None,
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::with_capacity(1024),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState {
                text_buffer: String::with_capacity(1024),
                turn_detection_handle: None,
                hard_timeout_handle: None,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
                turn_detection_last_fired_ms: AtomicUsize::new(0),
                last_forced_text: String::with_capacity(1024),
                segment_start_ms: AtomicUsize::new(0),
                hard_timeout_deadline_ms: AtomicUsize::new(0),
            };
        }

        // Send is_speech_final=true with no prior timer (direct speech final)
        let result = STTResult::new("Direct speech final".to_string(), true, true, 0.85);
        let processor = STTResultProcessor::default();
        let processed = processor
            .process_result(result, speech_final_state.clone(), None)
            .await;

        assert!(processed.is_some());
        let final_result = processed.unwrap();
        assert!(final_result.is_speech_final);
        assert!(final_result.is_final);
        // Should return original text as-is
        assert_eq!(final_result.transcript, "Direct speech final");
        assert_eq!(final_result.confidence, 0.85);

        // State should be reset
        let state = speech_final_state.read();
        assert!(!state.waiting_for_speech_final.load(Ordering::Acquire));
        assert!(state.text_buffer.is_empty());
    }
}

#[tokio::test]
async fn test_duplicate_speech_final_prevention() {
    // Test Case 1: Timer fires, then real speech_final arrives - should prevent duplicate
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::with_capacity(1024),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: None,
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::with_capacity(1024),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

        // Simulate the scenario:
        // 1. is_final=true arrives
        let result1 = STTResult::new("Hello world".to_string(), true, false, 0.9);
        let processor1 = STTResultProcessor::default();
        let processed1 = processor1
            .process_result(result1.clone(), speech_final_state.clone(), None)
            .await;

        assert!(processed1.is_some());
        assert_eq!(processed1.unwrap().transcript, "Hello world");

        // Verify timer was started
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.turn_detection_handle.is_some());
        }

        // 2. Simulate timer firing (mark as fired)
        {
            let mut state = speech_final_state.write();
            let fire_time_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as usize;
            state
                .turn_detection_last_fired_ms
                .store(fire_time_ms, Ordering::Release);
            state.last_forced_text = "Hello world".to_string();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);
        }

        // 3. Real speech_final arrives after timer fired
        let result2 = STTResult::new("Hello world".to_string(), true, true, 0.95);
        let processor2 = STTResultProcessor::default();
        let processed2 = processor2
            .process_result(result2, speech_final_state.clone(), None)
            .await;

        // Should be None (ignored) because timer already fired
        assert!(processed2.is_none());
    }

    // Test Case 2: Multiple is_final results after timer fired should not restart timer
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::with_capacity(1024),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: None,
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::with_capacity(1024),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

        // 1. First is_final=true
        let result1 = STTResult::new("First".to_string(), true, false, 0.9);
        let processor1 = STTResultProcessor::default();
        let processed1 = processor1
            .process_result(result1, speech_final_state.clone(), None)
            .await;

        assert!(processed1.is_some());

        // Mark timer as fired (simulate timer expiry)
        {
            let mut state = speech_final_state.write();
            let old_time_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as usize
                - 1000; // 1 second ago
            state
                .turn_detection_last_fired_ms
                .store(old_time_ms, Ordering::Release);
            state.last_forced_text = "First".to_string();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);
        }

        // 2. Another is_final=true arrives after timer fired
        let result2 = STTResult::new("Second".to_string(), true, false, 0.9);
        let processor2 = STTResultProcessor::default();
        let processed2 = processor2
            .process_result(result2, speech_final_state.clone(), None)
            .await;

        // Should still return the result but NOT start a new timer
        assert!(processed2.is_some());
        assert_eq!(processed2.unwrap().transcript, "Second");

        // Verify new timer WAS started (continuous speech should work)
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.turn_detection_handle.is_some());
        }
    }

    // Test Case 3: New speech segment after proper reset should work normally
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::with_capacity(1024),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: None,
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::with_capacity(1024),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

        // First sequence: is_final=true starts timer
        let result1 = STTResult::new("First segment".to_string(), true, false, 0.9);
        let processor1 = STTResultProcessor::default();
        let processed1 = processor1
            .process_result(result1, speech_final_state.clone(), None)
            .await;
        assert!(processed1.is_some());

        // Mark timer as fired (with recent timestamp)
        {
            let mut state = speech_final_state.write();
            let fire_time_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as usize;
            state
                .turn_detection_last_fired_ms
                .store(fire_time_ms, Ordering::Release);
            state.last_forced_text = "First segment".to_string();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);
        }

        // Real speech_final arrives but is ignored (timer already fired)
        let result2 = STTResult::new("First segment".to_string(), true, true, 0.9);
        let processor2 = STTResultProcessor::default();
        let processed2 = processor2
            .process_result(result2, speech_final_state.clone(), None)
            .await;
        assert!(processed2.is_none()); // Ignored due to timer fired recently with same text

        // Clear the state to simulate a clean new segment
        {
            let mut state = speech_final_state.write();
            state.text_buffer.clear();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);
            // Clear timer fired state to allow new timers
        }

        // Now a completely new segment starts (new is_final without speech_final)
        let new_result = STTResult::new("New segment".to_string(), true, false, 0.9);
        let processor_new = STTResultProcessor::default();
        let processed_new = processor_new
            .process_result(new_result, speech_final_state.clone(), None)
            .await;

        assert!(processed_new.is_some());
        assert_eq!(processed_new.unwrap().transcript, "New segment");

        // Verify new timer started for continuous speech
        {
            let state = speech_final_state.read();
            // New timer should be started for continuous speech
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.turn_detection_handle.is_some());
        }
    }
}

#[tokio::test]
async fn test_hard_timeout_fallback_without_turn_detector() {
    // Test that hard timeout fires when no turn detector is available
    // This is the regression test for the bug where utterances could hang indefinitely

    use crate::core::voice_manager::stt_result::STTProcessingConfig;

    let config = STTProcessingConfig {
        stt_speech_final_wait_ms: 50,
        turn_detection_inference_timeout_ms: 50,
        speech_final_hard_timeout_ms: 200, // 200ms hard timeout for faster testing
        duplicate_window_ms: 100,
    };

    let processor = crate::core::voice_manager::stt_result::STTResultProcessor::new(config);

    // Track callback invocations
    let callback_fired = Arc::new(AtomicBool::new(false));
    let callback_fired_clone = callback_fired.clone();

    use crate::core::voice_manager::callbacks::STTCallback;
    use std::future::Future;
    use std::pin::Pin;

    let callback: STTCallback = Arc::new(move |result: STTResult| {
        let fired = callback_fired_clone.clone();
        Box::pin(async move {
            if result.is_speech_final {
                fired.store(true, Ordering::SeqCst);
            }
        }) as Pin<Box<dyn Future<Output = ()> + Send>>
    });

    let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
        text_buffer: String::new(),
        turn_detection_handle: None,
        hard_timeout_handle: None,
        waiting_for_speech_final: AtomicBool::new(false),
        user_callback: Some(callback),
        turn_detection_last_fired_ms: AtomicUsize::new(0),
        last_forced_text: String::new(),
        segment_start_ms: AtomicUsize::new(0),
        hard_timeout_deadline_ms: AtomicUsize::new(0),
    }));

    // Send is_final result without speech_final (simulating Deepgram behavior)
    let result = STTResult::new("Hello world".to_string(), true, false, 0.95);

    // Process with NO turn detector
    let processed = processor
        .process_result(result, speech_final_state.clone(), None)
        .await;

    // Should return the result immediately
    assert!(processed.is_some());
    assert_eq!(processed.unwrap().transcript, "Hello world");

    // State should indicate we're waiting for speech_final
    {
        let state = speech_final_state.read();
        assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        assert!(state.hard_timeout_handle.is_some());
    }

    // Wait for hard timeout to fire
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Hard timeout should have fired the callback
    assert!(
        callback_fired.load(Ordering::SeqCst),
        "Hard timeout should force speech_final after 200ms"
    );

    // State should be reset
    {
        let state = speech_final_state.read();
        assert!(!state.waiting_for_speech_final.load(Ordering::Acquire));
        assert!(state.text_buffer.is_empty());
        assert!(state.hard_timeout_handle.is_none());
    }
}

#[tokio::test]
#[ignore = "Requires turn detector model files to be downloaded"]
async fn test_hard_timeout_with_turn_detector_failure() {
    // Test that hard timeout fires even if turn detector fails

    use crate::core::turn_detect::TurnDetector;
    use crate::core::voice_manager::stt_result::STTProcessingConfig;
    use tokio::sync::RwLock;

    let config = STTProcessingConfig {
        stt_speech_final_wait_ms: 50,
        turn_detection_inference_timeout_ms: 50,
        speech_final_hard_timeout_ms: 200,
        duplicate_window_ms: 100,
    };

    let processor = crate::core::voice_manager::stt_result::STTResultProcessor::new(config);

    // Create a turn detector with temporary cache directory
    use crate::core::turn_detect::TurnDetectorConfig;
    let temp_dir = std::env::temp_dir().join("sayna_test_turn_detect");
    let turn_config = TurnDetectorConfig {
        cache_path: Some(temp_dir),
        ..Default::default()
    };

    let turn_detector = Arc::new(RwLock::new(
        TurnDetector::with_config(turn_config)
            .await
            .expect("Failed to create turn detector"),
    ));

    let callback_fired = Arc::new(AtomicBool::new(false));
    let callback_fired_clone = callback_fired.clone();

    use crate::core::voice_manager::callbacks::STTCallback;
    use std::future::Future;
    use std::pin::Pin;

    let callback: STTCallback = Arc::new(move |result: STTResult| {
        let fired = callback_fired_clone.clone();
        Box::pin(async move {
            if result.is_speech_final {
                fired.store(true, Ordering::SeqCst);
            }
        }) as Pin<Box<dyn Future<Output = ()> + Send>>
    });

    let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
        text_buffer: String::new(),
        turn_detection_handle: None,
        hard_timeout_handle: None,
        waiting_for_speech_final: AtomicBool::new(false),
        user_callback: Some(callback),
        turn_detection_last_fired_ms: AtomicUsize::new(0),
        last_forced_text: String::new(),
        segment_start_ms: AtomicUsize::new(0),
        hard_timeout_deadline_ms: AtomicUsize::new(0),
    }));

    // Send is_final result
    let result = STTResult::new("Test utterance".to_string(), true, false, 0.95);

    // Process with turn detector that may fail or return false
    let processed = processor
        .process_result(result, speech_final_state.clone(), Some(turn_detector))
        .await;

    assert!(processed.is_some());

    // Wait for hard timeout to fire (even if turn detector fails)
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Hard timeout should still fire regardless of turn detector behavior
    assert!(
        callback_fired.load(Ordering::SeqCst),
        "Hard timeout should fire even if turn detector fails"
    );
}

#[tokio::test]
async fn test_cancellation_cleanup_on_real_speech_final() {
    // Test that both turn_detection_handle and hard_timeout_handle are properly
    // aborted and cleared when real speech_final arrives

    use crate::core::voice_manager::stt_result::STTProcessingConfig;

    let config = STTProcessingConfig {
        stt_speech_final_wait_ms: 1000, // Long enough to not interfere
        turn_detection_inference_timeout_ms: 100,
        speech_final_hard_timeout_ms: 5000, // Long timeout
        duplicate_window_ms: 500,
    };

    let processor = crate::core::voice_manager::stt_result::STTResultProcessor::new(config);

    use crate::core::voice_manager::callbacks::STTCallback;
    use std::future::Future;
    use std::pin::Pin;

    let callback: STTCallback = Arc::new(move |_result: STTResult| {
        Box::pin(async move {}) as Pin<Box<dyn Future<Output = ()> + Send>>
    });

    let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
        text_buffer: String::new(),
        turn_detection_handle: None,
        hard_timeout_handle: None,
        waiting_for_speech_final: AtomicBool::new(false),
        user_callback: Some(callback),
        turn_detection_last_fired_ms: AtomicUsize::new(0),
        last_forced_text: String::new(),
        segment_start_ms: AtomicUsize::new(0),
        hard_timeout_deadline_ms: AtomicUsize::new(0),
    }));

    // Send is_final to start both timers
    let result1 = STTResult::new("Hello world".to_string(), true, false, 0.9);
    let processed1 = processor
        .process_result(result1, speech_final_state.clone(), None)
        .await;

    assert!(processed1.is_some());

    // Verify both handles were created
    {
        let state = speech_final_state.read();
        assert!(
            state.turn_detection_handle.is_some(),
            "Turn detection handle should be created"
        );
        assert!(
            state.hard_timeout_handle.is_some(),
            "Hard timeout handle should be created"
        );
        assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
    }

    // Wait a bit
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Send real speech_final before any timeout fires
    let result2 = STTResult::new("Hello world complete".to_string(), true, true, 0.95);
    let processed2 = processor
        .process_result(result2, speech_final_state.clone(), None)
        .await;

    assert!(processed2.is_some());

    // Verify both handles were cleaned up
    {
        let state = speech_final_state.read();
        assert!(
            state.turn_detection_handle.is_none(),
            "Turn detection handle should be cancelled"
        );
        assert!(
            state.hard_timeout_handle.is_none(),
            "Hard timeout handle should be cancelled"
        );
        assert!(!state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(state.segment_start_ms.load(Ordering::Acquire), 0);
        assert_eq!(state.hard_timeout_deadline_ms.load(Ordering::Acquire), 0);
    }
}

#[tokio::test]
async fn test_continuous_speech_hard_timeout_not_restarted() {
    // Test that hard timeout is NOT restarted when new is_final results arrive
    // during the same speech segment (person still talking)

    use crate::core::voice_manager::stt_result::STTProcessingConfig;

    let config = STTProcessingConfig {
        stt_speech_final_wait_ms: 300, // Longer than hard timeout
        turn_detection_inference_timeout_ms: 50,
        speech_final_hard_timeout_ms: 200, // Hard timeout fires first
        duplicate_window_ms: 100,
    };

    let processor = crate::core::voice_manager::stt_result::STTResultProcessor::new(config);

    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_clone = callback_count.clone();

    use crate::core::voice_manager::callbacks::STTCallback;
    use std::future::Future;
    use std::pin::Pin;

    let callback: STTCallback = Arc::new(move |result: STTResult| {
        let count = callback_count_clone.clone();
        Box::pin(async move {
            if result.is_speech_final {
                count.fetch_add(1, Ordering::SeqCst);
            }
        }) as Pin<Box<dyn Future<Output = ()> + Send>>
    });

    let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
        text_buffer: String::new(),
        turn_detection_handle: None,
        hard_timeout_handle: None,
        waiting_for_speech_final: AtomicBool::new(false),
        user_callback: Some(callback),
        turn_detection_last_fired_ms: AtomicUsize::new(0),
        last_forced_text: String::new(),
        segment_start_ms: AtomicUsize::new(0),
        hard_timeout_deadline_ms: AtomicUsize::new(0),
    }));

    // Send first is_final at t=0
    let result1 = STTResult::new("Hello".to_string(), true, false, 0.95);
    processor
        .process_result(result1, speech_final_state.clone(), None)
        .await;

    // Record the initial deadline
    let initial_deadline = {
        let state = speech_final_state.read();
        state.hard_timeout_deadline_ms.load(Ordering::Acquire)
    };

    assert_ne!(initial_deadline, 0, "Hard timeout deadline should be set");

    // Wait 100ms and send another is_final (person still talking)
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    let result2 = STTResult::new(" world".to_string(), true, false, 0.95);
    processor
        .process_result(result2, speech_final_state.clone(), None)
        .await;

    // Verify deadline hasn't changed (hard timeout not restarted)
    let second_deadline = {
        let state = speech_final_state.read();
        state.hard_timeout_deadline_ms.load(Ordering::Acquire)
    };

    assert_eq!(
        initial_deadline, second_deadline,
        "Hard timeout deadline should NOT be restarted by new is_final"
    );

    // Wait for hard timeout to fire (should fire at t=200ms from first is_final)
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    // Hard timeout should fire exactly once
    assert_eq!(
        callback_count.load(Ordering::SeqCst),
        1,
        "Hard timeout should fire once based on first is_final timestamp"
    );
}

#[tokio::test]
async fn test_hard_timeout_observability() {
    // Test that hard timeout fires and enables observability
    // This is crucial for SREs to track fallback occurrences through logs
    //
    // Expected behavior:
    // - When hard timeout fires, it emits: tracing::warn!("Hard timeout fired after {}ms...")
    // - This log is at WARN level so it's easily alertable in production
    // - The log includes timing information for debugging
    //
    // See implementation at src/core/voice_manager/stt_result.rs:258-261

    use crate::core::voice_manager::stt_result::STTProcessingConfig;

    let config = STTProcessingConfig {
        stt_speech_final_wait_ms: 50,
        turn_detection_inference_timeout_ms: 50,
        speech_final_hard_timeout_ms: 200,
        duplicate_window_ms: 100,
    };

    let processor = crate::core::voice_manager::stt_result::STTResultProcessor::new(config);

    let callback_fired = Arc::new(AtomicBool::new(false));
    let callback_fired_clone = callback_fired.clone();

    use crate::core::voice_manager::callbacks::STTCallback;
    use std::future::Future;
    use std::pin::Pin;

    let callback: STTCallback = Arc::new(move |result: STTResult| {
        let fired = callback_fired_clone.clone();
        Box::pin(async move {
            if result.is_speech_final {
                fired.store(true, Ordering::SeqCst);
            }
        }) as Pin<Box<dyn Future<Output = ()> + Send>>
    });

    let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
        text_buffer: String::new(),
        turn_detection_handle: None,
        hard_timeout_handle: None,
        waiting_for_speech_final: AtomicBool::new(false),
        user_callback: Some(callback),
        turn_detection_last_fired_ms: AtomicUsize::new(0),
        last_forced_text: String::new(),
        segment_start_ms: AtomicUsize::new(0),
        hard_timeout_deadline_ms: AtomicUsize::new(0),
    }));

    // Send is_final result
    let result = STTResult::new("Test message".to_string(), true, false, 0.95);
    processor
        .process_result(result, speech_final_state.clone(), None)
        .await;

    // Wait for hard timeout to fire (this emits a warning log for observability)
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Verify the callback was fired
    assert!(
        callback_fired.load(Ordering::SeqCst),
        "Hard timeout should fire and emit warning log for SRE monitoring"
    );

    // Note: The actual log verification happens in production monitoring.
    // The implementation emits: tracing::warn!("Hard timeout fired after {}ms - forcing speech_final...")
    // This allows SREs to create alerts on fallback frequency.
}
