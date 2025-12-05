//! Integration tests for Microsoft Azure STT provider
//!
//! These tests verify:
//! - Provider creation through factory
//! - Configuration validation
//! - Callback registration
//! - Error handling
//! - Azure-specific configuration options
//!
//! Note: Tests requiring actual API calls are marked with #[ignore]
//! and require AZURE_SPEECH_SUBSCRIPTION_KEY and AZURE_SPEECH_REGION
//! environment variables.

use sayna::core::stt::{
    AzureOutputFormat, AzureProfanityOption, AzureRegion, AzureSTT, AzureSTTConfig, BaseSTT,
    STTConfig, STTError, STTProvider, STTResult, create_stt_provider,
    create_stt_provider_from_enum, get_supported_stt_providers,
};
use std::sync::Arc;

// =============================================================================
// Factory Integration Tests
// =============================================================================

/// Test that Azure is included in supported providers
#[test]
fn test_azure_in_supported_providers() {
    let providers = get_supported_stt_providers();
    assert!(providers.contains(&"microsoft-azure"));
    // At least 5 providers: deepgram, google, elevenlabs, microsoft-azure, cartesia
    // May include more with feature flags (e.g., whisper)
    assert!(providers.len() >= 5);
}

/// Test provider creation via string name
#[test]
fn test_create_azure_provider_by_name() {
    let config = STTConfig {
        provider: "microsoft-azure".to_string(),
        api_key: "test-subscription-key".to_string(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
        ..Default::default()
    };

    let result = create_stt_provider("microsoft-azure", config);
    assert!(result.is_ok());

    let stt = result.unwrap();
    assert_eq!(stt.get_provider_info(), "Microsoft Azure Speech-to-Text");
    assert!(!stt.is_ready());
}

/// Test provider creation via shorthand name "azure"
#[test]
fn test_create_azure_provider_by_shorthand() {
    let config = STTConfig {
        provider: "azure".to_string(),
        api_key: "test-subscription-key".to_string(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        ..Default::default()
    };

    let result = create_stt_provider("azure", config);
    assert!(result.is_ok());

    let stt = result.unwrap();
    assert_eq!(stt.get_provider_info(), "Microsoft Azure Speech-to-Text");
}

/// Test provider creation via enum
#[test]
fn test_create_azure_provider_by_enum() {
    let config = STTConfig {
        provider: "microsoft-azure".to_string(),
        api_key: "test-subscription-key".to_string(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        ..Default::default()
    };

    let result = create_stt_provider_from_enum(STTProvider::Azure, config);
    assert!(result.is_ok());
}

/// Test case-insensitive provider name parsing
#[test]
fn test_azure_provider_name_case_insensitive() {
    let config = STTConfig {
        api_key: "test-subscription-key".to_string(),
        ..Default::default()
    };

    // Test various cases
    assert!(create_stt_provider("microsoft-azure", config.clone()).is_ok());
    assert!(create_stt_provider("Microsoft-Azure", config.clone()).is_ok());
    assert!(create_stt_provider("MICROSOFT-AZURE", config.clone()).is_ok());
    assert!(create_stt_provider("azure", config.clone()).is_ok());
    assert!(create_stt_provider("Azure", config.clone()).is_ok());
    assert!(create_stt_provider("AZURE", config).is_ok());
}

/// Test API key validation
#[test]
fn test_azure_requires_api_key() {
    let config = STTConfig {
        provider: "microsoft-azure".to_string(),
        api_key: String::new(), // Empty API key
        language: "en-US".to_string(),
        sample_rate: 16000,
        ..Default::default()
    };

    let result = create_stt_provider("microsoft-azure", config);
    assert!(result.is_err());

    match result {
        Err(STTError::AuthenticationFailed(msg)) => {
            assert!(msg.contains("subscription key"));
        }
        _ => panic!("Expected AuthenticationFailed error"),
    }
}

// =============================================================================
// Region Tests
// =============================================================================

/// Test Azure region hostname generation
#[test]
fn test_azure_region_hostnames() {
    let test_cases = vec![
        (AzureRegion::EastUS, "eastus.stt.speech.microsoft.com"),
        (
            AzureRegion::WestEurope,
            "westeurope.stt.speech.microsoft.com",
        ),
        (
            AzureRegion::SoutheastAsia,
            "southeastasia.stt.speech.microsoft.com",
        ),
        (AzureRegion::JapanEast, "japaneast.stt.speech.microsoft.com"),
        (
            AzureRegion::AustraliaEast,
            "australiaeast.stt.speech.microsoft.com",
        ),
    ];

    for (region, expected_hostname) in test_cases {
        assert_eq!(
            region.stt_hostname(),
            expected_hostname,
            "Region {:?} hostname mismatch",
            region
        );
    }
}

/// Test Azure region WebSocket URL generation
#[test]
fn test_azure_region_websocket_urls() {
    let test_cases = vec![
        (AzureRegion::EastUS, "wss://eastus.stt.speech.microsoft.com"),
        (
            AzureRegion::WestEurope,
            "wss://westeurope.stt.speech.microsoft.com",
        ),
        (
            AzureRegion::JapanWest,
            "wss://japanwest.stt.speech.microsoft.com",
        ),
    ];

    for (region, expected_url) in test_cases {
        assert_eq!(
            region.stt_websocket_base_url(),
            expected_url,
            "Region {:?} WebSocket URL mismatch",
            region
        );
    }
}

/// Test Azure region token endpoint generation
#[test]
fn test_azure_region_token_endpoints() {
    let test_cases = vec![
        (
            AzureRegion::EastUS,
            "https://eastus.api.cognitive.microsoft.com/sts/v1.0/issueToken",
        ),
        (
            AzureRegion::WestEurope,
            "https://westeurope.api.cognitive.microsoft.com/sts/v1.0/issueToken",
        ),
    ];

    for (region, expected_endpoint) in test_cases {
        assert_eq!(
            region.token_endpoint(),
            expected_endpoint,
            "Region {:?} token endpoint mismatch",
            region
        );
    }
}

/// Test custom Azure region
#[test]
fn test_azure_custom_region() {
    let custom_region = AzureRegion::Custom("customregion".to_string());
    assert_eq!(custom_region.as_str(), "customregion");
    assert_eq!(
        custom_region.stt_hostname(),
        "customregion.stt.speech.microsoft.com"
    );
    assert_eq!(
        custom_region.stt_websocket_base_url(),
        "wss://customregion.stt.speech.microsoft.com"
    );
}

/// Test Azure region parsing from string
#[test]
fn test_azure_region_from_str() {
    use std::str::FromStr;

    // Known regions
    assert_eq!(
        AzureRegion::from_str("eastus").unwrap(),
        AzureRegion::EastUS
    );
    assert_eq!(
        AzureRegion::from_str("WESTEUROPE").unwrap(),
        AzureRegion::WestEurope
    );
    assert_eq!(
        AzureRegion::from_str("SoutheastAsia").unwrap(),
        AzureRegion::SoutheastAsia
    );

    // Unknown region becomes Custom
    let unknown = AzureRegion::from_str("unknownregion").unwrap();
    assert!(matches!(unknown, AzureRegion::Custom(_)));
    if let AzureRegion::Custom(s) = unknown {
        assert_eq!(s, "unknownregion");
    }
}

/// Test default Azure region
#[test]
fn test_azure_region_default() {
    assert_eq!(AzureRegion::default(), AzureRegion::EastUS);
}

// =============================================================================
// Output Format Tests
// =============================================================================

/// Test Azure output format values
#[test]
fn test_azure_output_format() {
    assert_eq!(AzureOutputFormat::Simple.as_str(), "simple");
    assert_eq!(AzureOutputFormat::Detailed.as_str(), "detailed");
    assert_eq!(AzureOutputFormat::default(), AzureOutputFormat::Detailed);
}

// =============================================================================
// Profanity Option Tests
// =============================================================================

/// Test Azure profanity options
#[test]
fn test_azure_profanity_options() {
    assert_eq!(AzureProfanityOption::Masked.as_str(), "masked");
    assert_eq!(AzureProfanityOption::Removed.as_str(), "removed");
    assert_eq!(AzureProfanityOption::Raw.as_str(), "raw");
    assert_eq!(
        AzureProfanityOption::default(),
        AzureProfanityOption::Masked
    );
}

// =============================================================================
// Configuration Tests
// =============================================================================

/// Test Azure STT config default values
#[test]
fn test_azure_stt_config_defaults() {
    let config = AzureSTTConfig::default();

    assert_eq!(config.region, AzureRegion::EastUS);
    assert_eq!(config.output_format, AzureOutputFormat::Detailed);
    assert_eq!(config.profanity, AzureProfanityOption::Masked);
    assert!(config.interim_results);
    assert!(!config.word_level_timing);
    assert!(config.endpoint_id.is_none());
    assert!(config.auto_detect_languages.is_none());
}

/// Test Azure STT config URL building
#[test]
fn test_azure_stt_config_url_building() {
    let config = AzureSTTConfig {
        base: STTConfig {
            language: "en-US".to_string(),
            ..Default::default()
        },
        region: AzureRegion::WestEurope,
        output_format: AzureOutputFormat::Detailed,
        profanity: AzureProfanityOption::Raw,
        ..Default::default()
    };

    let url = config.build_websocket_url();
    assert!(url.starts_with("wss://westeurope.stt.speech.microsoft.com"));
    assert!(url.contains("language=en-US"));
    assert!(url.contains("format=detailed"));
    assert!(url.contains("profanity=raw"));
}

/// Test Azure STT config URL with custom endpoint
#[test]
fn test_azure_stt_config_url_with_custom_endpoint() {
    let config = AzureSTTConfig {
        base: STTConfig {
            language: "en-US".to_string(),
            ..Default::default()
        },
        endpoint_id: Some("custom-model-abc123".to_string()),
        ..Default::default()
    };

    let url = config.build_websocket_url();
    assert!(url.contains("cid=custom-model-abc123"));
}

/// Test Azure STT config URL with auto-detect languages
#[test]
fn test_azure_stt_config_url_with_auto_detect() {
    let config = AzureSTTConfig {
        base: STTConfig {
            language: "en-US".to_string(),
            ..Default::default()
        },
        auto_detect_languages: Some(vec![
            "en-US".to_string(),
            "es-ES".to_string(),
            "fr-FR".to_string(),
        ]),
        ..Default::default()
    };

    let url = config.build_websocket_url();
    assert!(url.contains("languages=en-US,es-ES,fr-FR"));
}

/// Test Azure STT config from_base constructor
#[test]
fn test_azure_stt_config_from_base() {
    let base = STTConfig {
        api_key: "test-key".to_string(),
        language: "de-DE".to_string(),
        sample_rate: 16000,
        ..Default::default()
    };

    let config = AzureSTTConfig::from_base(base);
    assert_eq!(config.base.api_key, "test-key");
    assert_eq!(config.base.language, "de-DE");
    // Azure-specific defaults
    assert_eq!(config.region, AzureRegion::EastUS);
    assert_eq!(config.output_format, AzureOutputFormat::Detailed);
}

/// Test Azure STT config with_region constructor
#[test]
fn test_azure_stt_config_with_region() {
    let base = STTConfig::default();
    let config = AzureSTTConfig::with_region(base, AzureRegion::JapanEast);
    assert_eq!(config.region, AzureRegion::JapanEast);
}

// =============================================================================
// Client Tests
// =============================================================================

/// Test callback registration (without connection)
#[tokio::test]
async fn test_azure_callback_registration() {
    let config = STTConfig {
        api_key: "test-subscription-key".to_string(),
        ..Default::default()
    };

    let mut stt = AzureSTT::new(config).unwrap();

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
async fn test_azure_send_audio_requires_connection() {
    let config = STTConfig {
        api_key: "test-subscription-key".to_string(),
        ..Default::default()
    };

    let mut stt = AzureSTT::new(config).unwrap();

    let result = stt.send_audio(vec![0u8; 100]).await;
    assert!(result.is_err());

    match result {
        Err(STTError::ConnectionFailed(msg)) => {
            assert!(msg.contains("Not connected"));
        }
        _ => panic!("Expected ConnectionFailed error"),
    }
}

/// Test configuration retrieval
#[test]
fn test_azure_get_config() {
    let config = STTConfig {
        api_key: "test-subscription-key".to_string(),
        language: "ja-JP".to_string(),
        sample_rate: 16000,
        ..Default::default()
    };

    let stt = AzureSTT::new(config).unwrap();
    let retrieved = stt.get_config().unwrap();

    assert_eq!(retrieved.api_key, "test-subscription-key");
    assert_eq!(retrieved.language, "ja-JP");
    assert_eq!(retrieved.sample_rate, 16000);
}

/// Test Azure-specific config retrieval
#[test]
fn test_azure_get_azure_config() {
    let config = STTConfig {
        api_key: "test-subscription-key".to_string(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        ..Default::default()
    };

    let stt = AzureSTT::new(config).unwrap();
    let azure_config = stt.get_azure_config().unwrap();

    // Should have default Azure settings
    assert_eq!(azure_config.region, AzureRegion::EastUS);
    assert!(azure_config.interim_results);
    assert!(!azure_config.word_level_timing);
}

/// Test connection ID is generated
#[test]
fn test_azure_connection_id() {
    let config = STTConfig {
        api_key: "test-subscription-key".to_string(),
        ..Default::default()
    };

    let stt = AzureSTT::new(config).unwrap();

    // Connection ID should be a non-empty base64 string
    let conn_id = stt.get_connection_id();
    assert!(!conn_id.is_empty());
    // Base64 characters
    assert!(
        conn_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    );
}

/// Test provider info
#[test]
fn test_azure_provider_info() {
    let config = STTConfig {
        api_key: "test-subscription-key".to_string(),
        ..Default::default()
    };

    let stt = AzureSTT::new(config).unwrap();
    assert_eq!(stt.get_provider_info(), "Microsoft Azure Speech-to-Text");
}

/// Test is_ready returns false before connection
#[test]
fn test_azure_is_ready_before_connect() {
    let config = STTConfig {
        api_key: "test-subscription-key".to_string(),
        ..Default::default()
    };

    let stt = AzureSTT::new(config).unwrap();
    assert!(!stt.is_ready());
}

/// Test default client has no config
#[test]
fn test_azure_default_client() {
    let stt = AzureSTT::default();
    assert!(!stt.is_ready());
    assert!(stt.get_config().is_none());
    assert!(stt.get_azure_config().is_none());
}

// =============================================================================
// Real Connection Integration Tests (require credentials)
// =============================================================================

/// Helper function to get Azure credentials from environment
fn get_azure_credentials() -> Option<(String, String)> {
    let key = std::env::var("AZURE_SPEECH_SUBSCRIPTION_KEY").ok()?;
    let region = std::env::var("AZURE_SPEECH_REGION").unwrap_or_else(|_| "eastus".to_string());
    Some((key, region))
}

/// Integration test with real Azure API (requires credentials)
#[tokio::test]
#[ignore = "Requires AZURE_SPEECH_SUBSCRIPTION_KEY environment variable"]
async fn test_azure_real_connection() {
    let (api_key, region) = get_azure_credentials()
        .expect("AZURE_SPEECH_SUBSCRIPTION_KEY and optionally AZURE_SPEECH_REGION must be set");

    let config = STTConfig {
        api_key,
        language: "en-US".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
        provider: "microsoft-azure".to_string(),
        ..Default::default()
    };

    let mut stt = AzureSTT::new(config).unwrap();

    // Parse region and update config
    let azure_region: AzureRegion = region.parse().unwrap();
    stt.update_azure_settings(Some(azure_region), None, None, None)
        .await
        .ok(); // May fail without connection, that's ok

    // Register callback
    let received = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let received_clone = received.clone();

    let callback = Arc::new(move |result: STTResult| {
        let received = received_clone.clone();
        Box::pin(async move {
            println!(
                "Received: {} (final: {}, confidence: {})",
                result.transcript, result.is_final, result.confidence
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

    // Connection ID should still be set
    assert!(!stt.get_connection_id().is_empty());
    println!("Connection ID: {}", stt.get_connection_id());

    // Disconnect
    stt.disconnect().await.unwrap();
    assert!(!stt.is_ready());
}

/// Integration test sending silence audio (requires credentials)
#[tokio::test]
#[ignore = "Requires AZURE_SPEECH_SUBSCRIPTION_KEY environment variable"]
async fn test_azure_send_silence_audio() {
    use tokio::time::{Duration, sleep};

    let (api_key, region) = get_azure_credentials()
        .expect("AZURE_SPEECH_SUBSCRIPTION_KEY and optionally AZURE_SPEECH_REGION must be set");

    let mut base_config = STTConfig {
        api_key,
        language: "en-US".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
        encoding: "linear16".to_string(),
        model: "".to_string(),
        provider: "microsoft-azure".to_string(),
        ..Default::default()
    };

    // Parse and set region
    let azure_region: AzureRegion = region.parse().unwrap();
    let azure_config = AzureSTTConfig {
        base: base_config.clone(),
        region: azure_region,
        ..Default::default()
    };
    base_config = azure_config.base.clone();

    let mut stt = AzureSTT::new(base_config).unwrap();

    // Track results
    let results = Arc::new(std::sync::Mutex::new(Vec::new()));
    let results_clone = results.clone();

    let callback = Arc::new(move |result: STTResult| {
        let results = results_clone.clone();
        Box::pin(async move {
            println!(
                "Result: '{}' (final: {}, confidence: {})",
                result.transcript, result.is_final, result.confidence
            );
            results.lock().unwrap().push(result);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });

    stt.on_result(callback).await.unwrap();

    // Connect
    stt.connect().await.expect("Connection should succeed");

    // Send some silent audio (zeros = silence)
    // 16kHz * 2 bytes per sample * 0.1 seconds = 3200 bytes per 100ms chunk
    for _ in 0..10 {
        let audio_chunk = vec![0u8; 3200];
        stt.send_audio(audio_chunk).await.unwrap();
        sleep(Duration::from_millis(100)).await;
    }

    // Wait for processing
    sleep(Duration::from_secs(2)).await;

    // Disconnect
    stt.disconnect().await.unwrap();

    let final_results = results.lock().unwrap();
    println!(
        "Received {} results (may be 0 for silence)",
        final_results.len()
    );
    // Note: Silence may not produce transcription results, which is expected
}

/// Test connection error with invalid credentials
#[tokio::test]
#[ignore = "Tests authentication error handling with invalid key"]
async fn test_azure_invalid_credentials_error() {
    let config = STTConfig {
        api_key: "invalid-subscription-key-12345".to_string(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        provider: "microsoft-azure".to_string(),
        ..Default::default()
    };

    let mut stt = AzureSTT::new(config).unwrap();

    let result = stt.connect().await;
    assert!(result.is_err());

    // Should be an authentication or connection error
    match result {
        Err(STTError::AuthenticationFailed(_)) | Err(STTError::ConnectionFailed(_)) => {
            // Expected
        }
        Err(e) => panic!("Unexpected error type: {:?}", e),
        Ok(_) => panic!("Expected connection to fail with invalid credentials"),
    }
}
