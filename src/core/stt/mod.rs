mod base;
pub mod deepgram;

// Re-export public types and traits
pub use base::{
    BaseSTT, STTConfig, STTConnectionState, STTError, STTFactory, STTHelper, STTResult,
    STTResultCallback, STTStats,
};

// Re-export Deepgram implementation
pub use deepgram::{DeepgramSTT, DeepgramSTTConfig};

/// Supported STT providers
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum STTProvider {
    /// Deepgram STT WebSocket API
    Deepgram,
}

impl std::fmt::Display for STTProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            STTProvider::Deepgram => write!(f, "deepgram"),
        }
    }
}

impl std::str::FromStr for STTProvider {
    type Err = STTError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "deepgram" => Ok(STTProvider::Deepgram),
            _ => Err(STTError::ConfigurationError(format!(
                "Unsupported STT provider: {s}. Supported providers: deepgram"
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
    let provider_enum: STTProvider = provider.parse().expect("Invalid STT provider type");

    match provider_enum {
        STTProvider::Deepgram => {
            let deepgram_stt = <DeepgramSTT as BaseSTT>::new(config)?;
            Ok(Box::new(deepgram_stt))
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
///     };
///
///     // Create a Deepgram STT provider using enum
///     let mut stt = create_stt_provider_from_enum(STTProvider::Deepgram, config).await?;
///
///     Ok(())
/// }
/// ```
pub async fn create_stt_provider_from_enum(
    provider: STTProvider,
    config: STTConfig,
) -> Result<Box<dyn BaseSTT>, STTError> {
    match provider {
        STTProvider::Deepgram => {
            let deepgram_stt = <DeepgramSTT as BaseSTT>::new(config)?;
            Ok(Box::new(deepgram_stt))
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
/// // Output: ["deepgram"]
/// ```
pub fn get_supported_stt_providers() -> Vec<&'static str> {
    vec!["deepgram"]
}

#[cfg(test)]
mod factory_tests {
    use super::*;

    #[test]
    fn test_stt_provider_enum_from_string() {
        // Test valid provider names
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
    }

    #[test]
    fn test_get_supported_stt_providers() {
        let providers = get_supported_stt_providers();
        assert_eq!(providers, vec!["deepgram"]);
        assert!(providers.contains(&"deepgram"));
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
        };

        let result = create_stt_provider("deepgram", config);
        assert!(result.is_err());
        if let Err(STTError::AuthenticationFailed(msg)) = result {
            assert!(msg.contains("API key is required"));
        }
    }

    #[tokio::test]
    async fn test_create_stt_provider_from_enum() {
        let config = STTConfig {
            model: "nova-3".to_string(),
            provider: "deepgram".to_string(),
            api_key: String::new(), // Empty API key should fail
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
        };

        let result = create_stt_provider_from_enum(STTProvider::Deepgram, config).await;
        assert!(result.is_err());
        // Should fail because of empty API key
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
