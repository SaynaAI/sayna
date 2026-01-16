//! Integration tests for the smart-turn detection module.
//!
//! These tests validate:
//! - Feature extractor mel spectrogram output
//! - Turn detector initialization and prediction
//! - VAD + Turn Detection integration
//! - Performance benchmarks
//!
//! Tests marked with #[ignore] require model files to be downloaded.
//! Run with: cargo test --features stt-vad -- --ignored

use sayna::core::turn_detect::TurnDetectorConfig;
#[cfg(feature = "stt-vad")]
use sayna::core::turn_detect::{FeatureExtractor, TurnDetector, TurnDetectorBuilder};
use sayna::core::vad::{SilenceTracker, SilenceTrackerConfig, VADEvent};

// =============================================================================
// Feature Extractor Integration Tests
// =============================================================================

#[cfg(feature = "stt-vad")]
mod feature_extractor_tests {
    use super::*;

    #[test]
    fn test_mel_spectrogram_shape() {
        let config = TurnDetectorConfig::default();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Test with 8 seconds of audio (128000 samples at 16kHz)
        let audio: Vec<i16> = vec![0i16; 128000];
        let mel = extractor.extract(&audio).unwrap();

        // Use config values instead of hardcoded dimensions
        assert_eq!(
            mel.dim(),
            (config.mel_bins, config.mel_frames),
            "Expected mel shape ({}, {}), got {:?}",
            config.mel_bins, config.mel_frames, mel.dim()
        );
    }

    #[test]
    fn test_short_audio_padding() {
        let config = TurnDetectorConfig::default();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Test with 1 second of audio (16000 samples)
        let audio: Vec<i16> = vec![0i16; 16000];
        let mel = extractor.extract(&audio).unwrap();

        // Should still produce (mel_bins, mel_frames) shape (padded)
        assert_eq!(
            mel.dim(),
            (config.mel_bins, config.mel_frames),
            "Short audio should be padded to ({}, {})",
            config.mel_bins, config.mel_frames
        );
    }

    #[test]
    fn test_long_audio_truncation() {
        let config = TurnDetectorConfig::default();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Test with 16 seconds of audio (256000 samples)
        let audio: Vec<i16> = vec![0i16; 256000];
        let mel = extractor.extract(&audio).unwrap();

        // Should still produce (mel_bins, mel_frames) shape (truncated to last 8 seconds)
        assert_eq!(
            mel.dim(),
            (config.mel_bins, config.mel_frames),
            "Long audio should be truncated to ({}, {})",
            config.mel_bins, config.mel_frames
        );
    }

    #[test]
    fn test_extraction_with_sine_wave() {
        let config = TurnDetectorConfig::default();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Generate 440Hz sine wave (A4 note) for 2 seconds
        let audio: Vec<i16> = generate_sine_wave(440.0, 2.0, 16000);
        let mel = extractor.extract(&audio).unwrap();

        assert_eq!(mel.dim(), (config.mel_bins, config.mel_frames));

        // Verify mel spectrogram has non-zero values (not all zeros)
        let max_val = mel.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let min_val = mel.iter().cloned().fold(f32::INFINITY, f32::min);
        assert!(
            max_val > min_val,
            "Mel spectrogram should have varying values"
        );
    }

    #[test]
    fn test_extraction_performance() {
        use std::time::Instant;

        let config = TurnDetectorConfig::default();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Generate sine wave for more realistic test (8 seconds)
        let audio: Vec<i16> = (0..128000)
            .map(|i| ((i as f32 * 0.1).sin() * 16000.0) as i16)
            .collect();

        let start = Instant::now();
        let _mel = extractor.extract(&audio).unwrap();
        let elapsed = start.elapsed();

        // Performance thresholds differ between debug and release builds
        // Debug builds are unoptimized and significantly slower
        #[cfg(debug_assertions)]
        let max_ms = 2000; // 2 seconds for debug builds (unoptimized)
        #[cfg(not(debug_assertions))]
        let max_ms = 100; // 100ms for release builds

        println!("Feature extraction took: {:?}", elapsed);
        assert!(
            elapsed.as_millis() < max_ms,
            "Feature extraction too slow: {:?} (threshold: {}ms)",
            elapsed,
            max_ms
        );
    }

    #[test]
    fn test_extraction_consistency() {
        let config = TurnDetectorConfig::default();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Same input should produce same output
        let audio: Vec<i16> = generate_sine_wave(440.0, 1.0, 16000);

        let mel1 = extractor.extract(&audio).unwrap();
        let mel2 = extractor.extract(&audio).unwrap();

        // Compare all values
        for (v1, v2) in mel1.iter().zip(mel2.iter()) {
            assert!(
                (v1 - v2).abs() < 1e-6,
                "Mel spectrograms should be identical for same input"
            );
        }
    }

    #[test]
    fn test_empty_audio_produces_valid_shape() {
        let config = TurnDetectorConfig::default();
        let extractor = FeatureExtractor::new(&config).unwrap();

        let audio: Vec<i16> = vec![];
        let mel = extractor.extract(&audio).unwrap();

        // Should produce correct shape even for empty audio
        assert_eq!(mel.dim(), (config.mel_bins, config.mel_frames));
    }

    #[test]
    fn test_very_short_audio() {
        let config = TurnDetectorConfig::default();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Less than one FFT frame (400 samples)
        let audio: Vec<i16> = vec![1000i16; 100];
        let mel = extractor.extract(&audio).unwrap();

        assert_eq!(mel.dim(), (config.mel_bins, config.mel_frames));
    }
}

// =============================================================================
// Turn Detector Integration Tests
// =============================================================================

#[cfg(feature = "stt-vad")]
mod turn_detector_tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_turn_detector_builder_initialization() {
        let temp_dir = tempdir().unwrap();
        let cache_path = temp_dir.path().to_path_buf();

        let detector = TurnDetectorBuilder::new()
            .threshold(0.5)
            .cache_path(cache_path)
            .num_threads(4)
            .build()
            .await;

        assert!(
            detector.is_ok(),
            "Failed to initialize turn detector: {:?}",
            detector.err()
        );
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_builder_with_custom_threshold() {
        let detector = TurnDetectorBuilder::new()
            .threshold(0.7)
            .build()
            .await
            .unwrap();

        assert_eq!(detector.get_threshold(), 0.7);
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_builder_with_quantized_model() {
        let detector = TurnDetectorBuilder::new()
            .use_quantized(true)
            .num_threads(2)
            .build()
            .await
            .unwrap();

        let config = detector.get_config();
        assert!(config.use_quantized);
        assert_eq!(config.num_threads, Some(2));
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_turn_detection_with_silence() {
        let detector = TurnDetector::new(None).await.unwrap();

        // Pure silence should have some turn completion probability
        // (model behavior depends on training)
        let silence: Vec<i16> = vec![0i16; 128000];
        let probability = detector.predict_end_of_turn(&silence).await.unwrap();

        println!("Silence probability: {:.4}", probability);
        assert!(
            probability >= 0.0 && probability <= 1.0,
            "Probability out of range"
        );
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_turn_detection_with_audio() {
        let detector = TurnDetector::new(None).await.unwrap();

        // Generate 2 seconds of audio at 440Hz
        let audio = generate_sine_wave(440.0, 2.0, 16000);
        let probability = detector.predict_end_of_turn(&audio).await.unwrap();

        println!("Audio probability: {:.4}", probability);
        assert!(probability >= 0.0 && probability <= 1.0);
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_turn_detection_inference_time() {
        use std::time::Instant;

        let detector = TurnDetectorBuilder::new()
            .num_threads(4)
            .build()
            .await
            .unwrap();

        // Generate test audio
        let audio: Vec<i16> = (0..128000)
            .map(|i| ((i as f32 * 0.1).sin() * 16000.0) as i16)
            .collect();

        // Warm-up inference
        let _ = detector.predict_end_of_turn(&audio).await;

        // Measure inference time
        let start = Instant::now();
        let _ = detector.predict_end_of_turn(&audio).await;
        let elapsed = start.elapsed();

        println!("Total turn detection time: {:?}", elapsed);

        // Should complete in under 100ms (target: 50ms per smart-turn blog)
        assert!(
            elapsed.as_millis() < 100,
            "Turn detection too slow: {:?}",
            elapsed
        );
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_is_turn_complete_with_threshold() {
        let mut detector = TurnDetectorBuilder::new()
            .threshold(0.5)
            .build()
            .await
            .unwrap();

        let audio = generate_sine_wave(440.0, 2.0, 16000);

        // Test with default threshold
        let is_complete = detector.is_turn_complete(&audio).await.unwrap();
        println!("Is turn complete (threshold=0.5): {}", is_complete);

        // Adjust threshold and test again
        detector.set_threshold(0.9);
        let is_complete_high = detector.is_turn_complete(&audio).await.unwrap();
        println!("Is turn complete (threshold=0.9): {}", is_complete_high);

        // Higher threshold should make it less likely to be complete
        // (unless probability is very high)
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_empty_audio_returns_zero() {
        let detector = TurnDetector::new(None).await.unwrap();

        let empty: Vec<i16> = vec![];
        let probability = detector.predict_end_of_turn(&empty).await.unwrap();

        assert_eq!(probability, 0.0, "Empty audio should return 0.0");
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_max_duration_audio() {
        let detector = TurnDetector::new(None).await.unwrap();

        // Test with maximum duration audio (8 seconds)
        let max_samples = generate_sine_wave(440.0, 8.0, 16000);
        assert_eq!(max_samples.len(), 16000 * 8);

        let probability = detector.predict_end_of_turn(&max_samples).await.unwrap();
        println!("Max duration audio - Probability: {:.4}", probability);

        assert!(probability >= 0.0 && probability <= 1.0);
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_longer_than_max_duration_audio() {
        let detector = TurnDetector::new(None).await.unwrap();

        // Test with audio longer than max duration (10 seconds)
        let long_samples = generate_sine_wave(440.0, 10.0, 16000);
        assert_eq!(long_samples.len(), 16000 * 10);

        let probability = detector.predict_end_of_turn(&long_samples).await.unwrap();
        println!(
            "Longer than max duration audio - Probability: {:.4}",
            probability
        );

        assert!(probability >= 0.0 && probability <= 1.0);
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_concurrent_predictions() {
        use std::sync::Arc;
        use tokio::task::JoinSet;

        let detector = Arc::new(TurnDetector::new(None).await.unwrap());
        let audio = Arc::new(generate_sine_wave(440.0, 2.0, 16000));

        let mut handles = JoinSet::new();

        // Spawn 4 concurrent prediction tasks
        for _ in 0..4 {
            let detector = Arc::clone(&detector);
            let audio = Arc::clone(&audio);
            handles.spawn(async move { detector.predict_end_of_turn(&audio).await });
        }

        // All should complete successfully
        let mut results = Vec::new();
        while let Some(result) = handles.join_next().await {
            let prob = result.unwrap().unwrap();
            results.push(prob);
        }

        assert_eq!(results.len(), 4);

        // All results should be identical (deterministic inference)
        let first = results[0];
        for prob in &results {
            assert!(
                (prob - first).abs() < 1e-6,
                "Concurrent predictions should be identical"
            );
        }
    }
}

// =============================================================================
// VAD + Turn Detection Integration Tests
// =============================================================================

mod vad_turn_integration_tests {
    use super::*;

    #[test]
    fn test_silence_tracker_turn_end_triggers() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64,    // 2 frames for quick test
            min_speech_duration_ms: 32, // 1 frame
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Simulate speech followed by silence
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));
        assert_eq!(tracker.process(0.2), Some(VADEvent::SilenceDetected));
        assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd));

        // Turn detection would be triggered here in real system
    }

    #[test]
    fn test_speech_resume_cancels_turn_detection() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 96, // 3 frames
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Speech -> silence -> speech (should cancel turn detection)
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));
        assert_eq!(tracker.process(0.2), Some(VADEvent::SilenceDetected));
        let event = tracker.process(0.8); // SpeechResumed

        assert_eq!(event, Some(VADEvent::SpeechResumed));
        // In real system, SpeechResumed would cancel pending turn detection
    }

    #[test]
    fn test_multiple_silence_periods_without_turn_end() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 100, // ~3 frames
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Start speaking
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));

        // Multiple short pauses shouldn't trigger TurnEnd
        for _ in 0..3 {
            assert_eq!(tracker.process(0.2), Some(VADEvent::SilenceDetected));
            assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechResumed));
        }

        // Final silence should eventually trigger TurnEnd
        assert_eq!(tracker.process(0.2), Some(VADEvent::SilenceDetected));
        assert_eq!(tracker.process(0.2), None); // Not enough silence yet
        assert_eq!(tracker.process(0.2), None); // 96ms
        assert_eq!(tracker.process(0.2), Some(VADEvent::TurnEnd)); // 128ms >= 100ms
    }

    #[test]
    fn test_silence_tracker_reset_allows_new_turn() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 64,
            min_speech_duration_ms: 32,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Complete first turn
        tracker.process(0.8); // SpeechStart
        tracker.process(0.2); // SilenceDetected
        tracker.process(0.2); // TurnEnd

        // Reset for new turn
        tracker.reset();

        // New speech should trigger SpeechStart again
        assert_eq!(tracker.process(0.8), Some(VADEvent::SpeechStart));
        assert!(!tracker.has_speech() || tracker.current_speech_ms() == 32);
    }

    #[test]
    fn test_silence_tracker_state_getters() {
        let config = SilenceTrackerConfig {
            threshold: 0.5,
            silence_duration_ms: 300,
            min_speech_duration_ms: 100,
            frame_duration_ms: 32.0,
        };
        let tracker = SilenceTracker::new(config);

        // Initial state
        assert!(!tracker.is_speaking());
        assert!(!tracker.has_speech());
        assert_eq!(tracker.current_silence_ms(), 0);
        assert_eq!(tracker.current_speech_ms(), 0);

        // After speech
        tracker.process(0.8);
        assert!(tracker.is_speaking());
        assert!(tracker.has_speech());
        assert_eq!(tracker.current_speech_ms(), 32);

        // After silence
        tracker.process(0.2);
        assert!(!tracker.is_speaking());
        assert!(tracker.current_silence_ms() > 0);
    }

    #[test]
    fn test_silence_tracker_config_builder() {
        let config = SilenceTrackerConfig::default()
            .with_threshold(0.7)
            .with_silence_duration_ms(500)
            .with_min_speech_duration_ms(200);

        assert_eq!(config.threshold, 0.7);
        assert_eq!(config.silence_duration_ms, 500);
        assert_eq!(config.min_speech_duration_ms, 200);
    }

    #[test]
    fn test_silence_tracker_from_sample_rate() {
        // 16kHz sample rate
        let config_16k = SilenceTrackerConfig::from_sample_rate(16000, 0.5, 300, 100);
        assert_eq!(config_16k.frame_duration_ms, 32.0); // 512/16000 * 1000

        // 8kHz sample rate
        let config_8k = SilenceTrackerConfig::from_sample_rate(8000, 0.5, 300, 100);
        assert_eq!(config_8k.frame_duration_ms, 32.0); // 256/8000 * 1000
    }

    #[test]
    fn test_vad_event_equality() {
        assert_eq!(VADEvent::SpeechStart, VADEvent::SpeechStart);
        assert_eq!(VADEvent::SilenceDetected, VADEvent::SilenceDetected);
        assert_eq!(VADEvent::SpeechResumed, VADEvent::SpeechResumed);
        assert_eq!(VADEvent::TurnEnd, VADEvent::TurnEnd);

        assert_ne!(VADEvent::SpeechStart, VADEvent::TurnEnd);
    }

    #[test]
    fn test_silence_tracker_thread_safety() {
        use std::sync::Arc;
        use std::thread;

        let tracker = Arc::new(SilenceTracker::default());

        // Spawn multiple threads reading state
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let tracker = Arc::clone(&tracker);
                thread::spawn(move || {
                    for _ in 0..100 {
                        let _ = tracker.is_speaking();
                        let _ = tracker.has_speech();
                        let _ = tracker.current_silence_ms();
                        let _ = tracker.current_speech_ms();
                    }
                })
            })
            .collect();

        // Process some frames in main thread
        for i in 0..100 {
            let prob = if i % 3 == 0 { 0.8 } else { 0.3 };
            let _ = tracker.process(prob);
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }
    }
}

// =============================================================================
// Configuration Tests
// =============================================================================

mod config_tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = TurnDetectorConfig::default();
        assert_eq!(config.threshold, 0.5);
        assert_eq!(config.sample_rate, 16000);
        assert_eq!(config.max_audio_duration_seconds, 8);
        assert_eq!(config.mel_bins, 80);
        assert_eq!(config.mel_frames, 800);
        assert!(config.use_quantized);
        assert_eq!(config.num_threads, Some(4));
    }

    #[test]
    fn test_config_serialization() {
        let config = TurnDetectorConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: TurnDetectorConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.threshold, deserialized.threshold);
        assert_eq!(config.sample_rate, deserialized.sample_rate);
        assert_eq!(
            config.max_audio_duration_seconds,
            deserialized.max_audio_duration_seconds
        );
        assert_eq!(config.mel_bins, deserialized.mel_bins);
        assert_eq!(config.mel_frames, deserialized.mel_frames);
        assert_eq!(config.use_quantized, deserialized.use_quantized);
        assert_eq!(config.num_threads, deserialized.num_threads);
    }

    #[test]
    fn test_config_cache_dir() {
        use std::path::PathBuf;

        let config = TurnDetectorConfig {
            cache_path: Some(PathBuf::from("/tmp/test_cache")),
            ..Default::default()
        };

        let cache_dir = config.get_cache_dir().unwrap();
        assert_eq!(cache_dir, PathBuf::from("/tmp/test_cache/turn_detect"));
    }

    #[test]
    fn test_config_no_cache_path_error() {
        let config = TurnDetectorConfig::default();
        let result = config.get_cache_dir();
        assert!(result.is_err());
    }
}

// =============================================================================
// Performance Benchmark Tests
// =============================================================================

#[cfg(feature = "stt-vad")]
mod performance_tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_feature_extraction_benchmark() {
        let config = TurnDetectorConfig::default();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Benchmark with different audio durations
        let durations = [1.0, 2.0, 4.0, 8.0];

        for duration in durations {
            let samples = (duration * 16000.0) as usize;
            let audio: Vec<i16> = (0..samples)
                .map(|i| ((i as f32 * 0.1).sin() * 16000.0) as i16)
                .collect();

            let start = Instant::now();
            let _ = extractor.extract(&audio).unwrap();
            let elapsed = start.elapsed();

            println!("Feature extraction ({:.1}s audio): {:?}", duration, elapsed);
        }
    }

    #[test]
    fn test_feature_extraction_target_performance() {
        let config = TurnDetectorConfig::default();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // 8 seconds of audio - target: < 30ms
        let audio: Vec<i16> = (0..128000)
            .map(|i| ((i as f32 * 0.1).sin() * 16000.0) as i16)
            .collect();

        // Warm-up
        let _ = extractor.extract(&audio).unwrap();

        // Measure
        let iterations = 10;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = extractor.extract(&audio).unwrap();
        }
        let total_elapsed = start.elapsed();
        let avg_elapsed = total_elapsed / iterations;

        println!("Average feature extraction time: {:?}", avg_elapsed);

        // In release mode, should be under 30ms
        #[cfg(not(debug_assertions))]
        assert!(
            avg_elapsed.as_millis() < 30,
            "Feature extraction should be under 30ms in release, got {:?}",
            avg_elapsed
        );
    }

    #[tokio::test]
    #[ignore = "Requires model files to be downloaded"]
    async fn test_turn_detection_target_performance() {
        let detector = TurnDetectorBuilder::new()
            .num_threads(4)
            .build()
            .await
            .unwrap();

        let audio: Vec<i16> = (0..128000)
            .map(|i| ((i as f32 * 0.1).sin() * 16000.0) as i16)
            .collect();

        // Warm-up
        let _ = detector.predict_end_of_turn(&audio).await;

        // Measure multiple iterations
        let iterations = 10;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = detector.predict_end_of_turn(&audio).await;
        }
        let total_elapsed = start.elapsed();
        let avg_elapsed = total_elapsed / iterations;

        println!("Average turn detection time: {:?}", avg_elapsed);

        // Target: < 50ms total (feature extraction + inference)
        assert!(
            avg_elapsed.as_millis() < 50,
            "Turn detection should be under 50ms, got {:?}",
            avg_elapsed
        );
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Helper function to generate synthetic audio samples.
/// Creates a simple sine wave at the given frequency.
#[cfg(feature = "stt-vad")]
fn generate_sine_wave(frequency_hz: f32, duration_seconds: f32, sample_rate: u32) -> Vec<i16> {
    let num_samples = (sample_rate as f32 * duration_seconds) as usize;
    (0..num_samples)
        .map(|i| {
            let t = i as f32 / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * frequency_hz * t).sin();
            (sample * 16000.0) as i16
        })
        .collect()
}

/// Helper function to generate silence (zero samples).
#[cfg(feature = "stt-vad")]
#[allow(dead_code)]
fn generate_silence(duration_seconds: f32, sample_rate: u32) -> Vec<i16> {
    let num_samples = (sample_rate as f32 * duration_seconds) as usize;
    vec![0i16; num_samples]
}
