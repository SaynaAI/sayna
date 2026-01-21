//! Tests for VoiceManager

use crate::core::stt::{STTConfig, STTResult};
use crate::core::tts::TTSConfig;
use crate::core::voice_manager::state::SpeechFinalState;
use crate::core::voice_manager::stt_result::STTResultProcessor;
use crate::core::voice_manager::utils::get_current_time_ms;
use crate::core::voice_manager::{VoiceManager, VoiceManagerConfig};
use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc;

/// Helper macro to create VoiceManager with feature-gated VAD parameter
#[cfg(feature = "stt-vad")]
macro_rules! create_voice_manager {
    ($config:expr, $turn_detector:expr) => {
        VoiceManager::new($config, $turn_detector, None)
    };
}

#[cfg(not(feature = "stt-vad"))]
macro_rules! create_voice_manager {
    ($config:expr, $turn_detector:expr) => {
        VoiceManager::new($config, $turn_detector)
    };
}

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

    let result = create_voice_manager!(config, None);
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

    let voice_manager = create_voice_manager!(config, None).unwrap();
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

    let voice_manager = create_voice_manager!(config, None).unwrap();

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

    let voice_manager = create_voice_manager!(config, None).unwrap();

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
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState::new();
        }

        // Send is_final=true (turn detection will be spawned via VAD + Smart Turn path)
        let result1 = STTResult::new("Hello".to_string(), true, 0.9);
        let processor = STTResultProcessor::default();
        let processed = processor
            .process_result(result1, speech_final_state.clone())
            .await;

        // Should return original result immediately
        assert!(processed.is_some());
        let processed_result = processed.unwrap();
        assert_eq!(processed_result.transcript, "Hello");
        assert!(processed_result.is_final);

        // Timer should be started and state updated
        let state = speech_final_state.read();
        assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(state.text_buffer, "Hello");
        assert!(state.turn_detection_handle.is_some());
    }

    // Test Case 2: Timer should be started for final results and state should be set correctly
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState::new();
        }

        // Send is_final=true (turn detection spawned via VAD + Smart Turn path)
        let result1 = STTResult::new("Test message".to_string(), true, 0.8);
        let processor = STTResultProcessor::default();
        let processed = processor
            .process_result(result1, speech_final_state.clone())
            .await;

        // Should return the original result immediately
        assert!(processed.is_some());
        let processed_result = processed.unwrap();
        assert_eq!(processed_result.transcript, "Test message");
        assert!(processed_result.is_final);

        // Check that timer was started and state is correct
        let state = speech_final_state.read();
        assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(state.text_buffer, "Test message");
        assert!(state.turn_detection_handle.is_some());
    }

    // Test Case 3: Multiple is_final results should reset timer and accumulate text
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState::new();
        }

        // Send first is_final=true result to start timer
        let result1 = STTResult::new("Hello ".to_string(), true, 0.9);
        let processor = STTResultProcessor::default();
        let _processed1 = processor
            .process_result(result1, speech_final_state.clone())
            .await;

        // Verify timer was started
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.turn_detection_handle.is_some());
            assert_eq!(state.text_buffer, "Hello ");
        }

        // Send another is_final=true - should reset timer and accumulate text
        let result2 = STTResult::new("world".to_string(), true, 0.95);
        let processor2 = STTResultProcessor::default();
        let processed2 = processor2
            .process_result(result2, speech_final_state.clone())
            .await;

        // Should return the result
        assert!(processed2.is_some());
        let final_result = processed2.unwrap();
        assert!(final_result.is_final);
        assert_eq!(final_result.transcript, "world");
        assert_eq!(final_result.confidence, 0.95);

        // State should have accumulated text and still be waiting
        let state = speech_final_state.read();
        assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(state.text_buffer, "Hello world");
        assert!(state.turn_detection_handle.is_some());
    }
}

#[tokio::test]
async fn test_turn_detection_timer_behavior() {
    // Test Case 1: is_final result starts turn detection timer
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // is_final=true arrives
        let result1 = STTResult::new("Hello world".to_string(), true, 0.9);
        let processor1 = STTResultProcessor::default();
        let processed1 = processor1
            .process_result(result1.clone(), speech_final_state.clone())
            .await;

        assert!(processed1.is_some());
        assert_eq!(processed1.unwrap().transcript, "Hello world");

        // Verify timer was started
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.turn_detection_handle.is_some());
        }
    }

    // Test Case 2: Multiple is_final results should restart timer (continuous speech)
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // 1. First is_final=true
        let result1 = STTResult::new("First".to_string(), true, 0.9);
        let processor1 = STTResultProcessor::default();
        let processed1 = processor1
            .process_result(result1, speech_final_state.clone())
            .await;

        assert!(processed1.is_some());

        // Verify timer was started
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.turn_detection_handle.is_some());
        }

        // 2. Another is_final=true arrives (continuous speech)
        let result2 = STTResult::new("Second".to_string(), true, 0.9);
        let processor2 = STTResultProcessor::default();
        let processed2 = processor2
            .process_result(result2, speech_final_state.clone())
            .await;

        // Should still return the result and timer should be reset
        assert!(processed2.is_some());
        assert_eq!(processed2.unwrap().transcript, "Second");

        // Verify timer is still active (reset for continuous speech)
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.turn_detection_handle.is_some());
            // Text should be accumulated
            assert_eq!(state.text_buffer, "FirstSecond");
        }
    }

    // Test Case 3: New speech segment after state reset should work normally
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // First sequence: is_final=true starts timer
        let result1 = STTResult::new("First segment".to_string(), true, 0.9);
        let processor1 = STTResultProcessor::default();
        let processed1 = processor1
            .process_result(result1, speech_final_state.clone())
            .await;
        assert!(processed1.is_some());

        // Simulate turn detection firing and state reset
        {
            let mut state = speech_final_state.write();
            let fire_time_ms = get_current_time_ms();
            state
                .turn_detection_last_fired_ms
                .store(fire_time_ms, Ordering::Release);
            state.last_forced_text = "First segment".to_string();
            state.reset_for_next_segment();
        }

        // Now a completely new segment starts
        let new_result = STTResult::new("New segment".to_string(), true, 0.9);
        let processor_new = STTResultProcessor::default();
        let processed_new = processor_new
            .process_result(new_result, speech_final_state.clone())
            .await;

        assert!(processed_new.is_some());
        assert_eq!(processed_new.unwrap().transcript, "New segment");

        // Verify new timer started for the new segment
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.turn_detection_handle.is_some());
            assert_eq!(state.text_buffer, "New segment");
        }
    }
}

// Hard timeout tests
mod hard_timeout_tests {
    use super::*;
    use crate::core::voice_manager::callbacks::STTCallback;
    use crate::core::voice_manager::stt_config::STTProcessingConfig;
    use std::future::Future;
    use std::pin::Pin;
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_hard_timeout_fires_when_turn_detection_slow() {
        let config = STTProcessingConfig::new(
            50,  // stt_speech_final_wait_ms
            50,  // turn_detection_inference_timeout_ms
            200, // speech_final_hard_timeout_ms - 200ms hard timeout
            100, // duplicate_window_ms
        );

        let processor = STTResultProcessor::new(config);

        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        // Callback counts is_final results (turn detection fires these)
        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        let result = STTResult {
            transcript: "Hello world".to_string(),
            is_final: true,
            confidence: 0.95,
        };

        let processed = processor.process_result(result, state.clone()).await;
        assert!(processed.is_some());

        tokio::time::sleep(Duration::from_millis(300)).await;

        assert_eq!(
            callback_count.load(Ordering::SeqCst),
            1,
            "Hard timeout should have fired turn detection callback"
        );

        let final_state = state.read();
        assert!(!final_state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(final_state.segment_start_ms.load(Ordering::Acquire), 0);
        assert_eq!(
            final_state.hard_timeout_deadline_ms.load(Ordering::Acquire),
            0
        );
    }

    #[tokio::test]
    async fn test_hard_timeout_not_restarted_by_new_is_final() {
        let config = STTProcessingConfig::new(
            300, // stt_speech_final_wait_ms - Long turn detection wait
            50,  // turn_detection_inference_timeout_ms
            200, // speech_final_hard_timeout_ms - Hard timeout fires first
            100, // duplicate_window_ms
        );

        let processor = STTResultProcessor::new(config);

        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        // Callback counts is_final results (turn detection fires these)
        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        let result1 = STTResult {
            transcript: "Hello".to_string(),
            is_final: true,
            confidence: 0.95,
        };

        processor.process_result(result1, state.clone()).await;

        tokio::time::sleep(Duration::from_millis(100)).await;

        let result2 = STTResult {
            transcript: " world".to_string(),
            is_final: true,
            confidence: 0.95,
        };

        processor.process_result(result2, state.clone()).await;

        tokio::time::sleep(Duration::from_millis(150)).await;

        assert_eq!(
            callback_count.load(Ordering::SeqCst),
            1,
            "Hard timeout should fire once based on first is_final timestamp"
        );
    }

    #[tokio::test]
    async fn test_segment_timing_tracks_first_is_final() {
        let config = STTProcessingConfig::default();
        let processor = STTResultProcessor::new(config);

        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        let result = STTResult {
            transcript: "First utterance".to_string(),
            is_final: true,
            confidence: 0.95,
        };

        processor.process_result(result, state.clone()).await;

        // Verify segment timing is set
        {
            let s = state.read();
            assert_ne!(s.segment_start_ms.load(Ordering::Acquire), 0);
            assert_ne!(s.hard_timeout_deadline_ms.load(Ordering::Acquire), 0);
        }

        // Simulate state reset (as if turn detection fired)
        {
            let mut s = state.write();
            s.reset_for_next_segment();
        }

        // Verify timing is reset
        {
            let s = state.read();
            assert_eq!(s.segment_start_ms.load(Ordering::Acquire), 0);
            assert_eq!(s.hard_timeout_deadline_ms.load(Ordering::Acquire), 0);
        }

        // New segment starts
        let result2 = STTResult {
            transcript: "Second utterance".to_string(),
            is_final: true,
            confidence: 0.95,
        };

        processor.process_result(result2, state.clone()).await;

        // Verify new timing is set
        {
            let s = state.read();
            assert_ne!(s.segment_start_ms.load(Ordering::Acquire), 0);
            assert_ne!(s.hard_timeout_deadline_ms.load(Ordering::Acquire), 0);
        }
    }
}
