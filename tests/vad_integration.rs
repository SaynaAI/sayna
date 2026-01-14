//! Integration tests for Silero-VAD
//!
//! These tests verify the VAD system works correctly as a whole,
//! including SilenceTracker behavior and configuration handling.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sayna::core::vad::{SilenceTracker, SilenceTrackerConfig, VADEvent};

/// Test that SilenceTracker correctly identifies turn end after silence threshold
#[test]
fn test_silence_tracker_turn_detection() {
    let config = SilenceTrackerConfig::from_sample_rate(16000, 0.5, 300, 100);
    let tracker = SilenceTracker::new(config);

    // Simulate 200ms of speech (about 6 frames at 32ms)
    for _ in 0..6 {
        tracker.process(0.8);
    }

    // Simulate silence until turn end (300ms = ~10 frames)
    let mut turn_end_detected = false;
    for _ in 0..15 {
        if let Some(VADEvent::TurnEnd) = tracker.process(0.2) {
            turn_end_detected = true;
            break;
        }
    }

    assert!(
        turn_end_detected,
        "TurnEnd should be detected after 300ms silence"
    );
}

/// Test that brief pauses don't trigger turn end
#[test]
fn test_brief_pause_no_turn_end() {
    let config = SilenceTrackerConfig::from_sample_rate(16000, 0.5, 300, 100);
    let tracker = SilenceTracker::new(config);

    // Speech - 160ms (5 frames)
    for _ in 0..5 {
        tracker.process(0.8);
    }

    // Brief pause (64ms = 2 frames) - under 300ms threshold
    tracker.process(0.2);
    tracker.process(0.2);

    // Speech resumes
    let event = tracker.process(0.8);
    assert_eq!(event, Some(VADEvent::SpeechResumed));

    // More speech
    for _ in 0..5 {
        tracker.process(0.8);
    }

    // No TurnEnd should have been triggered
    assert!(tracker.is_speaking());
}

/// Test configuration from sample rate
#[test]
fn test_config_from_sample_rate_16khz() {
    let config = SilenceTrackerConfig::from_sample_rate(16000, 0.5, 300, 100);
    assert_eq!(config.frame_duration_ms, 32.0);
    assert_eq!(config.threshold, 0.5);
    assert_eq!(config.silence_duration_ms, 300);
    assert_eq!(config.min_speech_duration_ms, 100);
}

#[test]
fn test_config_from_sample_rate_8khz() {
    let config = SilenceTrackerConfig::from_sample_rate(8000, 0.5, 300, 100);
    assert_eq!(config.frame_duration_ms, 32.0); // 256 samples at 8kHz = 32ms
}

/// Test that VAD events trigger appropriate callbacks
#[test]
fn test_vad_callback_integration() {
    let speech_final_count = Arc::new(AtomicUsize::new(0));
    let count_clone = speech_final_count.clone();

    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 64, // 2 frames for faster testing
        min_speech_duration_ms: 32,
        frame_duration_ms: 32.0,
    };
    let tracker = SilenceTracker::new(config);

    // Speech
    for _ in 0..3 {
        tracker.process(0.8);
    }

    // Silence until turn end
    for _ in 0..5 {
        if let Some(VADEvent::TurnEnd) = tracker.process(0.2) {
            count_clone.fetch_add(1, Ordering::SeqCst);
            break;
        }
    }

    assert_eq!(speech_final_count.load(Ordering::SeqCst), 1);
}

/// Test multiple speech segments with reset between them
#[test]
fn test_multiple_speech_segments_with_reset() {
    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 64,
        min_speech_duration_ms: 32,
        frame_duration_ms: 32.0,
    };
    let tracker = SilenceTracker::new(config);

    // First speech segment
    assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));
    tracker.process(0.2); // SilenceDetected
    assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd));

    // Reset for new segment
    tracker.reset();

    // Second speech segment should start fresh
    assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));
    assert!(tracker.is_speaking());
    assert!(tracker.has_speech());
}

/// Test continuous speech doesn't trigger events
#[test]
fn test_continuous_speech_stability() {
    let tracker = SilenceTracker::default();

    // First frame triggers SpeechStart
    assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));

    // Subsequent speech frames should produce no events
    for _ in 0..100 {
        assert_eq!(tracker.process(0.9), None);
        assert!(tracker.is_speaking());
    }
}

/// Test VADEvent equality and debug representation
#[test]
fn test_vad_event_properties() {
    // Test equality
    assert_eq!(VADEvent::SpeechStart, VADEvent::SpeechStart);
    assert_eq!(VADEvent::SilenceDetected, VADEvent::SilenceDetected);
    assert_eq!(VADEvent::SpeechResumed, VADEvent::SpeechResumed);
    assert_eq!(VADEvent::TurnEnd, VADEvent::TurnEnd);

    assert_ne!(VADEvent::SpeechStart, VADEvent::TurnEnd);
    assert_ne!(VADEvent::SilenceDetected, VADEvent::SpeechResumed);

    // Test debug representation
    assert_eq!(format!("{:?}", VADEvent::SpeechStart), "SpeechStart");
    assert_eq!(format!("{:?}", VADEvent::TurnEnd), "TurnEnd");
}

/// Test config builder pattern
#[test]
fn test_config_builder_chain() {
    let config = SilenceTrackerConfig::default()
        .with_threshold(0.6)
        .with_silence_duration_ms(400)
        .with_min_speech_duration_ms(150);

    assert_eq!(config.threshold, 0.6);
    assert_eq!(config.silence_duration_ms, 400);
    assert_eq!(config.min_speech_duration_ms, 150);
    assert_eq!(config.frame_duration_ms, 32.0); // Default unchanged
}

/// Test tracker getters return correct values
#[test]
fn test_tracker_getters() {
    let config = SilenceTrackerConfig {
        threshold: 0.7,
        silence_duration_ms: 500,
        min_speech_duration_ms: 200,
        frame_duration_ms: 32.0,
    };
    let tracker = SilenceTracker::new(config);

    assert_eq!(tracker.speech_threshold(), 0.7);
    assert_eq!(tracker.silence_threshold_ms(), 500);
    assert_eq!(tracker.config().min_speech_duration_ms, 200);
}

/// Test realistic conversation scenario
#[test]
fn test_realistic_conversation_scenario() {
    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 300,
        min_speech_duration_ms: 100,
        frame_duration_ms: 32.0,
    };
    let tracker = SilenceTracker::new(config);

    // User starts speaking "Hello"
    let mut events = Vec::new();
    events.push(tracker.process(0.8)); // Start
    for _ in 0..4 {
        events.push(tracker.process(0.85)); // Speaking
    }

    // Brief pause (thinking)
    events.push(tracker.process(0.2));
    events.push(tracker.process(0.15));

    // Continues "my name is..."
    events.push(tracker.process(0.9)); // Resume
    for _ in 0..6 {
        events.push(tracker.process(0.8)); // Speaking
    }

    // User finishes, long silence
    for i in 0..15 {
        let event = tracker.process(0.1);
        events.push(event);
        if event == Some(VADEvent::TurnEnd) {
            // Should happen around frame 10 (300ms / 32ms)
            assert!(i >= 8, "TurnEnd should happen after ~300ms of silence");
            break;
        }
    }

    // Verify expected events occurred
    assert!(events.contains(&Some(VADEvent::SpeechStart)));
    assert!(events.contains(&Some(VADEvent::SilenceDetected)));
    assert!(events.contains(&Some(VADEvent::SpeechResumed)));
    assert!(events.contains(&Some(VADEvent::TurnEnd)));
}

/// Test silence without prior speech never triggers turn end
#[test]
fn test_silence_only_no_turn_end() {
    let config = SilenceTrackerConfig {
        threshold: 0.5,
        silence_duration_ms: 64, // Very short threshold
        min_speech_duration_ms: 32,
        frame_duration_ms: 32.0,
    };
    let tracker = SilenceTracker::new(config);

    // Only silence, no speech
    for _ in 0..100 {
        let event = tracker.process(0.2);
        assert_ne!(
            event,
            Some(VADEvent::TurnEnd),
            "TurnEnd should never fire without prior speech"
        );
    }

    assert!(!tracker.has_speech());
    assert!(!tracker.is_speaking());
}

/// Test concurrent access to tracker (thread safety)
#[test]
fn test_concurrent_tracker_access() {
    use std::thread;

    let tracker = Arc::new(SilenceTracker::default());
    let num_threads = 4;
    let iterations = 100;

    // Multiple reader threads
    let readers: Vec<_> = (0..num_threads)
        .map(|_| {
            let tracker = Arc::clone(&tracker);
            thread::spawn(move || {
                for _ in 0..iterations {
                    let _ = tracker.is_speaking();
                    let _ = tracker.has_speech();
                    let _ = tracker.current_silence_ms();
                    let _ = tracker.current_speech_ms();
                    let _ = tracker.speech_threshold();
                    let _ = tracker.silence_threshold_ms();
                }
            })
        })
        .collect();

    // Process frames in main thread while readers are active
    for i in 0..iterations {
        let prob = if i % 3 == 0 { 0.8 } else { 0.3 };
        let _ = tracker.process(prob);
    }

    // Wait for all readers
    for handle in readers {
        handle.join().expect("Reader thread panicked");
    }
}

/// Generate synthetic speech-like audio (for future model tests)
#[allow(dead_code)]
fn generate_speech_audio(samples: usize) -> Vec<i16> {
    use std::f32::consts::PI;
    (0..samples)
        .map(|i| {
            let t = i as f32 / 16000.0;
            let freq = 440.0 + (t * 100.0).sin() * 50.0; // Varying frequency
            ((2.0 * PI * freq * t).sin() * 16000.0) as i16
        })
        .collect()
}

/// Generate silence audio (for future model tests)
#[allow(dead_code)]
fn generate_silence_audio(samples: usize) -> Vec<i16> {
    vec![0i16; samples]
}

// Feature-gated tests that require the VAD model
#[cfg(feature = "stt-vad")]
mod vad_model_tests {
    use sayna::core::vad::{SileroVAD, SileroVADBuilder, SileroVADConfig, VADSampleRate};

    /// Test that VAD builder creates valid configuration
    /// Note: This test requires the model to be downloaded
    #[tokio::test]
    #[ignore]
    async fn test_vad_builder_configuration() {
        let vad = SileroVADBuilder::new()
            .threshold(0.6)
            .silence_duration_ms(400)
            .min_speech_duration_ms(150)
            .num_threads(2)
            .cache_path(".sayna_cache")
            .build()
            .await;

        // Verify the VAD was created successfully with the configured threshold
        if let Ok(vad) = vad {
            assert_eq!(vad.get_threshold(), 0.6);
            assert_eq!(vad.silence_duration_ms(), 400);
        }
    }

    /// Test VAD config sample rate settings
    #[test]
    fn test_vad_sample_rate_config() {
        assert_eq!(VADSampleRate::Rate16kHz.as_hz(), 16000);
        assert_eq!(VADSampleRate::Rate8kHz.as_hz(), 8000);

        assert_eq!(VADSampleRate::Rate16kHz.frame_size(), 512);
        assert_eq!(VADSampleRate::Rate8kHz.frame_size(), 256);

        assert_eq!(VADSampleRate::Rate16kHz.context_size(), 64);
        assert_eq!(VADSampleRate::Rate8kHz.context_size(), 32);
    }

    /// Test VAD config defaults
    #[test]
    fn test_vad_config_defaults() {
        let config = SileroVADConfig::default();
        assert_eq!(config.threshold, 0.5);
        assert_eq!(config.sample_rate, VADSampleRate::Rate16kHz);
        assert_eq!(config.silence_duration_ms, 300);
        assert_eq!(config.min_speech_duration_ms, 100);
    }

    /// Test that VAD can be created with default config
    /// Note: This test requires the model to be downloaded
    #[tokio::test]
    #[ignore]
    async fn test_vad_creation() {
        let config = SileroVADConfig {
            cache_path: Some(".sayna_cache".into()),
            ..Default::default()
        };

        let result = SileroVAD::with_config(config).await;
        assert!(result.is_ok(), "Failed to create VAD: {:?}", result.err());
    }

    /// Test VAD frame size matches configuration
    #[tokio::test]
    #[ignore]
    async fn test_vad_frame_size() {
        let config = SileroVADConfig {
            cache_path: Some(".sayna_cache".into()),
            ..Default::default()
        };

        let vad = SileroVAD::with_config(config).await.unwrap();
        assert_eq!(vad.frame_size(), 512);
        assert_eq!(vad.sample_rate(), 16000);
    }

    /// Test VAD threshold getter and setter
    #[tokio::test]
    #[ignore]
    async fn test_vad_threshold_operations() {
        let config = SileroVADConfig {
            cache_path: Some(".sayna_cache".into()),
            threshold: 0.5,
            ..Default::default()
        };

        let mut vad = SileroVAD::with_config(config).await.unwrap();
        assert_eq!(vad.get_threshold(), 0.5);

        vad.set_threshold(0.7);
        assert_eq!(vad.get_threshold(), 0.7);

        // Test clamping
        vad.set_threshold(1.5);
        assert_eq!(vad.get_threshold(), 1.0);

        vad.set_threshold(-0.5);
        assert_eq!(vad.get_threshold(), 0.0);
    }

    /// Test VAD inference with silence
    #[tokio::test]
    #[ignore]
    async fn test_vad_inference_silence() {
        let config = SileroVADConfig {
            cache_path: Some(".sayna_cache".into()),
            ..Default::default()
        };

        let vad = SileroVAD::with_config(config).await.unwrap();

        // Generate silence (zeros)
        let silence: Vec<i16> = vec![0; 512];
        let prob = vad.process_audio(&silence).await.unwrap();
        assert!(prob < 0.5, "Silence should have low speech probability");
    }

    /// Test VAD reset functionality
    #[tokio::test]
    #[ignore]
    async fn test_vad_reset() {
        let config = SileroVADConfig {
            cache_path: Some(".sayna_cache".into()),
            ..Default::default()
        };

        let vad = SileroVAD::with_config(config).await.unwrap();

        // Process some audio
        let audio: Vec<i16> = vec![0; 512];
        let _ = vad.process_audio(&audio).await;

        // Reset should not panic
        vad.reset().await;

        // Should be able to process again after reset
        let result = vad.process_audio(&audio).await;
        assert!(result.is_ok());
    }
}
