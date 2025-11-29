//! Configuration types for Cartesia Text-to-Speech API.
//!
//! This module contains all configuration-related types including:
//! - Audio container format specifications (`CartesiaAudioContainer`)
//! - Audio encoding format specifications (`CartesiaAudioEncoding`)
//! - Output format configuration (`CartesiaOutputFormat`)
//! - Provider-specific configuration options (`CartesiaTTSConfig`)
//!
//! # API Overview
//!
//! Cartesia TTS uses a REST API at `POST https://api.cartesia.ai/tts/bytes` with JSON
//! request bodies. The output format is specified as a discriminated union based on
//! the container type (raw, wav, or mp3).
//!
//! # Example
//!
//! ```rust,ignore
//! use sayna::core::tts::cartesia::{CartesiaTTSConfig, CartesiaOutputFormat};
//! use sayna::core::tts::TTSConfig;
//!
//! let base = TTSConfig {
//!     api_key: "your-api-key".to_string(),
//!     voice_id: Some("a0e99841-438c-4a64-b679-ae501e7d6091".to_string()),
//!     audio_format: Some("linear16".to_string()),
//!     sample_rate: Some(24000),
//!     ..Default::default()
//! };
//!
//! let config = CartesiaTTSConfig::from_base(base);
//! assert_eq!(config.model, "sonic-3");
//! ```

use crate::core::tts::base::{TTSConfig, TTSError};
use serde::{Deserialize, Serialize};

// =============================================================================
// Constants
// =============================================================================

/// Cartesia TTS REST API endpoint for byte streaming.
pub const CARTESIA_TTS_URL: &str = "https://api.cartesia.ai/tts/bytes";

/// Default API version for Cartesia TTS requests.
/// Format: YYYY-MM-DD
pub const DEFAULT_API_VERSION: &str = "2025-04-16";

/// Default Sonic model for TTS synthesis.
pub const DEFAULT_MODEL: &str = "sonic-3";

/// Supported sample rates for Cartesia TTS (in Hz).
/// Used for configuration validation.
pub const SUPPORTED_SAMPLE_RATES: &[u32] = &[8000, 16000, 22050, 24000, 44100, 48000];

/// Default MP3 bitrate in bits per second.
const DEFAULT_MP3_BITRATE: u32 = 128000;

// =============================================================================
// Audio Container
// =============================================================================

/// Audio container format for Cartesia TTS output.
///
/// The container type determines the structure of the audio output:
/// - `Raw`: Raw PCM bytes without container - lowest latency for streaming
/// - `Wav`: WAV container with headers - for file storage
/// - `Mp3`: MP3 compressed format - for bandwidth optimization
///
/// # Serialization
///
/// Container values are serialized as lowercase strings for the Cartesia API:
/// - `Raw` → `"raw"`
/// - `Wav` → `"wav"`
/// - `Mp3` → `"mp3"`
///
/// # Example
///
/// ```rust,ignore
/// use sayna::core::tts::cartesia::CartesiaAudioContainer;
///
/// let container = CartesiaAudioContainer::Raw;
/// assert_eq!(container.as_str(), "raw");
/// assert!(container.supports_streaming());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CartesiaAudioContainer {
    /// Raw PCM bytes without container - lowest latency for streaming.
    #[default]
    Raw,
    /// WAV container with headers - for file storage.
    Wav,
    /// MP3 compressed format - for bandwidth optimization.
    Mp3,
}

impl CartesiaAudioContainer {
    /// Returns the string representation for API requests.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::CartesiaAudioContainer;
    ///
    /// assert_eq!(CartesiaAudioContainer::Raw.as_str(), "raw");
    /// assert_eq!(CartesiaAudioContainer::Wav.as_str(), "wav");
    /// assert_eq!(CartesiaAudioContainer::Mp3.as_str(), "mp3");
    /// ```
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
        }
    }

    /// Returns true if this format supports streaming (raw and wav).
    ///
    /// MP3 format may have buffering requirements that make it less suitable
    /// for low-latency streaming applications.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::CartesiaAudioContainer;
    ///
    /// assert!(CartesiaAudioContainer::Raw.supports_streaming());
    /// assert!(CartesiaAudioContainer::Wav.supports_streaming());
    /// assert!(!CartesiaAudioContainer::Mp3.supports_streaming());
    /// ```
    #[inline]
    pub const fn supports_streaming(&self) -> bool {
        matches!(self, Self::Raw | Self::Wav)
    }
}

// =============================================================================
// Audio Encoding
// =============================================================================

/// Audio encoding format for Cartesia TTS PCM output.
///
/// This enum represents the PCM encoding type, which affects audio quality
/// and compatibility with different systems.
///
/// # Encoding Types
///
/// | Encoding | Description | Bytes per Sample | Use Case |
/// |----------|-------------|------------------|----------|
/// | `PcmF32le` | 32-bit float little-endian | 4 | High-precision processing |
/// | `PcmS16le` | 16-bit signed int little-endian | 2 | Standard format (default) |
/// | `PcmAlaw` | A-law companding | 1 | European telephony |
/// | `PcmMulaw` | μ-law companding | 1 | US/Japan telephony |
///
/// # Example
///
/// ```rust,ignore
/// use sayna::core::tts::cartesia::CartesiaAudioEncoding;
///
/// let encoding = CartesiaAudioEncoding::PcmS16le;
/// assert_eq!(encoding.as_str(), "pcm_s16le");
/// assert_eq!(encoding.bytes_per_sample(), 2);
/// assert!(encoding.is_pcm());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CartesiaAudioEncoding {
    /// 32-bit float little-endian PCM - highest precision.
    #[serde(rename = "pcm_f32le")]
    PcmF32le,

    /// 16-bit signed integer little-endian PCM - standard format.
    /// Matches Sayna's "linear16" format for seamless integration.
    #[default]
    #[serde(rename = "pcm_s16le")]
    PcmS16le,

    /// A-law companding - European telephony (8-bit).
    #[serde(rename = "pcm_alaw")]
    PcmAlaw,

    /// μ-law companding - US/Japan telephony (8-bit).
    #[serde(rename = "pcm_mulaw")]
    PcmMulaw,
}

impl CartesiaAudioEncoding {
    /// Returns the API string representation.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::CartesiaAudioEncoding;
    ///
    /// assert_eq!(CartesiaAudioEncoding::PcmF32le.as_str(), "pcm_f32le");
    /// assert_eq!(CartesiaAudioEncoding::PcmS16le.as_str(), "pcm_s16le");
    /// assert_eq!(CartesiaAudioEncoding::PcmAlaw.as_str(), "pcm_alaw");
    /// assert_eq!(CartesiaAudioEncoding::PcmMulaw.as_str(), "pcm_mulaw");
    /// ```
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PcmF32le => "pcm_f32le",
            Self::PcmS16le => "pcm_s16le",
            Self::PcmAlaw => "pcm_alaw",
            Self::PcmMulaw => "pcm_mulaw",
        }
    }

    /// Returns bytes per sample for this encoding.
    /// Used for audio duration calculation in TTSProvider.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::CartesiaAudioEncoding;
    ///
    /// assert_eq!(CartesiaAudioEncoding::PcmF32le.bytes_per_sample(), 4);
    /// assert_eq!(CartesiaAudioEncoding::PcmS16le.bytes_per_sample(), 2);
    /// assert_eq!(CartesiaAudioEncoding::PcmAlaw.bytes_per_sample(), 1);
    /// assert_eq!(CartesiaAudioEncoding::PcmMulaw.bytes_per_sample(), 1);
    /// ```
    #[inline]
    pub const fn bytes_per_sample(&self) -> usize {
        match self {
            Self::PcmF32le => 4,
            Self::PcmS16le => 2,
            Self::PcmAlaw | Self::PcmMulaw => 1,
        }
    }

    /// Returns true if this is a telephony encoding (8-bit μ-law or A-law).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::CartesiaAudioEncoding;
    ///
    /// assert!(!CartesiaAudioEncoding::PcmF32le.is_telephony());
    /// assert!(!CartesiaAudioEncoding::PcmS16le.is_telephony());
    /// assert!(CartesiaAudioEncoding::PcmAlaw.is_telephony());
    /// assert!(CartesiaAudioEncoding::PcmMulaw.is_telephony());
    /// ```
    #[inline]
    pub const fn is_telephony(&self) -> bool {
        matches!(self, Self::PcmAlaw | Self::PcmMulaw)
    }

    /// Returns true if this is a standard PCM encoding (not telephony).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::CartesiaAudioEncoding;
    ///
    /// assert!(CartesiaAudioEncoding::PcmF32le.is_pcm());
    /// assert!(CartesiaAudioEncoding::PcmS16le.is_pcm());
    /// assert!(!CartesiaAudioEncoding::PcmAlaw.is_pcm());
    /// assert!(!CartesiaAudioEncoding::PcmMulaw.is_pcm());
    /// ```
    #[inline]
    pub const fn is_pcm(&self) -> bool {
        matches!(self, Self::PcmF32le | Self::PcmS16le)
    }
}

// =============================================================================
// Output Format
// =============================================================================

/// Complete output format specification for Cartesia TTS API requests.
///
/// The output format is a discriminated union based on the container type:
/// - For `raw` and `wav` containers: requires `encoding` field
/// - For `mp3` container: requires `bit_rate` field
///
/// # JSON Serialization
///
/// Uses `#[serde(skip_serializing_if = "Option::is_none")]` to produce clean JSON
/// without null fields, matching Cartesia API expectations.
///
/// ## Raw PCM Format Example
///
/// ```json
/// {
///   "container": "raw",
///   "encoding": "pcm_s16le",
///   "sample_rate": 24000
/// }
/// ```
///
/// ## MP3 Format Example
///
/// ```json
/// {
///   "container": "mp3",
///   "sample_rate": 24000,
///   "bit_rate": 128000
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CartesiaOutputFormat {
    /// Audio container type (raw, wav, mp3).
    pub container: CartesiaAudioContainer,

    /// PCM encoding type - only for raw and wav containers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<CartesiaAudioEncoding>,

    /// Sample rate in Hz.
    pub sample_rate: u32,

    /// Bit rate in bits per second - only for MP3 container.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bit_rate: Option<u32>,
}

impl CartesiaOutputFormat {
    /// Create a raw PCM format (lowest latency).
    ///
    /// # Arguments
    ///
    /// * `encoding` - The PCM encoding to use
    /// * `sample_rate` - Sample rate in Hz
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::{CartesiaOutputFormat, CartesiaAudioEncoding};
    ///
    /// let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000);
    /// assert_eq!(format.sample_rate(), 24000);
    /// assert!(format.is_pcm());
    /// ```
    pub fn raw(encoding: CartesiaAudioEncoding, sample_rate: u32) -> Self {
        Self {
            container: CartesiaAudioContainer::Raw,
            encoding: Some(encoding),
            sample_rate,
            bit_rate: None,
        }
    }

    /// Create a WAV format.
    ///
    /// # Arguments
    ///
    /// * `encoding` - The PCM encoding to use
    /// * `sample_rate` - Sample rate in Hz
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::{CartesiaOutputFormat, CartesiaAudioEncoding};
    ///
    /// let format = CartesiaOutputFormat::wav(CartesiaAudioEncoding::PcmS16le, 44100);
    /// assert_eq!(format.container, CartesiaAudioContainer::Wav);
    /// ```
    pub fn wav(encoding: CartesiaAudioEncoding, sample_rate: u32) -> Self {
        Self {
            container: CartesiaAudioContainer::Wav,
            encoding: Some(encoding),
            sample_rate,
            bit_rate: None,
        }
    }

    /// Create an MP3 format with specified bitrate.
    ///
    /// # Arguments
    ///
    /// * `sample_rate` - Sample rate in Hz
    /// * `bit_rate` - Bit rate in bits per second
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::CartesiaOutputFormat;
    ///
    /// let format = CartesiaOutputFormat::mp3(24000, 128000);
    /// assert_eq!(format.container, CartesiaAudioContainer::Mp3);
    /// assert_eq!(format.bit_rate, Some(128000));
    /// ```
    pub fn mp3(sample_rate: u32, bit_rate: u32) -> Self {
        Self {
            container: CartesiaAudioContainer::Mp3,
            encoding: None,
            sample_rate,
            bit_rate: Some(bit_rate),
        }
    }

    /// Maps Sayna's base config format strings to Cartesia output format.
    ///
    /// # Arguments
    ///
    /// * `format` - Format string from TTSConfig (linear16, pcm, mp3, mulaw, alaw, wav)
    /// * `sample_rate` - Target sample rate in Hz
    ///
    /// # Mapping Logic
    ///
    /// | Input Format | Container | Encoding |
    /// |--------------|-----------|----------|
    /// | "linear16" or "pcm" | Raw | PcmS16le |
    /// | "mulaw" or "ulaw" | Raw | PcmMulaw |
    /// | "alaw" | Raw | PcmAlaw |
    /// | "wav" | Wav | PcmS16le |
    /// | "mp3" | Mp3 | N/A (uses bitrate) |
    /// | unknown | Raw | PcmS16le |
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::{CartesiaOutputFormat, CartesiaAudioContainer};
    ///
    /// let format = CartesiaOutputFormat::from_format_string("linear16", 24000);
    /// assert_eq!(format.container, CartesiaAudioContainer::Raw);
    /// assert!(format.is_pcm());
    ///
    /// let mp3 = CartesiaOutputFormat::from_format_string("mp3", 24000);
    /// assert_eq!(mp3.container, CartesiaAudioContainer::Mp3);
    /// ```
    pub fn from_format_string(format: &str, sample_rate: u32) -> Self {
        match format.to_lowercase().as_str() {
            "linear16" | "pcm" => Self::raw(CartesiaAudioEncoding::PcmS16le, sample_rate),
            "mulaw" | "ulaw" => Self::raw(CartesiaAudioEncoding::PcmMulaw, sample_rate),
            "alaw" => Self::raw(CartesiaAudioEncoding::PcmAlaw, sample_rate),
            "wav" => Self::wav(CartesiaAudioEncoding::PcmS16le, sample_rate),
            "mp3" => Self::mp3(sample_rate, DEFAULT_MP3_BITRATE),
            _ => Self::default(),
        }
    }

    /// Returns the sample rate.
    #[inline]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Returns true if this is a PCM format suitable for real-time streaming.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::{CartesiaOutputFormat, CartesiaAudioEncoding};
    ///
    /// let raw = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000);
    /// assert!(raw.is_pcm());
    ///
    /// let mp3 = CartesiaOutputFormat::mp3(24000, 128000);
    /// assert!(!mp3.is_pcm());
    /// ```
    #[inline]
    pub fn is_pcm(&self) -> bool {
        matches!(
            self.container,
            CartesiaAudioContainer::Raw | CartesiaAudioContainer::Wav
        ) && self.encoding.is_some_and(|e| e.is_pcm())
    }

    /// Returns true if this is a telephony format (8kHz μ-law or A-law).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::{CartesiaOutputFormat, CartesiaAudioEncoding};
    ///
    /// let mulaw = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmMulaw, 8000);
    /// assert!(mulaw.is_telephony());
    ///
    /// let pcm = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000);
    /// assert!(!pcm.is_telephony());
    /// ```
    #[inline]
    pub fn is_telephony(&self) -> bool {
        self.encoding.is_some_and(|e| e.is_telephony())
    }

    /// Returns the MIME content type for Accept header.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::{CartesiaOutputFormat, CartesiaAudioEncoding};
    ///
    /// let raw = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000);
    /// assert_eq!(raw.content_type(), "application/octet-stream");
    ///
    /// let wav = CartesiaOutputFormat::wav(CartesiaAudioEncoding::PcmS16le, 24000);
    /// assert_eq!(wav.content_type(), "audio/wav");
    ///
    /// let mp3 = CartesiaOutputFormat::mp3(24000, 128000);
    /// assert_eq!(mp3.content_type(), "audio/mpeg");
    /// ```
    #[inline]
    pub fn content_type(&self) -> &'static str {
        match self.container {
            CartesiaAudioContainer::Raw => "application/octet-stream",
            CartesiaAudioContainer::Wav => "audio/wav",
            CartesiaAudioContainer::Mp3 => "audio/mpeg",
        }
    }

    /// Validates the output format configuration.
    ///
    /// # Validation Rules
    ///
    /// 1. Sample rate must be in `SUPPORTED_SAMPLE_RATES`
    /// 2. Raw/Wav containers must have an encoding specified
    /// 3. MP3 container must have a bit_rate specified
    ///
    /// # Returns
    ///
    /// - `Ok(())` if configuration is valid
    /// - `Err(TTSError::InvalidConfiguration)` with descriptive message
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::{CartesiaOutputFormat, CartesiaAudioEncoding};
    ///
    /// let valid = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000);
    /// assert!(valid.validate().is_ok());
    ///
    /// let invalid = CartesiaOutputFormat {
    ///     container: CartesiaAudioContainer::Raw,
    ///     encoding: Some(CartesiaAudioEncoding::PcmS16le),
    ///     sample_rate: 12345, // Invalid sample rate
    ///     bit_rate: None,
    /// };
    /// assert!(invalid.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), TTSError> {
        // Validate sample rate
        if !SUPPORTED_SAMPLE_RATES.contains(&self.sample_rate) {
            return Err(TTSError::InvalidConfiguration(format!(
                "Unsupported sample rate: {}. Supported rates: {:?}",
                self.sample_rate, SUPPORTED_SAMPLE_RATES
            )));
        }

        // Validate encoding is present for PCM containers
        match self.container {
            CartesiaAudioContainer::Raw | CartesiaAudioContainer::Wav => {
                if self.encoding.is_none() {
                    return Err(TTSError::InvalidConfiguration(format!(
                        "{} container requires an encoding to be specified",
                        self.container.as_str()
                    )));
                }
            }
            CartesiaAudioContainer::Mp3 => {
                if self.bit_rate.is_none() {
                    return Err(TTSError::InvalidConfiguration(
                        "MP3 container requires a bit_rate to be specified".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }
}

impl Default for CartesiaOutputFormat {
    fn default() -> Self {
        Self::raw(CartesiaAudioEncoding::default(), 24000)
    }
}

// =============================================================================
// Main Configuration
// =============================================================================

/// Configuration specific to Cartesia Text-to-Speech API.
///
/// This configuration extends the base `TTSConfig` with Cartesia-specific
/// parameters for the Sonic voice models and output format control.
///
/// # Example
///
/// ```rust,ignore
/// use sayna::core::tts::cartesia::{CartesiaTTSConfig, CartesiaOutputFormat};
/// use sayna::core::tts::TTSConfig;
///
/// let base = TTSConfig {
///     api_key: "your-api-key".to_string(),
///     voice_id: Some("a0e99841-438c-4a64-b679-ae501e7d6091".to_string()),
///     audio_format: Some("linear16".to_string()),
///     sample_rate: Some(24000),
///     ..Default::default()
/// };
///
/// let config = CartesiaTTSConfig::from_base(base);
/// assert_eq!(config.model, "sonic-3");
/// ```
#[derive(Debug, Clone)]
pub struct CartesiaTTSConfig {
    /// Base TTS configuration (shared across all providers).
    /// Contains: api_key, voice_id, sample_rate, audio_format, pronunciations, etc.
    pub base: TTSConfig,

    /// Cartesia Sonic model identifier.
    /// Default: "sonic-3" (latest stable)
    pub model: String,

    /// Output format specification for audio synthesis.
    pub output_format: CartesiaOutputFormat,

    /// API version date string for Cartesia-Version header.
    /// Format: "YYYY-MM-DD"
    /// Default: "2025-04-16"
    pub api_version: String,
}

impl CartesiaTTSConfig {
    /// Creates a CartesiaTTSConfig from a base TTSConfig with default Cartesia settings.
    ///
    /// Maps base config fields to Cartesia-specific settings:
    /// - `audio_format` → `output_format` via `CartesiaOutputFormat::from_format_string`
    /// - `sample_rate` → `output_format.sample_rate`
    /// - `model` field → `self.model` (or default "sonic-3")
    ///
    /// # Arguments
    ///
    /// * `base` - Base TTS configuration
    ///
    /// # Returns
    ///
    /// A new CartesiaTTSConfig with mapped settings
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::{TTSConfig, cartesia::CartesiaTTSConfig};
    ///
    /// let base = TTSConfig {
    ///     voice_id: Some("voice-id-123".to_string()),
    ///     audio_format: Some("mp3".to_string()),
    ///     sample_rate: Some(24000),
    ///     ..Default::default()
    /// };
    ///
    /// let config = CartesiaTTSConfig::from_base(base);
    /// assert_eq!(config.model, "sonic-3");
    /// ```
    pub fn from_base(base: TTSConfig) -> Self {
        let sample_rate = base.sample_rate.unwrap_or(24000);
        let output_format = base
            .audio_format
            .as_deref()
            .map(|f| CartesiaOutputFormat::from_format_string(f, sample_rate))
            .unwrap_or_default();

        let model = if base.model.is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            base.model.clone()
        };

        Self {
            base,
            model,
            output_format,
            api_version: DEFAULT_API_VERSION.to_string(),
        }
    }

    /// Validates the configuration for API compatibility.
    ///
    /// # Validation Rules
    ///
    /// 1. API key must not be empty
    /// 2. Voice ID should be present (warning if missing, but not error)
    /// 3. Sample rate must be in SUPPORTED_SAMPLE_RATES
    /// 4. Model must not be empty
    ///
    /// # Returns
    ///
    /// - `Ok(())` if configuration is valid
    /// - `Err(TTSError::InvalidConfiguration)` with descriptive message
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::{TTSConfig, cartesia::CartesiaTTSConfig};
    ///
    /// let base = TTSConfig {
    ///     api_key: "valid-key".to_string(),
    ///     voice_id: Some("voice-id".to_string()),
    ///     sample_rate: Some(24000),
    ///     ..Default::default()
    /// };
    ///
    /// let config = CartesiaTTSConfig::from_base(base);
    /// assert!(config.validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<(), TTSError> {
        // Check API key
        if self.base.api_key.is_empty() {
            return Err(TTSError::InvalidConfiguration(
                "Cartesia API key is required".to_string(),
            ));
        }

        // Check model
        if self.model.is_empty() {
            return Err(TTSError::InvalidConfiguration(
                "Cartesia model identifier is required".to_string(),
            ));
        }

        // Validate output format (includes sample rate validation)
        self.output_format.validate()?;

        Ok(())
    }

    /// Returns the voice ID for API requests.
    /// Returns None if voice_id is empty or not set.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::{TTSConfig, cartesia::CartesiaTTSConfig};
    ///
    /// let base = TTSConfig {
    ///     voice_id: Some("voice-123".to_string()),
    ///     ..Default::default()
    /// };
    ///
    /// let config = CartesiaTTSConfig::from_base(base);
    /// assert_eq!(config.voice_id(), Some("voice-123"));
    /// ```
    #[inline]
    pub fn voice_id(&self) -> Option<&str> {
        self.base.voice_id.as_deref().filter(|s| !s.is_empty())
    }

    /// Builds the output_format JSON value for the request body.
    /// Uses serde_json::to_value for proper serialization.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::cartesia::CartesiaTTSConfig;
    ///
    /// let config = CartesiaTTSConfig::default();
    /// let json = config.build_output_format_json();
    ///
    /// assert!(json.get("container").is_some());
    /// assert!(json.get("sample_rate").is_some());
    /// ```
    pub fn build_output_format_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.output_format)
            .expect("CartesiaOutputFormat serialization should never fail")
    }
}

impl Default for CartesiaTTSConfig {
    fn default() -> Self {
        Self {
            base: TTSConfig::default(),
            model: DEFAULT_MODEL.to_string(),
            output_format: CartesiaOutputFormat::default(),
            api_version: DEFAULT_API_VERSION.to_string(),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // CartesiaAudioContainer Tests
    // =========================================================================

    #[test]
    fn test_container_as_str() {
        assert_eq!(CartesiaAudioContainer::Raw.as_str(), "raw");
        assert_eq!(CartesiaAudioContainer::Wav.as_str(), "wav");
        assert_eq!(CartesiaAudioContainer::Mp3.as_str(), "mp3");
    }

    #[test]
    fn test_container_default_is_raw() {
        assert_eq!(
            CartesiaAudioContainer::default(),
            CartesiaAudioContainer::Raw
        );
    }

    #[test]
    fn test_container_supports_streaming() {
        assert!(CartesiaAudioContainer::Raw.supports_streaming());
        assert!(CartesiaAudioContainer::Wav.supports_streaming());
        assert!(!CartesiaAudioContainer::Mp3.supports_streaming());
    }

    #[test]
    fn test_container_serialization() {
        assert_eq!(
            serde_json::to_string(&CartesiaAudioContainer::Raw).unwrap(),
            "\"raw\""
        );
        assert_eq!(
            serde_json::to_string(&CartesiaAudioContainer::Wav).unwrap(),
            "\"wav\""
        );
        assert_eq!(
            serde_json::to_string(&CartesiaAudioContainer::Mp3).unwrap(),
            "\"mp3\""
        );
    }

    #[test]
    fn test_container_deserialization() {
        assert_eq!(
            serde_json::from_str::<CartesiaAudioContainer>("\"raw\"").unwrap(),
            CartesiaAudioContainer::Raw
        );
        assert_eq!(
            serde_json::from_str::<CartesiaAudioContainer>("\"wav\"").unwrap(),
            CartesiaAudioContainer::Wav
        );
        assert_eq!(
            serde_json::from_str::<CartesiaAudioContainer>("\"mp3\"").unwrap(),
            CartesiaAudioContainer::Mp3
        );
    }

    // =========================================================================
    // CartesiaAudioEncoding Tests
    // =========================================================================

    #[test]
    fn test_encoding_as_str() {
        assert_eq!(CartesiaAudioEncoding::PcmF32le.as_str(), "pcm_f32le");
        assert_eq!(CartesiaAudioEncoding::PcmS16le.as_str(), "pcm_s16le");
        assert_eq!(CartesiaAudioEncoding::PcmAlaw.as_str(), "pcm_alaw");
        assert_eq!(CartesiaAudioEncoding::PcmMulaw.as_str(), "pcm_mulaw");
    }

    #[test]
    fn test_encoding_default_is_pcm_s16le() {
        assert_eq!(
            CartesiaAudioEncoding::default(),
            CartesiaAudioEncoding::PcmS16le
        );
    }

    #[test]
    fn test_encoding_bytes_per_sample() {
        assert_eq!(CartesiaAudioEncoding::PcmF32le.bytes_per_sample(), 4);
        assert_eq!(CartesiaAudioEncoding::PcmS16le.bytes_per_sample(), 2);
        assert_eq!(CartesiaAudioEncoding::PcmAlaw.bytes_per_sample(), 1);
        assert_eq!(CartesiaAudioEncoding::PcmMulaw.bytes_per_sample(), 1);
    }

    #[test]
    fn test_encoding_is_telephony() {
        assert!(!CartesiaAudioEncoding::PcmF32le.is_telephony());
        assert!(!CartesiaAudioEncoding::PcmS16le.is_telephony());
        assert!(CartesiaAudioEncoding::PcmAlaw.is_telephony());
        assert!(CartesiaAudioEncoding::PcmMulaw.is_telephony());
    }

    #[test]
    fn test_encoding_is_pcm() {
        assert!(CartesiaAudioEncoding::PcmF32le.is_pcm());
        assert!(CartesiaAudioEncoding::PcmS16le.is_pcm());
        assert!(!CartesiaAudioEncoding::PcmAlaw.is_pcm());
        assert!(!CartesiaAudioEncoding::PcmMulaw.is_pcm());
    }

    #[test]
    fn test_encoding_serialization() {
        assert_eq!(
            serde_json::to_string(&CartesiaAudioEncoding::PcmF32le).unwrap(),
            "\"pcm_f32le\""
        );
        assert_eq!(
            serde_json::to_string(&CartesiaAudioEncoding::PcmS16le).unwrap(),
            "\"pcm_s16le\""
        );
        assert_eq!(
            serde_json::to_string(&CartesiaAudioEncoding::PcmAlaw).unwrap(),
            "\"pcm_alaw\""
        );
        assert_eq!(
            serde_json::to_string(&CartesiaAudioEncoding::PcmMulaw).unwrap(),
            "\"pcm_mulaw\""
        );
    }

    #[test]
    fn test_encoding_deserialization() {
        assert_eq!(
            serde_json::from_str::<CartesiaAudioEncoding>("\"pcm_f32le\"").unwrap(),
            CartesiaAudioEncoding::PcmF32le
        );
        assert_eq!(
            serde_json::from_str::<CartesiaAudioEncoding>("\"pcm_s16le\"").unwrap(),
            CartesiaAudioEncoding::PcmS16le
        );
        assert_eq!(
            serde_json::from_str::<CartesiaAudioEncoding>("\"pcm_alaw\"").unwrap(),
            CartesiaAudioEncoding::PcmAlaw
        );
        assert_eq!(
            serde_json::from_str::<CartesiaAudioEncoding>("\"pcm_mulaw\"").unwrap(),
            CartesiaAudioEncoding::PcmMulaw
        );
    }

    // =========================================================================
    // CartesiaOutputFormat Tests
    // =========================================================================

    #[test]
    fn test_output_format_raw() {
        let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000);

        assert_eq!(format.container, CartesiaAudioContainer::Raw);
        assert_eq!(format.encoding, Some(CartesiaAudioEncoding::PcmS16le));
        assert_eq!(format.sample_rate, 24000);
        assert!(format.bit_rate.is_none());
    }

    #[test]
    fn test_output_format_wav() {
        let format = CartesiaOutputFormat::wav(CartesiaAudioEncoding::PcmS16le, 44100);

        assert_eq!(format.container, CartesiaAudioContainer::Wav);
        assert_eq!(format.encoding, Some(CartesiaAudioEncoding::PcmS16le));
        assert_eq!(format.sample_rate, 44100);
        assert!(format.bit_rate.is_none());
    }

    #[test]
    fn test_output_format_mp3() {
        let format = CartesiaOutputFormat::mp3(24000, 128000);

        assert_eq!(format.container, CartesiaAudioContainer::Mp3);
        assert!(format.encoding.is_none());
        assert_eq!(format.sample_rate, 24000);
        assert_eq!(format.bit_rate, Some(128000));
    }

    #[test]
    fn test_output_format_default() {
        let format = CartesiaOutputFormat::default();

        assert_eq!(format.container, CartesiaAudioContainer::Raw);
        assert_eq!(format.encoding, Some(CartesiaAudioEncoding::PcmS16le));
        assert_eq!(format.sample_rate, 24000);
        assert!(format.bit_rate.is_none());
    }

    #[test]
    fn test_output_format_from_format_string() {
        // linear16 -> Raw + PcmS16le
        let format = CartesiaOutputFormat::from_format_string("linear16", 24000);
        assert_eq!(format.container, CartesiaAudioContainer::Raw);
        assert_eq!(format.encoding, Some(CartesiaAudioEncoding::PcmS16le));

        // pcm -> Raw + PcmS16le
        let format = CartesiaOutputFormat::from_format_string("pcm", 16000);
        assert_eq!(format.container, CartesiaAudioContainer::Raw);
        assert_eq!(format.sample_rate, 16000);

        // mulaw -> Raw + PcmMulaw
        let format = CartesiaOutputFormat::from_format_string("mulaw", 8000);
        assert_eq!(format.container, CartesiaAudioContainer::Raw);
        assert_eq!(format.encoding, Some(CartesiaAudioEncoding::PcmMulaw));

        // ulaw -> Raw + PcmMulaw
        let format = CartesiaOutputFormat::from_format_string("ulaw", 8000);
        assert_eq!(format.encoding, Some(CartesiaAudioEncoding::PcmMulaw));

        // alaw -> Raw + PcmAlaw
        let format = CartesiaOutputFormat::from_format_string("alaw", 8000);
        assert_eq!(format.encoding, Some(CartesiaAudioEncoding::PcmAlaw));

        // wav -> Wav + PcmS16le
        let format = CartesiaOutputFormat::from_format_string("wav", 44100);
        assert_eq!(format.container, CartesiaAudioContainer::Wav);
        assert_eq!(format.encoding, Some(CartesiaAudioEncoding::PcmS16le));

        // mp3 -> Mp3 + default bitrate
        let format = CartesiaOutputFormat::from_format_string("mp3", 24000);
        assert_eq!(format.container, CartesiaAudioContainer::Mp3);
        assert_eq!(format.bit_rate, Some(128000));
        assert!(format.encoding.is_none());

        // unknown -> default (Raw + PcmS16le)
        let format = CartesiaOutputFormat::from_format_string("unknown", 24000);
        assert_eq!(format.container, CartesiaAudioContainer::Raw);
        assert_eq!(format.encoding, Some(CartesiaAudioEncoding::PcmS16le));
    }

    #[test]
    fn test_output_format_from_format_string_case_insensitive() {
        let format = CartesiaOutputFormat::from_format_string("LINEAR16", 24000);
        assert_eq!(format.container, CartesiaAudioContainer::Raw);
        assert_eq!(format.encoding, Some(CartesiaAudioEncoding::PcmS16le));

        let format = CartesiaOutputFormat::from_format_string("MP3", 24000);
        assert_eq!(format.container, CartesiaAudioContainer::Mp3);
    }

    #[test]
    fn test_output_format_sample_rate() {
        let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 48000);
        assert_eq!(format.sample_rate(), 48000);
    }

    #[test]
    fn test_output_format_is_pcm() {
        // Raw + PcmS16le is PCM
        let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000);
        assert!(format.is_pcm());

        // Raw + PcmF32le is PCM
        let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmF32le, 24000);
        assert!(format.is_pcm());

        // Wav + PcmS16le is PCM
        let format = CartesiaOutputFormat::wav(CartesiaAudioEncoding::PcmS16le, 24000);
        assert!(format.is_pcm());

        // Raw + PcmMulaw is NOT considered PCM (telephony)
        let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmMulaw, 8000);
        assert!(!format.is_pcm());

        // MP3 is not PCM
        let format = CartesiaOutputFormat::mp3(24000, 128000);
        assert!(!format.is_pcm());
    }

    #[test]
    fn test_output_format_is_telephony() {
        let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmMulaw, 8000);
        assert!(format.is_telephony());

        let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmAlaw, 8000);
        assert!(format.is_telephony());

        let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000);
        assert!(!format.is_telephony());

        let format = CartesiaOutputFormat::mp3(24000, 128000);
        assert!(!format.is_telephony());
    }

    #[test]
    fn test_output_format_content_type() {
        assert_eq!(
            CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000).content_type(),
            "application/octet-stream"
        );
        assert_eq!(
            CartesiaOutputFormat::wav(CartesiaAudioEncoding::PcmS16le, 24000).content_type(),
            "audio/wav"
        );
        assert_eq!(
            CartesiaOutputFormat::mp3(24000, 128000).content_type(),
            "audio/mpeg"
        );
    }

    #[test]
    fn test_output_format_validate_success() {
        let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000);
        assert!(format.validate().is_ok());

        let format = CartesiaOutputFormat::wav(CartesiaAudioEncoding::PcmS16le, 44100);
        assert!(format.validate().is_ok());

        let format = CartesiaOutputFormat::mp3(24000, 128000);
        assert!(format.validate().is_ok());
    }

    #[test]
    fn test_output_format_validate_invalid_sample_rate() {
        let format = CartesiaOutputFormat {
            container: CartesiaAudioContainer::Raw,
            encoding: Some(CartesiaAudioEncoding::PcmS16le),
            sample_rate: 12345, // Invalid sample rate
            bit_rate: None,
        };

        let result = format.validate();
        assert!(result.is_err());
        assert!(matches!(result, Err(TTSError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_output_format_validate_missing_encoding() {
        let format = CartesiaOutputFormat {
            container: CartesiaAudioContainer::Raw,
            encoding: None, // Missing encoding for Raw container
            sample_rate: 24000,
            bit_rate: None,
        };

        let result = format.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_output_format_validate_missing_bitrate() {
        let format = CartesiaOutputFormat {
            container: CartesiaAudioContainer::Mp3,
            encoding: None,
            sample_rate: 24000,
            bit_rate: None, // Missing bit_rate for MP3
        };

        let result = format.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_output_format_json_serialization_raw() {
        let format = CartesiaOutputFormat::raw(CartesiaAudioEncoding::PcmS16le, 24000);
        let json = serde_json::to_value(&format).unwrap();

        assert_eq!(json["container"], "raw");
        assert_eq!(json["encoding"], "pcm_s16le");
        assert_eq!(json["sample_rate"], 24000);
        // bit_rate should be absent (not null)
        assert!(json.get("bit_rate").is_none());
    }

    #[test]
    fn test_output_format_json_serialization_mp3() {
        let format = CartesiaOutputFormat::mp3(24000, 128000);
        let json = serde_json::to_value(&format).unwrap();

        assert_eq!(json["container"], "mp3");
        assert_eq!(json["sample_rate"], 24000);
        assert_eq!(json["bit_rate"], 128000);
        // encoding should be absent (not null)
        assert!(json.get("encoding").is_none());
    }

    // =========================================================================
    // CartesiaTTSConfig Tests
    // =========================================================================

    #[test]
    fn test_config_default() {
        let config = CartesiaTTSConfig::default();

        assert_eq!(config.model, "sonic-3");
        assert_eq!(config.api_version, "2025-04-16");
        assert_eq!(config.output_format.container, CartesiaAudioContainer::Raw);
        assert_eq!(
            config.output_format.encoding,
            Some(CartesiaAudioEncoding::PcmS16le)
        );
        assert_eq!(config.output_format.sample_rate, 24000);
    }

    #[test]
    fn test_config_from_base() {
        let base = TTSConfig {
            api_key: "test-api-key".to_string(),
            voice_id: Some("voice-123".to_string()),
            audio_format: Some("mp3".to_string()),
            sample_rate: Some(24000),
            ..Default::default()
        };

        let config = CartesiaTTSConfig::from_base(base);

        assert_eq!(config.base.api_key, "test-api-key");
        assert_eq!(config.model, "sonic-3"); // Default model
        assert_eq!(config.output_format.container, CartesiaAudioContainer::Mp3);
        assert_eq!(config.output_format.sample_rate, 24000);
    }

    #[test]
    fn test_config_from_base_with_model() {
        let base = TTSConfig {
            api_key: "test-api-key".to_string(),
            model: "sonic-3-2025-10-27".to_string(),
            ..Default::default()
        };

        let config = CartesiaTTSConfig::from_base(base);

        assert_eq!(config.model, "sonic-3-2025-10-27");
    }

    #[test]
    fn test_config_from_base_empty_model_uses_default() {
        let base = TTSConfig {
            api_key: "test-api-key".to_string(),
            model: String::new(),
            ..Default::default()
        };

        let config = CartesiaTTSConfig::from_base(base);

        assert_eq!(config.model, "sonic-3");
    }

    #[test]
    fn test_config_from_base_sample_rate_fallback() {
        let base = TTSConfig {
            api_key: "test-api-key".to_string(),
            sample_rate: None, // No sample rate specified
            ..Default::default()
        };

        let config = CartesiaTTSConfig::from_base(base);

        // Should fallback to 24000
        assert_eq!(config.output_format.sample_rate, 24000);
    }

    #[test]
    fn test_config_validate_success() {
        let base = TTSConfig {
            api_key: "test-api-key".to_string(),
            voice_id: Some("voice-123".to_string()),
            sample_rate: Some(24000),
            ..Default::default()
        };

        let config = CartesiaTTSConfig::from_base(base);

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_validate_empty_api_key() {
        let base = TTSConfig {
            api_key: String::new(), // Empty API key
            ..Default::default()
        };

        let config = CartesiaTTSConfig::from_base(base);
        let result = config.validate();

        assert!(result.is_err());
        assert!(matches!(result, Err(TTSError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_config_validate_invalid_sample_rate() {
        let mut config = CartesiaTTSConfig::default();
        config.base.api_key = "valid-key".to_string();
        config.output_format.sample_rate = 12345; // Invalid sample rate

        let result = config.validate();

        assert!(result.is_err());
    }

    #[test]
    fn test_config_voice_id() {
        let base = TTSConfig {
            voice_id: Some("voice-123".to_string()),
            ..Default::default()
        };

        let config = CartesiaTTSConfig::from_base(base);

        assert_eq!(config.voice_id(), Some("voice-123"));
    }

    #[test]
    fn test_config_voice_id_empty() {
        let base = TTSConfig {
            voice_id: Some(String::new()),
            ..Default::default()
        };

        let config = CartesiaTTSConfig::from_base(base);

        assert_eq!(config.voice_id(), None);
    }

    #[test]
    fn test_config_voice_id_none() {
        let base = TTSConfig {
            voice_id: None,
            ..Default::default()
        };

        let config = CartesiaTTSConfig::from_base(base);

        assert_eq!(config.voice_id(), None);
    }

    #[test]
    fn test_config_build_output_format_json() {
        let config = CartesiaTTSConfig::default();
        let json = config.build_output_format_json();

        assert_eq!(json["container"], "raw");
        assert_eq!(json["encoding"], "pcm_s16le");
        assert_eq!(json["sample_rate"], 24000);
    }

    #[test]
    fn test_config_build_output_format_json_mp3() {
        let base = TTSConfig {
            audio_format: Some("mp3".to_string()),
            sample_rate: Some(24000),
            ..Default::default()
        };

        let config = CartesiaTTSConfig::from_base(base);
        let json = config.build_output_format_json();

        assert_eq!(json["container"], "mp3");
        assert_eq!(json["sample_rate"], 24000);
        assert_eq!(json["bit_rate"], 128000);
    }

    // =========================================================================
    // Constants Tests
    // =========================================================================

    #[test]
    fn test_constants() {
        assert_eq!(CARTESIA_TTS_URL, "https://api.cartesia.ai/tts/bytes");
        assert_eq!(DEFAULT_API_VERSION, "2025-04-16");
        assert_eq!(DEFAULT_MODEL, "sonic-3");
        assert_eq!(
            SUPPORTED_SAMPLE_RATES,
            &[8000, 16000, 22050, 24000, 44100, 48000]
        );
    }
}
