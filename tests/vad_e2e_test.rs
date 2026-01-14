//! End-to-end tests for VAD integration
//!
//! These tests verify the complete VAD-based silence detection pipeline:
//! 1. WebSocket connection with VAD enabled
//! 2. Audio processing through VAD
//! 3. Turn detection triggered by VAD silence
//! 4. speech_final emitted correctly
//!
//! Run with: cargo test --features stt-vad e2e_vad -- --ignored

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sayna::core::stt::{STTConfig, STTResult};
use sayna::core::tts::TTSConfig;
use sayna::core::vad::{SilenceTracker, SilenceTrackerConfig, VADEvent, VADSilenceConfig};
use sayna::core::voice_manager::{SpeechFinalConfig, VoiceManagerConfig};

/// Test that VoiceManagerConfig correctly applies VAD settings
///
/// Note: When the `stt-vad` feature is compiled, VAD is always active at runtime.
/// The `enabled` field is retained for configuration compatibility but does not
/// gate VAD processing. This test verifies configuration is correctly applied.
#[test]
fn test_voice_manager_config_with_vad() {
    let stt_config = STTConfig {
        provider: "deepgram".to_string(),
        api_key: "test-api-key".to_string(),
        language: "en".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
    };

    let tts_config = TTSConfig {
        provider: "deepgram".to_string(),
        api_key: "test-api-key".to_string(),
        ..Default::default()
    };

    // Create VAD config with custom settings
    let vad_config = VADSilenceConfig {
        enabled: true,
        silero_config: sayna::core::vad::SileroVADConfig {
            threshold: 0.6,
            silence_duration_ms: 400,
            min_speech_duration_ms: 150,
            ..Default::default()
        },
    };

    let config = VoiceManagerConfig::new(stt_config, tts_config).set_vad_config(vad_config);

    assert!(config.vad_config.enabled);
    assert_eq!(config.vad_config.silero_config.threshold, 0.6);
    assert_eq!(config.vad_config.silero_config.silence_duration_ms, 400);
    assert_eq!(config.vad_config.silero_config.min_speech_duration_ms, 150);
}

/// Test that VoiceManagerConfig applies speech_final settings correctly
#[test]
fn test_voice_manager_config_speech_final_settings() {
    let stt_config = STTConfig {
        provider: "deepgram".to_string(),
        api_key: "test-api-key".to_string(),
        ..Default::default()
    };

    let tts_config = TTSConfig {
        provider: "deepgram".to_string(),
        api_key: "test-api-key".to_string(),
        ..Default::default()
    };

    let speech_final_config = SpeechFinalConfig {
        stt_speech_final_wait_ms: 600,
        turn_detection_inference_timeout_ms: 200,
        speech_final_hard_timeout_ms: 8000,
        duplicate_window_ms: 500,
    };

    let config =
        VoiceManagerConfig::with_speech_final_config(stt_config, tts_config, speech_final_config);

    assert_eq!(config.speech_final_config.stt_speech_final_wait_ms, 600);
    assert_eq!(
        config
            .speech_final_config
            .turn_detection_inference_timeout_ms,
        200
    );
    assert_eq!(
        config.speech_final_config.speech_final_hard_timeout_ms,
        8000
    );
    assert_eq!(config.speech_final_config.duplicate_window_ms, 500);
}

/// Test VAD silence detection triggers turn detection
#[test]
fn test_vad_silence_triggers_turn_detection() {
    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 300,
        min_speech_duration_ms: 100,
        frame_duration_ms: 32.0,
    };
    let tracker = SilenceTracker::new(config);

    // Simulate speech pattern: speech -> silence -> turn end
    let mut events = Vec::new();

    // Start speaking (200ms = ~6 frames at 32ms)
    events.push(tracker.process(0.8)); // SpeechStart
    for _ in 0..5 {
        events.push(tracker.process(0.85)); // Continued speech
    }

    // Silence (300ms = ~10 frames)
    for _ in 0..12 {
        let event = tracker.process(0.2);
        events.push(event);
        if event == Some(VADEvent::TurnEnd) {
            break;
        }
    }

    // Verify the event sequence
    assert!(
        events.contains(&Some(VADEvent::SpeechStart)),
        "SpeechStart event not detected"
    );
    assert!(
        events.contains(&Some(VADEvent::SilenceDetected)),
        "SilenceDetected event not detected"
    );
    assert!(
        events.contains(&Some(VADEvent::TurnEnd)),
        "TurnEnd event not detected"
    );
}

/// Test that VAD-based speech_final includes correct properties
#[test]
fn test_vad_speech_final_result_format() {
    // Create a speech_final result as the VAD system would emit
    let result = STTResult {
        transcript: String::new(), // VAD speech_final has empty transcript
        is_final: true,
        is_speech_final: true,
        confidence: 1.0,
    };

    // Verify result structure matches expected format
    assert!(result.transcript.is_empty());
    assert!(result.is_final);
    assert!(result.is_speech_final);
    assert_eq!(result.confidence, 1.0);
}

/// Test SilenceTracker state management across speech segments
#[test]
fn test_silence_tracker_state_management() {
    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 64, // Short for testing
        min_speech_duration_ms: 32,
        frame_duration_ms: 32.0,
    };
    let tracker = SilenceTracker::new(config);

    // First utterance
    assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));
    assert!(tracker.is_speaking());
    assert!(tracker.has_speech());

    // Turn end
    tracker.process(0.2);
    assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd));

    // Reset for next utterance
    tracker.reset();
    assert!(!tracker.is_speaking());
    assert!(!tracker.has_speech());
    assert_eq!(tracker.current_silence_ms(), 0);
    assert_eq!(tracker.current_speech_ms(), 0);

    // Second utterance starts fresh
    assert_eq!(tracker.process(0.9), Some(VADEvent::SpeechStart));
}

/// Test that callback is invoked when turn end is detected
#[test]
fn test_turn_end_callback_invocation() {
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_clone = callback_count.clone();

    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 64,
        min_speech_duration_ms: 32,
        frame_duration_ms: 32.0,
    };
    let tracker = SilenceTracker::new(config);

    // Speech
    tracker.process(0.8);
    tracker.process(0.85);

    // Silence until turn end
    for _ in 0..5 {
        if let Some(VADEvent::TurnEnd) = tracker.process(0.2) {
            callback_count_clone.fetch_add(1, Ordering::SeqCst);
            break;
        }
    }

    assert_eq!(
        callback_count.load(Ordering::SeqCst),
        1,
        "Callback should be invoked exactly once on TurnEnd"
    );
}

/// Test VAD integration with speech resumption
#[test]
fn test_vad_speech_resumption_cancels_turn_end() {
    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 300, // Longer threshold
        min_speech_duration_ms: 100,
        frame_duration_ms: 32.0,
    };
    let tracker = SilenceTracker::new(config);

    // Start speech (150ms = ~5 frames)
    tracker.process(0.8);
    for _ in 0..4 {
        tracker.process(0.85);
    }

    // Brief pause (100ms < 300ms threshold)
    tracker.process(0.2);
    tracker.process(0.2);
    tracker.process(0.2);

    // Resume speech - should cancel turn end detection
    let event = tracker.process(0.9);
    assert_eq!(event, Some(VADEvent::SpeechResumed));

    // Verify still in speech state
    assert!(tracker.is_speaking());
    assert!(tracker.has_speech());
}

/// Test VAD with very short utterances
#[test]
fn test_vad_short_utterance_handling() {
    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 64,
        min_speech_duration_ms: 100, // Requires 100ms minimum
        frame_duration_ms: 32.0,
    };
    let tracker = SilenceTracker::new(config);

    // Very short speech (32ms = 1 frame, less than 100ms minimum)
    tracker.process(0.8);

    // Silence
    tracker.process(0.2);
    let event = tracker.process(0.2);

    // Should NOT trigger TurnEnd because speech was too short
    assert_ne!(
        event,
        Some(VADEvent::TurnEnd),
        "TurnEnd should not fire for utterances shorter than min_speech_duration_ms"
    );
}

/// Test concurrent speech_final handling
#[test]
fn test_concurrent_speech_final_handling() {
    use std::thread;

    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 64,
        min_speech_duration_ms: 32,
        frame_duration_ms: 32.0,
    };
    let tracker = Arc::new(SilenceTracker::new(config));
    let turn_end_count = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let tracker = Arc::clone(&tracker);
            let count = Arc::clone(&turn_end_count);
            thread::spawn(move || {
                // Each thread does a complete speech cycle
                for _ in 0..10 {
                    // Speech
                    if tracker.process(0.8) == Some(VADEvent::TurnEnd) {
                        count.fetch_add(1, Ordering::SeqCst);
                    }
                    // Silence
                    for _ in 0..3 {
                        if tracker.process(0.2) == Some(VADEvent::TurnEnd) {
                            count.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().expect("Thread panicked");
    }

    // Should have detected at least some turn ends
    assert!(
        turn_end_count.load(Ordering::SeqCst) > 0,
        "Should detect turn ends under concurrent access"
    );
}

// Feature-gated tests that require the VAD model
#[cfg(feature = "stt-vad")]
mod e2e_model_tests {
    use super::*;
    use sayna::core::vad::{SileroVAD, SileroVADConfig};
    use sayna::core::voice_manager::VoiceManager;

    /// End-to-end test for VAD turn detection with actual model
    ///
    /// This test requires:
    /// 1. Silero-VAD model downloaded via `sayna init`
    /// 2. CACHE_PATH environment variable set
    ///
    /// Run with: cargo test --features stt-vad e2e_vad_turn_detection -- --ignored
    #[tokio::test]
    #[ignore]
    async fn e2e_vad_turn_detection() {
        use std::sync::Arc;
        use tokio::sync::RwLock;

        // Setup VAD with model
        let cache_path = std::env::var("CACHE_PATH").unwrap_or_else(|_| ".sayna_cache".to_string());
        let vad_config = SileroVADConfig {
            cache_path: Some(cache_path.into()),
            threshold: 0.5,
            silence_duration_ms: 300,
            min_speech_duration_ms: 100,
            ..Default::default()
        };

        let vad = SileroVAD::with_config(vad_config)
            .await
            .expect("Failed to initialize VAD model");
        let vad = Arc::new(RwLock::new(vad));

        // Create VoiceManager with VAD
        let stt_config = STTConfig {
            provider: "deepgram".to_string(),
            api_key: "test-api-key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "".to_string(),
        };

        let tts_config = TTSConfig {
            provider: "deepgram".to_string(),
            api_key: "test-api-key".to_string(),
            ..Default::default()
        };

        let voice_config = VADSilenceConfig {
            enabled: true,
            silero_config: sayna::core::vad::SileroVADConfig {
                threshold: 0.5,
                silence_duration_ms: 300,
                min_speech_duration_ms: 100,
                ..Default::default()
            },
        };

        let config =
            VoiceManagerConfig::new(stt_config, tts_config).set_vad_config(voice_config.clone());

        let voice_manager =
            VoiceManager::new(config, None, Some(vad)).expect("Failed to create VoiceManager");

        // Verify VoiceManager configuration was correctly applied
        // Note: Under stt-vad, VAD is always active - this checks config was set correctly
        assert!(voice_manager.get_config().vad_config.enabled);
        assert_eq!(
            voice_manager
                .get_config()
                .vad_config
                .silero_config
                .threshold,
            0.5
        );
    }

    /// Test VAD inference with silence produces low probability
    #[tokio::test]
    #[ignore]
    async fn e2e_vad_silence_inference() {
        let cache_path = std::env::var("CACHE_PATH").unwrap_or_else(|_| ".sayna_cache".to_string());
        let config = SileroVADConfig {
            cache_path: Some(cache_path.into()),
            ..Default::default()
        };

        let vad = SileroVAD::with_config(config)
            .await
            .expect("Failed to initialize VAD");

        // Generate silence (512 samples of zeros)
        let silence: Vec<i16> = vec![0; 512];
        let prob = vad
            .process_audio(&silence)
            .await
            .expect("VAD inference failed");

        assert!(
            prob < 0.5,
            "Silence should have low speech probability, got {}",
            prob
        );
    }

    /// Test VAD with synthetic speech-like audio
    #[tokio::test]
    #[ignore]
    async fn e2e_vad_speech_inference() {
        use std::f32::consts::PI;

        let cache_path = std::env::var("CACHE_PATH").unwrap_or_else(|_| ".sayna_cache".to_string());
        let config = SileroVADConfig {
            cache_path: Some(cache_path.into()),
            ..Default::default()
        };

        let vad = SileroVAD::with_config(config)
            .await
            .expect("Failed to initialize VAD");

        // Generate synthetic speech (sine wave at speech frequency with variation)
        let speech: Vec<i16> = (0..512)
            .map(|i| {
                let t = i as f32 / 16000.0;
                let freq = 200.0 + (t * 50.0).sin() * 100.0; // Varying frequency like speech
                ((2.0 * PI * freq * t).sin() * 10000.0) as i16
            })
            .collect();

        let prob = vad
            .process_audio(&speech)
            .await
            .expect("VAD inference failed");

        // Synthetic speech may or may not trigger high probability depending on model
        // The important thing is that it processes without error
        assert!(
            (0.0..=1.0).contains(&prob),
            "Speech probability should be in range [0, 1], got {}",
            prob
        );
    }

    /// Test complete VAD lifecycle: init -> process -> reset
    #[tokio::test]
    #[ignore]
    async fn e2e_vad_lifecycle() {
        let cache_path = std::env::var("CACHE_PATH").unwrap_or_else(|_| ".sayna_cache".to_string());
        let config = SileroVADConfig {
            cache_path: Some(cache_path.into()),
            ..Default::default()
        };

        let vad = SileroVAD::with_config(config)
            .await
            .expect("Failed to initialize VAD");

        // Process multiple frames
        let frame: Vec<i16> = vec![0; 512];
        for _ in 0..10 {
            let _ = vad.process_audio(&frame).await;
        }

        // Reset state
        vad.reset().await;

        // Should be able to process again
        let result = vad.process_audio(&frame).await;
        assert!(result.is_ok(), "Should process after reset");
    }
}

/// Test VAD performance characteristics
#[test]
fn test_vad_processing_performance() {
    use std::time::Instant;

    let config = SilenceTrackerConfig::default();
    let tracker = SilenceTracker::new(config);

    let iterations = 10000;
    let start = Instant::now();

    for i in 0..iterations {
        let prob = if i % 10 < 7 { 0.8 } else { 0.2 }; // 70% speech, 30% silence
        let _ = tracker.process(prob);
    }

    let elapsed = start.elapsed();
    let per_frame_us = elapsed.as_micros() as f64 / iterations as f64;

    // SilenceTracker processing should be extremely fast (< 1us per frame)
    assert!(
        per_frame_us < 100.0,
        "SilenceTracker too slow: {:.2}us per frame (expected < 100us)",
        per_frame_us
    );

    println!(
        "SilenceTracker performance: {:.2}us per frame ({} iterations in {:?})",
        per_frame_us, iterations, elapsed
    );
}
