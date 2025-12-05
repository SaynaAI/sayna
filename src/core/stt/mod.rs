pub mod azure;
mod base;
pub mod cartesia;
pub mod deepgram;
pub mod elevenlabs;
pub mod google;

#[cfg(feature = "whisper-stt")]
pub mod whisper;

// Re-export public types and traits
pub use base::{
    BaseSTT, STTConfig, STTConnectionState, STTError, STTErrorCallback, STTFactory, STTHelper,
    STTResult, STTResultCallback, STTStats,
};

// Re-export Deepgram implementation
pub use deepgram::{DeepgramSTT, DeepgramSTTConfig};

// Re-export ElevenLabs implementation
pub use elevenlabs::{
    CommitStrategy, ElevenLabsAudioFormat, ElevenLabsMessage, ElevenLabsRegion, ElevenLabsSTT,
    ElevenLabsSTTConfig,
};

// Re-export Google implementation
pub use google::{GoogleSTT, GoogleSTTConfig, STTGoogleAuthClient};

// Re-export Azure implementation
pub use azure::{AzureOutputFormat, AzureProfanityOption, AzureRegion, AzureSTT, AzureSTTConfig};

// Re-export Cartesia implementation
pub use cartesia::{CartesiaAudioEncoding, CartesiaMessage, CartesiaSTT, CartesiaSTTConfig};

// Re-export Whisper implementation (feature-gated)
#[cfg(feature = "whisper-stt")]
pub use whisper::{WhisperAssetConfig, WhisperSTT, WhisperSTTConfig};

/// Supported STT providers
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum STTProvider {
    /// Deepgram STT WebSocket API
    Deepgram,
    /// Google Speech-to-Text v2 API
    Google,
    /// ElevenLabs STT Real-Time WebSocket API
    ElevenLabs,
    /// Microsoft Azure Speech-to-Text WebSocket API
    Azure,
    /// Cartesia STT WebSocket API (ink-whisper)
    Cartesia,
    /// Whisper ONNX local model
    #[cfg(feature = "whisper-stt")]
    Whisper,
}

impl std::fmt::Display for STTProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            STTProvider::Deepgram => write!(f, "deepgram"),
            STTProvider::Google => write!(f, "google"),
            STTProvider::ElevenLabs => write!(f, "elevenlabs"),
            STTProvider::Azure => write!(f, "microsoft-azure"),
            STTProvider::Cartesia => write!(f, "cartesia"),
            #[cfg(feature = "whisper-stt")]
            STTProvider::Whisper => write!(f, "whisper"),
        }
    }
}

impl std::str::FromStr for STTProvider {
    type Err = STTError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "deepgram" => Ok(STTProvider::Deepgram),
            "google" => Ok(STTProvider::Google),
            "elevenlabs" => Ok(STTProvider::ElevenLabs),
            "microsoft-azure" | "azure" => Ok(STTProvider::Azure),
            "cartesia" => Ok(STTProvider::Cartesia),
            #[cfg(feature = "whisper-stt")]
            "whisper" | "whisper-onnx" => Ok(STTProvider::Whisper),
            _ => Err(STTError::ConfigurationError(format!(
                "Unsupported STT provider: {s}. Supported providers: {}",
                get_supported_stt_providers().join(", ")
            ))),
        }
    }
}

/// Factory function to create STT providers by name
///
/// # Arguments
/// * `provider` - The name of the STT provider (e.g., "deepgram")
/// * `config` - Configuration for the STT provider
///
/// # Returns
/// * `Result<Box<dyn BaseSTT>, STTError>` - A boxed STT provider or error
///
/// # Examples
/// ```rust,no_run
/// use sayna::core::stt::{create_stt_provider, STTConfig};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = STTConfig {
///         provider: "deepgram".to_string(),
///         api_key: "your-deepgram-api-key".to_string(),
///         language: "en-US".to_string(),
///         sample_rate: 16000,
///         channels: 1,
///         punctuation: true,
///         encoding: "linear16".to_string(),
///         model: "nova-3".to_string(),
///         ..Default::default()
///     };
///
///     // Create a Deepgram STT provider
///     let mut stt = create_stt_provider("deepgram", config)?;
///
///     // Use the provider
///     if stt.is_ready() {
///         let audio_data = vec![0u8; 1024];
///         stt.send_audio(audio_data).await?;
///     }
///
///     Ok(())
/// }
/// ```
pub fn create_stt_provider(
    provider: &str,
    config: STTConfig,
) -> Result<Box<dyn BaseSTT>, STTError> {
    let provider_enum: STTProvider = provider.parse()?;

    match provider_enum {
        STTProvider::Deepgram => {
            let deepgram_stt = <DeepgramSTT as BaseSTT>::new(config)?;
            Ok(Box::new(deepgram_stt))
        }
        STTProvider::Google => {
            let google_stt = <GoogleSTT as BaseSTT>::new(config)?;
            Ok(Box::new(google_stt))
        }
        STTProvider::ElevenLabs => {
            let elevenlabs_stt = <ElevenLabsSTT as BaseSTT>::new(config)?;
            Ok(Box::new(elevenlabs_stt))
        }
        STTProvider::Azure => {
            let azure_stt = <AzureSTT as BaseSTT>::new(config)?;
            Ok(Box::new(azure_stt))
        }
        STTProvider::Cartesia => {
            let cartesia_stt = <CartesiaSTT as BaseSTT>::new(config)?;
            Ok(Box::new(cartesia_stt))
        }
        #[cfg(feature = "whisper-stt")]
        STTProvider::Whisper => {
            let whisper_stt = <WhisperSTT as BaseSTT>::new(config)?;
            Ok(Box::new(whisper_stt))
        }
    }
}

/// Factory function to create STT providers using the enum directly
///
/// # Arguments
/// * `provider` - The STT provider enum
/// * `config` - Configuration for the STT provider
///
/// # Returns
/// * `Result<Box<dyn BaseSTT>, STTError>` - A boxed STT provider or error
///
/// # Examples
/// ```rust,no_run
/// use sayna::core::stt::{create_stt_provider_from_enum, STTProvider, STTConfig};
///
/// fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = STTConfig {
///         provider: "deepgram".to_string(),
///         api_key: "your-deepgram-api-key".to_string(),
///         language: "en-US".to_string(),
///         sample_rate: 16000,
///         channels: 1,
///         punctuation: true,
///         encoding: "linear16".to_string(),
///         model: "nova-3".to_string(),
///         ..Default::default()
///     };
///
///     // Create a Deepgram STT provider using enum
///     let mut stt = create_stt_provider_from_enum(STTProvider::Deepgram, config)?;
///
///     Ok(())
/// }
/// ```
pub fn create_stt_provider_from_enum(
    provider: STTProvider,
    config: STTConfig,
) -> Result<Box<dyn BaseSTT>, STTError> {
    match provider {
        STTProvider::Deepgram => {
            let deepgram_stt = <DeepgramSTT as BaseSTT>::new(config)?;
            Ok(Box::new(deepgram_stt))
        }
        STTProvider::Google => {
            let google_stt = <GoogleSTT as BaseSTT>::new(config)?;
            Ok(Box::new(google_stt))
        }
        STTProvider::ElevenLabs => {
            let elevenlabs_stt = <ElevenLabsSTT as BaseSTT>::new(config)?;
            Ok(Box::new(elevenlabs_stt))
        }
        STTProvider::Azure => {
            let azure_stt = <AzureSTT as BaseSTT>::new(config)?;
            Ok(Box::new(azure_stt))
        }
        STTProvider::Cartesia => {
            let cartesia_stt = <CartesiaSTT as BaseSTT>::new(config)?;
            Ok(Box::new(cartesia_stt))
        }
        #[cfg(feature = "whisper-stt")]
        STTProvider::Whisper => {
            let whisper_stt = <WhisperSTT as BaseSTT>::new(config)?;
            Ok(Box::new(whisper_stt))
        }
    }
}

/// Get a list of all supported STT providers
///
/// # Returns
/// * `Vec<&'static str>` - List of supported provider names
///
/// # Examples
/// ```rust
/// use sayna::core::stt::get_supported_stt_providers;
///
/// let providers = get_supported_stt_providers();
/// println!("Supported STT providers: {:?}", providers);
/// // Output: ["deepgram", "google", "elevenlabs"]
/// ```
pub fn get_supported_stt_providers() -> Vec<&'static str> {
    #[allow(unused_mut)]
    let mut providers = vec![
        "deepgram",
        "google",
        "elevenlabs",
        "microsoft-azure",
        "cartesia",
    ];

    #[cfg(feature = "whisper-stt")]
    providers.push("whisper");

    providers
}

#[cfg(test)]
mod factory_tests {
    use super::*;

    #[test]
    fn test_stt_provider_enum_from_string() {
        // Test valid provider names - Deepgram
        assert_eq!(
            "deepgram".parse::<STTProvider>().unwrap(),
            STTProvider::Deepgram
        );
        assert_eq!(
            "Deepgram".parse::<STTProvider>().unwrap(),
            STTProvider::Deepgram
        );
        assert_eq!(
            "DEEPGRAM".parse::<STTProvider>().unwrap(),
            STTProvider::Deepgram
        );

        // Test valid provider names - Google
        assert_eq!(
            "google".parse::<STTProvider>().unwrap(),
            STTProvider::Google
        );
        assert_eq!(
            "Google".parse::<STTProvider>().unwrap(),
            STTProvider::Google
        );
        assert_eq!(
            "GOOGLE".parse::<STTProvider>().unwrap(),
            STTProvider::Google
        );

        // Test valid provider names - ElevenLabs
        assert_eq!(
            "elevenlabs".parse::<STTProvider>().unwrap(),
            STTProvider::ElevenLabs
        );
        assert_eq!(
            "ElevenLabs".parse::<STTProvider>().unwrap(),
            STTProvider::ElevenLabs
        );
        assert_eq!(
            "ELEVENLABS".parse::<STTProvider>().unwrap(),
            STTProvider::ElevenLabs
        );

        // Test valid provider names - Azure (both canonical and shorthand)
        assert_eq!(
            "microsoft-azure".parse::<STTProvider>().unwrap(),
            STTProvider::Azure
        );
        assert_eq!(
            "Microsoft-Azure".parse::<STTProvider>().unwrap(),
            STTProvider::Azure
        );
        assert_eq!(
            "MICROSOFT-AZURE".parse::<STTProvider>().unwrap(),
            STTProvider::Azure
        );
        assert_eq!("azure".parse::<STTProvider>().unwrap(), STTProvider::Azure);
        assert_eq!("Azure".parse::<STTProvider>().unwrap(), STTProvider::Azure);
        assert_eq!("AZURE".parse::<STTProvider>().unwrap(), STTProvider::Azure);

        // Test invalid provider name
        let result = "invalid".parse::<STTProvider>();
        assert!(result.is_err());
        if let Err(STTError::ConfigurationError(msg)) = result {
            assert!(msg.contains("Unsupported STT provider: invalid"));
        }
    }

    #[test]
    fn test_stt_provider_enum_display() {
        assert_eq!(STTProvider::Deepgram.to_string(), "deepgram");
        assert_eq!(STTProvider::Google.to_string(), "google");
        assert_eq!(STTProvider::ElevenLabs.to_string(), "elevenlabs");
        assert_eq!(STTProvider::Azure.to_string(), "microsoft-azure");
        assert_eq!(STTProvider::Cartesia.to_string(), "cartesia");
    }

    #[test]
    fn test_get_supported_stt_providers() {
        let providers = get_supported_stt_providers();

        // Core providers should always be present
        assert!(providers.contains(&"deepgram"));
        assert!(providers.contains(&"google"));
        assert!(providers.contains(&"elevenlabs"));
        assert!(providers.contains(&"microsoft-azure"));
        assert!(providers.contains(&"cartesia"));

        // Check expected count based on feature flags
        #[cfg(not(feature = "whisper-stt"))]
        assert_eq!(providers.len(), 5);

        #[cfg(feature = "whisper-stt")]
        {
            assert!(providers.contains(&"whisper"));
            assert_eq!(providers.len(), 6);
        }
    }

    #[tokio::test]
    async fn test_create_stt_provider_with_invalid_config() {
        let config = STTConfig {
            model: "nova-3".to_string(),
            provider: "deepgram".to_string(),
            api_key: String::new(), // Empty API key should fail
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider("deepgram", config);
        assert!(result.is_err());
        if let Err(STTError::AuthenticationFailed(msg)) = result {
            assert!(msg.contains("API key is required"));
        }
    }

    #[test]
    fn test_create_stt_provider_from_enum() {
        let config = STTConfig {
            model: "nova-3".to_string(),
            provider: "deepgram".to_string(),
            api_key: String::new(), // Empty API key should fail
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider_from_enum(STTProvider::Deepgram, config);
        assert!(result.is_err());
        // Should fail because of empty API key
    }

    #[test]
    fn test_create_stt_provider_elevenlabs_valid() {
        let config = STTConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test_key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider("elevenlabs", config);
        assert!(result.is_ok());

        let stt = result.unwrap();
        assert_eq!(
            stt.get_provider_info(),
            "ElevenLabs STT Real-Time WebSocket"
        );
    }

    #[test]
    fn test_create_stt_provider_elevenlabs_empty_api_key() {
        let config = STTConfig {
            provider: "elevenlabs".to_string(),
            api_key: String::new(), // Empty API key should fail
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider("elevenlabs", config);
        assert!(result.is_err());

        if let Err(STTError::AuthenticationFailed(msg)) = result {
            assert!(msg.contains("API key is required"));
        } else {
            panic!("Expected AuthenticationFailed error");
        }
    }

    #[test]
    fn test_create_stt_provider_from_enum_elevenlabs() {
        let config = STTConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test_key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider_from_enum(STTProvider::ElevenLabs, config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_error_message_includes_elevenlabs() {
        let result = "invalid".parse::<STTProvider>();
        assert!(result.is_err());
        if let Err(STTError::ConfigurationError(msg)) = result {
            assert!(msg.contains("elevenlabs"));
        }
    }

    #[test]
    fn test_create_stt_provider_azure_valid() {
        let config = STTConfig {
            provider: "microsoft-azure".to_string(),
            api_key: "test_subscription_key".to_string(),
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
        assert!(!stt.is_ready()); // Not connected yet
    }

    #[test]
    fn test_create_stt_provider_azure_shorthand() {
        let config = STTConfig {
            provider: "azure".to_string(),
            api_key: "test_subscription_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "".to_string(),
            ..Default::default()
        };

        // Test that "azure" shorthand also works
        let result = create_stt_provider("azure", config);
        assert!(result.is_ok());

        let stt = result.unwrap();
        assert_eq!(stt.get_provider_info(), "Microsoft Azure Speech-to-Text");
    }

    #[test]
    fn test_create_stt_provider_azure_empty_api_key() {
        let config = STTConfig {
            provider: "microsoft-azure".to_string(),
            api_key: String::new(), // Empty API key should fail
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider("microsoft-azure", config);
        assert!(result.is_err());

        if let Err(STTError::AuthenticationFailed(msg)) = result {
            assert!(msg.contains("subscription key"));
        } else {
            panic!("Expected AuthenticationFailed error");
        }
    }

    #[test]
    fn test_create_stt_provider_from_enum_azure() {
        let config = STTConfig {
            provider: "microsoft-azure".to_string(),
            api_key: "test_subscription_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider_from_enum(STTProvider::Azure, config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_error_message_includes_microsoft_azure() {
        let result = "invalid".parse::<STTProvider>();
        assert!(result.is_err());
        if let Err(STTError::ConfigurationError(msg)) = result {
            assert!(msg.contains("microsoft-azure"));
        }
    }

    // Cartesia STT provider tests

    #[test]
    fn test_stt_provider_enum_cartesia_from_string() {
        // Test valid provider names - Cartesia
        assert_eq!(
            "cartesia".parse::<STTProvider>().unwrap(),
            STTProvider::Cartesia
        );
        assert_eq!(
            "Cartesia".parse::<STTProvider>().unwrap(),
            STTProvider::Cartesia
        );
        assert_eq!(
            "CARTESIA".parse::<STTProvider>().unwrap(),
            STTProvider::Cartesia
        );
    }

    #[test]
    fn test_create_stt_provider_cartesia_valid() {
        let config = STTConfig {
            provider: "cartesia".to_string(),
            api_key: "test_key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "pcm_s16le".to_string(),
            model: "ink-whisper".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider("cartesia", config);
        assert!(result.is_ok());

        let stt = result.unwrap();
        assert_eq!(stt.get_provider_info(), "Cartesia STT (ink-whisper)");
        assert!(!stt.is_ready()); // Not connected yet
    }

    #[test]
    fn test_create_stt_provider_cartesia_empty_api_key() {
        let config = STTConfig {
            provider: "cartesia".to_string(),
            api_key: String::new(), // Empty API key should fail
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "pcm_s16le".to_string(),
            model: "ink-whisper".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider("cartesia", config);
        assert!(result.is_err());

        if let Err(STTError::AuthenticationFailed(msg)) = result {
            assert!(msg.contains("API key is required"));
        } else {
            panic!("Expected AuthenticationFailed error");
        }
    }

    #[test]
    fn test_create_stt_provider_from_enum_cartesia() {
        let config = STTConfig {
            provider: "cartesia".to_string(),
            api_key: "test_key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "pcm_s16le".to_string(),
            model: "ink-whisper".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider_from_enum(STTProvider::Cartesia, config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_error_message_includes_cartesia() {
        let result = "invalid".parse::<STTProvider>();
        assert!(result.is_err());
        if let Err(STTError::ConfigurationError(msg)) = result {
            assert!(msg.contains("cartesia"));
        }
    }

    // Whisper STT provider tests (feature-gated)

    #[cfg(feature = "whisper-stt")]
    #[test]
    fn test_stt_provider_enum_whisper_from_string() {
        // Test valid provider names - Whisper
        assert_eq!(
            "whisper".parse::<STTProvider>().unwrap(),
            STTProvider::Whisper
        );
        assert_eq!(
            "Whisper".parse::<STTProvider>().unwrap(),
            STTProvider::Whisper
        );
        assert_eq!(
            "WHISPER".parse::<STTProvider>().unwrap(),
            STTProvider::Whisper
        );
        // Test alternative name
        assert_eq!(
            "whisper-onnx".parse::<STTProvider>().unwrap(),
            STTProvider::Whisper
        );
        assert_eq!(
            "Whisper-ONNX".parse::<STTProvider>().unwrap(),
            STTProvider::Whisper
        );
    }

    #[cfg(feature = "whisper-stt")]
    #[test]
    fn test_stt_provider_enum_whisper_display() {
        assert_eq!(STTProvider::Whisper.to_string(), "whisper");
    }

    #[cfg(feature = "whisper-stt")]
    #[test]
    fn test_get_supported_providers_includes_whisper() {
        let providers = get_supported_stt_providers();
        assert!(providers.contains(&"whisper"));
    }

    #[cfg(feature = "whisper-stt")]
    #[test]
    fn test_create_stt_provider_whisper_valid() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            api_key: String::new(), // Not required for Whisper
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "whisper-base".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider("whisper", config);
        assert!(result.is_ok());

        let stt = result.unwrap();
        assert_eq!(stt.get_provider_info(), "Whisper ONNX (Local)");
        assert!(!stt.is_ready()); // Not connected yet
    }

    #[cfg(feature = "whisper-stt")]
    #[test]
    fn test_create_stt_provider_from_enum_whisper() {
        let config = STTConfig {
            provider: "whisper".to_string(),
            api_key: String::new(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "whisper-base".to_string(),
            ..Default::default()
        };

        let result = create_stt_provider_from_enum(STTProvider::Whisper, config);
        assert!(result.is_ok());
    }

    #[cfg(feature = "whisper-stt")]
    #[test]
    fn test_error_message_includes_whisper() {
        let result = "invalid".parse::<STTProvider>();
        assert!(result.is_err());
        if let Err(STTError::ConfigurationError(msg)) = result {
            assert!(msg.contains("whisper"));
        }
    }
}

/// Example usage of the STT trait abstraction
///
/// This demonstrates how to create a custom STT provider implementation
/// and use it with the unified interface.
///
/// ```rust,no_run
/// use sayna::core::stt::{BaseSTT, STTConfig, STTResult, STTResultCallback, create_stt_provider};
/// use std::sync::Arc;
/// use std::pin::Pin;
/// use std::future::Future;
///
/// // Usage example:
/// async fn example_usage() {
///     // Configure the provider
///     let config = STTConfig {
///         model: "nova-3".to_string(),
///         provider: "deepgram".to_string(),
///         api_key: "your-api-key".to_string(),
///         language: "en-US".to_string(),
///         sample_rate: 16000,
///         channels: 1,
///         punctuation: true,
///         encoding: "linear16".to_string(),
///         ..Default::default()
///     };
///     
///     // Create provider using factory function
///     let mut stt_provider = create_stt_provider("deepgram", config).unwrap();
///     
///     // Register a callback for results
///     let callback = Arc::new(|result: STTResult| {
///         Box::pin(async move {
///             println!("Transcription: {}", result.transcript);
///             println!("Final: {}, Confidence: {:.2}", result.is_final, result.confidence);
///         }) as Pin<Box<dyn Future<Output = ()> + Send>>
///     });
///     
///     stt_provider.on_result(callback).await.unwrap();
///     
///     // Send audio data
///     let audio_data = vec![0u8; 1024]; // Your audio bytes here
///     stt_provider.send_audio(audio_data).await.unwrap();
///     
///     // Disconnect when done
///     stt_provider.disconnect().await.unwrap();
/// }
/// ```
#[cfg(doc)]
pub mod example {
    use super::*;

    /// Example implementation showing how to create a custom STT provider
    pub struct ExampleSTTProvider {
        // Implementation details would go here
    }

    /// Factory function to create STT providers
    pub fn create_stt_provider() -> Box<dyn BaseSTT> {
        // This would return an actual provider implementation
        // Use the new API pattern with trait method and config:
        // let config = STTConfig { ... };
        // let stt = <DeepgramSTT as BaseSTT>::new(config).await.unwrap();
        // Box::new(stt)
        todo!("Implement actual STT provider")
    }
}
