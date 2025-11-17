use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::base::{AudioCallback, BaseTTS, ConnectionState, TTSConfig, TTSResult};
use super::provider::{TTSProvider, TTSRequestBuilder};
use crate::utils::req_manager::ReqManager;

/// Voice settings for ElevenLabs TTS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSettings {
    /// Voice stability (0.0 to 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stability: Option<f32>,
    /// Similarity boost (0.0 to 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity_boost: Option<f32>,
    /// Style strength (0.0 to 1.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<f32>,
    /// Use speaker boost
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_speaker_boost: Option<bool>,
    /// Speaking rate (0.25 to 4.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            stability: Some(0.5),
            similarity_boost: Some(0.8),
            style: Some(0.0),
            use_speaker_boost: Some(false),
            speed: Some(1.0),
        }
    }
}

pub const ELEVENLABS_TTS_URL: &str = "https://api.elevenlabs.io/v1/text-to-speech";

/// ElevenLabs-specific request builder
#[derive(Clone)]
struct ElevenLabsRequestBuilder {
    config: TTSConfig,
    voice_settings: VoiceSettings,
}

impl TTSRequestBuilder for ElevenLabsRequestBuilder {
    /// Build the ElevenLabs-specific HTTP request with URL, headers and body
    fn build_http_request(&self, client: &reqwest::Client, text: &str) -> reqwest::RequestBuilder {
        // Forward to the context method without previous_text
        self.build_http_request_with_context(client, text, None)
    }

    /// Build the ElevenLabs-specific HTTP request with context support
    fn build_http_request_with_context(
        &self,
        client: &reqwest::Client,
        text: &str,
        previous_text: Option<&str>,
    ) -> reqwest::RequestBuilder {
        // Get voice_id from config, required for ElevenLabs
        let default_voice = "21m00Tcm4TlvDq8ikWAM".to_string();
        let voice_id = self.config.voice_id.as_ref().unwrap_or(&default_voice);

        // Build the URL with voice_id
        let url = format!("{ELEVENLABS_TTS_URL}/{voice_id}");

        // Build query parameters
        let mut query_params = Vec::new();

        // Add output format based on config
        // ElevenLabs expects format like "pcm_24000", "pcm_16000", "pcm_22050", etc.
        // For linear16/pcm format, we need to specify PCM with the correct sample rate
        let output_format = if let Some(format) = &self.config.audio_format {
            match format.as_str() {
                "linear16" | "pcm" => {
                    // ElevenLabs supports PCM at specific sample rates
                    let sample_rate = self.config.sample_rate.unwrap_or(24000);
                    // Map to supported ElevenLabs PCM formats
                    match sample_rate {
                        16000 => "pcm_16000".to_string(),
                        22050 => "pcm_22050".to_string(),
                        24000 => "pcm_24000".to_string(),
                        44100 => "pcm_44100".to_string(),
                        _ => "pcm_24000".to_string(), // Default to 24kHz
                    }
                }
                "mp3" => {
                    let sample_rate = self.config.sample_rate.unwrap_or(44100);
                    match sample_rate {
                        22050 => "mp3_22050_32".to_string(),
                        44100 => "mp3_44100_128".to_string(),
                        _ => "mp3_44100_128".to_string(),
                    }
                }
                "ulaw" => "ulaw_8000".to_string(),
                _ => {
                    // Default to PCM for compatibility with the rest of the system
                    let sample_rate = self.config.sample_rate.unwrap_or(24000);
                    format!("pcm_{sample_rate}")
                }
            }
        } else {
            // Default to PCM 24kHz for consistency with the rest of the system
            "pcm_24000".to_string()
        };

        query_params.push(format!("output_format={output_format}"));

        // Build the final URL with query parameters
        let final_url = if !query_params.is_empty() {
            format!("{}?{}", url, query_params.join("&"))
        } else {
            url
        };

        // Build request body
        let mut body = json!({
            "text": text,
            "voice_settings": self.voice_settings,
        });

        // Add previous_text for context continuity if available
        if let Some(prev) = previous_text {
            body["previous_text"] = json!(prev);
        }

        // Add model_id if specified
        if !self.config.model.is_empty() {
            body["model_id"] = json!(self.config.model);
        } else {
            body["model_id"] = json!("eleven_v3");
        }

        // Build the request with ElevenLabs-specific headers
        // Set Accept header based on the format
        let accept_header = if output_format.starts_with("pcm") {
            "audio/pcm"
        } else if output_format.starts_with("mp3") {
            "audio/mpeg"
        } else if output_format.starts_with("ulaw") {
            "audio/basic"
        } else {
            "audio/pcm"
        };

        client
            .post(final_url)
            .header("xi-api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", accept_header)
            .json(&body)
    }

    /// Get the configuration
    fn get_config(&self) -> &TTSConfig {
        &self.config
    }
}

/// ElevenLabs TTS provider implementation using the ElevenLabs HTTP REST API
pub struct ElevenLabsTTS {
    /// Generic HTTP-based TTS provider
    provider: TTSProvider,
    /// Request builder
    request_builder: ElevenLabsRequestBuilder,
}

impl ElevenLabsTTS {
    /// Create a new ElevenLabs TTS instance
    pub fn new(config: TTSConfig) -> TTSResult<Self> {
        // Validate required fields for ElevenLabs
        if config.api_key.is_empty() {
            return Err(super::base::TTSError::InvalidConfiguration(
                "API key is required for ElevenLabs".to_string(),
            ));
        }

        // Create voice settings from config
        let voice_settings = VoiceSettings {
            speed: config.speaking_rate,
            ..Default::default()
        };

        let request_builder = ElevenLabsRequestBuilder {
            config,
            voice_settings,
        };

        Ok(Self {
            provider: TTSProvider::new()?,
            request_builder,
        })
    }

    /// Set the request manager for this instance
    pub async fn set_req_manager(&mut self, req_manager: Arc<ReqManager>) {
        self.provider.set_req_manager(req_manager).await;
    }
}

impl Default for ElevenLabsTTS {
    fn default() -> Self {
        Self::new(TTSConfig::default()).unwrap()
    }
}

#[async_trait]
impl BaseTTS for ElevenLabsTTS {
    fn new(config: TTSConfig) -> TTSResult<Self> {
        ElevenLabsTTS::new(config)
    }

    fn get_provider(&mut self) -> Option<&mut TTSProvider> {
        Some(&mut self.provider)
    }

    async fn connect(&mut self) -> TTSResult<()> {
        // Use the base URL for ElevenLabs API with config-based request manager
        self.provider
            .generic_connect_with_config("https://api.elevenlabs.io", &self.request_builder.config)
            .await
    }

    async fn disconnect(&mut self) -> TTSResult<()> {
        self.provider.generic_disconnect().await
    }

    fn is_ready(&self) -> bool {
        self.provider.is_ready()
    }

    fn get_connection_state(&self) -> ConnectionState {
        self.provider.get_connection_state()
    }

    async fn speak(&mut self, text: &str, flush: bool) -> TTSResult<()> {
        // Handle reconnection if needed
        if !self.is_ready() {
            tracing::info!("ElevenLabs TTS not ready, attempting to connect...");
            self.connect().await?;
        }
        self.provider
            .generic_speak(self.request_builder.clone(), text, flush)
            .await
    }

    async fn clear(&mut self) -> TTSResult<()> {
        self.provider.generic_clear().await
    }

    async fn flush(&self) -> TTSResult<()> {
        self.provider.generic_flush().await
    }

    fn on_audio(&mut self, callback: Arc<dyn AudioCallback>) -> TTSResult<()> {
        self.provider.generic_on_audio(callback)
    }

    fn remove_audio_callback(&mut self) -> TTSResult<()> {
        self.provider.generic_remove_audio_callback()
    }

    fn get_provider_info(&self) -> serde_json::Value {
        serde_json::json!({
            "provider": "elevenlabs",
            "version": "2.0.0",
            "api_type": "HTTP REST",
            "connection_pooling": true,
            "supported_formats": ["mp3", "pcm", "ulaw"],
            "supported_sample_rates": [8000, 11025, 16000, 22050, 24000, 44100, 48000],
            "supported_models": [
                "eleven_multilingual_v2",
                "eleven_multilingual_v1",
                "eleven_monolingual_v1",
                "eleven_turbo_v2",
                "eleven_turbo_v2_5",
                "eleven_flash_v2",
                "eleven_flash_v2_5"
            ],
            "endpoint": "https://api.elevenlabs.io/v1/text-to-speech",
            "documentation": "https://elevenlabs.io/docs/api-reference/text-to-speech",
            "features": {
                "voice_settings": true,
                "pronunciation_dictionaries": true,
                "streaming_optimization": true,
                "text_normalization": true
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_elevenlabs_tts_creation() {
        let config = TTSConfig {
            api_key: "test_key".to_string(),
            voice_id: Some("test_voice_id".to_string()),
            ..Default::default()
        };
        let tts = ElevenLabsTTS::new(config).unwrap();
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_elevenlabs_tts_invalid_config() {
        let config = TTSConfig {
            api_key: "".to_string(), // Missing API key
            ..Default::default()
        };
        let result = ElevenLabsTTS::new(config);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_voice_settings_creation() {
        let config = TTSConfig {
            api_key: "test_key".to_string(),
            speaking_rate: Some(1.5),
            ..Default::default()
        };

        let default_voice_settings = VoiceSettings::default();

        let tts = ElevenLabsTTS::new(config).unwrap();
        assert_eq!(tts.request_builder.voice_settings.speed, Some(1.5));
        assert_eq!(
            tts.request_builder.voice_settings.stability,
            default_voice_settings.stability
        );
        assert_eq!(
            tts.request_builder.voice_settings.similarity_boost,
            default_voice_settings.similarity_boost
        );
    }

    #[tokio::test]
    async fn test_http_request_building() {
        let config = TTSConfig {
            voice_id: Some("test_voice_id".to_string()),
            audio_format: Some("linear16".to_string()),
            sample_rate: Some(24000),
            api_key: "test_key".to_string(),
            model: "eleven_multilingual_v2".to_string(),
            ..Default::default()
        };

        let voice_settings = VoiceSettings::default();
        let builder = ElevenLabsRequestBuilder {
            config,
            voice_settings,
        };
        let client = reqwest::Client::new();
        let request = builder.build_http_request(&client, "Test text");

        // Get the request as built
        let built_request = request.build().unwrap();
        let url = built_request.url().to_string();

        assert!(url.contains("test_voice_id"));
        assert!(url.contains("output_format=pcm_24000"));
        assert!(url.starts_with("https://api.elevenlabs.io/v1/text-to-speech/"));

        // Check headers
        let headers = built_request.headers();
        assert_eq!(headers.get("xi-api-key").unwrap(), "test_key");
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
        assert_eq!(headers.get("accept").unwrap(), "audio/pcm");
    }

    #[tokio::test]
    async fn test_mp3_format_request() {
        let config = TTSConfig {
            voice_id: Some("test_voice_id".to_string()),
            audio_format: Some("mp3".to_string()),
            sample_rate: Some(44100),
            api_key: "test_key".to_string(),
            model: "eleven_multilingual_v2".to_string(),
            ..Default::default()
        };

        let voice_settings = VoiceSettings::default();
        let builder = ElevenLabsRequestBuilder {
            config,
            voice_settings,
        };
        let client = reqwest::Client::new();
        let request = builder.build_http_request(&client, "Test text");

        // Get the request as built
        let built_request = request.build().unwrap();
        let url = built_request.url().to_string();

        assert!(url.contains("output_format=mp3_44100_128"));

        // Check headers
        let headers = built_request.headers();
        assert_eq!(headers.get("accept").unwrap(), "audio/mpeg");
    }

    #[tokio::test]
    async fn test_elevenlabs_lifecycle() {
        let config = TTSConfig {
            api_key: "test_key".to_string(),
            voice_id: Some("test_voice_id".to_string()),
            ..Default::default()
        };

        let tts = ElevenLabsTTS::new(config).unwrap();

        // Initially disconnected
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);

        // The provider should be properly initialized
        assert!(tts.request_builder.voice_settings.speed.is_some());
        assert!(tts.request_builder.voice_settings.stability.is_some());
    }

    #[tokio::test]
    async fn test_previous_text_included_when_provided() {
        let config = TTSConfig {
            voice_id: Some("test_voice_id".to_string()),
            audio_format: Some("linear16".to_string()),
            sample_rate: Some(24000),
            api_key: "test_key".to_string(),
            model: "eleven_multilingual_v2".to_string(),
            ..Default::default()
        };

        let voice_settings = VoiceSettings::default();
        let builder = ElevenLabsRequestBuilder {
            config,
            voice_settings,
        };
        let client = reqwest::Client::new();

        // Build request with previous_text
        let request = builder.build_http_request_with_context(
            &client,
            "Second utterance",
            Some("First utterance"),
        );

        // Get the request as built
        let built_request = request.build().unwrap();

        // Extract and verify the body contains previous_text
        let body_bytes = built_request.body().and_then(|b| b.as_bytes());
        assert!(body_bytes.is_some());

        let body_str = std::str::from_utf8(body_bytes.unwrap()).unwrap();
        let body_json: serde_json::Value = serde_json::from_str(body_str).unwrap();

        assert_eq!(body_json["text"], "Second utterance");
        assert_eq!(body_json["previous_text"], "First utterance");
        assert!(body_json["model_id"].is_string());
    }

    #[tokio::test]
    async fn test_previous_text_omitted_when_none() {
        let config = TTSConfig {
            voice_id: Some("test_voice_id".to_string()),
            audio_format: Some("linear16".to_string()),
            sample_rate: Some(24000),
            api_key: "test_key".to_string(),
            model: "eleven_multilingual_v2".to_string(),
            ..Default::default()
        };

        let voice_settings = VoiceSettings::default();
        let builder = ElevenLabsRequestBuilder {
            config,
            voice_settings,
        };
        let client = reqwest::Client::new();

        // Build request without previous_text
        let request = builder.build_http_request_with_context(&client, "First utterance", None);

        // Get the request as built
        let built_request = request.build().unwrap();

        // Extract and verify the body does NOT contain previous_text
        let body_bytes = built_request.body().and_then(|b| b.as_bytes());
        assert!(body_bytes.is_some());

        let body_str = std::str::from_utf8(body_bytes.unwrap()).unwrap();
        let body_json: serde_json::Value = serde_json::from_str(body_str).unwrap();

        assert_eq!(body_json["text"], "First utterance");
        assert!(body_json["previous_text"].is_null());
        assert!(body_json["model_id"].is_string());
    }
}
