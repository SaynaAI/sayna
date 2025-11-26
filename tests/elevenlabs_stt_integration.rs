//! Integration tests for ElevenLabs STT provider
//!
//! These tests verify:
//! - Provider creation through factory
//! - Configuration validation
//! - Callback registration
//! - Error handling
//!
//! Note: Tests requiring actual API calls are marked with #[ignore]
//! and require ELEVENLABS_API_KEY environment variable.

use sayna::core::stt::{
    BaseSTT, CommitStrategy, ElevenLabsAudioFormat, ElevenLabsRegion, ElevenLabsSTT, STTConfig,
    STTError, STTProvider, STTResult, create_stt_provider, create_stt_provider_from_enum,
    get_supported_stt_providers,
};
use std::sync::Arc;

/// Test that ElevenLabs is included in supported providers
#[test]
fn test_elevenlabs_in_supported_providers() {
    let providers = get_supported_stt_providers();
    assert!(providers.contains(&"elevenlabs"));
    assert_eq!(providers.len(), 3); // deepgram, google, elevenlabs
}

/// Test provider creation via string name
#[test]
fn test_create_elevenlabs_provider_by_name() {
    let config = STTConfig {
        provider: "elevenlabs".to_string(),
        api_key: "test-api-key".to_string(),
        language: "en".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
    };

    let result = create_stt_provider("elevenlabs", config);
    assert!(result.is_ok());

    let stt = result.unwrap();
    assert_eq!(
        stt.get_provider_info(),
        "ElevenLabs STT Real-Time WebSocket"
    );
    assert!(!stt.is_ready());
}

/// Test provider creation via enum
#[test]
fn test_create_elevenlabs_provider_by_enum() {
    let config = STTConfig {
        provider: "elevenlabs".to_string(),
        api_key: "test-api-key".to_string(),
        language: "en-US".to_string(),
        sample_rate: 24000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
    };

    let result = create_stt_provider_from_enum(STTProvider::ElevenLabs, config);
    assert!(result.is_ok());
}

/// Test case-insensitive provider name parsing
#[test]
fn test_elevenlabs_provider_name_case_insensitive() {
    let config = STTConfig {
        provider: "elevenlabs".to_string(),
        api_key: "test-api-key".to_string(),
        ..Default::default()
    };

    // Test various cases
    assert!(create_stt_provider("elevenlabs", config.clone()).is_ok());
    assert!(create_stt_provider("ElevenLabs", config.clone()).is_ok());
    assert!(create_stt_provider("ELEVENLABS", config).is_ok());
}

/// Test API key validation
#[test]
fn test_elevenlabs_requires_api_key() {
    let config = STTConfig {
        provider: "elevenlabs".to_string(),
        api_key: String::new(), // Empty API key
        language: "en".to_string(),
        sample_rate: 16000,
        ..Default::default()
    };

    let result = create_stt_provider("elevenlabs", config);
    assert!(result.is_err());

    match result {
        Err(STTError::AuthenticationFailed(msg)) => {
            assert!(msg.contains("API key is required"));
        }
        _ => panic!("Expected AuthenticationFailed error"),
    }
}

/// Test sample rate to audio format mapping
#[test]
fn test_sample_rate_to_audio_format() {
    let test_cases = vec![
        (8000, ElevenLabsAudioFormat::Pcm8000),
        (16000, ElevenLabsAudioFormat::Pcm16000),
        (22050, ElevenLabsAudioFormat::Pcm22050),
        (24000, ElevenLabsAudioFormat::Pcm24000),
        (44100, ElevenLabsAudioFormat::Pcm44100),
        (48000, ElevenLabsAudioFormat::Pcm48000),
    ];

    for (sample_rate, expected_format) in test_cases {
        let format = ElevenLabsAudioFormat::from_sample_rate(sample_rate);
        assert_eq!(
            format, expected_format,
            "Sample rate {} mapping failed",
            sample_rate
        );
    }
}

/// Test audio format to query string conversion
#[test]
fn test_audio_format_query_string() {
    assert_eq!(ElevenLabsAudioFormat::Pcm8000.as_str(), "pcm_8000");
    assert_eq!(ElevenLabsAudioFormat::Pcm16000.as_str(), "pcm_16000");
    assert_eq!(ElevenLabsAudioFormat::Pcm24000.as_str(), "pcm_24000");
    assert_eq!(ElevenLabsAudioFormat::Ulaw8000.as_str(), "ulaw_8000");
}

/// Test commit strategy enum
#[test]
fn test_commit_strategy() {
    assert_eq!(CommitStrategy::Vad.as_str(), "vad");
    assert_eq!(CommitStrategy::Manual.as_str(), "manual");

    // Default should be VAD
    assert_eq!(CommitStrategy::default(), CommitStrategy::Vad);
}

/// Test regional endpoints
#[test]
fn test_regional_endpoints() {
    assert_eq!(
        ElevenLabsRegion::Default.websocket_base_url(),
        "wss://api.elevenlabs.io"
    );
    assert_eq!(
        ElevenLabsRegion::Us.websocket_base_url(),
        "wss://api.us.elevenlabs.io"
    );
    assert_eq!(
        ElevenLabsRegion::Eu.websocket_base_url(),
        "wss://api.eu.residency.elevenlabs.io"
    );
    assert_eq!(
        ElevenLabsRegion::India.websocket_base_url(),
        "wss://api.in.residency.elevenlabs.io"
    );
}

/// Test callback registration (without connection)
#[tokio::test]
async fn test_callback_registration() {
    let config = STTConfig {
        api_key: "test-api-key".to_string(),
        ..Default::default()
    };

    let mut stt = ElevenLabsSTT::new(config).unwrap();

    // Register result callback
    let callback = Arc::new(|_result: STTResult| {
        Box::pin(async move {
            println!("Got result");
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    let result = stt.on_result(callback).await;
    assert!(result.is_ok());

    // Register error callback
    let error_callback = Arc::new(|_error: STTError| {
        Box::pin(async move {
            println!("Got error");
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    let result = stt.on_error(error_callback).await;
    assert!(result.is_ok());
}

/// Test send_audio fails when not connected
#[tokio::test]
async fn test_send_audio_requires_connection() {
    let config = STTConfig {
        api_key: "test-api-key".to_string(),
        ..Default::default()
    };

    let mut stt = ElevenLabsSTT::new(config).unwrap();

    let result = stt.send_audio(vec![0u8; 100]).await;
    assert!(result.is_err());

    match result {
        Err(STTError::ConnectionFailed(_)) => {}
        _ => panic!("Expected ConnectionFailed error"),
    }
}

/// Test configuration retrieval
#[test]
fn test_get_config() {
    let config = STTConfig {
        api_key: "test-api-key".to_string(),
        language: "es".to_string(),
        sample_rate: 24000,
        ..Default::default()
    };

    let stt = ElevenLabsSTT::new(config).unwrap();
    let retrieved = stt.get_config().unwrap();

    assert_eq!(retrieved.api_key, "test-api-key");
    assert_eq!(retrieved.language, "es");
    assert_eq!(retrieved.sample_rate, 24000);
}

/// Test default audio format is 16kHz PCM
#[test]
fn test_default_audio_format() {
    assert_eq!(
        ElevenLabsAudioFormat::default(),
        ElevenLabsAudioFormat::Pcm16000
    );
}

/// Test default region is Default (global)
#[test]
fn test_default_region() {
    assert_eq!(ElevenLabsRegion::default(), ElevenLabsRegion::Default);
}

/// Test unknown sample rate defaults to 16kHz
#[test]
fn test_unknown_sample_rate_defaults_to_16k() {
    let format = ElevenLabsAudioFormat::from_sample_rate(12345);
    assert_eq!(format, ElevenLabsAudioFormat::Pcm16000);
}

/// Test audio format sample rate getter
#[test]
fn test_audio_format_sample_rate_getter() {
    assert_eq!(ElevenLabsAudioFormat::Pcm8000.sample_rate(), 8000);
    assert_eq!(ElevenLabsAudioFormat::Pcm16000.sample_rate(), 16000);
    assert_eq!(ElevenLabsAudioFormat::Pcm22050.sample_rate(), 22050);
    assert_eq!(ElevenLabsAudioFormat::Pcm24000.sample_rate(), 24000);
    assert_eq!(ElevenLabsAudioFormat::Pcm44100.sample_rate(), 44100);
    assert_eq!(ElevenLabsAudioFormat::Pcm48000.sample_rate(), 48000);
    assert_eq!(ElevenLabsAudioFormat::Ulaw8000.sample_rate(), 8000);
}

/// Integration test with real API (requires ELEVENLABS_API_KEY)
#[tokio::test]
#[ignore = "Requires ELEVENLABS_API_KEY environment variable"]
async fn test_real_connection() {
    let api_key = std::env::var("ELEVENLABS_API_KEY").expect("ELEVENLABS_API_KEY not set");

    let config = STTConfig {
        api_key,
        language: "en".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
        provider: "elevenlabs".to_string(),
    };

    let mut stt = ElevenLabsSTT::new(config).unwrap();

    // Register callback
    let received = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let received_clone = received.clone();

    let callback = Arc::new(move |result: STTResult| {
        let received = received_clone.clone();
        Box::pin(async move {
            println!(
                "Received: {} (final: {})",
                result.transcript, result.is_final
            );
            received.store(true, std::sync::atomic::Ordering::SeqCst);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    stt.on_result(callback).await.unwrap();

    // Connect
    let connect_result = stt.connect().await;
    assert!(
        connect_result.is_ok(),
        "Connection failed: {:?}",
        connect_result
    );
    assert!(stt.is_ready());

    // Check session ID is set
    assert!(stt.get_session_id().is_some());
    println!("Session ID: {}", stt.get_session_id().unwrap());

    // Disconnect
    stt.disconnect().await.unwrap();
    assert!(!stt.is_ready());
}

/// Test with audio file (requires ELEVENLABS_API_KEY and audio file)
#[tokio::test]
#[ignore = "Requires ELEVENLABS_API_KEY and test audio file"]
async fn test_transcription_with_audio() {
    use tokio::time::{Duration, sleep};

    let api_key = std::env::var("ELEVENLABS_API_KEY").expect("ELEVENLABS_API_KEY not set");

    let config = STTConfig {
        api_key,
        language: "en".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
        provider: "elevenlabs".to_string(),
    };

    let mut stt = ElevenLabsSTT::new(config).unwrap();

    // Track results
    let results = Arc::new(std::sync::Mutex::new(Vec::new()));
    let results_clone = results.clone();

    let callback = Arc::new(move |result: STTResult| {
        let results = results_clone.clone();
        Box::pin(async move {
            results.lock().unwrap().push(result);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    stt.on_result(callback).await.unwrap();
    stt.connect().await.unwrap();

    // Send some silent audio (zeros)
    // In real usage, this would be actual audio data
    for _ in 0..10 {
        let audio_chunk = vec![0u8; 3200]; // 100ms of 16kHz 16-bit mono audio
        stt.send_audio(audio_chunk).await.unwrap();
        sleep(Duration::from_millis(100)).await;
    }

    // Wait for processing
    sleep(Duration::from_secs(2)).await;

    stt.disconnect().await.unwrap();

    let final_results = results.lock().unwrap();
    println!("Received {} results", final_results.len());
}
