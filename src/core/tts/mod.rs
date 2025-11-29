pub mod azure;
mod base;
pub mod cartesia;
pub mod deepgram;
pub mod elevenlabs;
pub mod google;
pub mod provider;

pub use azure::{AZURE_TTS_URL, AzureAudioEncoding, AzureTTS, AzureTTSConfig};
pub use base::{
    AudioCallback, AudioData, BaseTTS, BoxedTTS, ConnectionState, Pronunciation, TTSConfig,
    TTSError, TTSFactory, TTSResult,
};
pub use cartesia::{CARTESIA_TTS_URL, CartesiaTTS};
pub use deepgram::{DEEPGRAM_TTS_URL, DeepgramTTS};
pub use elevenlabs::{ELEVENLABS_TTS_URL, ElevenLabsTTS};
pub use google::{GOOGLE_TTS_URL, GoogleTTS};
pub use provider::{TTSProvider, TTSRequestBuilder};
use std::collections::HashMap;

/// Factory function to create a TTS provider.
///
/// # Supported Providers
///
/// - `"deepgram"` - Deepgram TTS API
/// - `"elevenlabs"` - ElevenLabs TTS API
/// - `"google"` - Google Cloud Text-to-Speech API
/// - `"azure"` or `"microsoft-azure"` - Microsoft Azure Text-to-Speech API
/// - `"cartesia"` - Cartesia TTS API (Sonic voice models)
///
/// # Example
///
/// ```rust,ignore
/// use sayna::core::tts::{create_tts_provider, TTSConfig};
///
/// let config = TTSConfig {
///     api_key: "your-api-key".to_string(),
///     voice_id: Some("en-US-JennyNeural".to_string()),
///     ..Default::default()
/// };
///
/// let provider = create_tts_provider("azure", config)?;
/// ```
pub fn create_tts_provider(provider_type: &str, config: TTSConfig) -> TTSResult<Box<dyn BaseTTS>> {
    match provider_type.to_lowercase().as_str() {
        "deepgram" => Ok(Box::new(DeepgramTTS::new(config)?)),
        "elevenlabs" => Ok(Box::new(ElevenLabsTTS::new(config)?)),
        "google" => Ok(Box::new(GoogleTTS::new(config)?)),
        "azure" | "microsoft-azure" => Ok(Box::new(AzureTTS::new(config)?)),
        "cartesia" => Ok(Box::new(CartesiaTTS::new(config)?)),
        _ => Err(TTSError::InvalidConfiguration(format!(
            "Unsupported TTS provider: {provider_type}. Supported providers: deepgram, elevenlabs, google, azure, cartesia"
        ))),
    }
}

/// Returns a map of provider names to their default API endpoint URLs.
///
/// Note: Azure uses regional endpoints. The URL returned here is for the
/// default region (eastus). For specific regions, use `AzureRegion::tts_rest_url()`.
pub fn get_tts_provider_urls() -> HashMap<String, String> {
    let mut urls = HashMap::new();
    urls.insert("deepgram".to_string(), DEEPGRAM_TTS_URL.to_string());
    urls.insert("elevenlabs".to_string(), ELEVENLABS_TTS_URL.to_string());
    urls.insert("google".to_string(), GOOGLE_TTS_URL.to_string());
    urls.insert("azure".to_string(), AZURE_TTS_URL.to_string());
    urls.insert("cartesia".to_string(), CARTESIA_TTS_URL.to_string());
    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_tts_provider() {
        let config = TTSConfig::default();
        let result = create_tts_provider("deepgram", config);
        assert!(result.is_ok());

        let invalid_result = create_tts_provider("invalid", TTSConfig::default());
        assert!(invalid_result.is_err());
    }

    #[tokio::test]
    async fn test_create_elevenlabs_tts_provider() {
        let config = TTSConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test_key".to_string(),
            voice_id: Some("test_voice_id".to_string()),
            ..Default::default()
        };
        let result = create_tts_provider("elevenlabs", config);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_azure_tts_provider() {
        let config = TTSConfig {
            provider: "azure".to_string(),
            api_key: "test_subscription_key".to_string(),
            voice_id: Some("en-US-JennyNeural".to_string()),
            ..Default::default()
        };
        let result = create_tts_provider("azure", config);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_azure_tts_provider_alias() {
        let config = TTSConfig {
            provider: "microsoft-azure".to_string(),
            api_key: "test_subscription_key".to_string(),
            voice_id: Some("en-US-JennyNeural".to_string()),
            ..Default::default()
        };
        // Both "azure" and "microsoft-azure" should work
        let result = create_tts_provider("microsoft-azure", config);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_azure_tts_provider_case_insensitive() {
        let config = TTSConfig {
            provider: "azure".to_string(),
            api_key: "test_subscription_key".to_string(),
            voice_id: Some("en-US-JennyNeural".to_string()),
            ..Default::default()
        };
        // Case should not matter
        let result = create_tts_provider("AZURE", config.clone());
        assert!(result.is_ok());

        let result = create_tts_provider("Azure", config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_tts_provider_urls_includes_azure() {
        let urls = get_tts_provider_urls();
        assert!(urls.contains_key("azure"));
        assert_eq!(urls.get("azure").unwrap(), AZURE_TTS_URL);
    }

    #[test]
    fn test_invalid_provider_error_message_includes_azure() {
        let config = TTSConfig::default();
        let result = create_tts_provider("invalid_provider", config);

        match result {
            Err(TTSError::InvalidConfiguration(msg)) => {
                assert!(
                    msg.contains("azure"),
                    "Error message should mention azure as a supported provider"
                );
            }
            Err(other) => panic!("Expected InvalidConfiguration error, got: {:?}", other),
            Ok(_) => panic!("Expected error for invalid provider"),
        }
    }
}
