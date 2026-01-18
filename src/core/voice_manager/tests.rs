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

        // Send is_final=true without is_speech_final=true
        let result1 = STTResult::new("Hello".to_string(), true, false, 0.9);
        let processor = STTResultProcessor::default();
        let processed = processor
            .process_result(result1, speech_final_state.clone())
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
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState::new();
        }

        // Send is_final=true without is_speech_final=true
        let result1 = STTResult::new("Test message".to_string(), true, false, 0.8);
        let processor = STTResultProcessor::default();
        let processed = processor
            .process_result(result1, speech_final_state.clone())
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
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState::new();
        }

        // Send is_final=true result to start timer
        let result1 = STTResult::new("Hello world".to_string(), true, false, 0.9);
        let processor = STTResultProcessor::default();
        let _processed1 = processor
            .process_result(result1, speech_final_state.clone())
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
            .process_result(result2, speech_final_state.clone())
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
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState::new();
        }

        // Send is_speech_final=true with no prior timer (direct speech final)
        let result = STTResult::new("Direct speech final".to_string(), true, true, 0.85);
        let processor = STTResultProcessor::default();
        let processed = processor
            .process_result(result, speech_final_state.clone())
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
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // Simulate the scenario:
        // 1. is_final=true arrives
        let result1 = STTResult::new("Hello world".to_string(), true, false, 0.9);
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

        // 2. Simulate timer firing (mark as fired)
        {
            let mut state = speech_final_state.write();
            let fire_time_ms = get_current_time_ms();
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
            .process_result(result2, speech_final_state.clone())
            .await;

        // Should be None (ignored) because timer already fired
        assert!(processed2.is_none());
    }

    // Test Case 2: Multiple is_final results after timer fired should not restart timer
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // 1. First is_final=true
        let result1 = STTResult::new("First".to_string(), true, false, 0.9);
        let processor1 = STTResultProcessor::default();
        let processed1 = processor1
            .process_result(result1, speech_final_state.clone())
            .await;

        assert!(processed1.is_some());

        // Mark timer as fired (simulate timer expiry)
        {
            let mut state = speech_final_state.write();
            let old_time_ms = get_current_time_ms() - 1000; // 1 second ago
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
            .process_result(result2, speech_final_state.clone())
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
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // First sequence: is_final=true starts timer
        let result1 = STTResult::new("First segment".to_string(), true, false, 0.9);
        let processor1 = STTResultProcessor::default();
        let processed1 = processor1
            .process_result(result1, speech_final_state.clone())
            .await;
        assert!(processed1.is_some());

        // Mark timer as fired (with recent timestamp)
        {
            let mut state = speech_final_state.write();
            let fire_time_ms = get_current_time_ms();
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
            .process_result(result2, speech_final_state.clone())
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
            .process_result(new_result, speech_final_state.clone())
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

// Hard timeout tests
mod hard_timeout_tests {
    use super::*;
    use crate::core::voice_manager::callbacks::STTCallback;
    use crate::core::voice_manager::stt_config::STTProcessingConfig;
    use std::future::Future;
    use std::pin::Pin;
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_hard_timeout_fires_when_no_speech_final() {
        let config = STTProcessingConfig::new(
            50,  // stt_speech_final_wait_ms
            50,  // turn_detection_inference_timeout_ms
            200, // speech_final_hard_timeout_ms - 200ms hard timeout
            100, // duplicate_window_ms
        );

        let processor = STTResultProcessor::new(config);

        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_speech_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        let result = STTResult {
            transcript: "Hello world".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        let processed = processor.process_result(result, state.clone()).await;
        assert!(processed.is_some());

        tokio::time::sleep(Duration::from_millis(300)).await;

        assert_eq!(
            callback_count.load(Ordering::SeqCst),
            1,
            "Hard timeout should have fired speech_final callback"
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
    async fn test_hard_timeout_cancelled_by_real_speech_final() {
        let config = STTProcessingConfig::new(
            50,  // stt_speech_final_wait_ms
            50,  // turn_detection_inference_timeout_ms
            500, // speech_final_hard_timeout_ms - Long timeout
            100, // duplicate_window_ms
        );

        let processor = STTResultProcessor::new(config);

        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_speech_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        let is_final_result = STTResult {
            transcript: "Hello".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        processor
            .process_result(is_final_result, state.clone())
            .await;

        tokio::time::sleep(Duration::from_millis(100)).await;

        let speech_final_result = STTResult {
            transcript: "Hello world".to_string(),
            is_final: true,
            is_speech_final: true,
            confidence: 0.95,
        };

        processor
            .process_result(speech_final_result, state.clone())
            .await;

        tokio::time::sleep(Duration::from_millis(500)).await;

        assert_eq!(
            callback_count.load(Ordering::SeqCst),
            1,
            "Only real speech_final should fire, not hard timeout"
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

        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_speech_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        let result1 = STTResult {
            transcript: "Hello".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        processor.process_result(result1, state.clone()).await;

        tokio::time::sleep(Duration::from_millis(100)).await;

        let result2 = STTResult {
            transcript: " world".to_string(),
            is_final: true,
            is_speech_final: false,
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
    async fn test_segment_timing_reset_after_speech_final() {
        let config = STTProcessingConfig::default();
        let processor = STTResultProcessor::new(config);

        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_speech_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        let result = STTResult {
            transcript: "First utterance".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        processor.process_result(result, state.clone()).await;

        {
            let s = state.read();
            assert_ne!(s.segment_start_ms.load(Ordering::Acquire), 0);
            assert_ne!(s.hard_timeout_deadline_ms.load(Ordering::Acquire), 0);
        }

        let speech_final = STTResult {
            transcript: "First utterance complete".to_string(),
            is_final: true,
            is_speech_final: true,
            confidence: 0.95,
        };

        processor.process_result(speech_final, state.clone()).await;

        {
            let s = state.read();
            assert_eq!(s.segment_start_ms.load(Ordering::Acquire), 0);
            assert_eq!(s.hard_timeout_deadline_ms.load(Ordering::Acquire), 0);
        }

        let result2 = STTResult {
            transcript: "Second utterance".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        processor.process_result(result2, state.clone()).await;

        {
            let s = state.read();
            assert_ne!(s.segment_start_ms.load(Ordering::Acquire), 0);
            assert_ne!(s.hard_timeout_deadline_ms.load(Ordering::Acquire), 0);
        }
    }
}
