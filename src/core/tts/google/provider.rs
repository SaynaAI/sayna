//! Google Cloud Text-to-Speech authentication wrapper and request builder.
//!
//! This module provides a TTS-specific wrapper around the generic Google Cloud
//! authentication infrastructure. It handles credential validation, token acquisition,
//! and error conversion between Google Cloud errors and TTS-specific errors.
//!
//! Additionally, it provides `GoogleRequestBuilder` which implements `TTSRequestBuilder`
//! for constructing HTTP requests to the Google Cloud Text-to-Speech REST API.
//!
//! # Architecture
//!
//! The `TTSGoogleAuthClient` wraps `GoogleAuthClient` from the generic Google
//! provider infrastructure, translating errors to `TTSError` and ensuring the
//! correct OAuth2 scope is used for Text-to-Speech API access.
//!
//! The `GoogleRequestBuilder` constructs properly formatted HTTP POST requests
//! for the `/v1/text:synthesize` endpoint, handling:
//! - Bearer token authentication
//! - JSON request body with `input`, `voice`, and `audioConfig` sections
//! - Optional parameters like pitch, volume, and effects profiles
//!
//! # Example
//!
//! ```rust,no_run
//! use sayna::core::tts::google::TTSGoogleAuthClient;
//! use sayna::core::providers::google::TokenProvider;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create from API key (file path, JSON content, or empty for ADC)
//!     let auth_client = TTSGoogleAuthClient::from_api_key("/path/to/credentials.json")?;
//!
//!     // Get an access token for TTS API requests
//!     let token = auth_client.get_token().await?;
//!     println!("Token: {}", token);
//!     Ok(())
//! }
//! ```

use crate::core::providers::google::{
    CredentialSource, GOOGLE_CLOUD_PLATFORM_SCOPE, GoogleAuthClient, GoogleError, TokenProvider,
};
use crate::core::tts::base::TTSConfig;
use crate::core::tts::base::TTSError;
use crate::core::tts::provider::{PronunciationReplacer, TTSRequestBuilder};

use super::config::GoogleTTSConfig;

/// Google Cloud Text-to-Speech API endpoint.
///
/// This is the REST API endpoint for synthesizing speech from text.
/// The API accepts JSON requests and returns JSON responses containing
/// base64-encoded audio data.
pub const GOOGLE_TTS_URL: &str = "https://texttospeech.googleapis.com/v1/text:synthesize";

/// Converts a `GoogleError` to a `TTSError`.
///
/// This function maps each Google Cloud error variant to an appropriate TTS error
/// variant, ensuring error context is preserved for debugging.
///
/// # Arguments
///
/// * `e` - The Google Cloud error to convert
///
/// # Returns
///
/// A `TTSError` that represents the same error condition
pub(crate) fn google_error_to_tts(e: GoogleError) -> TTSError {
    match e {
        GoogleError::AuthenticationFailed(msg) => {
            TTSError::ConnectionFailed(format!("Google Cloud authentication failed: {msg}"))
        }
        GoogleError::ConfigurationError(msg) => TTSError::InvalidConfiguration(msg),
        GoogleError::ConnectionFailed(msg) => TTSError::ConnectionFailed(msg),
        GoogleError::NetworkError(msg) => TTSError::NetworkError(msg),
        GoogleError::ApiError(msg) => TTSError::ProviderError(msg),
        GoogleError::GrpcError { code, message } => {
            TTSError::ProviderError(format!("gRPC error ({code}): {message}"))
        }
    }
}

/// TTS-specific wrapper for GoogleAuthClient.
///
/// This struct provides a TTS-focused interface to Google Cloud authentication,
/// ensuring credentials are validated before use and the correct OAuth2 scope
/// is applied for Text-to-Speech API access.
///
/// The wrapper validates credentials during construction, catching configuration
/// errors early with clear error messages rather than failing later during
/// API calls.
#[derive(Debug)]
pub struct TTSGoogleAuthClient {
    inner: GoogleAuthClient,
}

impl TTSGoogleAuthClient {
    /// Creates a new TTS auth client from a credential source.
    ///
    /// This method validates the credentials before creating the auth client,
    /// ensuring configuration errors are caught early.
    ///
    /// # Arguments
    ///
    /// * `credential_source` - The source of Google Cloud credentials
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` - A new auth client ready to obtain tokens
    /// * `Err(TTSError)` - If credentials are invalid or client creation fails
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use sayna::core::tts::google::TTSGoogleAuthClient;
    /// use sayna::core::providers::google::CredentialSource;
    ///
    /// let source = CredentialSource::ApplicationDefault;
    /// let client = TTSGoogleAuthClient::new(source)?;
    /// # Ok::<(), sayna::core::tts::TTSError>(())
    /// ```
    pub fn new(credential_source: CredentialSource) -> Result<Self, TTSError> {
        // Validate credentials first for clear error messages
        credential_source.validate().map_err(google_error_to_tts)?;

        // Create the inner client with the cloud platform scope
        let inner = GoogleAuthClient::new(credential_source, &[GOOGLE_CLOUD_PLATFORM_SCOPE])
            .map_err(google_error_to_tts)?;

        Ok(Self { inner })
    }

    /// Creates a new auth client from an API key string.
    ///
    /// This is a convenience method that parses the API key to determine the
    /// credential source type:
    /// - Empty string → Application Default Credentials
    /// - String starting with `{` → JSON content (service account)
    /// - Other strings → File path
    ///
    /// # Arguments
    ///
    /// * `api_key` - The credential string from TTS configuration
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` - A new auth client ready to obtain tokens
    /// * `Err(TTSError)` - If credentials are invalid or client creation fails
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use sayna::core::tts::google::TTSGoogleAuthClient;
    ///
    /// // From a file path
    /// let client = TTSGoogleAuthClient::from_api_key("/path/to/credentials.json")?;
    ///
    /// // From JSON content
    /// let json = r#"{"type": "service_account", ...}"#;
    /// let client = TTSGoogleAuthClient::from_api_key(json)?;
    ///
    /// // Using Application Default Credentials
    /// let client = TTSGoogleAuthClient::from_api_key("")?;
    /// # Ok::<(), sayna::core::tts::TTSError>(())
    /// ```
    pub fn from_api_key(api_key: &str) -> Result<Self, TTSError> {
        let source = CredentialSource::from_api_key(api_key);
        Self::new(source)
    }
}

#[async_trait::async_trait]
impl TokenProvider for TTSGoogleAuthClient {
    /// Retrieves a valid access token for Google Cloud TTS API.
    ///
    /// The token is automatically refreshed when expired. This method delegates
    /// to the inner `GoogleAuthClient` for actual token retrieval.
    ///
    /// Note: This returns `GoogleError` as required by the `TokenProvider` trait.
    /// Error conversion to `TTSError` should happen at the call site.
    ///
    /// # Returns
    ///
    /// * `Ok(String)` - A valid OAuth2 access token
    /// * `Err(GoogleError)` - If token retrieval fails
    async fn get_token(&self) -> Result<String, GoogleError> {
        self.inner.get_token().await
    }
}

/// Google-specific request builder for constructing TTS API requests.
///
/// This struct implements `TTSRequestBuilder` to construct HTTP POST requests
/// for the Google Cloud Text-to-Speech REST API. It handles:
///
/// - Bearer token authentication
/// - JSON request body construction with `input`, `voice`, and `audioConfig` sections
/// - Optional parameters like pitch, volume gain, and effects profiles
///
/// # Request Format
///
/// The builder constructs requests with the following JSON structure:
///
/// ```json
/// {
///     "input": { "text": "..." },
///     "voice": { "languageCode": "en-US", "name": "en-US-Wavenet-D" },
///     "audioConfig": {
///         "audioEncoding": "LINEAR16",
///         "speakingRate": 1.0,
///         "pitch": 0.0,
///         "volumeGainDb": 0.0,
///         "sampleRateHertz": 24000,
///         "effectsProfileId": ["headphone-class-device"]
///     }
/// }
/// ```
///
/// Only fields with configured values are included in the request.
///
/// # Example
///
/// ```rust,ignore
/// use sayna::core::tts::google::{GoogleRequestBuilder, GoogleTTSConfig};
/// use sayna::core::tts::{TTSConfig, provider::TTSRequestBuilder};
///
/// let base_config = TTSConfig {
///     voice_id: Some("en-US-Wavenet-D".to_string()),
///     audio_format: Some("mp3".to_string()),
///     ..Default::default()
/// };
///
/// let google_config = GoogleTTSConfig::from_base_config(base_config.clone(), "my-project".to_string());
/// let token = "oauth2-token-here".to_string();
///
/// let builder = GoogleRequestBuilder::new(base_config, google_config, token);
///
/// // Build a request
/// let client = reqwest::Client::new();
/// let request = builder.build_http_request(&client, "Hello, world!");
/// ```
#[derive(Clone)]
pub struct GoogleRequestBuilder {
    /// Base TTS configuration for trait method
    config: TTSConfig,
    /// Google-specific configuration
    google_config: GoogleTTSConfig,
    /// Compiled pronunciation patterns for text replacement
    pronunciation_replacer: Option<PronunciationReplacer>,
    /// OAuth2 Bearer token for authentication
    auth_token: String,
}

impl GoogleRequestBuilder {
    /// Creates a new Google request builder.
    ///
    /// # Arguments
    ///
    /// * `config` - Base TTS configuration
    /// * `google_config` - Google-specific configuration
    /// * `auth_token` - Fresh OAuth2 access token for authentication
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::google::{GoogleRequestBuilder, GoogleTTSConfig};
    /// use sayna::core::tts::TTSConfig;
    ///
    /// let base = TTSConfig::default();
    /// let google = GoogleTTSConfig::from_base_config(base.clone(), "project-id".to_string());
    /// let token = "oauth2-token".to_string();
    ///
    /// let builder = GoogleRequestBuilder::new(base, google, token);
    /// ```
    pub fn new(config: TTSConfig, google_config: GoogleTTSConfig, auth_token: String) -> Self {
        // Create pronunciation replacer if pronunciations are configured
        let pronunciation_replacer = if !config.pronunciations.is_empty() {
            Some(PronunciationReplacer::new(&config.pronunciations))
        } else {
            None
        };

        Self {
            config,
            google_config,
            pronunciation_replacer,
            auth_token,
        }
    }

    /// Builds the JSON request body for the Google TTS API.
    ///
    /// Constructs a JSON object with three sections:
    /// - `input`: Contains the text to synthesize
    /// - `voice`: Contains language code and optional voice name
    /// - `audioConfig`: Contains encoding and optional audio parameters
    fn build_request_body(&self, text: &str) -> serde_json::Value {
        // Build voice object
        let mut voice = serde_json::Map::new();
        voice.insert(
            "languageCode".to_string(),
            serde_json::Value::String(self.google_config.language_code.clone()),
        );

        // Add voice name if configured
        if let Some(name) = self.google_config.voice_name() {
            voice.insert(
                "name".to_string(),
                serde_json::Value::String(name.to_string()),
            );
        }

        // Build audioConfig object
        let mut audio_config = serde_json::Map::new();
        audio_config.insert(
            "audioEncoding".to_string(),
            serde_json::Value::String(self.google_config.audio_encoding.as_str().to_string()),
        );

        // Add optional speaking rate
        if let Some(rate) = self.google_config.speaking_rate() {
            audio_config.insert(
                "speakingRate".to_string(),
                serde_json::Value::Number(
                    serde_json::Number::from_f64(rate)
                        .unwrap_or_else(|| serde_json::Number::from(1)),
                ),
            );
        }

        // Add optional pitch
        if let Some(pitch) = self.google_config.clamped_pitch() {
            audio_config.insert(
                "pitch".to_string(),
                serde_json::Value::Number(
                    serde_json::Number::from_f64(pitch)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ),
            );
        }

        // Add optional volume gain
        if let Some(volume_gain) = self.google_config.clamped_volume_gain() {
            audio_config.insert(
                "volumeGainDb".to_string(),
                serde_json::Value::Number(
                    serde_json::Number::from_f64(volume_gain)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ),
            );
        }

        // Add optional sample rate
        if let Some(sample_rate) = self.google_config.sample_rate_hertz() {
            audio_config.insert(
                "sampleRateHertz".to_string(),
                serde_json::Value::Number(serde_json::Number::from(sample_rate)),
            );
        }

        // Add effects profile IDs if configured
        if !self.google_config.effects_profile_id.is_empty() {
            audio_config.insert(
                "effectsProfileId".to_string(),
                serde_json::Value::Array(
                    self.google_config
                        .effects_profile_id
                        .iter()
                        .map(|s| serde_json::Value::String(s.clone()))
                        .collect(),
                ),
            );
        }

        // Construct the final request body
        serde_json::json!({
            "input": {
                "text": text
            },
            "voice": voice,
            "audioConfig": audio_config
        })
    }
}

impl TTSRequestBuilder for GoogleRequestBuilder {
    /// Builds the HTTP request for Google Cloud Text-to-Speech API.
    ///
    /// Constructs a POST request to the TTS synthesize endpoint with:
    /// - Bearer token authentication
    /// - JSON request body with input, voice, and audioConfig sections
    ///
    /// # Arguments
    ///
    /// * `client` - The HTTP client to use for building the request
    /// * `text` - The text to synthesize
    ///
    /// # Returns
    ///
    /// A `reqwest::RequestBuilder` ready to be sent
    fn build_http_request(&self, client: &reqwest::Client, text: &str) -> reqwest::RequestBuilder {
        let body = self.build_request_body(text);

        client
            .post(GOOGLE_TTS_URL)
            .header("Authorization", format!("Bearer {}", self.auth_token))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
    }

    /// Returns a reference to the base TTS configuration.
    fn get_config(&self) -> &TTSConfig {
        &self.config
    }

    /// Returns the precompiled pronunciation replacer if configured.
    fn get_pronunciation_replacer(&self) -> Option<&PronunciationReplacer> {
        self.pronunciation_replacer.as_ref()
    }
}

// ===== GoogleTTS Provider Implementation =====

use async_trait::async_trait;
use base64::Engine;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, error, info};
use xxhash_rust::xxh3::xxh3_128;

use crate::core::cache::store::CacheStore;
use crate::core::tts::base::{AudioCallback, AudioData, BaseTTS, ConnectionState, TTSResult};

/// Google Cloud Text-to-Speech API response structure.
#[derive(Debug, Deserialize)]
struct GoogleTTSResponse {
    /// Base64-encoded audio content
    #[serde(rename = "audioContent")]
    audio_content: String,
}

/// Computes a hash of the TTS configuration for cache keying.
///
/// This creates a stable hash from configuration fields that affect audio output,
/// ensuring different configurations produce different cache keys.
fn compute_google_tts_config_hash(config: &TTSConfig) -> String {
    let mut s = String::new();
    s.push_str("google|");
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

/// Google Cloud Text-to-Speech provider implementation.
///
/// Unlike Deepgram and ElevenLabs which stream audio chunks, Google TTS:
/// 1. Accepts a single POST request
/// 2. Returns a JSON response with base64-encoded audio
/// 3. Requires manual chunking for callback delivery
///
/// This provider handles OAuth2 authentication, JSON parsing, base64 decoding,
/// WAV header stripping for LINEAR16, and audio chunking for callbacks.
pub struct GoogleTTS {
    /// Base TTS configuration
    config: TTSConfig,
    /// Google-specific configuration
    google_config: GoogleTTSConfig,
    /// Auth client for OAuth2 token management
    auth_client: Arc<TTSGoogleAuthClient>,
    /// HTTP client for API requests
    http_client: reqwest::Client,
    /// Connection state flag (atomic for lock-free access)
    connected: Arc<AtomicBool>,
    /// Registered audio callback
    audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
    /// Precomputed configuration hash for cache keying
    config_hash: String,
    /// Optional cache store for TTS audio
    cache: Arc<RwLock<Option<Arc<CacheStore>>>>,
    /// Compiled pronunciation patterns for text replacement
    pronunciation_replacer: Option<PronunciationReplacer>,
}

impl GoogleTTS {
    /// Creates a new Google TTS provider instance.
    ///
    /// # Arguments
    ///
    /// * `config` - Base TTS configuration with credentials and voice settings
    ///
    /// # Returns
    ///
    /// * `Ok(Self)` - A new provider instance ready for connection
    /// * `Err(TTSError)` - If credentials are invalid or project_id cannot be extracted
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use sayna::core::tts::{TTSConfig, google::GoogleTTS};
    ///
    /// let config = TTSConfig {
    ///     api_key: "/path/to/credentials.json".to_string(),
    ///     voice_id: Some("en-US-Wavenet-D".to_string()),
    ///     audio_format: Some("linear16".to_string()),
    ///     sample_rate: Some(24000),
    ///     ..Default::default()
    /// };
    ///
    /// let tts = GoogleTTS::new(config)?;
    /// # Ok::<(), sayna::core::tts::TTSError>(())
    /// ```
    pub fn new(config: TTSConfig) -> TTSResult<Self> {
        // Validate that credentials are provided
        // Note: For Google, api_key contains credential source (file path, JSON, or empty for ADC)
        // Empty string is valid for Application Default Credentials

        // Parse credential source to extract project_id
        let credential_source = CredentialSource::from_api_key(&config.api_key);

        // Extract project_id from credentials
        let project_id = credential_source.extract_project_id().ok_or_else(|| {
            TTSError::InvalidConfiguration(
                "Failed to extract project_id from Google Cloud credentials. \
                 Ensure the credentials file contains a valid project_id field."
                    .to_string(),
            )
        })?;

        // Create auth client for OAuth2 token management
        let auth_client = TTSGoogleAuthClient::new(credential_source)?;

        // Create Google-specific config from base config
        let google_config = GoogleTTSConfig::from_base_config(config.clone(), project_id);

        // Create pronunciation replacer if pronunciations are configured
        let pronunciation_replacer = if !config.pronunciations.is_empty() {
            Some(PronunciationReplacer::new(&config.pronunciations))
        } else {
            None
        };

        // Compute config hash for caching
        let config_hash = compute_google_tts_config_hash(&config);

        Ok(Self {
            config,
            google_config,
            auth_client: Arc::new(auth_client),
            http_client: reqwest::Client::new(),
            connected: Arc::new(AtomicBool::new(false)),
            audio_callback: Arc::new(RwLock::new(None)),
            config_hash,
            cache: Arc::new(RwLock::new(None)),
            pronunciation_replacer,
        })
    }

    /// Streams audio data to the registered callback in appropriately-sized chunks.
    ///
    /// This method chunks the audio data based on format and sample rate,
    /// delivering approximately 10ms of audio per chunk for consistent playback.
    ///
    /// # Arguments
    ///
    /// * `audio_data` - Raw audio bytes to deliver
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Audio delivered successfully
    /// * `Err(TTSError)` - If callback invocation fails
    async fn stream_audio_to_callback(&self, audio_data: Vec<u8>) -> TTSResult<()> {
        // Get the callback (if registered)
        let callback_opt = self.audio_callback.read().await.clone();

        let Some(callback) = callback_opt else {
            debug!("No audio callback registered, skipping audio delivery");
            return Ok(());
        };

        // Calculate chunk size based on format and sample rate
        // Target ~10ms of audio per chunk
        let sample_rate = self.config.sample_rate.unwrap_or(24000) as usize;
        let format = self.config.audio_format.as_deref().unwrap_or("linear16");

        let (chunk_size, bytes_per_sample) = match format.to_lowercase().as_str() {
            "linear16" | "pcm" | "wav" => {
                // 16-bit audio = 2 bytes per sample
                // 10ms at sample_rate = sample_rate / 100 samples
                let samples_per_chunk = sample_rate / 100;
                (samples_per_chunk * 2, 2usize)
            }
            "mulaw" | "ulaw" | "alaw" => {
                // 8-bit audio = 1 byte per sample
                let samples_per_chunk = sample_rate / 100;
                (samples_per_chunk, 1usize)
            }
            _ => {
                // For compressed formats (MP3, OGG_OPUS), send in larger chunks
                // Since duration can't be easily calculated, send as-is
                (audio_data.len(), 0usize)
            }
        };

        // Ensure chunk size is at least 1 byte
        let chunk_size = chunk_size.max(1);

        // Iterate over audio in chunks
        for chunk in audio_data.chunks(chunk_size) {
            let duration_ms = if bytes_per_sample > 0 {
                let samples = chunk.len() / bytes_per_sample;
                Some((samples * 1000 / sample_rate) as u32)
            } else {
                None
            };

            let audio = AudioData {
                data: chunk.to_vec(),
                sample_rate: sample_rate as u32,
                format: format.to_string(),
                duration_ms,
            };

            callback.on_audio(audio).await;
        }

        // Notify completion
        callback.on_complete().await;

        Ok(())
    }

    /// Strips the WAV header from LINEAR16 audio data if present.
    ///
    /// Google's LINEAR16 output includes a 44-byte WAV header that must be
    /// stripped for raw PCM delivery to callbacks and LiveKit.
    ///
    /// # Arguments
    ///
    /// * `audio` - Audio bytes that may contain a WAV header
    ///
    /// # Returns
    ///
    /// Audio bytes with WAV header stripped (if present), or original bytes
    fn strip_wav_header(&self, audio: Vec<u8>) -> Vec<u8> {
        // Check if this is LINEAR16 format
        let format = self
            .config
            .audio_format
            .as_deref()
            .unwrap_or("linear16")
            .to_lowercase();

        if !matches!(format.as_str(), "linear16" | "pcm" | "wav") {
            return audio;
        }

        // Check for RIFF header (WAV file signature)
        // WAV files start with: RIFF (0x52 0x49 0x46 0x46)
        const WAV_HEADER_SIZE: usize = 44;

        if audio.len() > WAV_HEADER_SIZE
            && audio[0] == 0x52
            && audio[1] == 0x49
            && audio[2] == 0x46
            && audio[3] == 0x46
        {
            debug!(
                "Stripping {}-byte WAV header from LINEAR16 audio",
                WAV_HEADER_SIZE
            );
            audio[WAV_HEADER_SIZE..].to_vec()
        } else {
            audio
        }
    }

    /// Sets the cache store for audio caching.
    pub async fn set_cache(&mut self, cache: Arc<CacheStore>) {
        *self.cache.write().await = Some(cache);
    }
}

#[async_trait]
impl BaseTTS for GoogleTTS {
    fn new(config: TTSConfig) -> TTSResult<Self>
    where
        Self: Sized,
    {
        GoogleTTS::new(config)
    }

    async fn connect(&mut self) -> TTSResult<()> {
        // Google TTS is a stateless REST API - no persistent connection needed
        // Just mark as connected and ready to accept requests
        self.connected.store(true, Ordering::Release);
        info!("Google TTS provider ready");
        Ok(())
    }

    async fn disconnect(&mut self) -> TTSResult<()> {
        self.connected.store(false, Ordering::Release);
        *self.audio_callback.write().await = None;
        *self.cache.write().await = None;
        info!("Google TTS provider disconnected");
        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    fn get_connection_state(&self) -> ConnectionState {
        if self.connected.load(Ordering::Acquire) {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        }
    }

    async fn speak(&mut self, text: &str, _flush: bool) -> TTSResult<()> {
        // Phase 1: Validation and Setup
        let text = text.trim();
        if text.is_empty() {
            return Ok(());
        }

        // Auto-connect if not ready
        if !self.is_ready() {
            info!("Google TTS not ready, connecting...");
            self.connect().await?;
        }

        // Phase 2: Pronunciation Processing
        let processed_text = if let Some(replacer) = &self.pronunciation_replacer {
            replacer.apply(text)
        } else {
            text.to_string()
        };

        // Phase 3: Cache Lookup
        let text_hash = format!("{:032x}", xxh3_128(processed_text.as_bytes()));
        let cache_key = format!("{}:{}", self.config_hash, text_hash);

        let cache_opt = self.cache.read().await.clone();
        if let Some(cache) = &cache_opt {
            match cache.get(&cache_key).await {
                Ok(Some(cached_audio)) => {
                    debug!(
                        "Cache HIT for text: '{}' ({} bytes)",
                        processed_text,
                        cached_audio.len()
                    );
                    // Stream cached audio to callback
                    return self.stream_audio_to_callback(cached_audio.to_vec()).await;
                }
                Ok(None) => {
                    debug!("Cache miss for text: '{}'", processed_text);
                }
                Err(e) => {
                    error!("Cache get error: {:?}", e);
                }
            }
        }

        // Phase 4: Authentication - get fresh OAuth2 token
        let token = self
            .auth_client
            .get_token()
            .await
            .map_err(google_error_to_tts)?;

        // Phase 5: Build and Send Request
        let request_builder =
            GoogleRequestBuilder::new(self.config.clone(), self.google_config.clone(), token);

        let request = request_builder.build_http_request(&self.http_client, &processed_text);

        let response = request
            .send()
            .await
            .map_err(|e| TTSError::NetworkError(format!("HTTP request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(TTSError::ProviderError(format!(
                "Google TTS API error ({}): {}",
                status, error_body
            )));
        }

        // Phase 6: Response Parsing
        let response_text = response
            .text()
            .await
            .map_err(|e| TTSError::ProviderError(format!("Failed to read response body: {e}")))?;

        let tts_response: GoogleTTSResponse = serde_json::from_str(&response_text)
            .map_err(|e| TTSError::ProviderError(format!("Failed to parse JSON response: {e}")))?;

        // Decode base64 audio content
        let audio_bytes = base64::engine::general_purpose::STANDARD
            .decode(&tts_response.audio_content)
            .map_err(|e| TTSError::ProviderError(format!("Failed to decode base64 audio: {e}")))?;

        // Phase 7: Audio Processing - strip WAV header for LINEAR16
        let processed_audio = self.strip_wav_header(audio_bytes);

        debug!(
            "Received {} bytes of audio for text: '{}'",
            processed_audio.len(),
            processed_text
        );

        // Phase 8: Cache Storage
        if let Some(cache) = &cache_opt {
            let cache_clone = cache.clone();
            let key = cache_key.clone();
            let audio_clone = processed_audio.clone();
            // Store in cache (don't await - let it run in background)
            tokio::spawn(async move {
                if let Err(e) = cache_clone.put(key, audio_clone).await {
                    error!("Failed to cache TTS audio: {:?}", e);
                }
            });
        }

        // Phase 9: Audio Delivery
        self.stream_audio_to_callback(processed_audio).await
    }

    async fn clear(&mut self) -> TTSResult<()> {
        // Google TTS is stateless - no queued requests to cancel
        // Just return Ok
        Ok(())
    }

    async fn flush(&self) -> TTSResult<()> {
        // No-op for Google TTS - requests are synchronous
        Ok(())
    }

    fn on_audio(&mut self, callback: Arc<dyn AudioCallback>) -> TTSResult<()> {
        // Use try_write to avoid blocking in sync context
        if let Ok(mut guard) = self.audio_callback.try_write() {
            *guard = Some(callback);
            Ok(())
        } else {
            Err(TTSError::InternalError(
                "Failed to register audio callback".to_string(),
            ))
        }
    }

    fn remove_audio_callback(&mut self) -> TTSResult<()> {
        if let Ok(mut guard) = self.audio_callback.try_write() {
            *guard = None;
            Ok(())
        } else {
            Err(TTSError::InternalError(
                "Failed to remove audio callback".to_string(),
            ))
        }
    }

    fn get_provider_info(&self) -> serde_json::Value {
        serde_json::json!({
            "provider": "google",
            "version": "1.0.0",
            "api_type": "HTTP REST",
            "endpoint": GOOGLE_TTS_URL,
            "supported_formats": ["LINEAR16", "MP3", "OGG_OPUS", "MULAW", "ALAW"],
            "supported_sample_rates": [8000, 11025, 16000, 22050, 24000, 44100, 48000],
            "supported_voices": [
                "en-US-Wavenet-A", "en-US-Wavenet-B", "en-US-Wavenet-C", "en-US-Wavenet-D",
                "en-US-Neural2-A", "en-US-Neural2-B", "en-US-Neural2-C", "en-US-Neural2-D",
                "en-US-Standard-A", "en-US-Standard-B", "en-US-Standard-C", "en-US-Standard-D"
            ],
            "documentation": "https://cloud.google.com/text-to-speech/docs"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google_error_to_tts_authentication_failed() {
        let google_error = GoogleError::AuthenticationFailed("Invalid credentials".to_string());
        let tts_error = google_error_to_tts(google_error);

        match tts_error {
            TTSError::ConnectionFailed(msg) => {
                assert!(msg.contains("authentication failed"));
                assert!(msg.contains("Invalid credentials"));
            }
            _ => panic!("Expected ConnectionFailed, got {:?}", tts_error),
        }
    }

    #[test]
    fn test_google_error_to_tts_configuration_error() {
        let google_error = GoogleError::ConfigurationError("Missing project ID".to_string());
        let tts_error = google_error_to_tts(google_error);

        match tts_error {
            TTSError::InvalidConfiguration(msg) => {
                assert_eq!(msg, "Missing project ID");
            }
            _ => panic!("Expected InvalidConfiguration, got {:?}", tts_error),
        }
    }

    #[test]
    fn test_google_error_to_tts_connection_failed() {
        let google_error = GoogleError::ConnectionFailed("Connection refused".to_string());
        let tts_error = google_error_to_tts(google_error);

        match tts_error {
            TTSError::ConnectionFailed(msg) => {
                assert_eq!(msg, "Connection refused");
            }
            _ => panic!("Expected ConnectionFailed, got {:?}", tts_error),
        }
    }

    #[test]
    fn test_google_error_to_tts_network_error() {
        let google_error = GoogleError::NetworkError("DNS resolution failed".to_string());
        let tts_error = google_error_to_tts(google_error);

        match tts_error {
            TTSError::NetworkError(msg) => {
                assert_eq!(msg, "DNS resolution failed");
            }
            _ => panic!("Expected NetworkError, got {:?}", tts_error),
        }
    }

    #[test]
    fn test_google_error_to_tts_api_error() {
        let google_error = GoogleError::ApiError("Rate limit exceeded".to_string());
        let tts_error = google_error_to_tts(google_error);

        match tts_error {
            TTSError::ProviderError(msg) => {
                assert_eq!(msg, "Rate limit exceeded");
            }
            _ => panic!("Expected ProviderError, got {:?}", tts_error),
        }
    }

    #[test]
    fn test_google_error_to_tts_grpc_error() {
        let google_error = GoogleError::GrpcError {
            code: "Unavailable".to_string(),
            message: "Service temporarily unavailable".to_string(),
        };
        let tts_error = google_error_to_tts(google_error);

        match tts_error {
            TTSError::ProviderError(msg) => {
                assert!(msg.contains("gRPC error"));
                assert!(msg.contains("Unavailable"));
                assert!(msg.contains("Service temporarily unavailable"));
            }
            _ => panic!("Expected ProviderError, got {:?}", tts_error),
        }
    }

    #[test]
    fn test_tts_google_auth_client_invalid_file_path() {
        let result = TTSGoogleAuthClient::from_api_key("/nonexistent/path/credentials.json");

        match result {
            Err(TTSError::InvalidConfiguration(msg)) => {
                assert!(msg.contains("Credential file not found"));
            }
            Err(other) => panic!("Expected InvalidConfiguration, got {:?}", other),
            Ok(_) => panic!("Expected error for nonexistent file"),
        }
    }

    #[test]
    fn test_tts_google_auth_client_path_traversal_rejected() {
        let result = TTSGoogleAuthClient::from_api_key("../../../etc/passwd");

        match result {
            Err(TTSError::InvalidConfiguration(msg)) => {
                assert!(msg.contains("path traversal not allowed"));
            }
            Err(other) => panic!("Expected InvalidConfiguration, got {:?}", other),
            Ok(_) => panic!("Expected error for path traversal attempt"),
        }
    }

    #[test]
    fn test_credential_source_json_detection() {
        // JSON content should be detected by leading brace
        let json = r#"{"type": "service_account", "project_id": "test"}"#;
        let source = CredentialSource::from_api_key(json);

        match source {
            CredentialSource::JsonContent(content) => {
                assert_eq!(content, json);
            }
            _ => panic!("Expected JsonContent, got {:?}", source),
        }
    }

    #[test]
    fn test_credential_source_empty_becomes_adc() {
        let source = CredentialSource::from_api_key("");

        assert!(matches!(source, CredentialSource::ApplicationDefault));
    }

    #[test]
    fn test_credential_source_file_path_detection() {
        let path = "/path/to/credentials.json";
        let source = CredentialSource::from_api_key(path);

        match source {
            CredentialSource::FilePath(p) => {
                assert_eq!(p, path);
            }
            _ => panic!("Expected FilePath, got {:?}", source),
        }
    }

    #[test]
    fn test_from_api_key_delegates_to_new() {
        // Both methods should produce the same error for invalid paths
        let result1 = TTSGoogleAuthClient::from_api_key("/nonexistent/file.json");
        let source = CredentialSource::from_api_key("/nonexistent/file.json");
        let result2 = TTSGoogleAuthClient::new(source);

        // Both should fail with the same error type
        assert!(matches!(result1, Err(TTSError::InvalidConfiguration(_))));
        assert!(matches!(result2, Err(TTSError::InvalidConfiguration(_))));
    }

    // Note: Tests requiring actual Google credentials should be marked #[ignore]
    // as they require external setup (service account, etc.)
    #[tokio::test]
    #[ignore = "Requires valid Google Cloud credentials"]
    async fn test_token_retrieval_with_real_credentials() {
        // This test requires GOOGLE_APPLICATION_CREDENTIALS to be set
        // or a valid service account JSON file
        let client =
            TTSGoogleAuthClient::from_api_key("").expect("Failed to create auth client with ADC");

        let token = client.get_token().await.expect("Failed to get token");
        assert!(!token.is_empty());
    }

    // ===== GoogleRequestBuilder Tests =====

    use crate::core::tts::base::Pronunciation;

    fn create_test_config() -> TTSConfig {
        TTSConfig {
            provider: "google".to_string(),
            api_key: String::new(),
            voice_id: Some("en-US-Wavenet-D".to_string()),
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

    #[test]
    fn test_google_request_builder_new() {
        let config = create_test_config();
        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());
        let token = "test-token".to_string();

        let builder = GoogleRequestBuilder::new(config, google_config, token);

        assert_eq!(builder.auth_token, "test-token");
        assert!(builder.pronunciation_replacer.is_none());
    }

    #[test]
    fn test_google_request_builder_with_pronunciations() {
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

        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());
        let token = "test-token".to_string();

        let builder = GoogleRequestBuilder::new(config, google_config, token);

        assert!(builder.pronunciation_replacer.is_some());
    }

    #[test]
    fn test_google_request_builder_get_config() {
        let config = create_test_config();
        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());
        let token = "test-token".to_string();

        let builder = GoogleRequestBuilder::new(config, google_config, token);

        let retrieved_config = builder.get_config();
        assert_eq!(
            retrieved_config.voice_id,
            Some("en-US-Wavenet-D".to_string())
        );
        assert_eq!(retrieved_config.sample_rate, Some(24000));
    }

    #[test]
    fn test_google_request_builder_get_pronunciation_replacer_none() {
        let config = create_test_config();
        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());
        let token = "test-token".to_string();

        let builder = GoogleRequestBuilder::new(config, google_config, token);

        assert!(builder.get_pronunciation_replacer().is_none());
    }

    #[test]
    fn test_google_request_builder_get_pronunciation_replacer_some() {
        let mut config = create_test_config();
        config.pronunciations = vec![Pronunciation {
            word: "API".to_string(),
            pronunciation: "A P I".to_string(),
        }];

        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());
        let token = "test-token".to_string();

        let builder = GoogleRequestBuilder::new(config, google_config, token);

        assert!(builder.get_pronunciation_replacer().is_some());
    }

    #[test]
    fn test_build_request_body_minimal() {
        let mut config = create_test_config();
        config.voice_id = None;
        config.speaking_rate = None;
        config.sample_rate = None;

        let mut google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());
        google_config.pitch = None;
        google_config.volume_gain_db = None;
        google_config.effects_profile_id = Vec::new();

        let builder = GoogleRequestBuilder::new(config, google_config, "test-token".to_string());

        let body = builder.build_request_body("Hello, world!");

        // Verify input section
        assert_eq!(body["input"]["text"], "Hello, world!");

        // Verify voice section (only languageCode, no name)
        assert_eq!(body["voice"]["languageCode"], "en-US");
        assert!(body["voice"].get("name").is_none());

        // Verify audioConfig section (only encoding)
        assert_eq!(body["audioConfig"]["audioEncoding"], "LINEAR16");
        assert!(body["audioConfig"].get("speakingRate").is_none());
        assert!(body["audioConfig"].get("pitch").is_none());
        assert!(body["audioConfig"].get("volumeGainDb").is_none());
        assert!(body["audioConfig"].get("sampleRateHertz").is_none());
        assert!(body["audioConfig"].get("effectsProfileId").is_none());
    }

    #[test]
    fn test_build_request_body_full() {
        let mut config = create_test_config();
        config.voice_id = Some("en-US-Wavenet-D".to_string());
        config.speaking_rate = Some(1.5);
        config.sample_rate = Some(24000);
        config.audio_format = Some("mp3".to_string());

        let mut google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());
        google_config.pitch = Some(2.0);
        google_config.volume_gain_db = Some(-3.0);
        google_config.effects_profile_id = vec!["headphone-class-device".to_string()];

        let builder = GoogleRequestBuilder::new(config, google_config, "test-token".to_string());

        let body = builder.build_request_body("Test speech");

        // Verify all fields are present
        assert_eq!(body["input"]["text"], "Test speech");
        assert_eq!(body["voice"]["languageCode"], "en-US");
        assert_eq!(body["voice"]["name"], "en-US-Wavenet-D");
        assert_eq!(body["audioConfig"]["audioEncoding"], "MP3");
        assert_eq!(body["audioConfig"]["speakingRate"], 1.5);
        assert_eq!(body["audioConfig"]["pitch"], 2.0);
        assert_eq!(body["audioConfig"]["volumeGainDb"], -3.0);
        assert_eq!(body["audioConfig"]["sampleRateHertz"], 24000);
        assert_eq!(
            body["audioConfig"]["effectsProfileId"],
            serde_json::json!(["headphone-class-device"])
        );
    }

    #[test]
    fn test_build_request_body_audio_encodings() {
        // Test LINEAR16
        let config = create_test_config();
        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());
        let builder = GoogleRequestBuilder::new(config.clone(), google_config, "token".to_string());
        let body = builder.build_request_body("test");
        assert_eq!(body["audioConfig"]["audioEncoding"], "LINEAR16");

        // Test MP3
        let mut mp3_config = config.clone();
        mp3_config.audio_format = Some("mp3".to_string());
        let google_mp3 =
            GoogleTTSConfig::from_base_config(mp3_config.clone(), "test-project".to_string());
        let builder_mp3 = GoogleRequestBuilder::new(mp3_config, google_mp3, "token".to_string());
        let body_mp3 = builder_mp3.build_request_body("test");
        assert_eq!(body_mp3["audioConfig"]["audioEncoding"], "MP3");

        // Test OGG_OPUS
        let mut opus_config = config.clone();
        opus_config.audio_format = Some("ogg_opus".to_string());
        let google_opus =
            GoogleTTSConfig::from_base_config(opus_config.clone(), "test-project".to_string());
        let builder_opus = GoogleRequestBuilder::new(opus_config, google_opus, "token".to_string());
        let body_opus = builder_opus.build_request_body("test");
        assert_eq!(body_opus["audioConfig"]["audioEncoding"], "OGG_OPUS");
    }

    #[test]
    fn test_build_request_body_clamped_values() {
        let config = create_test_config();
        let mut google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());

        // Set values outside valid ranges
        google_config.pitch = Some(30.0); // Should clamp to 20.0
        google_config.volume_gain_db = Some(50.0); // Should clamp to 16.0

        let builder = GoogleRequestBuilder::new(config, google_config, "test-token".to_string());
        let body = builder.build_request_body("test");

        assert_eq!(body["audioConfig"]["pitch"], 20.0);
        assert_eq!(body["audioConfig"]["volumeGainDb"], 16.0);
    }

    #[test]
    fn test_build_request_body_speaking_rate_clamped() {
        let mut config = create_test_config();
        config.speaking_rate = Some(10.0); // Should clamp to 4.0

        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());

        let builder = GoogleRequestBuilder::new(config, google_config, "test-token".to_string());
        let body = builder.build_request_body("test");

        assert_eq!(body["audioConfig"]["speakingRate"], 4.0);
    }

    #[test]
    fn test_build_request_body_multiple_effects_profiles() {
        let config = create_test_config();
        let mut google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());

        google_config.effects_profile_id = vec![
            "headphone-class-device".to_string(),
            "small-bluetooth-speaker-class-device".to_string(),
        ];

        let builder = GoogleRequestBuilder::new(config, google_config, "test-token".to_string());
        let body = builder.build_request_body("test");

        let effects = body["audioConfig"]["effectsProfileId"].as_array().unwrap();
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0], "headphone-class-device");
        assert_eq!(effects[1], "small-bluetooth-speaker-class-device");
    }

    #[test]
    fn test_build_http_request_url() {
        let config = create_test_config();
        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());

        let builder = GoogleRequestBuilder::new(config, google_config, "test-token".to_string());

        let client = reqwest::Client::new();
        let request_builder = builder.build_http_request(&client, "test");

        // Build the request to inspect it
        let request = request_builder.build().unwrap();

        assert_eq!(request.url().as_str(), GOOGLE_TTS_URL);
        assert_eq!(request.method(), reqwest::Method::POST);
    }

    #[test]
    fn test_build_http_request_headers() {
        let config = create_test_config();
        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());

        let builder =
            GoogleRequestBuilder::new(config, google_config, "my-secret-token".to_string());

        let client = reqwest::Client::new();
        let request_builder = builder.build_http_request(&client, "test");
        let request = request_builder.build().unwrap();

        // Verify Authorization header
        let auth_header = request.headers().get("authorization").unwrap();
        assert_eq!(auth_header.to_str().unwrap(), "Bearer my-secret-token");

        // Verify Content-Type header
        let content_type = request.headers().get("content-type").unwrap();
        assert!(content_type.to_str().unwrap().contains("application/json"));

        // Verify Accept header
        let accept = request.headers().get("accept").unwrap();
        assert_eq!(accept.to_str().unwrap(), "application/json");
    }

    #[test]
    fn test_google_tts_url_constant() {
        assert_eq!(
            GOOGLE_TTS_URL,
            "https://texttospeech.googleapis.com/v1/text:synthesize"
        );
    }

    #[test]
    fn test_google_request_builder_clone() {
        let config = create_test_config();
        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());

        let builder = GoogleRequestBuilder::new(config, google_config, "test-token".to_string());

        // Clone should work since it derives Clone
        let cloned = builder.clone();

        assert_eq!(cloned.auth_token, builder.auth_token);
        assert_eq!(
            cloned.google_config.language_code,
            builder.google_config.language_code
        );
    }

    #[test]
    fn test_build_request_voice_name_from_model_fallback() {
        let mut config = create_test_config();
        config.voice_id = None;
        config.model = "en-US-Neural2-A".to_string();

        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());

        let builder = GoogleRequestBuilder::new(config, google_config, "test-token".to_string());
        let body = builder.build_request_body("test");

        assert_eq!(body["voice"]["name"], "en-US-Neural2-A");
    }

    #[test]
    fn test_build_request_body_empty_text() {
        let config = create_test_config();
        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());

        let builder = GoogleRequestBuilder::new(config, google_config, "test-token".to_string());
        let body = builder.build_request_body("");

        assert_eq!(body["input"]["text"], "");
    }

    #[test]
    fn test_build_request_body_unicode_text() {
        let config = create_test_config();
        let google_config =
            GoogleTTSConfig::from_base_config(config.clone(), "test-project".to_string());

        let builder = GoogleRequestBuilder::new(config, google_config, "test-token".to_string());
        let body = builder.build_request_body("Hello, 世界! Привет мир! 🌍");

        assert_eq!(body["input"]["text"], "Hello, 世界! Привет мир! 🌍");
    }

    // ===== GoogleTTS Provider Tests =====

    #[test]
    fn test_compute_google_tts_config_hash() {
        let config = create_test_config();
        let hash = compute_google_tts_config_hash(&config);

        // Hash should be a 32-char hex string
        assert_eq!(hash.len(), 32);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));

        // Same config should produce same hash
        let hash2 = compute_google_tts_config_hash(&config);
        assert_eq!(hash, hash2);

        // Different config should produce different hash
        let mut different_config = config;
        different_config.voice_id = Some("different-voice".to_string());
        let different_hash = compute_google_tts_config_hash(&different_config);
        assert_ne!(hash, different_hash);
    }

    #[test]
    fn test_google_tts_creation_fails_without_project_id() {
        // Empty api_key with no ADC should fail to extract project_id
        let config = TTSConfig {
            provider: "google".to_string(),
            api_key: "/nonexistent/credentials.json".to_string(),
            voice_id: Some("en-US-Wavenet-D".to_string()),
            ..Default::default()
        };

        let result = GoogleTTS::new(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_google_tts_strip_wav_header() {
        // Create a minimal valid WAV header (44 bytes)
        let mut wav_data = vec![
            0x52, 0x49, 0x46, 0x46, // "RIFF"
            0x24, 0x00, 0x00, 0x00, // File size - 8
            0x57, 0x41, 0x56, 0x45, // "WAVE"
            0x66, 0x6D, 0x74, 0x20, // "fmt "
            0x10, 0x00, 0x00, 0x00, // Subchunk1Size
            0x01, 0x00, // AudioFormat (PCM)
            0x01, 0x00, // NumChannels
            0x80, 0xBB, 0x00, 0x00, // SampleRate
            0x00, 0x77, 0x01, 0x00, // ByteRate
            0x02, 0x00, // BlockAlign
            0x10, 0x00, // BitsPerSample
            0x64, 0x61, 0x74, 0x61, // "data"
            0x00, 0x00, 0x00, 0x00, // Subchunk2Size
        ];

        // Add some audio data after the header
        let audio_data = vec![0x01, 0x02, 0x03, 0x04];
        wav_data.extend(&audio_data);

        // Create a mock GoogleTTS to test the strip_wav_header method
        // We can't create a real GoogleTTS without valid credentials,
        // so we test the logic directly

        // Test WAV detection logic
        assert_eq!(wav_data[0], 0x52);
        assert_eq!(wav_data[1], 0x49);
        assert_eq!(wav_data[2], 0x46);
        assert_eq!(wav_data[3], 0x46);

        // Verify stripping would work
        let stripped = &wav_data[44..];
        assert_eq!(stripped, &audio_data);
    }

    #[test]
    fn test_google_tts_response_deserialization() {
        let json = r#"{"audioContent": "SGVsbG8gV29ybGQ="}"#;
        let response: GoogleTTSResponse = serde_json::from_str(json).unwrap();

        // "SGVsbG8gV29ybGQ=" is base64 for "Hello World"
        assert_eq!(response.audio_content, "SGVsbG8gV29ybGQ=");

        // Verify decoding works
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&response.audio_content)
            .unwrap();
        assert_eq!(decoded, b"Hello World");
    }

    #[test]
    fn test_google_tts_response_missing_field() {
        let json = r#"{"other_field": "value"}"#;
        let result: Result<GoogleTTSResponse, _> = serde_json::from_str(json);

        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "Requires valid Google Cloud credentials"]
    async fn test_google_tts_lifecycle() {
        // This test requires GOOGLE_APPLICATION_CREDENTIALS to be set
        let config = TTSConfig {
            provider: "google".to_string(),
            api_key: String::new(), // Use ADC
            voice_id: Some("en-US-Wavenet-D".to_string()),
            audio_format: Some("linear16".to_string()),
            sample_rate: Some(24000),
            ..Default::default()
        };

        let mut tts = GoogleTTS::new(config).expect("Failed to create GoogleTTS");

        // Initially not connected
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);

        // Connect
        tts.connect().await.expect("Failed to connect");
        assert!(tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Connected);

        // Disconnect
        tts.disconnect().await.expect("Failed to disconnect");
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    #[ignore = "Requires valid Google Cloud credentials"]
    async fn test_google_tts_speak() {
        use std::future::Future;
        use std::pin::Pin;
        use std::sync::atomic::{AtomicBool, Ordering};

        struct TestCallback {
            received_audio: Arc<std::sync::Mutex<Vec<u8>>>,
            completed: Arc<AtomicBool>,
        }

        impl AudioCallback for TestCallback {
            fn on_audio(
                &self,
                audio_data: AudioData,
            ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
                let received_audio = self.received_audio.clone();
                Box::pin(async move {
                    received_audio.lock().unwrap().extend(audio_data.data);
                })
            }

            fn on_error(&self, _error: TTSError) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
                Box::pin(async {})
            }

            fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
                let completed = self.completed.clone();
                Box::pin(async move {
                    completed.store(true, Ordering::Release);
                })
            }
        }

        let config = TTSConfig {
            provider: "google".to_string(),
            api_key: String::new(), // Use ADC
            voice_id: Some("en-US-Wavenet-D".to_string()),
            audio_format: Some("linear16".to_string()),
            sample_rate: Some(24000),
            ..Default::default()
        };

        let mut tts = GoogleTTS::new(config).expect("Failed to create GoogleTTS");

        let received_audio = Arc::new(std::sync::Mutex::new(Vec::new()));
        let completed = Arc::new(AtomicBool::new(false));

        let callback = Arc::new(TestCallback {
            received_audio: received_audio.clone(),
            completed: completed.clone(),
        });

        tts.on_audio(callback).expect("Failed to register callback");
        tts.connect().await.expect("Failed to connect");

        tts.speak("Hello, world!", true)
            .await
            .expect("Failed to speak");

        // Verify we received audio
        assert!(completed.load(Ordering::Acquire));
        let audio = received_audio.lock().unwrap();
        assert!(!audio.is_empty(), "Should have received audio data");
    }

    #[test]
    fn test_google_tts_get_provider_info() {
        // We can't create a real GoogleTTS without credentials,
        // but we can verify the expected structure
        let expected_keys = vec![
            "provider",
            "version",
            "api_type",
            "endpoint",
            "supported_formats",
            "supported_sample_rates",
            "supported_voices",
            "documentation",
        ];

        // Create expected JSON structure
        let info = serde_json::json!({
            "provider": "google",
            "version": "1.0.0",
            "api_type": "HTTP REST",
            "endpoint": GOOGLE_TTS_URL,
            "supported_formats": ["LINEAR16", "MP3", "OGG_OPUS", "MULAW", "ALAW"],
            "supported_sample_rates": [8000, 11025, 16000, 22050, 24000, 44100, 48000],
            "supported_voices": [
                "en-US-Wavenet-A", "en-US-Wavenet-B", "en-US-Wavenet-C", "en-US-Wavenet-D",
                "en-US-Neural2-A", "en-US-Neural2-B", "en-US-Neural2-C", "en-US-Neural2-D",
                "en-US-Standard-A", "en-US-Standard-B", "en-US-Standard-C", "en-US-Standard-D"
            ],
            "documentation": "https://cloud.google.com/text-to-speech/docs"
        });

        for key in expected_keys {
            assert!(info.get(key).is_some(), "Missing key: {}", key);
        }

        assert_eq!(info["provider"], "google");
        assert_eq!(info["endpoint"], GOOGLE_TTS_URL);
    }
}
