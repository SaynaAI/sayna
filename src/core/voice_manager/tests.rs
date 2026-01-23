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

    // Test Case 1: is_final result should return immediately and set waiting flag
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

        // State should be updated (waiting for VAD TurnEnd or real speech_final)
        let state = speech_final_state.read();
        assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(state.text_buffer, "Hello");
    }

    // Test Case 2: State should be set correctly for final results
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

        // Check that state is correct
        let state = speech_final_state.read();
        assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(state.text_buffer, "Test message");
    }

    // Test Case 3: Real speech_final should reset state
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // Reset state first
        {
            let mut state = speech_final_state.write();
            *state = SpeechFinalState::new();
        }

        // Send is_final=true result to set waiting state
        let result1 = STTResult::new("Hello world".to_string(), true, false, 0.9);
        let processor = STTResultProcessor::default();
        let _processed1 = processor
            .process_result(result1, speech_final_state.clone())
            .await;

        // Verify state was set
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert_eq!(state.text_buffer, "Hello world");
        }

        // Send is_speech_final=true (should reset state)
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
    // Test Case 1: Forced speech_final fires, then real speech_final arrives - should prevent duplicate
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

        // Verify state was set
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        }

        // 2. Simulate VAD turn detection firing (mark as fired)
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

        // 3. Real speech_final arrives after forced speech_final
        let result2 = STTResult::new("Hello world".to_string(), true, true, 0.95);
        let processor2 = STTResultProcessor::default();
        let processed2 = processor2
            .process_result(result2, speech_final_state.clone())
            .await;

        // Should be None (ignored) because forced speech_final already fired
        assert!(processed2.is_none());
    }

    // Test Case 2: Multiple is_final results should continue accumulating text
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // 1. First is_final=true
        let result1 = STTResult::new("First".to_string(), true, false, 0.9);
        let processor1 = STTResultProcessor::default();
        let processed1 = processor1
            .process_result(result1, speech_final_state.clone())
            .await;

        assert!(processed1.is_some());

        // Mark as fired (simulate VAD turn detection)
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

        // 2. Another is_final=true arrives after forced speech_final
        let result2 = STTResult::new("Second".to_string(), true, false, 0.9);
        let processor2 = STTResultProcessor::default();
        let processed2 = processor2
            .process_result(result2, speech_final_state.clone())
            .await;

        // Should still return the result
        assert!(processed2.is_some());
        assert_eq!(processed2.unwrap().transcript, "Second");

        // Verify waiting state was set for continuous speech
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        }
    }

    // Test Case 3: New speech segment after proper reset should work normally
    {
        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState::new()));

        // First sequence: is_final=true sets waiting state
        let result1 = STTResult::new("First segment".to_string(), true, false, 0.9);
        let processor1 = STTResultProcessor::default();
        let processed1 = processor1
            .process_result(result1, speech_final_state.clone())
            .await;
        assert!(processed1.is_some());

        // Mark as fired (with recent timestamp)
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

        // Real speech_final arrives but is ignored (forced speech_final already fired)
        let result2 = STTResult::new("First segment".to_string(), true, true, 0.9);
        let processor2 = STTResultProcessor::default();
        let processed2 = processor2
            .process_result(result2, speech_final_state.clone())
            .await;
        assert!(processed2.is_none()); // Ignored due to forced speech_final recently with same text

        // Clear the state to simulate a clean new segment
        {
            let mut state = speech_final_state.write();
            state.text_buffer.clear();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);
        }

        // Now a completely new segment starts (new is_final without speech_final)
        let new_result = STTResult::new("New segment".to_string(), true, false, 0.9);
        let processor_new = STTResultProcessor::default();
        let processed_new = processor_new
            .process_result(new_result, speech_final_state.clone())
            .await;

        assert!(processed_new.is_some());
        assert_eq!(processed_new.unwrap().transcript, "New segment");

        // Verify waiting state was set for continuous speech
        {
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
        }
    }
}

/// Tests for VAD failure not blocking STT audio processing.
///
/// Per CLAUDE.md "Feature Flags" section, these tests are gated under `stt-vad`.
/// Per CLAUDE.md "Testing" section, async tests use `#[tokio::test]`.
#[cfg(feature = "stt-vad")]
mod vad_failure_tests {
    use crate::core::stt::{BaseSTT, STTConfig, STTError, STTErrorCallback, STTResultCallback};
    use crate::core::tts::{AudioCallback, BaseTTS, ConnectionState, TTSConfig, TTSError};
    use crate::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::time::Duration;

    /// Stub STT implementation that records `send_audio` calls.
    ///
    /// This test-only provider tracks:
    /// - `send_audio_called`: Whether `send_audio` was invoked
    /// - `send_audio_call_count`: Total number of `send_audio` invocations
    /// - `audio_bytes_received`: Total bytes received across all calls
    struct StubSTT {
        config: Option<STTConfig>,
        connected: AtomicBool,
        send_audio_called: Arc<AtomicBool>,
        send_audio_call_count: Arc<AtomicUsize>,
        audio_bytes_received: Arc<AtomicUsize>,
    }

    impl StubSTT {
        fn new_with_tracking(
            send_audio_called: Arc<AtomicBool>,
            send_audio_call_count: Arc<AtomicUsize>,
            audio_bytes_received: Arc<AtomicUsize>,
        ) -> Self {
            Self {
                config: Some(STTConfig::default()),
                connected: AtomicBool::new(false),
                send_audio_called,
                send_audio_call_count,
                audio_bytes_received,
            }
        }
    }

    #[async_trait::async_trait]
    impl BaseSTT for StubSTT {
        fn new(config: STTConfig) -> Result<Self, STTError>
        where
            Self: Sized,
        {
            Ok(Self {
                config: Some(config),
                connected: AtomicBool::new(false),
                send_audio_called: Arc::new(AtomicBool::new(false)),
                send_audio_call_count: Arc::new(AtomicUsize::new(0)),
                audio_bytes_received: Arc::new(AtomicUsize::new(0)),
            })
        }

        async fn connect(&mut self) -> Result<(), STTError> {
            self.connected.store(true, Ordering::Relaxed);
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), STTError> {
            self.connected.store(false, Ordering::Relaxed);
            Ok(())
        }

        fn is_ready(&self) -> bool {
            self.connected.load(Ordering::Relaxed)
        }

        async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
            self.send_audio_called.store(true, Ordering::SeqCst);
            self.send_audio_call_count.fetch_add(1, Ordering::SeqCst);
            self.audio_bytes_received
                .fetch_add(audio_data.len(), Ordering::SeqCst);
            Ok(())
        }

        async fn on_result(&mut self, _callback: STTResultCallback) -> Result<(), STTError> {
            Ok(())
        }

        async fn on_error(&mut self, _callback: STTErrorCallback) -> Result<(), STTError> {
            Ok(())
        }

        fn get_config(&self) -> Option<&STTConfig> {
            self.config.as_ref()
        }

        async fn update_config(&mut self, config: STTConfig) -> Result<(), STTError> {
            self.config = Some(config);
            Ok(())
        }

        fn get_provider_info(&self) -> &'static str {
            "StubSTT v1.0 (test-only)"
        }
    }

    /// Stub TTS implementation for testing VoiceManager.
    struct StubTTS {
        connected: bool,
    }

    #[async_trait::async_trait]
    impl BaseTTS for StubTTS {
        fn new(_config: TTSConfig) -> Result<Self, TTSError>
        where
            Self: Sized,
        {
            Ok(Self { connected: false })
        }

        async fn connect(&mut self) -> Result<(), TTSError> {
            self.connected = true;
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), TTSError> {
            self.connected = false;
            Ok(())
        }

        fn is_ready(&self) -> bool {
            self.connected
        }

        fn get_connection_state(&self) -> ConnectionState {
            if self.connected {
                ConnectionState::Connected
            } else {
                ConnectionState::Disconnected
            }
        }

        async fn speak(&mut self, _text: &str, _flush: bool) -> Result<(), TTSError> {
            Ok(())
        }

        async fn clear(&mut self) -> Result<(), TTSError> {
            Ok(())
        }

        async fn flush(&self) -> Result<(), TTSError> {
            Ok(())
        }

        fn on_audio(&mut self, _callback: Arc<dyn AudioCallback>) -> Result<(), TTSError> {
            Ok(())
        }

        fn remove_audio_callback(&mut self) -> Result<(), TTSError> {
            Ok(())
        }

        fn get_provider_info(&self) -> serde_json::Value {
            serde_json::json!({
                "provider": "stub",
                "version": "1.0.0"
            })
        }
    }

    /// Test that `receive_audio` sends audio to STT when VAD is not available (None).
    ///
    /// This is a regression test for the VAD non-blocking behavior. When `stt-vad`
    /// feature is enabled but VAD is None (not initialized or unavailable), STT
    /// should still receive audio normally.
    ///
    /// The test flow:
    /// 1. Create VoiceManager with stub STT that tracks `send_audio` calls
    /// 2. Use VAD = None (simulating VAD unavailable/failed initialization)
    /// 3. Call `receive_audio` with sample audio
    /// 4. Verify STT's `send_audio` was called
    #[tokio::test]
    async fn test_stt_receives_audio_when_vad_unavailable() {
        // Set up tracking for STT send_audio calls
        let send_audio_called = Arc::new(AtomicBool::new(false));
        let send_audio_call_count = Arc::new(AtomicUsize::new(0));
        let audio_bytes_received = Arc::new(AtomicUsize::new(0));

        // Create stub providers
        let mut stub_stt = StubSTT::new_with_tracking(
            send_audio_called.clone(),
            send_audio_call_count.clone(),
            audio_bytes_received.clone(),
        );
        stub_stt.connect().await.unwrap();

        let mut stub_tts = StubTTS::new(TTSConfig::default()).unwrap();
        stub_tts.connect().await.unwrap();

        // Create VoiceManager with our stub providers and NO VAD (None)
        // This simulates the case where VAD initialization failed or VAD is disabled
        let stt_config = STTConfig {
            provider: "stub".to_string(),
            api_key: "test".to_string(),
            ..Default::default()
        };
        let tts_config = TTSConfig {
            provider: "stub".to_string(),
            api_key: "test".to_string(),
            ..Default::default()
        };
        let config = VoiceManagerConfig::new(stt_config, tts_config);

        let vm = VoiceManager::new_with_providers(
            config,
            Box::new(stub_stt),
            Box::new(stub_tts),
            None, // turn_detector
            None, // VAD is None - simulating VAD unavailable/error
        )
        .unwrap();

        // Create sample audio data
        let sample_audio = vec![0u8; 1024]; // 512 samples as 16-bit PCM

        // Call receive_audio
        let result = vm.receive_audio(sample_audio.clone()).await;

        // receive_audio should return Ok
        assert!(
            result.is_ok(),
            "receive_audio should succeed when VAD is unavailable"
        );

        // Give a small window for async STT processing
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Verify STT received the audio
        assert!(
            send_audio_called.load(Ordering::SeqCst),
            "STT send_audio should have been called even when VAD is None"
        );
        assert_eq!(
            send_audio_call_count.load(Ordering::SeqCst),
            1,
            "STT send_audio should have been called exactly once"
        );
        assert_eq!(
            audio_bytes_received.load(Ordering::SeqCst),
            sample_audio.len(),
            "STT should have received the exact audio bytes"
        );
    }

    /// Test that multiple receive_audio calls all reach STT when VAD is unavailable.
    #[tokio::test]
    async fn test_multiple_audio_chunks_reach_stt_without_vad() {
        let send_audio_called = Arc::new(AtomicBool::new(false));
        let send_audio_call_count = Arc::new(AtomicUsize::new(0));
        let audio_bytes_received = Arc::new(AtomicUsize::new(0));

        let mut stub_stt = StubSTT::new_with_tracking(
            send_audio_called.clone(),
            send_audio_call_count.clone(),
            audio_bytes_received.clone(),
        );
        stub_stt.connect().await.unwrap();

        let mut stub_tts = StubTTS::new(TTSConfig::default()).unwrap();
        stub_tts.connect().await.unwrap();

        let stt_config = STTConfig {
            provider: "stub".to_string(),
            api_key: "test".to_string(),
            ..Default::default()
        };
        let tts_config = TTSConfig {
            provider: "stub".to_string(),
            api_key: "test".to_string(),
            ..Default::default()
        };
        let config = VoiceManagerConfig::new(stt_config, tts_config);

        let vm = VoiceManager::new_with_providers(
            config,
            Box::new(stub_stt),
            Box::new(stub_tts),
            None, // turn_detector
            None, // VAD is None
        )
        .unwrap();

        // Send multiple audio chunks
        let chunk_size = 1024;
        let num_chunks = 5;
        for _ in 0..num_chunks {
            let audio = vec![0u8; chunk_size];
            let result = vm.receive_audio(audio).await;
            assert!(result.is_ok());
        }

        // Allow processing
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Verify all chunks reached STT
        assert_eq!(
            send_audio_call_count.load(Ordering::SeqCst),
            num_chunks,
            "All {} audio chunks should have reached STT",
            num_chunks
        );
        assert_eq!(
            audio_bytes_received.load(Ordering::SeqCst),
            chunk_size * num_chunks,
            "Total bytes received should match all chunks combined"
        );
    }

    /// Test that STT receives audio even when VAD processing returns an error.
    ///
    /// This is a regression test for the VAD error handling path. When VAD processing
    /// fails (e.g., ONNX inference error), STT should still receive the audio because:
    /// 1. VAD processing is spawned as a separate tokio task
    /// 2. Errors are logged but not propagated
    /// 3. STT `send_audio` is called after the spawn, not awaiting the VAD result
    ///
    /// This test uses a stub VAD that doesn't require loading a real ONNX model,
    /// combined with `VADState::set_force_error(true)` to trigger the error path
    /// before any model access occurs.
    #[tokio::test]
    async fn test_stt_receives_audio_when_vad_processing_errors() {
        use crate::core::vad::{SileroVAD, SileroVADConfig};
        use crate::core::voice_manager::vad_processor::VADState;
        use tokio::sync::RwLock;

        // Create a stub SileroVAD that doesn't require loading a real ONNX model.
        // This uses the test-only constructor that creates a minimal instance.
        let vad = Arc::new(RwLock::new(SileroVAD::stub_for_testing(
            SileroVADConfig::default(),
        )));

        // Set up tracking for STT send_audio calls
        let send_audio_called = Arc::new(AtomicBool::new(false));
        let send_audio_call_count = Arc::new(AtomicUsize::new(0));
        let audio_bytes_received = Arc::new(AtomicUsize::new(0));

        // Create stub providers
        let mut stub_stt = StubSTT::new_with_tracking(
            send_audio_called.clone(),
            send_audio_call_count.clone(),
            audio_bytes_received.clone(),
        );
        stub_stt.connect().await.unwrap();

        let mut stub_tts = StubTTS::new(TTSConfig::default()).unwrap();
        stub_tts.connect().await.unwrap();

        // Create configs
        let stt_config = STTConfig {
            provider: "stub".to_string(),
            api_key: "test".to_string(),
            ..Default::default()
        };
        let tts_config = TTSConfig {
            provider: "stub".to_string(),
            api_key: "test".to_string(),
            ..Default::default()
        };
        let config = VoiceManagerConfig::new(stt_config, tts_config);

        // Create VADState with force_error enabled.
        // This causes process_audio_chunk to return an error BEFORE any model access,
        // allowing us to test the error path without a real ONNX model.
        let vad_state = Arc::new(VADState::new(16000 * 8, 512));
        vad_state.set_force_error(true);

        // Create VoiceManager with our stub providers and stub VAD
        // but with force_error set in VADState
        let vm = VoiceManager::new_with_providers_and_vad_state(
            config,
            Box::new(stub_stt),
            Box::new(stub_tts),
            None, // turn_detector
            Some(vad),
            vad_state,
        )
        .unwrap();

        // Create sample audio data (512 samples as 16-bit PCM = 1024 bytes)
        let sample_audio = vec![0u8; 1024];

        // Call receive_audio
        let result = vm.receive_audio(sample_audio.clone()).await;

        // receive_audio should return Ok even though VAD will error
        assert!(
            result.is_ok(),
            "receive_audio should succeed even when VAD processing errors"
        );

        // Give time for async STT processing and VAD task to complete
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Verify STT received the audio despite VAD error
        assert!(
            send_audio_called.load(Ordering::SeqCst),
            "STT send_audio should have been called despite VAD processing error"
        );
        assert_eq!(
            send_audio_call_count.load(Ordering::SeqCst),
            1,
            "STT send_audio should have been called exactly once"
        );
        assert_eq!(
            audio_bytes_received.load(Ordering::SeqCst),
            sample_audio.len(),
            "STT should have received the exact audio bytes despite VAD error"
        );
    }
}
