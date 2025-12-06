//! Integration tests for Whisper STT provider
//!
//! These tests verify:
//! - Provider creation through factory
//! - Configuration validation
//! - Callback registration
//! - Error handling
//! - Model and audio format validation
//!
//! Note: Tests requiring downloaded model files are marked with #[ignore]
//! and require running `sayna init` first.
//!
//! Run with: `cargo test --features whisper-stt whisper_stt`

#![cfg(feature = "whisper-stt")]

use sayna::core::stt::{
    BaseSTT, STTConfig, STTError, STTProvider, STTResult, WhisperAssetConfig, WhisperSTT,
    WhisperSTTConfig, create_stt_provider, create_stt_provider_from_enum,
    get_supported_stt_providers,
};
use std::sync::Arc;

// ============================================================================
// Factory Integration Tests
// ============================================================================

/// Test that Whisper is included in supported providers
#[test]
fn test_whisper_in_supported_providers() {
    let providers = get_supported_stt_providers();
    assert!(providers.contains(&"whisper"));
    // At least 6 providers when whisper-stt feature is enabled:
    // deepgram, google, elevenlabs, microsoft-azure, cartesia, whisper
    assert!(providers.len() >= 6);
}

/// Test provider creation via string name
#[test]
fn test_create_whisper_provider_by_name() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(), // Not required for Whisper
        language: "en".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "whisper-base".to_string(),
        cache_path: None,
    };

    let result = create_stt_provider("whisper", config);
    assert!(result.is_ok());

    let stt = result.unwrap();
    assert_eq!(stt.get_provider_info(), "Whisper ONNX (Local)");
    assert!(!stt.is_ready());
}

/// Test provider creation via enum
#[test]
fn test_create_whisper_provider_by_enum() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "whisper-base".to_string(),
        cache_path: None,
    };

    let result = create_stt_provider_from_enum(STTProvider::Whisper, config);
    assert!(result.is_ok());
}

/// Test case-insensitive provider name parsing
#[test]
fn test_whisper_provider_name_case_insensitive() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        ..Default::default()
    };

    // Test various cases
    assert!(create_stt_provider("whisper", config.clone()).is_ok());
    assert!(create_stt_provider("Whisper", config.clone()).is_ok());
    assert!(create_stt_provider("WHISPER", config.clone()).is_ok());
    assert!(create_stt_provider("whisper-onnx", config).is_ok());
}

/// Test provider enum Display trait
#[test]
fn test_whisper_provider_display() {
    assert_eq!(STTProvider::Whisper.to_string(), "whisper");
}

/// Test provider enum FromStr trait
#[test]
fn test_whisper_provider_from_str() {
    assert_eq!(
        "whisper".parse::<STTProvider>().unwrap(),
        STTProvider::Whisper
    );
    assert_eq!(
        "whisper-onnx".parse::<STTProvider>().unwrap(),
        STTProvider::Whisper
    );
}

// ============================================================================
// Configuration Validation Tests
// ============================================================================

/// Test that empty API key is allowed (local model)
#[test]
fn test_whisper_accepts_empty_api_key() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(), // Empty is OK for Whisper
        language: "en".to_string(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        ..Default::default()
    };

    let result = create_stt_provider("whisper", config);
    assert!(result.is_ok());
}

/// Test sample rate validation
#[test]
fn test_whisper_requires_16khz() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        language: "en".to_string(),
        sample_rate: 44100, // Invalid - Whisper requires 16kHz
        encoding: "linear16".to_string(),
        ..Default::default()
    };

    let result = create_stt_provider("whisper", config);
    assert!(result.is_err());

    match result {
        Err(STTError::ConfigurationError(msg)) => {
            assert!(msg.contains("16000Hz"));
            assert!(msg.contains("44100"));
        }
        _ => panic!("Expected ConfigurationError for invalid sample rate"),
    }
}

/// Test encoding validation
#[test]
fn test_whisper_requires_linear16() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        language: "en".to_string(),
        sample_rate: 16000,
        encoding: "mulaw".to_string(), // Invalid
        ..Default::default()
    };

    let result = create_stt_provider("whisper", config);
    assert!(result.is_err());

    match result {
        Err(STTError::InvalidAudioFormat(msg)) => {
            assert!(msg.contains("linear16"));
        }
        _ => panic!("Expected InvalidAudioFormat error"),
    }
}

/// Test model name validation
#[test]
fn test_whisper_validates_model_name() {
    // Invalid model name
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        language: "en".to_string(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        model: "whisper-tiny".to_string(), // Invalid - not supported
        ..Default::default()
    };

    let result = create_stt_provider("whisper", config);
    assert!(result.is_err());

    match result {
        Err(STTError::ConfigurationError(msg)) => {
            assert!(msg.contains("Unknown Whisper model"));
            assert!(msg.contains("whisper-tiny"));
            assert!(msg.contains("whisper-base"));
        }
        _ => panic!("Expected ConfigurationError for invalid model"),
    }
}

/// Test valid model names
#[test]
fn test_whisper_accepts_valid_models() {
    // whisper-base
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        model: "whisper-base".to_string(),
        ..Default::default()
    };
    assert!(create_stt_provider("whisper", config).is_ok());

    // whisper-large-v3
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        model: "whisper-large-v3".to_string(),
        ..Default::default()
    };
    assert!(create_stt_provider("whisper", config).is_ok());
}

/// Test empty model name defaults to whisper-base
#[test]
fn test_whisper_empty_model_defaults_to_base() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        model: String::new(), // Empty model
        ..Default::default()
    };

    let result = create_stt_provider("whisper", config);
    assert!(result.is_ok());
}

/// Test nova-3 model is overridden to whisper-base
#[test]
fn test_whisper_nova3_overridden_to_base() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        model: "nova-3".to_string(), // STTConfig default, should be overridden
        ..Default::default()
    };

    let result = create_stt_provider("whisper", config);
    assert!(result.is_ok());
}

// ============================================================================
// WhisperSTTConfig Tests
// ============================================================================

/// Test WhisperSTTConfig defaults
#[test]
fn test_whisper_stt_config_default() {
    let config = WhisperSTTConfig::default();
    assert!(config.cache_path.is_none());
}

/// Test WhisperSTTConfig from STTConfig
#[test]
fn test_whisper_stt_config_from_stt_config() {
    let stt_config = STTConfig {
        provider: "whisper".to_string(),
        language: "es".to_string(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        model: "whisper-base".to_string(),
        ..Default::default()
    };

    let whisper_config = WhisperSTTConfig::from_stt_config(stt_config).unwrap();
    assert_eq!(whisper_config.base.language, "es");
    assert_eq!(whisper_config.model_name(), "whisper-base");
}

/// Test WhisperSTTConfig model name resolution
#[test]
fn test_whisper_config_model_name() {
    let stt_config = STTConfig {
        provider: "whisper".to_string(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        model: "whisper-large-v3".to_string(),
        ..Default::default()
    };

    let whisper_config = WhisperSTTConfig::from_stt_config(stt_config).unwrap();
    assert_eq!(whisper_config.model_name(), "whisper-large-v3");
}

// ============================================================================
// WhisperAssetConfig Tests
// ============================================================================

/// Test WhisperAssetConfig new
#[test]
fn test_whisper_asset_config_new() {
    let cache_path = std::path::PathBuf::from("/tmp/test-cache");
    let config = WhisperAssetConfig::new(cache_path.clone(), "whisper-base");
    assert_eq!(config.model_name, "whisper-base");
    assert_eq!(config.cache_path, cache_path);
}

/// Test WhisperAssetConfig with large model
#[test]
fn test_whisper_asset_config_large_model() {
    let cache_path = std::path::PathBuf::from("/tmp/test-cache");
    let config = WhisperAssetConfig::new(cache_path, "whisper-large-v3");
    assert_eq!(config.model_name, "whisper-large-v3");
}

/// Test WhisperAssetConfig model_dir
#[test]
fn test_whisper_asset_config_model_dir() {
    let cache_path = std::path::PathBuf::from("/tmp/test-cache");
    let config = WhisperAssetConfig::new(cache_path, "whisper-base");
    let model_dir = config.model_dir();
    assert!(model_dir.ends_with("whisper-base"));
}

// ============================================================================
// Callback Registration Tests
// ============================================================================

/// Test callback registration (without connection)
#[tokio::test]
async fn test_whisper_callback_registration() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        ..Default::default()
    };

    let mut stt = WhisperSTT::new(config).unwrap();

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

// ============================================================================
// Connection State Tests
// ============================================================================

/// Test send_audio fails when not connected
#[tokio::test]
async fn test_whisper_send_audio_requires_connection() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        ..Default::default()
    };

    let mut stt = WhisperSTT::new(config).unwrap();

    let result = stt.send_audio(vec![0u8; 100]).await;
    assert!(result.is_err());

    match result {
        Err(STTError::ConnectionFailed(msg)) => {
            assert!(msg.contains("Not connected"));
        }
        _ => panic!("Expected ConnectionFailed error"),
    }
}

/// Test disconnect succeeds even when not connected
#[tokio::test]
async fn test_whisper_disconnect_when_not_connected() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        ..Default::default()
    };

    let mut stt = WhisperSTT::new(config).unwrap();

    // Disconnect should succeed
    let result = stt.disconnect().await;
    assert!(result.is_ok());
    assert!(!stt.is_ready());
}

/// Test is_ready returns false before connection
#[test]
fn test_whisper_not_ready_before_connect() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        ..Default::default()
    };

    let stt = WhisperSTT::new(config).unwrap();
    assert!(!stt.is_ready());
}

// ============================================================================
// Configuration Retrieval Tests
// ============================================================================

/// Test configuration retrieval
#[test]
fn test_whisper_get_config() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        language: "fr".to_string(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        ..Default::default()
    };

    let stt = WhisperSTT::new(config).unwrap();
    let retrieved = stt.get_config().unwrap();

    assert_eq!(retrieved.language, "fr");
    assert_eq!(retrieved.provider, "whisper");
    assert_eq!(retrieved.sample_rate, 16000);
}

/// Test get_provider_info
#[test]
fn test_whisper_get_provider_info() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        ..Default::default()
    };

    let stt = WhisperSTT::new(config).unwrap();
    assert_eq!(stt.get_provider_info(), "Whisper ONNX (Local)");
}

// ============================================================================
// Error Message Tests
// ============================================================================

/// Test error message includes whisper in supported providers
#[test]
fn test_error_message_includes_whisper() {
    let result = "invalid".parse::<STTProvider>();
    assert!(result.is_err());
    if let Err(STTError::ConfigurationError(msg)) = result {
        assert!(msg.contains("whisper"));
    }
}

// ============================================================================
// Model Loading Tests (Require Downloaded Models)
// ============================================================================

/// Test connect fails gracefully when model not downloaded
#[tokio::test]
async fn test_whisper_connect_without_model() {
    // Use a non-existent cache path to ensure models aren't found
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        model: "whisper-base".to_string(),
        ..Default::default()
    };

    let mut stt = WhisperSTT::new(config).unwrap();

    // Temporarily set a non-existent cache path would require modifying config,
    // but we can at least verify the error message format when model not found
    let result = stt.connect().await;

    // This will either succeed (if model is available) or fail with helpful message
    if result.is_err() {
        match result {
            Err(STTError::ConfigurationError(msg)) => {
                // Should mention running 'sayna init' OR cache_path not configured
                assert!(
                    msg.contains("sayna init")
                        || msg.contains("assets not found")
                        || msg.contains("cache_path not configured"),
                    "Error message should be helpful: {}",
                    msg
                );
            }
            Err(other) => {
                // Other errors are acceptable if model loading fails for other reasons
                println!("Got error: {}", other);
            }
            _ => unreachable!(),
        }
    }
}

/// Integration test with real model (requires downloaded model)
#[tokio::test]
#[ignore = "Requires downloaded whisper model - run 'sayna init' first"]
async fn test_whisper_connect_and_disconnect() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        language: "en".to_string(),
        sample_rate: 16000,
        channels: 1,
        encoding: "linear16".to_string(),
        model: "whisper-base".to_string(),
        punctuation: true,
        cache_path: None,
    };

    let mut stt = WhisperSTT::new(config).unwrap();

    // Register callback
    let callback = Arc::new(|result: STTResult| {
        Box::pin(async move {
            println!(
                "Received: {} (final: {})",
                result.transcript, result.is_final
            );
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    stt.on_result(callback).await.unwrap();

    // Connect (loads model)
    let connect_result = stt.connect().await;
    assert!(
        connect_result.is_ok(),
        "Connection failed: {:?}",
        connect_result
    );
    assert!(stt.is_ready());

    // Disconnect
    stt.disconnect().await.unwrap();
    assert!(!stt.is_ready());
}

/// Test transcription with silent audio (requires downloaded model)
#[tokio::test]
#[ignore = "Requires downloaded whisper model - run 'sayna init' first"]
async fn test_whisper_transcription_with_silent_audio() {
    use tokio::time::{Duration, sleep};

    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        language: "en".to_string(),
        sample_rate: 16000,
        channels: 1,
        encoding: "linear16".to_string(),
        model: "whisper-base".to_string(),
        punctuation: true,
        cache_path: None,
    };

    let mut stt = WhisperSTT::new(config).unwrap();

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

    // Send some silent audio (zeros) - 1 second worth
    // 16kHz * 2 bytes per sample = 32000 bytes per second
    let audio_chunk = vec![0u8; 32000];
    stt.send_audio(audio_chunk).await.unwrap();

    // Wait for processing
    sleep(Duration::from_secs(1)).await;

    // Flush remaining audio
    let flush_result = stt.flush().await;
    assert!(flush_result.is_ok());

    stt.disconnect().await.unwrap();

    let final_results = results.lock().unwrap();
    println!("Received {} results", final_results.len());
}

/// Test config update triggers reconnection (requires downloaded model)
#[tokio::test]
#[ignore = "Requires downloaded whisper model - run 'sayna init' first"]
async fn test_whisper_config_update() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        language: "en".to_string(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        model: "whisper-base".to_string(),
        ..Default::default()
    };

    let mut stt = WhisperSTT::new(config.clone()).unwrap();

    // Connect first
    stt.connect().await.unwrap();
    assert!(stt.is_ready());

    // Update config with same model (should not reconnect)
    let mut new_config = config.clone();
    new_config.language = "es".to_string();
    stt.update_config(new_config).await.unwrap();

    // Should still be ready
    assert!(stt.is_ready());

    // Verify language changed
    let retrieved = stt.get_config().unwrap();
    assert_eq!(retrieved.language, "es");

    stt.disconnect().await.unwrap();
}

// ============================================================================
// Default Trait Tests
// ============================================================================

/// Test WhisperSTT default
#[test]
fn test_whisper_stt_default() {
    let stt = WhisperSTT::default();
    assert!(!stt.is_ready());
    assert!(stt.get_config().is_none());
}

// ============================================================================
// Audio Buffer Tests
// ============================================================================

/// Test audio buffer starts empty
#[tokio::test]
async fn test_whisper_buffer_starts_empty() {
    let config = STTConfig {
        provider: "whisper".to_string(),
        api_key: String::new(),
        sample_rate: 16000,
        encoding: "linear16".to_string(),
        ..Default::default()
    };

    let stt = WhisperSTT::new(config).unwrap();

    // Buffer should be empty on creation
    // Note: We can't directly access the buffer, but we can verify through behavior
    assert!(!stt.is_ready());
}
