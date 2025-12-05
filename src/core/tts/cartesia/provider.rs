//! Cartesia Text-to-Speech request builder and provider implementation.
//!
//! This module implements the `TTSRequestBuilder` trait for Cartesia TTS, which builds
//! HTTP POST requests with proper headers, authentication, and JSON body for the
//! Cartesia Text-to-Speech REST API.
//!
//! # Architecture
//!
//! The `CartesiaRequestBuilder` constructs HTTP requests for the Cartesia TTS API:
//! - URL: `https://api.cartesia.ai/tts/bytes`
//! - Authentication: `Authorization: Bearer {api_key}` header
//! - Version: `Cartesia-Version` header
//! - Content-Type: `application/json`
//! - Accept: Based on output format (application/octet-stream, audio/wav, audio/mpeg)
//!
//! The `CartesiaTTS` struct is the main TTS provider that uses the generic `TTSProvider`
//! infrastructure with `CartesiaRequestBuilder` for Cartesia-specific request construction.
//!
//! # Example
//!
//! ```rust,ignore
//! use sayna::core::tts::cartesia::CartesiaTTS;
//! use sayna::core::tts::{TTSConfig, BaseTTS};
//!
//! let config = TTSConfig {
//!     api_key: "your-cartesia-api-key".to_string(),
//!     voice_id: Some("a0e99841-438c-4a64-b679-ae501e7d6091".to_string()),
//!     audio_format: Some("linear16".to_string()),
//!     sample_rate: Some(24000),
//!     ..Default::default()
//! };
//!
//! let mut tts = CartesiaTTS::new(config)?;
//! tts.connect().await?;
//! tts.speak("Hello, world!", true).await?;
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tracing::{debug, info};
use xxhash_rust::xxh3::xxh3_128;

use super::config::{CARTESIA_TTS_URL, CartesiaTTSConfig};
use crate::core::tts::base::{AudioCallback, BaseTTS, ConnectionState, TTSConfig, TTSResult};
use crate::core::tts::provider::{PronunciationReplacer, TTSProvider, TTSRequestBuilder};
use crate::utils::req_manager::ReqManager;

// =============================================================================
// CartesiaRequestBuilder
// =============================================================================

/// Cartesia-specific request builder for constructing TTS API requests.
///
/// Implements `TTSRequestBuilder` to construct HTTP POST requests for the
/// Cartesia Text-to-Speech REST API with proper headers and JSON body.
///
/// # Performance Notes
/// - Pronunciation replacer is compiled once at construction time
/// - JSON body is built fresh for each request (necessary for different texts)
/// - Uses references to config to avoid cloning during request building
#[derive(Clone)]
pub struct CartesiaRequestBuilder {
    /// Base TTS configuration (contains api_key, voice_id, etc.)
    config: TTSConfig,

    /// Cartesia-specific configuration (model, output_format, api_version)
    cartesia_config: CartesiaTTSConfig,

    /// Pre-compiled pronunciation replacement patterns
    /// Compiled once at construction to avoid regex compilation per request
    pronunciation_replacer: Option<PronunciationReplacer>,
}

impl CartesiaRequestBuilder {
    /// Creates a new Cartesia request builder.
    ///
    /// # Arguments
    /// * `config` - Base TTS configuration
    /// * `cartesia_config` - Cartesia-specific configuration
    ///
    /// # Performance
    /// Compiles pronunciation patterns once at construction time.
    /// This avoids regex compilation overhead during request building.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::{CartesiaRequestBuilder, CartesiaTTSConfig};
    /// use sayna::core::tts::TTSConfig;
    ///
    /// let base = TTSConfig::default();
    /// let cartesia = CartesiaTTSConfig::from_base(base.clone());
    ///
    /// let builder = CartesiaRequestBuilder::new(base, cartesia);
    /// ```
    pub fn new(config: TTSConfig, cartesia_config: CartesiaTTSConfig) -> Self {
        // Pre-compile pronunciation replacer if pronunciations are configured
        let pronunciation_replacer = if !config.pronunciations.is_empty() {
            Some(PronunciationReplacer::new(&config.pronunciations))
        } else {
            None
        };

        Self {
            config,
            cartesia_config,
            pronunciation_replacer,
        }
    }

    /// Returns the Accept header value based on output format.
    ///
    /// Maps container type to appropriate MIME type:
    /// - Raw -> application/octet-stream
    /// - Wav -> audio/wav
    /// - Mp3 -> audio/mpeg
    #[inline]
    fn get_accept_header(&self) -> &'static str {
        self.cartesia_config.output_format.content_type()
    }

    /// Builds the voice object for the request body.
    ///
    /// Cartesia uses a discriminated union for voice specification:
    /// - mode: "id" for voice UUID
    /// - id: the voice UUID
    fn build_voice_json(&self) -> serde_json::Value {
        let voice_id = self.cartesia_config.voice_id().unwrap_or("");

        json!({
            "mode": "id",
            "id": voice_id
        })
    }

    /// Determines the language code for the request.
    ///
    /// Currently defaults to "en". Future enhancement could:
    /// - Extract from voice metadata
    /// - Accept language in TTSConfig
    /// - Auto-detect from text
    #[inline]
    fn get_language(&self) -> &'static str {
        // TODO: Consider adding language to TTSConfig or CartesiaTTSConfig
        "en"
    }
}

impl TTSRequestBuilder for CartesiaRequestBuilder {
    /// Build the Cartesia TTS HTTP request with URL, headers, and JSON body.
    ///
    /// # Request Format
    ///
    /// **URL**: `https://api.cartesia.ai/tts/bytes`
    /// **Method**: POST
    ///
    /// **Headers**:
    /// | Header | Value | Purpose |
    /// |--------|-------|---------|
    /// | Authorization | Bearer {api_key} | Authentication |
    /// | Cartesia-Version | 2025-04-16 | API version |
    /// | Content-Type | application/json | Request body format |
    /// | Accept | {based on container} | Response format |
    ///
    /// **Body**:
    /// ```json
    /// {
    ///   "model_id": "sonic-3",
    ///   "transcript": "Hello, world!",
    ///   "voice": { "mode": "id", "id": "uuid" },
    ///   "output_format": { ... },
    ///   "language": "en"
    /// }
    /// ```
    ///
    /// # Arguments
    /// * `client` - The reqwest HTTP client (from connection pool)
    /// * `text` - The text to synthesize
    ///
    /// # Returns
    /// A `reqwest::RequestBuilder` ready to be sent.
    fn build_http_request(&self, client: &reqwest::Client, text: &str) -> reqwest::RequestBuilder {
        // Build the request body
        let body = json!({
            "model_id": &self.cartesia_config.model,
            "transcript": text,
            "voice": self.build_voice_json(),
            "output_format": self.cartesia_config.build_output_format_json(),
            "language": self.get_language()
        });

        debug!(
            "Building Cartesia TTS request: model={}, voice={:?}, format={:?}",
            self.cartesia_config.model,
            self.cartesia_config.voice_id(),
            self.cartesia_config.output_format.container
        );

        // Build the HTTP request with all required headers
        client
            .post(CARTESIA_TTS_URL)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Cartesia-Version", &self.cartesia_config.api_version)
            .header("Content-Type", "application/json")
            .header("Accept", self.get_accept_header())
            .json(&body)
    }

    /// Returns a reference to the base TTS configuration.
    ///
    /// Used by TTSProvider for:
    /// - Audio format detection (for chunking strategy)
    /// - Sample rate (for duration calculation)
    #[inline]
    fn get_config(&self) -> &TTSConfig {
        &self.config
    }

    /// Returns the precompiled pronunciation replacer if configured.
    ///
    /// The TTSProvider applies pronunciation replacements before sending
    /// text to the API, improving pronunciation consistency.
    #[inline]
    fn get_pronunciation_replacer(&self) -> Option<&PronunciationReplacer> {
        self.pronunciation_replacer.as_ref()
    }
}

// =============================================================================
// Config Hash
// =============================================================================

/// Computes a hash of the TTS configuration for cache keying.
///
/// The hash includes all fields that affect audio output:
/// - Provider name ("cartesia")
/// - Voice ID
/// - Model
/// - Output format (container, encoding, sample_rate)
/// - Speaking rate (if applicable)
///
/// # Cache Key Format
/// When combined with text hash: `{config_hash}:{text_hash}`
///
/// # Returns
/// A 32-character lowercase hexadecimal string
fn compute_cartesia_tts_config_hash(
    config: &TTSConfig,
    cartesia_config: &CartesiaTTSConfig,
) -> String {
    let mut s = String::with_capacity(256); // Pre-allocate to avoid reallocations

    // Provider identifier
    s.push_str("cartesia|");

    // Voice ID
    s.push_str(config.voice_id.as_deref().unwrap_or(""));
    s.push('|');

    // Model
    s.push_str(&cartesia_config.model);
    s.push('|');

    // Output format components
    s.push_str(cartesia_config.output_format.container.as_str());
    s.push('|');
    if let Some(encoding) = &cartesia_config.output_format.encoding {
        s.push_str(encoding.as_str());
    }
    s.push('|');
    s.push_str(&cartesia_config.output_format.sample_rate.to_string());
    s.push('|');

    // Speaking rate (affects prosody)
    if let Some(rate) = config.speaking_rate {
        s.push_str(&format!("{rate:.3}"));
    }

    // Compute xxHash3-128 and format as hex
    let hash = xxh3_128(s.as_bytes());
    format!("{hash:032x}")
}

// =============================================================================
// CartesiaTTS Provider
// =============================================================================

/// Cartesia Text-to-Speech provider implementation.
///
/// Uses the Cartesia TTS REST API with the Sonic voice models for
/// high-quality, low-latency speech synthesis.
///
/// # Architecture
///
/// This provider delegates connection pooling and audio streaming to the
/// generic `TTSProvider` infrastructure, which handles:
/// - HTTP connection pooling via `ReqManager`
/// - Audio chunk buffering and streaming
/// - Ordered delivery via dispatcher task
/// - Audio caching with config+text hash keys
///
/// # Features
///
/// - Sonic voice models with natural prosody
/// - Multiple audio formats (PCM, WAV, MP3)
/// - Pronunciation replacement via precompiled patterns
/// - Automatic reconnection on connection loss
/// - Audio caching for repeated phrases
///
/// # Example
///
/// ```rust,ignore
/// use sayna::core::tts::cartesia::CartesiaTTS;
/// use sayna::core::tts::{TTSConfig, BaseTTS};
///
/// let config = TTSConfig {
///     api_key: "your-api-key".to_string(),
///     voice_id: Some("voice-uuid".to_string()),
///     audio_format: Some("linear16".to_string()),
///     sample_rate: Some(24000),
///     ..Default::default()
/// };
///
/// let mut tts = CartesiaTTS::new(config)?;
/// tts.connect().await?;
/// tts.speak("Hello, world!", true).await?;
/// tts.disconnect().await?;
/// ```
pub struct CartesiaTTS {
    /// Generic HTTP-based TTS provider for connection pooling and streaming
    provider: TTSProvider,

    /// Cartesia-specific request builder
    request_builder: CartesiaRequestBuilder,

    /// Precomputed configuration hash for cache keying
    /// Computed once at construction, reused for all requests
    config_hash: String,
}

impl CartesiaTTS {
    /// Creates a new Cartesia TTS provider instance.
    ///
    /// # Arguments
    /// * `config` - Base TTS configuration with API key and voice settings
    ///
    /// # Validation
    /// Validates that sample rate is supported (if specified).
    /// Note: API key validation happens when connecting or making requests,
    /// allowing for late binding of credentials.
    ///
    /// # Returns
    /// * `Ok(Self)` - A new provider instance ready for connection
    /// * `Err(TTSError::InvalidConfiguration)` - If configuration is invalid
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = TTSConfig {
    ///     api_key: "your-api-key".to_string(),
    ///     voice_id: Some("voice-uuid".to_string()),
    ///     ..Default::default()
    /// };
    ///
    /// let tts = CartesiaTTS::new(config)?;
    /// ```
    pub fn new(config: TTSConfig) -> TTSResult<Self> {
        // Create Cartesia-specific config
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        // Create request builder with precompiled pronunciation patterns
        let request_builder = CartesiaRequestBuilder::new(config.clone(), cartesia_config.clone());

        // Compute config hash for caching
        let config_hash = compute_cartesia_tts_config_hash(&config, &cartesia_config);

        info!(
            "Created CartesiaTTS provider: model={}, voice={:?}, format={:?}",
            cartesia_config.model,
            cartesia_config.voice_id(),
            cartesia_config.output_format.container
        );

        Ok(Self {
            provider: TTSProvider::new()?,
            request_builder,
            config_hash,
        })
    }

    /// Sets the request manager for connection pooling.
    ///
    /// Allows sharing a ReqManager across multiple provider instances
    /// for efficient connection reuse.
    ///
    /// # Arguments
    /// * `req_manager` - Shared request manager instance
    pub async fn set_req_manager(&mut self, req_manager: Arc<ReqManager>) {
        self.provider.set_req_manager(req_manager).await;
    }

    /// Returns a reference to the Cartesia-specific configuration.
    ///
    /// Useful for debugging or inspecting the resolved configuration.
    #[inline]
    pub fn cartesia_config(&self) -> &CartesiaTTSConfig {
        &self.request_builder.cartesia_config
    }
}

impl Default for CartesiaTTS {
    fn default() -> Self {
        Self::new(TTSConfig::default())
            .expect("Default CartesiaTTS should have valid configuration")
    }
}

#[async_trait]
impl BaseTTS for CartesiaTTS {
    /// Create a new instance of the TTS provider.
    ///
    /// This is the factory method used by `create_tts_provider`.
    fn new(config: TTSConfig) -> TTSResult<Self> {
        CartesiaTTS::new(config)
    }

    /// Get the underlying TTSProvider for HTTP-based providers.
    ///
    /// Returns a mutable reference to enable the generic provider
    /// infrastructure to manage audio callbacks, caching, etc.
    fn get_provider(&mut self) -> Option<&mut TTSProvider> {
        Some(&mut self.provider)
    }

    /// Connect to the TTS provider.
    ///
    /// Initializes the HTTP connection pool with configuration-based
    /// timeouts and pool sizes.
    ///
    /// # Connection Pool Configuration
    /// - Pool size: From `config.request_pool_size` (default: 4)
    /// - Connect timeout: From `config.connection_timeout` (default: 30s)
    /// - Request timeout: From `config.request_timeout` (default: 60s)
    ///
    /// # Returns
    /// * `Ok(())` - Connection pool initialized successfully
    /// * `Err(TTSError::ConnectionFailed)` - Failed to create connection pool
    async fn connect(&mut self) -> TTSResult<()> {
        self.provider
            .generic_connect_with_config(CARTESIA_TTS_URL, &self.request_builder.config)
            .await?;

        info!("Cartesia TTS provider connected and ready");
        Ok(())
    }

    /// Disconnect from the TTS provider.
    ///
    /// Stops all background tasks (queue worker, dispatcher),
    /// clears pending requests, and releases the connection pool.
    async fn disconnect(&mut self) -> TTSResult<()> {
        self.provider.generic_disconnect().await
    }

    /// Check if the TTS provider is ready to process requests.
    ///
    /// Returns true after `connect()` succeeds and before `disconnect()`.
    #[inline]
    fn is_ready(&self) -> bool {
        self.provider.is_ready()
    }

    /// Get the current connection state.
    ///
    /// # States
    /// - `Disconnected` - Not connected
    /// - `Connected` - Ready for requests
    #[inline]
    fn get_connection_state(&self) -> ConnectionState {
        self.provider.get_connection_state()
    }

    /// Send text to the TTS provider for synthesis.
    ///
    /// # Auto-Reconnection
    /// If the provider is not ready, attempts automatic reconnection
    /// before processing the request.
    ///
    /// # Request Flow
    /// 1. Validates provider is ready (reconnects if needed)
    /// 2. Sets config hash for caching (idempotent)
    /// 3. Enqueues text for synthesis via queue worker
    /// 4. Audio chunks delivered via registered callback
    ///
    /// # Arguments
    /// * `text` - The text to synthesize
    /// * `flush` - Whether to immediately process queued text
    ///
    /// # Returns
    /// * `Ok(())` - Request enqueued successfully
    /// * `Err(TTSError::ProviderNotReady)` - Connection failed
    async fn speak(&mut self, text: &str, flush: bool) -> TTSResult<()> {
        // Auto-reconnect if needed
        if !self.is_ready() {
            info!("Cartesia TTS not ready, attempting to connect...");
            self.connect().await?;
        }

        // Set config hash once on first speak (idempotent)
        self.provider
            .set_tts_config_hash(self.config_hash.clone())
            .await;

        // Delegate to generic provider
        self.provider
            .generic_speak(self.request_builder.clone(), text, flush)
            .await
    }

    /// Clear any queued text from the synthesis queue.
    ///
    /// Cancels all pending requests and clears the queue.
    /// Does not affect audio already being played.
    async fn clear(&mut self) -> TTSResult<()> {
        self.provider.generic_clear().await
    }

    /// Flush the TTS provider queue.
    ///
    /// Forces immediate processing of any queued text.
    /// Note: In the current implementation, this is a no-op since
    /// requests are processed immediately when enqueued.
    async fn flush(&self) -> TTSResult<()> {
        self.provider.generic_flush().await
    }

    /// Register an audio callback.
    ///
    /// The callback is invoked for each audio chunk received from the API.
    ///
    /// # Callback Events
    /// - `on_audio(AudioData)` - Audio chunk received
    /// - `on_error(TTSError)` - Error occurred
    /// - `on_complete()` - All audio for current text delivered
    ///
    /// # Arguments
    /// * `callback` - Arc-wrapped callback implementation
    fn on_audio(&mut self, callback: Arc<dyn AudioCallback>) -> TTSResult<()> {
        self.provider.generic_on_audio(callback)
    }

    /// Remove the registered audio callback.
    ///
    /// After removal, no callbacks will be invoked for audio events.
    fn remove_audio_callback(&mut self) -> TTSResult<()> {
        self.provider.generic_remove_audio_callback()
    }

    /// Get provider-specific information.
    ///
    /// Returns metadata about this provider instance for debugging
    /// and informational purposes.
    fn get_provider_info(&self) -> serde_json::Value {
        serde_json::json!({
            "provider": "cartesia",
            "version": "1.0.0",
            "api_type": "HTTP REST",
            "connection_pooling": true,
            "endpoint": CARTESIA_TTS_URL,
            "model": &self.request_builder.cartesia_config.model,
            "api_version": &self.request_builder.cartesia_config.api_version,
            "supported_formats": ["raw", "wav", "mp3"],
            "supported_encodings": ["pcm_s16le", "pcm_f32le", "pcm_alaw", "pcm_mulaw"],
            "supported_sample_rates": [8000, 16000, 22050, 24000, 44100, 48000],
            "supported_languages": [
                "en", "fr", "de", "es", "pt", "zh", "ja", "hi", "it", "ko",
                "nl", "pl", "ru", "sv", "tr", "ar", "cs", "da", "el", "fi",
                "he", "hu", "id", "ms", "no", "ro", "sk", "th", "uk", "vi"
            ],
            "documentation": "https://docs.cartesia.ai/api-reference/tts/bytes"
        })
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tts::base::Pronunciation;
    use crate::core::tts::cartesia::{CartesiaAudioContainer, CartesiaAudioEncoding};

    // =========================================================================
    // Helper Functions
    // =========================================================================

    fn create_test_config() -> TTSConfig {
        TTSConfig {
            provider: "cartesia".to_string(),
            api_key: "test-api-key".to_string(),
            voice_id: Some("a0e99841-438c-4a64-b679-ae501e7d6091".to_string()),
            model: String::new(),
            speaking_rate: None,
            audio_format: Some("linear16".to_string()),
            sample_rate: Some(24000),
            connection_timeout: Some(30),
            request_timeout: Some(60),
            pronunciations: Vec::new(),
            request_pool_size: Some(4),
            ..Default::default()
        }
    }

    // =========================================================================
    // CartesiaRequestBuilder Tests
    // =========================================================================

    #[test]
    fn test_cartesia_request_builder_new() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        assert!(builder.pronunciation_replacer.is_none());
        assert_eq!(
            builder.cartesia_config.output_format.container,
            CartesiaAudioContainer::Raw
        );
        assert_eq!(
            builder.cartesia_config.output_format.encoding,
            Some(CartesiaAudioEncoding::PcmS16le)
        );
    }

    #[test]
    fn test_cartesia_request_builder_with_pronunciations() {
        let mut config = create_test_config();
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

        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());
        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        assert!(builder.pronunciation_replacer.is_some());
    }

    #[test]
    fn test_cartesia_request_builder_get_config() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let retrieved_config = builder.get_config();
        assert_eq!(
            retrieved_config.voice_id,
            Some("a0e99841-438c-4a64-b679-ae501e7d6091".to_string())
        );
        assert_eq!(retrieved_config.sample_rate, Some(24000));
    }

    #[test]
    fn test_cartesia_request_builder_get_pronunciation_replacer_none() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        assert!(builder.get_pronunciation_replacer().is_none());
    }

    #[test]
    fn test_cartesia_request_builder_get_pronunciation_replacer_some() {
        let mut config = create_test_config();
        config.pronunciations = vec![Pronunciation {
            word: "API".to_string(),
            pronunciation: "A P I".to_string(),
        }];

        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());
        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        assert!(builder.get_pronunciation_replacer().is_some());
    }

    #[test]
    fn test_cartesia_request_builder_get_accept_header() {
        // Test raw format
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());
        let builder = CartesiaRequestBuilder::new(config.clone(), cartesia_config);
        assert_eq!(builder.get_accept_header(), "application/octet-stream");

        // Test wav format
        let mut wav_config = config.clone();
        wav_config.audio_format = Some("wav".to_string());
        let cartesia_config = CartesiaTTSConfig::from_base(wav_config.clone());
        let builder = CartesiaRequestBuilder::new(wav_config, cartesia_config);
        assert_eq!(builder.get_accept_header(), "audio/wav");

        // Test mp3 format
        let mut mp3_config = config.clone();
        mp3_config.audio_format = Some("mp3".to_string());
        let cartesia_config = CartesiaTTSConfig::from_base(mp3_config.clone());
        let builder = CartesiaRequestBuilder::new(mp3_config, cartesia_config);
        assert_eq!(builder.get_accept_header(), "audio/mpeg");
    }

    #[test]
    fn test_cartesia_request_builder_build_voice_json() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());
        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let voice_json = builder.build_voice_json();
        assert_eq!(voice_json["mode"], "id");
        assert_eq!(voice_json["id"], "a0e99841-438c-4a64-b679-ae501e7d6091");
    }

    #[test]
    fn test_cartesia_request_builder_clone() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        // Clone should work since it derives Clone
        let cloned = builder.clone();

        assert_eq!(cloned.config.api_key, builder.config.api_key);
        assert_eq!(cloned.cartesia_config.model, builder.cartesia_config.model);
    }

    // =========================================================================
    // HTTP Request Building Tests
    // =========================================================================

    #[test]
    fn test_build_http_request_url() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let client = reqwest::Client::new();
        let request_builder = builder.build_http_request(&client, "Hello world");
        let request = request_builder.build().unwrap();

        assert_eq!(request.url().as_str(), CARTESIA_TTS_URL);
        assert_eq!(request.method(), reqwest::Method::POST);
    }

    #[test]
    fn test_build_http_request_headers() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let client = reqwest::Client::new();
        let request_builder = builder.build_http_request(&client, "Hello world");
        let request = request_builder.build().unwrap();

        // Verify Authorization header
        let auth_header = request.headers().get("authorization").unwrap();
        assert_eq!(auth_header.to_str().unwrap(), "Bearer test-api-key");

        // Verify Cartesia-Version header
        let version_header = request.headers().get("cartesia-version").unwrap();
        assert_eq!(version_header.to_str().unwrap(), "2025-04-16");

        // Verify Content-Type header
        let content_type = request.headers().get("content-type").unwrap();
        assert_eq!(content_type.to_str().unwrap(), "application/json");

        // Verify Accept header
        let accept_header = request.headers().get("accept").unwrap();
        assert_eq!(accept_header.to_str().unwrap(), "application/octet-stream");
    }

    #[test]
    fn test_build_http_request_body_structure() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let client = reqwest::Client::new();
        let request_builder = builder.build_http_request(&client, "Hello world");
        let request = request_builder.build().unwrap();

        let body = request.body().unwrap().as_bytes().unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(body).unwrap();

        // Verify model_id
        assert_eq!(body_json["model_id"], "sonic-3");

        // Verify transcript
        assert_eq!(body_json["transcript"], "Hello world");

        // Verify voice structure
        assert_eq!(body_json["voice"]["mode"], "id");
        assert_eq!(
            body_json["voice"]["id"],
            "a0e99841-438c-4a64-b679-ae501e7d6091"
        );

        // Verify output_format structure
        assert_eq!(body_json["output_format"]["container"], "raw");
        assert_eq!(body_json["output_format"]["encoding"], "pcm_s16le");
        assert_eq!(body_json["output_format"]["sample_rate"], 24000);

        // Verify language
        assert_eq!(body_json["language"], "en");
    }

    #[test]
    fn test_build_http_request_mp3_format() {
        let mut config = create_test_config();
        config.audio_format = Some("mp3".to_string());

        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());
        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let client = reqwest::Client::new();
        let request_builder = builder.build_http_request(&client, "Test");
        let request = request_builder.build().unwrap();

        let body = request.body().unwrap().as_bytes().unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(body).unwrap();

        // Verify MP3 output format
        assert_eq!(body_json["output_format"]["container"], "mp3");
        assert_eq!(body_json["output_format"]["sample_rate"], 24000);
        assert_eq!(body_json["output_format"]["bit_rate"], 128000);
        // encoding should not be present for MP3
        assert!(body_json["output_format"].get("encoding").is_none());
    }

    #[test]
    fn test_build_http_request_mulaw_format() {
        let mut config = create_test_config();
        config.audio_format = Some("mulaw".to_string());
        config.sample_rate = Some(8000);

        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());
        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let client = reqwest::Client::new();
        let request_builder = builder.build_http_request(&client, "Test");
        let request = request_builder.build().unwrap();

        let body = request.body().unwrap().as_bytes().unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(body).unwrap();

        assert_eq!(body_json["output_format"]["container"], "raw");
        assert_eq!(body_json["output_format"]["encoding"], "pcm_mulaw");
        assert_eq!(body_json["output_format"]["sample_rate"], 8000);
    }

    // =========================================================================
    // Config Hash Tests
    // =========================================================================

    #[test]
    fn test_compute_cartesia_tts_config_hash() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let hash = compute_cartesia_tts_config_hash(&config, &cartesia_config);

        // Hash should be a 32-char hex string
        assert_eq!(hash.len(), 32);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));

        // Same config should produce same hash
        let hash2 = compute_cartesia_tts_config_hash(&config, &cartesia_config);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_compute_cartesia_tts_config_hash_different_voice() {
        let config1 = create_test_config();
        let cartesia_config1 = CartesiaTTSConfig::from_base(config1.clone());

        let mut config2 = create_test_config();
        config2.voice_id = Some("different-voice-id".to_string());
        let cartesia_config2 = CartesiaTTSConfig::from_base(config2.clone());

        let hash1 = compute_cartesia_tts_config_hash(&config1, &cartesia_config1);
        let hash2 = compute_cartesia_tts_config_hash(&config2, &cartesia_config2);

        // Different voice should produce different hash
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_compute_cartesia_tts_config_hash_different_model() {
        let config1 = create_test_config();
        let cartesia_config1 = CartesiaTTSConfig::from_base(config1.clone());

        let mut config2 = create_test_config();
        config2.model = "sonic-2".to_string();
        let cartesia_config2 = CartesiaTTSConfig::from_base(config2.clone());

        let hash1 = compute_cartesia_tts_config_hash(&config1, &cartesia_config1);
        let hash2 = compute_cartesia_tts_config_hash(&config2, &cartesia_config2);

        // Different model should produce different hash
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_compute_cartesia_tts_config_hash_different_sample_rate() {
        let config1 = create_test_config();
        let cartesia_config1 = CartesiaTTSConfig::from_base(config1.clone());

        let mut config2 = create_test_config();
        config2.sample_rate = Some(16000);
        let cartesia_config2 = CartesiaTTSConfig::from_base(config2.clone());

        let hash1 = compute_cartesia_tts_config_hash(&config1, &cartesia_config1);
        let hash2 = compute_cartesia_tts_config_hash(&config2, &cartesia_config2);

        // Different sample rate should produce different hash
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_compute_cartesia_tts_config_hash_different_format() {
        let config1 = create_test_config();
        let cartesia_config1 = CartesiaTTSConfig::from_base(config1.clone());

        let mut config2 = create_test_config();
        config2.audio_format = Some("mp3".to_string());
        let cartesia_config2 = CartesiaTTSConfig::from_base(config2.clone());

        let hash1 = compute_cartesia_tts_config_hash(&config1, &cartesia_config1);
        let hash2 = compute_cartesia_tts_config_hash(&config2, &cartesia_config2);

        // Different format should produce different hash
        assert_ne!(hash1, hash2);
    }

    // =========================================================================
    // CartesiaTTS Provider Tests
    // =========================================================================

    #[test]
    fn test_cartesia_tts_creation() {
        let config = create_test_config();
        let tts = CartesiaTTS::new(config).unwrap();

        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);
    }

    #[test]
    fn test_cartesia_tts_creation_empty_api_key() {
        // API key validation happens during connect/request, not during creation
        // This allows late binding of credentials
        let mut config = create_test_config();
        config.api_key = String::new();

        let result = CartesiaTTS::new(config);

        // Should succeed - API key is validated lazily
        assert!(result.is_ok());
    }

    #[test]
    fn test_cartesia_tts_default() {
        let tts = CartesiaTTS::default();

        assert!(!tts.is_ready());
        assert_eq!(tts.cartesia_config().model, "sonic-3");
    }

    #[test]
    fn test_cartesia_tts_cartesia_config() {
        let config = create_test_config();
        let tts = CartesiaTTS::new(config).unwrap();

        let cartesia_config = tts.cartesia_config();
        assert_eq!(cartesia_config.model, "sonic-3");
        assert_eq!(
            cartesia_config.voice_id(),
            Some("a0e99841-438c-4a64-b679-ae501e7d6091")
        );
    }

    #[test]
    fn test_cartesia_tts_get_provider_info() {
        let config = create_test_config();
        let tts = CartesiaTTS::new(config).unwrap();

        let info = tts.get_provider_info();

        assert_eq!(info["provider"], "cartesia");
        assert_eq!(info["version"], "1.0.0");
        assert_eq!(info["api_type"], "HTTP REST");
        assert_eq!(info["connection_pooling"], true);
        assert_eq!(info["endpoint"], CARTESIA_TTS_URL);
        assert_eq!(info["model"], "sonic-3");
        assert_eq!(info["api_version"], "2025-04-16");
        assert!(info["supported_formats"].is_array());
        assert!(info["supported_encodings"].is_array());
        assert!(info["supported_sample_rates"].is_array());
        assert!(info["supported_languages"].is_array());
        assert!(
            info["documentation"]
                .as_str()
                .unwrap()
                .contains("cartesia.ai")
        );
    }

    // =========================================================================
    // Edge Cases
    // =========================================================================

    #[test]
    fn test_build_http_request_empty_text() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let client = reqwest::Client::new();
        let request_builder = builder.build_http_request(&client, "");
        let request = request_builder.build().unwrap();

        let body = request.body().unwrap().as_bytes().unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(body).unwrap();

        // Empty text should be in transcript
        assert_eq!(body_json["transcript"], "");
    }

    #[test]
    fn test_build_http_request_unicode_text() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let client = reqwest::Client::new();
        let text = "Hello, 世界! Привет мир! 🌍";
        let request_builder = builder.build_http_request(&client, text);
        let request = request_builder.build().unwrap();

        let body = request.body().unwrap().as_bytes().unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(body).unwrap();

        // Unicode text should be preserved
        assert_eq!(body_json["transcript"], text);
    }

    #[test]
    fn test_build_http_request_special_characters() {
        let config = create_test_config();
        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());

        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let client = reqwest::Client::new();
        let text = "Test <tag> & \"quote\" 'apos' {brace} end";
        let request_builder = builder.build_http_request(&client, text);
        let request = request_builder.build().unwrap();

        let body = request.body().unwrap().as_bytes().unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(body).unwrap();

        // Special characters should be properly escaped in JSON
        assert_eq!(body_json["transcript"], text);
    }

    #[test]
    fn test_build_http_request_empty_voice_id() {
        let mut config = create_test_config();
        config.voice_id = None;

        let cartesia_config = CartesiaTTSConfig::from_base(config.clone());
        let builder = CartesiaRequestBuilder::new(config, cartesia_config);

        let voice_json = builder.build_voice_json();
        assert_eq!(voice_json["mode"], "id");
        assert_eq!(voice_json["id"], "");
    }

    #[test]
    fn test_config_hash_with_speaking_rate() {
        let mut config1 = create_test_config();
        config1.speaking_rate = Some(1.0);
        let cartesia_config1 = CartesiaTTSConfig::from_base(config1.clone());

        let mut config2 = create_test_config();
        config2.speaking_rate = Some(1.5);
        let cartesia_config2 = CartesiaTTSConfig::from_base(config2.clone());

        let hash1 = compute_cartesia_tts_config_hash(&config1, &cartesia_config1);
        let hash2 = compute_cartesia_tts_config_hash(&config2, &cartesia_config2);

        // Different speaking rates should produce different hashes
        assert_ne!(hash1, hash2);
    }
}
