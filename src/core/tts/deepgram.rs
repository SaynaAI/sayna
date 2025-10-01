use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::base::{AudioCallback, BaseTTS, ConnectionState, TTSConfig, TTSResult};
use super::provider::{PronunciationReplacer, TTSProvider, TTSRequestBuilder};
use crate::utils::req_manager::ReqManager;
use xxhash_rust::xxh3::xxh3_128;

/// Deepgram TTS endpoint
pub const DEEPGRAM_TTS_URL: &str = "https://api.deepgram.com/v1/speak";

/// Deepgram-specific request builder
#[derive(Clone)]
struct DeepgramRequestBuilder {
    config: TTSConfig,
    pronunciation_replacer: Option<PronunciationReplacer>,
}

impl TTSRequestBuilder for DeepgramRequestBuilder {
    /// Build the Deepgram-specific HTTP request with URL, headers and body
    fn build_http_request(&self, client: &reqwest::Client, text: &str) -> reqwest::RequestBuilder {
        // Build the URL with query parameters
        let mut url = String::from(DEEPGRAM_TTS_URL);
        let mut params = Vec::new();

        // Use model field if provided, otherwise fall back to voice_id
        if !self.config.model.is_empty() {
            params.push(format!("model={}", self.config.model));
        } else if let Some(voice_id) = &self.config.voice_id {
            params.push(format!("model={voice_id}"));
        }

        // Encoding (default to raw linear PCM)
        let encoding = self.config.audio_format.as_deref().unwrap_or("linear16");
        params.push(format!("encoding={encoding}"));

        // Ensure no container when requesting raw PCM to avoid WAV headers
        // Aligns with WS behavior which delivers raw binary frames without headers
        match encoding {
            "linear16" | "pcm" | "mulaw" | "ulaw" | "alaw" => {
                params.push("container=none".to_string());
            }
            _ => {}
        }

        if let Some(sample_rate) = self.config.sample_rate {
            params.push(format!("sample_rate={sample_rate}"));
        } else {
            // Use 24000 to match defaults elsewhere (e.g., WS path)
            params.push("sample_rate=24000".to_string());
        }

        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }

        // Build the request with Deepgram-specific headers and body
        client
            .post(url)
            .header("Authorization", format!("Token {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "text": text
            }))
    }

    /// Get the configuration
    fn get_config(&self) -> &TTSConfig {
        &self.config
    }

    /// Get precompiled pronunciation replacer
    fn get_pronunciation_replacer(&self) -> Option<&PronunciationReplacer> {
        self.pronunciation_replacer.as_ref()
    }
}

fn compute_tts_config_hash(config: &TTSConfig) -> String {
    // Build a stable representation of config fields that impact audio output
    let mut s = String::new();
    s.push_str(config.provider.as_str());
    s.push('|');
    s.push_str(config.voice_id.as_deref().unwrap_or(""));
    s.push('|');
    s.push_str(&config.model);
    s.push('|');
    s.push_str(config.audio_format.as_deref().unwrap_or(""));
    s.push('|');
    if let Some(sr) = config.sample_rate {
        s.push_str(&sr.to_string());
    }
    s.push('|');
    if let Some(rate) = config.speaking_rate {
        s.push_str(&format!("{rate:.3}"));
    }
    let hash = xxh3_128(s.as_bytes());
    format!("{hash:032x}")
}

/// Deepgram TTS provider implementation using the Deepgram HTTP REST API
pub struct DeepgramTTS {
    /// Generic HTTP-based TTS provider
    provider: TTSProvider,
    /// Request builder
    request_builder: DeepgramRequestBuilder,
    /// Precomputed config hash for caching
    config_hash: String,
}

impl DeepgramTTS {
    /// Create a new Deepgram TTS instance
    pub fn new(config: TTSConfig) -> TTSResult<Self> {
        let pronunciation_replacer = if !config.pronunciations.is_empty() {
            Some(PronunciationReplacer::new(&config.pronunciations))
        } else {
            None
        };
        let request_builder = DeepgramRequestBuilder {
            config: config.clone(),
            pronunciation_replacer,
        };
        let hash = compute_tts_config_hash(&config);
        Ok(Self {
            provider: TTSProvider::new()?,
            request_builder,
            config_hash: hash,
        })
    }

    /// Set the request manager for this instance
    pub async fn set_req_manager(&mut self, req_manager: Arc<ReqManager>) {
        self.provider.set_req_manager(req_manager).await;
    }
}

impl Default for DeepgramTTS {
    fn default() -> Self {
        Self::new(TTSConfig::default()).unwrap()
    }
}

#[async_trait]
impl BaseTTS for DeepgramTTS {
    fn new(config: TTSConfig) -> TTSResult<Self> {
        DeepgramTTS::new(config)
    }

    fn get_provider(&mut self) -> Option<&mut TTSProvider> {
        Some(&mut self.provider)
    }

    async fn connect(&mut self) -> TTSResult<()> {
        self.provider
            .generic_connect_with_config(DEEPGRAM_TTS_URL, &self.request_builder.config)
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
            tracing::info!("Deepgram TTS not ready, attempting to connect...");
            self.connect().await?;
        }
        // Set config hash once on first speak (idempotent)
        self.provider
            .set_tts_config_hash(self.config_hash.clone())
            .await;
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
            "provider": "deepgram",
            "version": "2.0.0",
            "api_type": "HTTP REST",
            "connection_pooling": true,
            "supported_formats": ["mp3", "wav", "pcm", "aac", "flac", "opus"],
            "supported_sample_rates": [8000, 16000, 22050, 24000, 44100, 48000],
            "supported_models": [
                "aura-asteria-en",
                "aura-luna-en",
                "aura-stella-en",
                "aura-athena-en",
                "aura-hera-en",
                "aura-orion-en",
                "aura-arcas-en",
                "aura-perseus-en",
                "aura-angus-en",
                "aura-orpheus-en",
                "aura-helios-en",
                "aura-zeus-en"
            ],
            "endpoint": DEEPGRAM_TTS_URL,
            "documentation": "https://developers.deepgram.com/reference/text-to-speech-api",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_deepgram_tts_creation() {
        let config = TTSConfig::default();
        let tts = DeepgramTTS::new(config).unwrap();
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_http_request_building() {
        let config = TTSConfig {
            voice_id: Some("aura-asteria-en".to_string()),
            audio_format: Some("mp3".to_string()),
            sample_rate: Some(24000),
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let builder = DeepgramRequestBuilder {
            config,
            pronunciation_replacer: None,
        };
        let client = reqwest::Client::new();
        let request = builder.build_http_request(&client, "Test text");

        // Get the request as built
        let built_request = request.build().unwrap();
        let url = built_request.url().to_string();

        assert!(url.contains("model=aura-asteria-en"));
        assert!(url.contains("encoding=mp3"));
        assert!(url.contains("sample_rate=24000"));
        assert!(url.starts_with(DEEPGRAM_TTS_URL));
    }
}
