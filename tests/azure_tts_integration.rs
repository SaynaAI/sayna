//! Integration tests for Microsoft Azure Text-to-Speech provider.
//!
//! These tests verify the Azure TTS implementation works correctly
//! with the Azure Speech Services REST API.
//!
//! ## Environment Variables
//!
//! Set the following environment variables to run these tests:
//! - `AZURE_SPEECH_SUBSCRIPTION_KEY`: Your Azure subscription key
//! - `AZURE_SPEECH_REGION`: The Azure region (e.g., "eastus")
//!
//! ## Running Tests
//!
//! ```bash
//! # Run all non-ignored tests
//! cargo test azure_tts_integration
//!
//! # Run all tests including ignored
//! cargo test azure_tts_integration -- --ignored
//!
//! # Run a specific test
//! cargo test test_azure_tts_basic_synthesis -- --ignored
//! ```

use futures::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Mutex;

use sayna::core::providers::azure::AzureRegion;
use sayna::core::tts::{
    AudioCallback, AudioData, AzureAudioEncoding, AzureTTS, AzureTTSConfig, BaseTTS,
    ConnectionState, TTSConfig, TTSError, create_tts_provider,
};

// ============================================================================
// Test Helpers
// ============================================================================

/// Check if Azure credentials are available in the environment.
///
/// Returns the subscription key and region if available.
fn get_azure_credentials() -> Option<(String, String)> {
    let key = std::env::var("AZURE_SPEECH_SUBSCRIPTION_KEY").ok()?;
    let region = std::env::var("AZURE_SPEECH_REGION").unwrap_or_else(|_| "eastus".to_string());
    Some((key, region))
}

/// Create a test TTS config with Azure credentials.
///
/// Uses the credentials from environment variables.
/// Returns None if credentials are not available.
fn create_azure_tts_config() -> Option<TTSConfig> {
    let (subscription_key, _region) = get_azure_credentials()?;

    Some(TTSConfig {
        provider: "azure".to_string(),
        api_key: subscription_key,
        voice_id: Some("en-US-JennyNeural".to_string()),
        model: String::new(),
        audio_format: Some("linear16".to_string()),
        sample_rate: Some(24000),
        speaking_rate: Some(1.0),
        connection_timeout: Some(30),
        request_timeout: Some(60),
        pronunciations: Vec::new(),
        request_pool_size: Some(4),
        ..Default::default()
    })
}

/// Audio collector for capturing TTS output in tests.
///
/// This struct implements `AudioCallback` to collect audio chunks, track completion,
/// and capture any errors that occur during synthesis.
#[derive(Clone)]
pub struct TestAudioCollector {
    /// Collected audio bytes
    audio_buffer: Arc<Mutex<Vec<u8>>>,
    /// Total number of chunks received
    chunk_count: Arc<AtomicUsize>,
    /// Flag indicating synthesis is complete
    completed: Arc<AtomicBool>,
    /// Error message if an error occurred
    error: Arc<Mutex<Option<String>>>,
}

impl TestAudioCollector {
    /// Create a new audio collector.
    pub fn new() -> Self {
        Self {
            audio_buffer: Arc::new(Mutex::new(Vec::new())),
            chunk_count: Arc::new(AtomicUsize::new(0)),
            completed: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
        }
    }

    /// Get the collected audio data.
    pub async fn get_audio(&self) -> Vec<u8> {
        self.audio_buffer.lock().await.clone()
    }

    /// Get the number of audio chunks received.
    pub fn get_chunk_count(&self) -> usize {
        self.chunk_count.load(Ordering::SeqCst)
    }

    /// Check if synthesis has completed.
    pub fn is_completed(&self) -> bool {
        self.completed.load(Ordering::SeqCst)
    }

    /// Get any error that occurred.
    pub async fn get_error(&self) -> Option<String> {
        self.error.lock().await.clone()
    }

    /// Wait for completion with timeout.
    ///
    /// Returns true if completed within the timeout, false otherwise.
    pub async fn wait_for_completion(&self, timeout_ms: u64) -> bool {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        while !self.is_completed() && start.elapsed() < timeout {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        self.is_completed()
    }
}

impl Default for TestAudioCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioCallback for TestAudioCollector {
    fn on_audio(&self, audio_data: AudioData) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let buffer = self.audio_buffer.clone();
        let count = self.chunk_count.clone();

        Box::pin(async move {
            buffer.lock().await.extend(&audio_data.data);
            count.fetch_add(1, Ordering::SeqCst);
        })
    }

    fn on_error(&self, error: TTSError) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let error_store = self.error.clone();
        let completed = self.completed.clone();

        Box::pin(async move {
            *error_store.lock().await = Some(error.to_string());
            completed.store(true, Ordering::SeqCst);
        })
    }

    fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let completed = self.completed.clone();

        Box::pin(async move {
            completed.store(true, Ordering::SeqCst);
        })
    }
}

// ============================================================================
// Tests Without Credentials
// ============================================================================

/// Test that Azure is recognized as a valid provider.
#[tokio::test]
async fn test_factory_recognizes_azure_provider() {
    let config = TTSConfig {
        provider: "azure".to_string(),
        api_key: "test-subscription-key".to_string(),
        voice_id: Some("en-US-JennyNeural".to_string()),
        ..Default::default()
    };

    let result = create_tts_provider("azure", config);

    assert!(
        result.is_ok(),
        "Azure should be recognized as a valid provider"
    );
}

/// Test that "microsoft-azure" alias works.
#[tokio::test]
async fn test_factory_recognizes_microsoft_azure_alias() {
    let config = TTSConfig {
        provider: "microsoft-azure".to_string(),
        api_key: "test-subscription-key".to_string(),
        voice_id: Some("en-US-JennyNeural".to_string()),
        ..Default::default()
    };

    let result = create_tts_provider("microsoft-azure", config);

    assert!(result.is_ok(), "microsoft-azure alias should be recognized");
}

/// Test case-insensitive provider name.
#[tokio::test]
async fn test_azure_provider_case_insensitive() {
    let config = TTSConfig {
        provider: "azure".to_string(),
        api_key: "test-subscription-key".to_string(),
        voice_id: Some("en-US-JennyNeural".to_string()),
        ..Default::default()
    };

    // Test various cases
    assert!(create_tts_provider("AZURE", config.clone()).is_ok());
    assert!(create_tts_provider("Azure", config.clone()).is_ok());
    assert!(create_tts_provider("azure", config).is_ok());
}

/// Test Azure TTS creation without credentials.
#[test]
fn test_azure_tts_creation() {
    let config = TTSConfig {
        provider: "azure".to_string(),
        api_key: "test-subscription-key".to_string(),
        voice_id: Some("en-US-JennyNeural".to_string()),
        audio_format: Some("linear16".to_string()),
        sample_rate: Some(24000),
        ..Default::default()
    };

    let tts = AzureTTS::new(config);
    assert!(tts.is_ok());

    let tts = tts.unwrap();
    assert!(!tts.is_ready());
    assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);
}

/// Test Azure TTS creation with specific region.
#[test]
fn test_azure_tts_with_region() {
    let config = TTSConfig {
        provider: "azure".to_string(),
        api_key: "test-subscription-key".to_string(),
        voice_id: Some("en-US-JennyNeural".to_string()),
        ..Default::default()
    };

    let tts = AzureTTS::with_region(config, AzureRegion::WestEurope);
    assert!(tts.is_ok());

    let tts = tts.unwrap();
    assert_eq!(tts.azure_config().region, AzureRegion::WestEurope);
    assert_eq!(
        tts.azure_config().build_tts_url(),
        "https://westeurope.tts.speech.microsoft.com/cognitiveservices/v1"
    );
}

/// Test Azure TTS default region is EastUS.
#[test]
fn test_azure_tts_default_region() {
    let config = TTSConfig {
        provider: "azure".to_string(),
        api_key: "test-subscription-key".to_string(),
        voice_id: Some("en-US-JennyNeural".to_string()),
        ..Default::default()
    };

    let tts = AzureTTS::new(config).unwrap();
    assert_eq!(tts.azure_config().region, AzureRegion::EastUS);
}

/// Test Azure TTS provider info structure.
#[test]
fn test_azure_tts_provider_info() {
    let config = TTSConfig {
        provider: "azure".to_string(),
        api_key: "test-subscription-key".to_string(),
        voice_id: Some("en-US-JennyNeural".to_string()),
        ..Default::default()
    };

    let tts = AzureTTS::new(config).unwrap();
    let info = tts.get_provider_info();

    assert_eq!(info["provider"], "azure");
    assert_eq!(info["version"], "1.0.0");
    assert_eq!(info["api_type"], "HTTP REST");
    assert_eq!(info["region"], "eastus");
    assert!(
        info["endpoint"]
            .as_str()
            .unwrap()
            .contains("tts.speech.microsoft.com")
    );
    assert!(info["supported_formats"].is_array());
    assert!(info["supported_sample_rates"].is_array());
}

/// Test Azure audio encoding from format string.
#[test]
fn test_azure_audio_encoding_from_format_strings() {
    // PCM formats
    assert_eq!(
        AzureAudioEncoding::from_format_string("linear16", 24000),
        AzureAudioEncoding::Raw24Khz16BitMonoPcm
    );
    assert_eq!(
        AzureAudioEncoding::from_format_string("pcm", 16000),
        AzureAudioEncoding::Raw16Khz16BitMonoPcm
    );

    // MP3 formats
    assert_eq!(
        AzureAudioEncoding::from_format_string("mp3", 24000),
        AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3
    );
    assert_eq!(
        AzureAudioEncoding::from_format_string("mp3", 48000),
        AzureAudioEncoding::Audio48Khz192KbitrateMonoMp3
    );

    // Telephony formats
    assert_eq!(
        AzureAudioEncoding::from_format_string("mulaw", 8000),
        AzureAudioEncoding::Raw8Khz8BitMonoMulaw
    );
    assert_eq!(
        AzureAudioEncoding::from_format_string("alaw", 8000),
        AzureAudioEncoding::Raw8Khz8BitMonoAlaw
    );

    // Opus formats
    assert_eq!(
        AzureAudioEncoding::from_format_string("opus", 24000),
        AzureAudioEncoding::Audio24Khz16Bit48KbpsMonoOpus
    );

    // Unknown defaults to PCM 24kHz
    assert_eq!(
        AzureAudioEncoding::from_format_string("unknown", 24000),
        AzureAudioEncoding::Raw24Khz16BitMonoPcm
    );
}

/// Test Azure TTS config language extraction from voice name.
#[test]
fn test_azure_tts_config_language_code_extraction() {
    // English US
    let config = AzureTTSConfig {
        base: TTSConfig {
            voice_id: Some("en-US-JennyNeural".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(config.language_code(), "en-US");

    // German
    let config = AzureTTSConfig {
        base: TTSConfig {
            voice_id: Some("de-DE-KatjaNeural".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(config.language_code(), "de-DE");

    // Japanese
    let config = AzureTTSConfig {
        base: TTSConfig {
            voice_id: Some("ja-JP-NanamiNeural".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(config.language_code(), "ja-JP");

    // Empty voice defaults to en-US
    let config = AzureTTSConfig {
        base: TTSConfig {
            voice_id: None,
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(config.language_code(), "en-US");
}

/// Test Azure audio encoding sample rates.
#[test]
fn test_azure_audio_encoding_sample_rates() {
    assert_eq!(AzureAudioEncoding::Raw8Khz16BitMonoPcm.sample_rate(), 8000);
    assert_eq!(
        AzureAudioEncoding::Raw16Khz16BitMonoPcm.sample_rate(),
        16000
    );
    assert_eq!(
        AzureAudioEncoding::Raw24Khz16BitMonoPcm.sample_rate(),
        24000
    );
    assert_eq!(
        AzureAudioEncoding::Raw48Khz16BitMonoPcm.sample_rate(),
        48000
    );
    assert_eq!(
        AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.sample_rate(),
        24000
    );
}

/// Test Azure audio encoding content types.
#[test]
fn test_azure_audio_encoding_content_types() {
    assert_eq!(
        AzureAudioEncoding::Raw24Khz16BitMonoPcm.content_type(),
        "audio/pcm"
    );
    assert_eq!(
        AzureAudioEncoding::Raw8Khz8BitMonoMulaw.content_type(),
        "audio/mulaw"
    );
    assert_eq!(
        AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.content_type(),
        "audio/mpeg"
    );
    assert_eq!(
        AzureAudioEncoding::Audio24Khz16Bit48KbpsMonoOpus.content_type(),
        "audio/opus"
    );
}

/// Test Azure audio encoding is_pcm helper.
#[test]
fn test_azure_audio_encoding_is_pcm() {
    assert!(AzureAudioEncoding::Raw24Khz16BitMonoPcm.is_pcm());
    assert!(AzureAudioEncoding::Raw48Khz16BitMonoPcm.is_pcm());
    assert!(!AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.is_pcm());
    assert!(!AzureAudioEncoding::Audio24Khz16Bit48KbpsMonoOpus.is_pcm());
    assert!(!AzureAudioEncoding::Raw8Khz8BitMonoMulaw.is_pcm());
}

/// Test Azure audio encoding is_telephony helper.
#[test]
fn test_azure_audio_encoding_is_telephony() {
    assert!(AzureAudioEncoding::Raw8Khz8BitMonoMulaw.is_telephony());
    assert!(AzureAudioEncoding::Raw8Khz8BitMonoAlaw.is_telephony());
    assert!(!AzureAudioEncoding::Raw24Khz16BitMonoPcm.is_telephony());
    assert!(!AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.is_telephony());
}

// ============================================================================
// Tests With Credentials (Ignored by Default)
// ============================================================================

/// Test basic synthesis with LINEAR16 format.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_basic_synthesis -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_basic_synthesis() {
    let config = match create_azure_tts_config() {
        Some(c) => c,
        None => {
            println!("Skipping test: Azure credentials not set");
            return;
        }
    };

    // Create provider
    let mut provider = create_tts_provider("azure", config).expect("Failed to create provider");

    // Create audio collector
    let collector = TestAudioCollector::new();
    provider
        .on_audio(Arc::new(collector.clone()))
        .expect("Failed to register callback");

    // Connect
    provider.connect().await.expect("Failed to connect");
    assert!(
        provider.is_ready(),
        "Provider should be ready after connect"
    );

    // Synthesize
    provider
        .speak("Hello, this is a test of Azure Text-to-Speech.", true)
        .await
        .expect("Failed to synthesize");

    // Wait for completion
    let completed = collector.wait_for_completion(15000).await;
    assert!(completed, "Synthesis should complete within timeout");

    // Verify audio was received
    let audio = collector.get_audio().await;
    assert!(!audio.is_empty(), "Should receive audio data");

    // LINEAR16 audio should have even length (16-bit samples)
    assert_eq!(audio.len() % 2, 0, "LINEAR16 audio length should be even");

    // Verify no errors
    let error = collector.get_error().await;
    assert!(error.is_none(), "Should not have any errors: {:?}", error);

    // Verify chunks received
    let chunk_count = collector.get_chunk_count();
    assert!(chunk_count > 0, "Should have received at least one chunk");

    println!(
        "✓ Basic synthesis test passed: {} bytes in {} chunks",
        audio.len(),
        chunk_count
    );

    // Cleanup
    provider.disconnect().await.expect("Failed to disconnect");
}

/// Test synthesis with MP3 format.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_mp3_format -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_mp3_format() {
    let mut config = match create_azure_tts_config() {
        Some(c) => c,
        None => {
            println!("Skipping test: Azure credentials not set");
            return;
        }
    };

    // Use MP3 format
    config.audio_format = Some("mp3".to_string());

    let mut provider = create_tts_provider("azure", config).expect("Failed to create provider");

    let collector = TestAudioCollector::new();
    provider
        .on_audio(Arc::new(collector.clone()))
        .expect("Failed to register callback");

    provider.connect().await.expect("Failed to connect");

    provider
        .speak("Testing MP3 audio output format.", true)
        .await
        .expect("Failed to synthesize");

    let completed = collector.wait_for_completion(15000).await;
    assert!(completed, "Synthesis should complete within timeout");

    let audio = collector.get_audio().await;
    assert!(!audio.is_empty(), "Should receive MP3 audio data");

    // MP3 data typically starts with ID3 tag or sync bytes
    // ID3 starts with 0x49 0x44 0x33 ("ID3")
    // MP3 frame sync starts with 0xFF 0xFB or similar
    let has_id3 = audio.len() >= 3 && audio[0] == 0x49 && audio[1] == 0x44 && audio[2] == 0x33;
    let has_mp3_sync = audio.len() >= 2 && audio[0] == 0xFF && (audio[1] & 0xE0) == 0xE0;

    assert!(
        has_id3 || has_mp3_sync,
        "Audio should have MP3 header signature"
    );

    println!("✓ MP3 format test passed: {} bytes received", audio.len());

    provider.disconnect().await.expect("Failed to disconnect");
}

/// Test synthesis with different voices.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_voice_selection -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_voice_selection() {
    let voices = vec![
        ("en-US-JennyNeural", "English US - Jenny"),
        ("en-US-GuyNeural", "English US - Guy"),
        ("en-GB-SoniaNeural", "English UK - Sonia"),
    ];

    for (voice_id, voice_name) in voices {
        let mut config = match create_azure_tts_config() {
            Some(c) => c,
            None => {
                println!("Skipping test: Azure credentials not set");
                return;
            }
        };

        config.voice_id = Some(voice_id.to_string());

        let mut provider = create_tts_provider("azure", config).expect("Failed to create provider");

        let collector = TestAudioCollector::new();
        provider
            .on_audio(Arc::new(collector.clone()))
            .expect("Failed to register callback");

        provider.connect().await.expect("Failed to connect");

        provider
            .speak("Testing voice selection.", true)
            .await
            .expect("Failed to synthesize");

        let completed = collector.wait_for_completion(15000).await;
        assert!(
            completed,
            "Synthesis should complete for voice {}",
            voice_name
        );

        let audio = collector.get_audio().await;
        assert!(
            !audio.is_empty(),
            "Should receive audio for voice {}",
            voice_name
        );

        let error = collector.get_error().await;
        assert!(
            error.is_none(),
            "Should not have errors for voice {}: {:?}",
            voice_name,
            error
        );

        println!(
            "✓ Voice {} test passed: {} bytes received",
            voice_name,
            audio.len()
        );

        provider.disconnect().await.expect("Failed to disconnect");
    }
}

/// Test synthesis with different speaking rates.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_speaking_rate -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_speaking_rate() {
    let base_config = match create_azure_tts_config() {
        Some(c) => c,
        None => {
            println!("Skipping test: Azure credentials not set");
            return;
        }
    };

    let text = "Testing speaking rate adjustment.";

    // Test with normal rate (1.0)
    let mut normal_config = base_config.clone();
    normal_config.speaking_rate = Some(1.0);

    let mut provider =
        create_tts_provider("azure", normal_config).expect("Failed to create provider");
    let collector_normal = TestAudioCollector::new();
    provider
        .on_audio(Arc::new(collector_normal.clone()))
        .expect("Failed to register callback");
    provider.connect().await.expect("Failed to connect");
    provider
        .speak(text, true)
        .await
        .expect("Failed to synthesize");
    collector_normal.wait_for_completion(15000).await;
    let normal_audio = collector_normal.get_audio().await;
    provider.disconnect().await.expect("Failed to disconnect");

    // Test with fast rate (2.0)
    let mut fast_config = base_config.clone();
    fast_config.speaking_rate = Some(2.0);

    let mut provider =
        create_tts_provider("azure", fast_config).expect("Failed to create provider");
    let collector_fast = TestAudioCollector::new();
    provider
        .on_audio(Arc::new(collector_fast.clone()))
        .expect("Failed to register callback");
    provider.connect().await.expect("Failed to connect");
    provider
        .speak(text, true)
        .await
        .expect("Failed to synthesize");
    collector_fast.wait_for_completion(15000).await;
    let fast_audio = collector_fast.get_audio().await;
    provider.disconnect().await.expect("Failed to disconnect");

    // Fast audio should be shorter than normal audio
    assert!(
        fast_audio.len() < normal_audio.len(),
        "Fast speech should produce less audio data: normal={}, fast={}",
        normal_audio.len(),
        fast_audio.len()
    );

    println!(
        "✓ Speaking rate test passed: normal={} bytes, fast={} bytes",
        normal_audio.len(),
        fast_audio.len()
    );
}

/// Test synthesis with different audio formats.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_audio_formats -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_audio_formats() {
    let formats = vec![
        ("linear16", 24000u32, "PCM 24kHz"),
        ("linear16", 16000u32, "PCM 16kHz"),
        ("mp3", 24000u32, "MP3 24kHz"),
    ];

    for (format, sample_rate, description) in formats {
        let mut config = match create_azure_tts_config() {
            Some(c) => c,
            None => {
                println!("Skipping test: Azure credentials not set");
                return;
            }
        };

        config.audio_format = Some(format.to_string());
        config.sample_rate = Some(sample_rate);

        let mut provider = create_tts_provider("azure", config).expect("Failed to create provider");

        let collector = TestAudioCollector::new();
        provider
            .on_audio(Arc::new(collector.clone()))
            .expect("Failed to register callback");

        provider.connect().await.expect("Failed to connect");

        provider
            .speak("Testing audio format.", true)
            .await
            .expect("Failed to synthesize");

        let completed = collector.wait_for_completion(15000).await;
        assert!(
            completed,
            "Synthesis should complete for format {}",
            description
        );

        let audio = collector.get_audio().await;
        assert!(
            !audio.is_empty(),
            "Should receive audio for format {}",
            description
        );

        println!(
            "✓ Format {} test passed: {} bytes received",
            description,
            audio.len()
        );

        provider.disconnect().await.expect("Failed to disconnect");
    }
}

/// Test connection state management.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_connection_state -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_connection_state() {
    let config = match create_azure_tts_config() {
        Some(c) => c,
        None => {
            println!("Skipping test: Azure credentials not set");
            return;
        }
    };

    let mut provider = create_tts_provider("azure", config).expect("Failed to create provider");

    // Initially not connected
    assert!(!provider.is_ready(), "Should not be ready before connect");
    assert_eq!(
        provider.get_connection_state(),
        ConnectionState::Disconnected,
        "Should be Disconnected before connect"
    );

    // Connect
    provider.connect().await.expect("Failed to connect");
    assert!(provider.is_ready(), "Should be ready after connect");
    assert_eq!(
        provider.get_connection_state(),
        ConnectionState::Connected,
        "Should be Connected after connect"
    );

    // Disconnect
    provider.disconnect().await.expect("Failed to disconnect");
    assert!(!provider.is_ready(), "Should not be ready after disconnect");
    assert_eq!(
        provider.get_connection_state(),
        ConnectionState::Disconnected,
        "Should be Disconnected after disconnect"
    );

    println!("✓ Connection state management test passed");
}

/// Test auto-reconnect on speak when not connected.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_auto_reconnect -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_auto_reconnect() {
    let config = match create_azure_tts_config() {
        Some(c) => c,
        None => {
            println!("Skipping test: Azure credentials not set");
            return;
        }
    };

    let mut provider = create_tts_provider("azure", config).expect("Failed to create provider");

    let collector = TestAudioCollector::new();
    provider
        .on_audio(Arc::new(collector.clone()))
        .expect("Failed to register callback");

    // Don't explicitly connect - speak should auto-connect
    assert!(!provider.is_ready(), "Should not be ready before speak");

    provider
        .speak("Testing auto-reconnect functionality.", true)
        .await
        .expect("Speak should auto-connect");

    let completed = collector.wait_for_completion(15000).await;
    assert!(completed, "Synthesis should complete");

    let audio = collector.get_audio().await;
    assert!(
        !audio.is_empty(),
        "Should receive audio even without explicit connect"
    );

    // Should be connected now
    assert!(provider.is_ready(), "Should be ready after auto-connect");

    println!(
        "✓ Auto-reconnect test passed: {} bytes received",
        audio.len()
    );

    provider.disconnect().await.expect("Failed to disconnect");
}

/// Test empty text handling.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_empty_text -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_empty_text() {
    let config = match create_azure_tts_config() {
        Some(c) => c,
        None => {
            println!("Skipping test: Azure credentials not set");
            return;
        }
    };

    let mut provider = create_tts_provider("azure", config).expect("Failed to create provider");
    provider.connect().await.expect("Failed to connect");

    // Empty text should be handled gracefully (either no-op or minimal audio)
    let result = provider.speak("", true).await;
    // The behavior may vary - either Ok (no-op) or minimal audio
    // We just want to ensure it doesn't crash or return a hard error
    assert!(result.is_ok(), "Empty text should not cause hard error");

    println!("✓ Empty text handling test passed");

    provider.disconnect().await.expect("Failed to disconnect");
}

/// Test pronunciation replacement.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_pronunciations -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_pronunciations() {
    use sayna::core::tts::Pronunciation;

    let mut config = match create_azure_tts_config() {
        Some(c) => c,
        None => {
            println!("Skipping test: Azure credentials not set");
            return;
        }
    };

    // Add pronunciation replacements
    config.pronunciations = vec![
        Pronunciation {
            word: "API".to_string(),
            pronunciation: "A P I".to_string(),
        },
        Pronunciation {
            word: "SDK".to_string(),
            pronunciation: "S D K".to_string(),
        },
    ];

    let mut provider = create_tts_provider("azure", config).expect("Failed to create provider");

    let collector = TestAudioCollector::new();
    provider
        .on_audio(Arc::new(collector.clone()))
        .expect("Failed to register callback");

    provider.connect().await.expect("Failed to connect");

    // Speak text containing words that should be replaced
    provider
        .speak("The API and SDK are working correctly.", true)
        .await
        .expect("Failed to synthesize");

    let completed = collector.wait_for_completion(15000).await;
    assert!(completed, "Synthesis should complete");

    let audio = collector.get_audio().await;
    assert!(
        !audio.is_empty(),
        "Should receive audio with pronunciations"
    );

    println!(
        "✓ Pronunciation replacement test passed: {} bytes received",
        audio.len()
    );

    provider.disconnect().await.expect("Failed to disconnect");
}

/// Test different Azure regions.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_different_regions -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_different_regions() {
    // This test just verifies that the TTS can be configured with different regions
    // We only test EastUS since that's the region where the credentials are valid
    let (subscription_key, region_str) = match get_azure_credentials() {
        Some(creds) => creds,
        None => {
            println!("Skipping test: Azure credentials not set");
            return;
        }
    };

    let config = TTSConfig {
        provider: "azure".to_string(),
        api_key: subscription_key,
        voice_id: Some("en-US-JennyNeural".to_string()),
        audio_format: Some("linear16".to_string()),
        sample_rate: Some(24000),
        ..Default::default()
    };

    // Parse region from environment
    let region: AzureRegion = region_str.parse().unwrap_or(AzureRegion::EastUS);

    let mut provider =
        AzureTTS::with_region(config, region.clone()).expect("Failed to create provider");

    assert_eq!(provider.azure_config().region, region);

    let collector = TestAudioCollector::new();
    provider
        .on_audio(Arc::new(collector.clone()))
        .expect("Failed to register callback");

    provider.connect().await.expect("Failed to connect");

    provider
        .speak("Testing regional endpoint.", true)
        .await
        .expect("Failed to synthesize");

    let completed = collector.wait_for_completion(15000).await;
    assert!(completed, "Synthesis should complete for region");

    let audio = collector.get_audio().await;
    assert!(!audio.is_empty(), "Should receive audio from region");

    println!(
        "✓ Region {} test passed: {} bytes received",
        region.as_str(),
        audio.len()
    );

    provider.disconnect().await.expect("Failed to disconnect");
}

/// Test clear queue functionality.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_clear_queue -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_clear_queue() {
    let config = match create_azure_tts_config() {
        Some(c) => c,
        None => {
            println!("Skipping test: Azure credentials not set");
            return;
        }
    };

    let mut provider = create_tts_provider("azure", config).expect("Failed to create provider");

    let collector = TestAudioCollector::new();
    provider
        .on_audio(Arc::new(collector.clone()))
        .expect("Failed to register callback");

    provider.connect().await.expect("Failed to connect");

    // Queue multiple speaks without flush
    for i in 0..3 {
        provider
            .speak(&format!("Sentence number {}.", i), false)
            .await
            .expect("Failed to speak");
    }

    // Clear should cancel pending
    let clear_result = provider.clear().await;
    assert!(clear_result.is_ok(), "Clear should succeed");

    println!("✓ Clear queue test passed");

    provider.disconnect().await.expect("Failed to disconnect");
}

/// Test error handling for invalid voice.
///
/// This test requires valid Azure credentials.
/// Run with: `cargo test test_azure_tts_invalid_voice -- --ignored`
#[tokio::test]
#[ignore = "Requires valid Azure credentials"]
async fn test_azure_tts_invalid_voice() {
    let mut config = match create_azure_tts_config() {
        Some(c) => c,
        None => {
            println!("Skipping test: Azure credentials not set");
            return;
        }
    };

    // Use an invalid voice name
    config.voice_id = Some("invalid-voice-name-xyz".to_string());

    let mut provider = create_tts_provider("azure", config).expect("Failed to create provider");

    let collector = TestAudioCollector::new();
    provider
        .on_audio(Arc::new(collector.clone()))
        .expect("Failed to register callback");

    provider.connect().await.expect("Failed to connect");

    // Synthesis should fail with invalid voice
    let result = provider
        .speak("This should fail with invalid voice.", true)
        .await;

    // Either speak() returns error, or error callback is invoked
    let speak_error = result.is_err();

    // Wait a bit for potential async error
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let callback_error = collector.get_error().await.is_some();

    assert!(
        speak_error || callback_error,
        "Should get error with invalid voice"
    );

    println!("✓ Invalid voice error handling test passed");

    provider.disconnect().await.expect("Failed to disconnect");
}

/// Test that an unknown provider is correctly rejected.
#[tokio::test]
async fn test_factory_rejects_unknown_provider() {
    let config = TTSConfig {
        provider: "unknown_provider".to_string(),
        api_key: "some_key".to_string(),
        ..Default::default()
    };

    let result = create_tts_provider("unknown_provider", config);

    assert!(result.is_err(), "Should reject unknown provider");

    if let Err(e) = result {
        let error_msg = e.to_string();
        assert!(
            error_msg.contains("Unsupported TTS provider"),
            "Error should indicate unsupported provider: {error_msg}"
        );
    }
}
