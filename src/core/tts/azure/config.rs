//! Configuration types for Microsoft Azure Text-to-Speech API.
//!
//! This module contains all configuration-related types including:
//! - Audio encoding format specifications
//! - Provider-specific configuration options
//! - SSML generation utilities
//!
//! Note: The `AzureRegion` type is defined in `crate::core::providers::azure`
//! and should be used from there.

use crate::core::providers::azure::AzureRegion;
use crate::core::tts::base::TTSConfig;
use serde::{Deserialize, Serialize};

/// HTTP header name for Azure TTS output format.
pub const AZURE_OUTPUT_FORMAT_HEADER: &str = "X-Microsoft-OutputFormat";

/// Default Azure TTS endpoint URL pattern.
///
/// Azure TTS uses regional endpoints in the format:
/// `https://{region}.tts.speech.microsoft.com/cognitiveservices/v1`
///
/// This constant provides the default (eastus) endpoint for compatibility with
/// other TTS providers. For specific regions, use `AzureRegion::tts_rest_url()`.
pub const AZURE_TTS_URL: &str = "https://eastus.tts.speech.microsoft.com/cognitiveservices/v1";

// =============================================================================
// Audio Encoding
// =============================================================================

/// Azure Text-to-Speech audio output format options.
///
/// These formats map to Azure's `X-Microsoft-OutputFormat` header values.
/// Formats are grouped into categories:
///
/// - **Raw PCM**: Uncompressed audio for real-time streaming
/// - **MP3**: Compressed audio for storage/bandwidth optimization
/// - **Opus**: Low-latency compressed audio for real-time applications
///
/// # Example
///
/// ```rust
/// use sayna::core::tts::azure::AzureAudioEncoding;
///
/// let format = AzureAudioEncoding::Raw24Khz16BitMonoPcm;
/// assert_eq!(format.as_str(), "raw-24khz-16bit-mono-pcm");
/// assert_eq!(format.sample_rate(), 24000);
/// assert!(format.is_pcm());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AzureAudioEncoding {
    // =========================================================================
    // Raw PCM Formats (streaming, no container)
    // =========================================================================
    /// 8kHz, 8-bit μ-law mono (telephony)
    Raw8Khz8BitMonoMulaw,
    /// 8kHz, 8-bit A-law mono (telephony)
    Raw8Khz8BitMonoAlaw,
    /// 8kHz, 16-bit PCM mono
    Raw8Khz16BitMonoPcm,
    /// 16kHz, 16-bit PCM mono
    Raw16Khz16BitMonoPcm,
    /// 22.05kHz, 16-bit PCM mono
    Raw22050Hz16BitMonoPcm,
    /// 24kHz, 16-bit PCM mono (recommended for most use cases)
    #[default]
    Raw24Khz16BitMonoPcm,
    /// 44.1kHz, 16-bit PCM mono (CD quality)
    Raw44100Hz16BitMonoPcm,
    /// 48kHz, 16-bit PCM mono (highest quality)
    Raw48Khz16BitMonoPcm,

    // =========================================================================
    // MP3 Streaming Formats
    // =========================================================================
    /// 16kHz, 32kbps MP3 mono
    Audio16Khz32KbitrateMonoMp3,
    /// 16kHz, 64kbps MP3 mono
    Audio16Khz64KbitrateMonoMp3,
    /// 24kHz, 48kbps MP3 mono
    Audio24Khz48KbitrateMonoMp3,
    /// 24kHz, 96kbps MP3 mono
    Audio24Khz96KbitrateMonoMp3,
    /// 48kHz, 96kbps MP3 mono
    Audio48Khz96KbitrateMonoMp3,
    /// 48kHz, 192kbps MP3 mono (highest MP3 quality)
    Audio48Khz192KbitrateMonoMp3,

    // =========================================================================
    // Opus Formats (low-latency streaming)
    // =========================================================================
    /// 16kHz, 16-bit, 32kbps Opus mono
    Audio16Khz16Bit32KbpsMonoOpus,
    /// 24kHz, 16-bit, 24kbps Opus mono
    Audio24Khz16Bit24KbpsMonoOpus,
    /// 24kHz, 16-bit, 48kbps Opus mono
    Audio24Khz16Bit48KbpsMonoOpus,
}

impl AzureAudioEncoding {
    /// Returns the Azure API format string for the `X-Microsoft-OutputFormat` header.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::azure::AzureAudioEncoding;
    ///
    /// assert_eq!(AzureAudioEncoding::Raw24Khz16BitMonoPcm.as_str(), "raw-24khz-16bit-mono-pcm");
    /// assert_eq!(AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.as_str(), "audio-24khz-96kbitrate-mono-mp3");
    /// ```
    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            // Raw PCM formats
            Self::Raw8Khz8BitMonoMulaw => "raw-8khz-8bit-mono-mulaw",
            Self::Raw8Khz8BitMonoAlaw => "raw-8khz-8bit-mono-alaw",
            Self::Raw8Khz16BitMonoPcm => "raw-8khz-16bit-mono-pcm",
            Self::Raw16Khz16BitMonoPcm => "raw-16khz-16bit-mono-pcm",
            Self::Raw22050Hz16BitMonoPcm => "raw-22050hz-16bit-mono-pcm",
            Self::Raw24Khz16BitMonoPcm => "raw-24khz-16bit-mono-pcm",
            Self::Raw44100Hz16BitMonoPcm => "raw-44100hz-16bit-mono-pcm",
            Self::Raw48Khz16BitMonoPcm => "raw-48khz-16bit-mono-pcm",
            // MP3 formats
            Self::Audio16Khz32KbitrateMonoMp3 => "audio-16khz-32kbitrate-mono-mp3",
            Self::Audio16Khz64KbitrateMonoMp3 => "audio-16khz-64kbitrate-mono-mp3",
            Self::Audio24Khz48KbitrateMonoMp3 => "audio-24khz-48kbitrate-mono-mp3",
            Self::Audio24Khz96KbitrateMonoMp3 => "audio-24khz-96kbitrate-mono-mp3",
            Self::Audio48Khz96KbitrateMonoMp3 => "audio-48khz-96kbitrate-mono-mp3",
            Self::Audio48Khz192KbitrateMonoMp3 => "audio-48khz-192kbitrate-mono-mp3",
            // Opus formats
            Self::Audio16Khz16Bit32KbpsMonoOpus => "audio-16khz-16bit-32kbps-mono-opus",
            Self::Audio24Khz16Bit24KbpsMonoOpus => "audio-24khz-16bit-24kbps-mono-opus",
            Self::Audio24Khz16Bit48KbpsMonoOpus => "audio-24khz-16bit-48kbps-mono-opus",
        }
    }

    /// Returns the sample rate in Hz for this audio format.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::azure::AzureAudioEncoding;
    ///
    /// assert_eq!(AzureAudioEncoding::Raw8Khz16BitMonoPcm.sample_rate(), 8000);
    /// assert_eq!(AzureAudioEncoding::Raw24Khz16BitMonoPcm.sample_rate(), 24000);
    /// assert_eq!(AzureAudioEncoding::Audio48Khz192KbitrateMonoMp3.sample_rate(), 48000);
    /// ```
    #[inline]
    pub fn sample_rate(&self) -> u32 {
        match self {
            Self::Raw8Khz8BitMonoMulaw | Self::Raw8Khz8BitMonoAlaw | Self::Raw8Khz16BitMonoPcm => {
                8000
            }

            Self::Raw16Khz16BitMonoPcm
            | Self::Audio16Khz32KbitrateMonoMp3
            | Self::Audio16Khz64KbitrateMonoMp3
            | Self::Audio16Khz16Bit32KbpsMonoOpus => 16000,

            Self::Raw22050Hz16BitMonoPcm => 22050,

            Self::Raw24Khz16BitMonoPcm
            | Self::Audio24Khz48KbitrateMonoMp3
            | Self::Audio24Khz96KbitrateMonoMp3
            | Self::Audio24Khz16Bit24KbpsMonoOpus
            | Self::Audio24Khz16Bit48KbpsMonoOpus => 24000,

            Self::Raw44100Hz16BitMonoPcm => 44100,

            Self::Raw48Khz16BitMonoPcm
            | Self::Audio48Khz96KbitrateMonoMp3
            | Self::Audio48Khz192KbitrateMonoMp3 => 48000,
        }
    }

    /// Returns true if this is a raw PCM format (uncompressed audio).
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::azure::AzureAudioEncoding;
    ///
    /// assert!(AzureAudioEncoding::Raw24Khz16BitMonoPcm.is_pcm());
    /// assert!(!AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.is_pcm());
    /// assert!(!AzureAudioEncoding::Audio24Khz16Bit48KbpsMonoOpus.is_pcm());
    /// ```
    #[inline]
    pub fn is_pcm(&self) -> bool {
        matches!(
            self,
            Self::Raw8Khz16BitMonoPcm
                | Self::Raw16Khz16BitMonoPcm
                | Self::Raw22050Hz16BitMonoPcm
                | Self::Raw24Khz16BitMonoPcm
                | Self::Raw44100Hz16BitMonoPcm
                | Self::Raw48Khz16BitMonoPcm
        )
    }

    /// Returns true if this is a μ-law or A-law format (telephony).
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::azure::AzureAudioEncoding;
    ///
    /// assert!(AzureAudioEncoding::Raw8Khz8BitMonoMulaw.is_telephony());
    /// assert!(AzureAudioEncoding::Raw8Khz8BitMonoAlaw.is_telephony());
    /// assert!(!AzureAudioEncoding::Raw24Khz16BitMonoPcm.is_telephony());
    /// ```
    #[inline]
    pub fn is_telephony(&self) -> bool {
        matches!(self, Self::Raw8Khz8BitMonoMulaw | Self::Raw8Khz8BitMonoAlaw)
    }

    /// Returns the appropriate MIME content type for this audio format.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::azure::AzureAudioEncoding;
    ///
    /// assert_eq!(AzureAudioEncoding::Raw24Khz16BitMonoPcm.content_type(), "audio/pcm");
    /// assert_eq!(AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.content_type(), "audio/mpeg");
    /// assert_eq!(AzureAudioEncoding::Audio24Khz16Bit48KbpsMonoOpus.content_type(), "audio/opus");
    /// assert_eq!(AzureAudioEncoding::Raw8Khz8BitMonoMulaw.content_type(), "audio/mulaw");
    /// ```
    #[inline]
    pub fn content_type(&self) -> &'static str {
        match self {
            Self::Raw8Khz8BitMonoMulaw => "audio/mulaw",
            Self::Raw8Khz8BitMonoAlaw => "audio/alaw",
            Self::Raw8Khz16BitMonoPcm
            | Self::Raw16Khz16BitMonoPcm
            | Self::Raw22050Hz16BitMonoPcm
            | Self::Raw24Khz16BitMonoPcm
            | Self::Raw44100Hz16BitMonoPcm
            | Self::Raw48Khz16BitMonoPcm => "audio/pcm",
            Self::Audio16Khz32KbitrateMonoMp3
            | Self::Audio16Khz64KbitrateMonoMp3
            | Self::Audio24Khz48KbitrateMonoMp3
            | Self::Audio24Khz96KbitrateMonoMp3
            | Self::Audio48Khz96KbitrateMonoMp3
            | Self::Audio48Khz192KbitrateMonoMp3 => "audio/mpeg",
            Self::Audio16Khz16Bit32KbpsMonoOpus
            | Self::Audio24Khz16Bit24KbpsMonoOpus
            | Self::Audio24Khz16Bit48KbpsMonoOpus => "audio/opus",
        }
    }

    /// Converts a base config format string and sample rate to the appropriate Azure encoding.
    ///
    /// This method maps common format string variations to the correct enum variant:
    /// - "linear16", "pcm", "wav" → Raw PCM format at specified sample rate
    /// - "mp3" → MP3 format at specified sample rate
    /// - "mulaw", "ulaw" → `Raw8Khz8BitMonoMulaw`
    /// - "alaw" → `Raw8Khz8BitMonoAlaw`
    /// - "opus" → Opus format at specified sample rate
    /// - Unknown formats default to `Raw24Khz16BitMonoPcm`
    ///
    /// # Arguments
    ///
    /// * `format` - The audio format string from base config
    /// * `sample_rate` - The sample rate to match
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::azure::AzureAudioEncoding;
    ///
    /// assert_eq!(
    ///     AzureAudioEncoding::from_format_string("pcm", 24000),
    ///     AzureAudioEncoding::Raw24Khz16BitMonoPcm
    /// );
    /// assert_eq!(
    ///     AzureAudioEncoding::from_format_string("mp3", 24000),
    ///     AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3
    /// );
    /// assert_eq!(
    ///     AzureAudioEncoding::from_format_string("mulaw", 8000),
    ///     AzureAudioEncoding::Raw8Khz8BitMonoMulaw
    /// );
    /// ```
    pub fn from_format_string(format: &str, sample_rate: u32) -> Self {
        match format.to_lowercase().as_str() {
            "linear16" | "pcm" | "wav" => Self::pcm_for_sample_rate(sample_rate),
            "mp3" => Self::mp3_for_sample_rate(sample_rate),
            "mulaw" | "ulaw" => Self::Raw8Khz8BitMonoMulaw,
            "alaw" => Self::Raw8Khz8BitMonoAlaw,
            "opus" => Self::opus_for_sample_rate(sample_rate),
            _ => Self::default(),
        }
    }

    /// Select the best PCM format for a given sample rate.
    fn pcm_for_sample_rate(sample_rate: u32) -> Self {
        match sample_rate {
            0..=8000 => Self::Raw8Khz16BitMonoPcm,
            8001..=16000 => Self::Raw16Khz16BitMonoPcm,
            16001..=22050 => Self::Raw22050Hz16BitMonoPcm,
            22051..=24000 => Self::Raw24Khz16BitMonoPcm,
            24001..=44100 => Self::Raw44100Hz16BitMonoPcm,
            _ => Self::Raw48Khz16BitMonoPcm,
        }
    }

    /// Select the best MP3 format for a given sample rate (using highest bitrate).
    fn mp3_for_sample_rate(sample_rate: u32) -> Self {
        match sample_rate {
            0..=16000 => Self::Audio16Khz64KbitrateMonoMp3,
            16001..=24000 => Self::Audio24Khz96KbitrateMonoMp3,
            _ => Self::Audio48Khz192KbitrateMonoMp3,
        }
    }

    /// Select the best Opus format for a given sample rate (using highest bitrate).
    fn opus_for_sample_rate(sample_rate: u32) -> Self {
        match sample_rate {
            0..=16000 => Self::Audio16Khz16Bit32KbpsMonoOpus,
            _ => Self::Audio24Khz16Bit48KbpsMonoOpus,
        }
    }
}

// =============================================================================
// SSML Generation
// =============================================================================

/// Escapes special XML characters in text for use in SSML.
///
/// Replaces the following characters:
/// - `&` → `&amp;`
/// - `<` → `&lt;`
/// - `>` → `&gt;`
/// - `"` → `&quot;`
/// - `'` → `&apos;`
///
/// # Example
///
/// ```rust
/// use sayna::core::tts::azure::escape_xml;
///
/// assert_eq!(escape_xml("Hello & goodbye"), "Hello &amp; goodbye");
/// assert_eq!(escape_xml("<script>alert('xss')</script>"), "&lt;script&gt;alert(&apos;xss&apos;)&lt;/script&gt;");
/// ```
pub fn escape_xml(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            _ => result.push(c),
        }
    }
    result
}

/// Builds an SSML document for Azure TTS.
///
/// Wraps the provided text in a valid SSML document with the specified voice
/// and language. Optionally includes a prosody element for speaking rate control.
///
/// # Arguments
///
/// * `text` - The text to synthesize (will be XML-escaped)
/// * `voice_name` - Azure voice name (e.g., "en-US-JennyNeural")
/// * `language` - BCP-47 language code (e.g., "en-US")
/// * `speaking_rate` - Optional speaking rate multiplier (0.5 to 2.0, where 1.0 is normal)
///
/// # Example
///
/// ```rust
/// use sayna::core::tts::azure::build_ssml;
///
/// let ssml = build_ssml("Hello world!", "en-US-JennyNeural", "en-US", None);
/// assert!(ssml.contains("<speak"));
/// assert!(ssml.contains("en-US-JennyNeural"));
/// assert!(ssml.contains("Hello world!"));
///
/// let ssml_with_rate = build_ssml("Fast speech", "en-US-JennyNeural", "en-US", Some(1.5));
/// assert!(ssml_with_rate.contains("rate=\"150%\""));
/// ```
pub fn build_ssml(
    text: &str,
    voice_name: &str,
    language: &str,
    speaking_rate: Option<f32>,
) -> String {
    let escaped_text = escape_xml(text);

    // Build the inner content with optional prosody
    let inner_content = match speaking_rate {
        Some(rate) if (rate - 1.0).abs() > 0.01 => {
            // Convert rate multiplier to percentage (1.0 = 100%, 1.5 = 150%, 0.5 = 50%)
            let rate_percent = (rate * 100.0).round() as i32;
            format!("<prosody rate=\"{rate_percent}%\">{escaped_text}</prosody>",)
        }
        _ => escaped_text,
    };

    format!(
        r#"<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='{language}'>
    <voice name='{voice_name}'>
        {inner_content}
    </voice>
</speak>"#,
    )
}

// =============================================================================
// Main Configuration
// =============================================================================

/// Configuration specific to Microsoft Azure Text-to-Speech API.
///
/// This configuration extends the base `TTSConfig` with Azure-specific
/// parameters for the REST API synthesis endpoint.
///
/// # Example
///
/// ```rust
/// use sayna::core::stt::azure::AzureRegion;
/// use sayna::core::tts::azure::{AzureTTSConfig, AzureAudioEncoding};
/// use sayna::core::tts::TTSConfig;
///
/// let config = AzureTTSConfig {
///     base: TTSConfig {
///         api_key: "your-subscription-key".to_string(),
///         voice_id: Some("en-US-JennyNeural".to_string()),
///         ..Default::default()
///     },
///     region: AzureRegion::WestEurope,
///     output_format: AzureAudioEncoding::Raw24Khz16BitMonoPcm,
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone)]
pub struct AzureTTSConfig {
    /// Base TTS configuration (shared across all providers).
    ///
    /// Contains common settings like api_key, voice_id, sample_rate, etc.
    pub base: TTSConfig,

    /// Azure region for the Speech Service endpoint.
    ///
    /// Choose the region closest to your users for optimal latency.
    pub region: AzureRegion,

    /// Output audio format for synthesis results.
    ///
    /// Maps to the `X-Microsoft-OutputFormat` header value.
    pub output_format: AzureAudioEncoding,

    /// Whether to wrap text in SSML for synthesis.
    ///
    /// When `true` (default), plain text is wrapped in SSML with the
    /// configured voice and speaking rate. When `false`, the text is
    /// sent as-is (assumes caller provides valid SSML).
    pub use_ssml: bool,
}

impl Default for AzureTTSConfig {
    fn default() -> Self {
        Self {
            base: TTSConfig::default(),
            region: AzureRegion::default(),
            output_format: AzureAudioEncoding::default(),
            use_ssml: true,
        }
    }
}

impl AzureTTSConfig {
    /// Creates an `AzureTTSConfig` from a base `TTSConfig` with default Azure settings.
    ///
    /// Maps the base configuration's audio format and sample rate to the
    /// appropriate Azure encoding.
    ///
    /// # Arguments
    ///
    /// * `base` - The base TTS configuration
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::{TTSConfig, azure::AzureTTSConfig};
    ///
    /// let base = TTSConfig {
    ///     voice_id: Some("en-US-JennyNeural".to_string()),
    ///     audio_format: Some("mp3".to_string()),
    ///     sample_rate: Some(24000),
    ///     ..Default::default()
    /// };
    ///
    /// let azure = AzureTTSConfig::from_base(base);
    /// ```
    pub fn from_base(base: TTSConfig) -> Self {
        let sample_rate = base.sample_rate.unwrap_or(24000);
        let output_format = base
            .audio_format
            .as_deref()
            .map(|f| AzureAudioEncoding::from_format_string(f, sample_rate))
            .unwrap_or_default();

        Self {
            base,
            region: AzureRegion::default(),
            output_format,
            use_ssml: true,
        }
    }

    /// Creates an `AzureTTSConfig` with a specific region.
    ///
    /// # Arguments
    ///
    /// * `base` - The base TTS configuration
    /// * `region` - The Azure region to use
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::{TTSConfig, azure::AzureTTSConfig};
    /// use sayna::core::providers::azure::AzureRegion;
    ///
    /// let base = TTSConfig::default();
    /// let azure = AzureTTSConfig::with_region(base, AzureRegion::WestEurope);
    ///
    /// assert_eq!(azure.region, AzureRegion::WestEurope);
    /// ```
    pub fn with_region(base: TTSConfig, region: AzureRegion) -> Self {
        let mut config = Self::from_base(base);
        config.region = region;
        config
    }

    /// Builds the TTS synthesis endpoint URL for this configuration.
    ///
    /// Format: `https://{region}.tts.speech.microsoft.com/cognitiveservices/v1`
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::azure::AzureTTSConfig;
    /// use sayna::core::providers::azure::AzureRegion;
    ///
    /// let config = AzureTTSConfig {
    ///     region: AzureRegion::WestEurope,
    ///     ..Default::default()
    /// };
    ///
    /// assert_eq!(
    ///     config.build_tts_url(),
    ///     "https://westeurope.tts.speech.microsoft.com/cognitiveservices/v1"
    /// );
    /// ```
    pub fn build_tts_url(&self) -> String {
        self.region.tts_rest_url()
    }

    /// Extracts the language code from the voice name.
    ///
    /// Azure voice names follow the pattern `{lang}-{region}-{name}Neural`,
    /// for example "en-US-JennyNeural" or "de-DE-KatjaNeural".
    ///
    /// # Returns
    ///
    /// The extracted language code, or "en-US" if extraction fails.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::{TTSConfig, azure::AzureTTSConfig};
    ///
    /// let config = AzureTTSConfig {
    ///     base: TTSConfig {
    ///         voice_id: Some("de-DE-KatjaNeural".to_string()),
    ///         ..Default::default()
    ///     },
    ///     ..Default::default()
    /// };
    ///
    /// assert_eq!(config.language_code(), "de-DE");
    /// ```
    pub fn language_code(&self) -> String {
        const DEFAULT_LANGUAGE: &str = "en-US";

        let voice_name = match &self.base.voice_id {
            Some(name) if !name.is_empty() => name,
            _ => return DEFAULT_LANGUAGE.to_string(),
        };

        // Split by '-' and try to extract language-region
        let parts: Vec<&str> = voice_name.split('-').collect();

        // Need at least 2 parts for language-region (e.g., "en-US")
        if parts.len() >= 2 {
            let first_part = parts[0];
            let second_part = parts[1];

            // If second part looks like a region code (2 uppercase letters), combine them
            if second_part.len() == 2 && second_part.chars().all(|c| c.is_ascii_uppercase()) {
                return format!("{first_part}-{second_part}");
            }
        }

        DEFAULT_LANGUAGE.to_string()
    }

    /// Returns the voice name for the API request.
    ///
    /// Checks `voice_id` first, falling back to `model` if `voice_id` is empty.
    /// Returns a default voice if both are empty.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::{TTSConfig, azure::AzureTTSConfig};
    ///
    /// let config = AzureTTSConfig {
    ///     base: TTSConfig {
    ///         voice_id: Some("en-US-JennyNeural".to_string()),
    ///         ..Default::default()
    ///     },
    ///     ..Default::default()
    /// };
    ///
    /// assert_eq!(config.voice_name(), "en-US-JennyNeural");
    /// ```
    pub fn voice_name(&self) -> &str {
        const DEFAULT_VOICE: &str = "en-US-JennyNeural";

        // Check voice_id first
        if let Some(voice_id) = &self.base.voice_id
            && !voice_id.is_empty()
        {
            return voice_id;
        }

        // Fall back to model
        if !self.base.model.is_empty() {
            return &self.base.model;
        }

        DEFAULT_VOICE
    }

    /// Builds the SSML document for a given text input.
    ///
    /// Uses the configuration's voice name, language, and speaking rate
    /// to construct a valid SSML document.
    ///
    /// # Arguments
    ///
    /// * `text` - The text to synthesize
    ///
    /// # Returns
    ///
    /// The SSML document as a string, or the original text if `use_ssml` is false.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::tts::{TTSConfig, azure::AzureTTSConfig};
    ///
    /// let config = AzureTTSConfig {
    ///     base: TTSConfig {
    ///         voice_id: Some("en-US-JennyNeural".to_string()),
    ///         speaking_rate: Some(1.2),
    ///         ..Default::default()
    ///     },
    ///     use_ssml: true,
    ///     ..Default::default()
    /// };
    ///
    /// let ssml = config.build_ssml_for_text("Hello world!");
    /// assert!(ssml.contains("en-US-JennyNeural"));
    /// assert!(ssml.contains("rate=\"120%\""));
    /// ```
    pub fn build_ssml_for_text(&self, text: &str) -> String {
        if !self.use_ssml {
            return text.to_string();
        }

        build_ssml(
            text,
            self.voice_name(),
            &self.language_code(),
            self.base.speaking_rate,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // AzureAudioEncoding Tests
    // =========================================================================

    #[test]
    fn test_audio_encoding_as_str() {
        // PCM formats
        assert_eq!(
            AzureAudioEncoding::Raw8Khz8BitMonoMulaw.as_str(),
            "raw-8khz-8bit-mono-mulaw"
        );
        assert_eq!(
            AzureAudioEncoding::Raw8Khz8BitMonoAlaw.as_str(),
            "raw-8khz-8bit-mono-alaw"
        );
        assert_eq!(
            AzureAudioEncoding::Raw8Khz16BitMonoPcm.as_str(),
            "raw-8khz-16bit-mono-pcm"
        );
        assert_eq!(
            AzureAudioEncoding::Raw16Khz16BitMonoPcm.as_str(),
            "raw-16khz-16bit-mono-pcm"
        );
        assert_eq!(
            AzureAudioEncoding::Raw22050Hz16BitMonoPcm.as_str(),
            "raw-22050hz-16bit-mono-pcm"
        );
        assert_eq!(
            AzureAudioEncoding::Raw24Khz16BitMonoPcm.as_str(),
            "raw-24khz-16bit-mono-pcm"
        );
        assert_eq!(
            AzureAudioEncoding::Raw44100Hz16BitMonoPcm.as_str(),
            "raw-44100hz-16bit-mono-pcm"
        );
        assert_eq!(
            AzureAudioEncoding::Raw48Khz16BitMonoPcm.as_str(),
            "raw-48khz-16bit-mono-pcm"
        );

        // MP3 formats
        assert_eq!(
            AzureAudioEncoding::Audio16Khz32KbitrateMonoMp3.as_str(),
            "audio-16khz-32kbitrate-mono-mp3"
        );
        assert_eq!(
            AzureAudioEncoding::Audio16Khz64KbitrateMonoMp3.as_str(),
            "audio-16khz-64kbitrate-mono-mp3"
        );
        assert_eq!(
            AzureAudioEncoding::Audio24Khz48KbitrateMonoMp3.as_str(),
            "audio-24khz-48kbitrate-mono-mp3"
        );
        assert_eq!(
            AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.as_str(),
            "audio-24khz-96kbitrate-mono-mp3"
        );
        assert_eq!(
            AzureAudioEncoding::Audio48Khz96KbitrateMonoMp3.as_str(),
            "audio-48khz-96kbitrate-mono-mp3"
        );
        assert_eq!(
            AzureAudioEncoding::Audio48Khz192KbitrateMonoMp3.as_str(),
            "audio-48khz-192kbitrate-mono-mp3"
        );

        // Opus formats
        assert_eq!(
            AzureAudioEncoding::Audio16Khz16Bit32KbpsMonoOpus.as_str(),
            "audio-16khz-16bit-32kbps-mono-opus"
        );
        assert_eq!(
            AzureAudioEncoding::Audio24Khz16Bit24KbpsMonoOpus.as_str(),
            "audio-24khz-16bit-24kbps-mono-opus"
        );
        assert_eq!(
            AzureAudioEncoding::Audio24Khz16Bit48KbpsMonoOpus.as_str(),
            "audio-24khz-16bit-48kbps-mono-opus"
        );
    }

    #[test]
    fn test_audio_encoding_sample_rate() {
        // 8kHz formats
        assert_eq!(AzureAudioEncoding::Raw8Khz8BitMonoMulaw.sample_rate(), 8000);
        assert_eq!(AzureAudioEncoding::Raw8Khz8BitMonoAlaw.sample_rate(), 8000);
        assert_eq!(AzureAudioEncoding::Raw8Khz16BitMonoPcm.sample_rate(), 8000);

        // 16kHz formats
        assert_eq!(
            AzureAudioEncoding::Raw16Khz16BitMonoPcm.sample_rate(),
            16000
        );
        assert_eq!(
            AzureAudioEncoding::Audio16Khz32KbitrateMonoMp3.sample_rate(),
            16000
        );
        assert_eq!(
            AzureAudioEncoding::Audio16Khz16Bit32KbpsMonoOpus.sample_rate(),
            16000
        );

        // 22.05kHz format
        assert_eq!(
            AzureAudioEncoding::Raw22050Hz16BitMonoPcm.sample_rate(),
            22050
        );

        // 24kHz formats
        assert_eq!(
            AzureAudioEncoding::Raw24Khz16BitMonoPcm.sample_rate(),
            24000
        );
        assert_eq!(
            AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.sample_rate(),
            24000
        );
        assert_eq!(
            AzureAudioEncoding::Audio24Khz16Bit48KbpsMonoOpus.sample_rate(),
            24000
        );

        // 44.1kHz format
        assert_eq!(
            AzureAudioEncoding::Raw44100Hz16BitMonoPcm.sample_rate(),
            44100
        );

        // 48kHz formats
        assert_eq!(
            AzureAudioEncoding::Raw48Khz16BitMonoPcm.sample_rate(),
            48000
        );
        assert_eq!(
            AzureAudioEncoding::Audio48Khz192KbitrateMonoMp3.sample_rate(),
            48000
        );
    }

    #[test]
    fn test_audio_encoding_is_pcm() {
        // PCM formats return true
        assert!(AzureAudioEncoding::Raw8Khz16BitMonoPcm.is_pcm());
        assert!(AzureAudioEncoding::Raw16Khz16BitMonoPcm.is_pcm());
        assert!(AzureAudioEncoding::Raw22050Hz16BitMonoPcm.is_pcm());
        assert!(AzureAudioEncoding::Raw24Khz16BitMonoPcm.is_pcm());
        assert!(AzureAudioEncoding::Raw44100Hz16BitMonoPcm.is_pcm());
        assert!(AzureAudioEncoding::Raw48Khz16BitMonoPcm.is_pcm());

        // Non-PCM formats return false
        assert!(!AzureAudioEncoding::Raw8Khz8BitMonoMulaw.is_pcm());
        assert!(!AzureAudioEncoding::Raw8Khz8BitMonoAlaw.is_pcm());
        assert!(!AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.is_pcm());
        assert!(!AzureAudioEncoding::Audio24Khz16Bit48KbpsMonoOpus.is_pcm());
    }

    #[test]
    fn test_audio_encoding_is_telephony() {
        assert!(AzureAudioEncoding::Raw8Khz8BitMonoMulaw.is_telephony());
        assert!(AzureAudioEncoding::Raw8Khz8BitMonoAlaw.is_telephony());

        assert!(!AzureAudioEncoding::Raw8Khz16BitMonoPcm.is_telephony());
        assert!(!AzureAudioEncoding::Raw24Khz16BitMonoPcm.is_telephony());
        assert!(!AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.is_telephony());
    }

    #[test]
    fn test_audio_encoding_content_type() {
        // PCM formats
        assert_eq!(
            AzureAudioEncoding::Raw24Khz16BitMonoPcm.content_type(),
            "audio/pcm"
        );
        assert_eq!(
            AzureAudioEncoding::Raw48Khz16BitMonoPcm.content_type(),
            "audio/pcm"
        );

        // Telephony formats
        assert_eq!(
            AzureAudioEncoding::Raw8Khz8BitMonoMulaw.content_type(),
            "audio/mulaw"
        );
        assert_eq!(
            AzureAudioEncoding::Raw8Khz8BitMonoAlaw.content_type(),
            "audio/alaw"
        );

        // MP3 formats
        assert_eq!(
            AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3.content_type(),
            "audio/mpeg"
        );
        assert_eq!(
            AzureAudioEncoding::Audio48Khz192KbitrateMonoMp3.content_type(),
            "audio/mpeg"
        );

        // Opus formats
        assert_eq!(
            AzureAudioEncoding::Audio24Khz16Bit48KbpsMonoOpus.content_type(),
            "audio/opus"
        );
    }

    #[test]
    fn test_audio_encoding_default() {
        assert_eq!(
            AzureAudioEncoding::default(),
            AzureAudioEncoding::Raw24Khz16BitMonoPcm
        );
    }

    #[test]
    fn test_audio_encoding_from_format_string() {
        // PCM formats
        assert_eq!(
            AzureAudioEncoding::from_format_string("linear16", 24000),
            AzureAudioEncoding::Raw24Khz16BitMonoPcm
        );
        assert_eq!(
            AzureAudioEncoding::from_format_string("pcm", 16000),
            AzureAudioEncoding::Raw16Khz16BitMonoPcm
        );
        assert_eq!(
            AzureAudioEncoding::from_format_string("wav", 48000),
            AzureAudioEncoding::Raw48Khz16BitMonoPcm
        );
        assert_eq!(
            AzureAudioEncoding::from_format_string("PCM", 8000),
            AzureAudioEncoding::Raw8Khz16BitMonoPcm
        );

        // MP3 formats
        assert_eq!(
            AzureAudioEncoding::from_format_string("mp3", 24000),
            AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3
        );
        assert_eq!(
            AzureAudioEncoding::from_format_string("MP3", 16000),
            AzureAudioEncoding::Audio16Khz64KbitrateMonoMp3
        );
        assert_eq!(
            AzureAudioEncoding::from_format_string("mp3", 48000),
            AzureAudioEncoding::Audio48Khz192KbitrateMonoMp3
        );

        // Telephony formats
        assert_eq!(
            AzureAudioEncoding::from_format_string("mulaw", 8000),
            AzureAudioEncoding::Raw8Khz8BitMonoMulaw
        );
        assert_eq!(
            AzureAudioEncoding::from_format_string("ulaw", 8000),
            AzureAudioEncoding::Raw8Khz8BitMonoMulaw
        );
        assert_eq!(
            AzureAudioEncoding::from_format_string("alaw", 8000),
            AzureAudioEncoding::Raw8Khz8BitMonoAlaw
        );

        // Opus formats
        assert_eq!(
            AzureAudioEncoding::from_format_string("opus", 24000),
            AzureAudioEncoding::Audio24Khz16Bit48KbpsMonoOpus
        );
        assert_eq!(
            AzureAudioEncoding::from_format_string("opus", 16000),
            AzureAudioEncoding::Audio16Khz16Bit32KbpsMonoOpus
        );

        // Unknown format defaults
        assert_eq!(
            AzureAudioEncoding::from_format_string("unknown", 24000),
            AzureAudioEncoding::Raw24Khz16BitMonoPcm
        );
        assert_eq!(
            AzureAudioEncoding::from_format_string("", 24000),
            AzureAudioEncoding::Raw24Khz16BitMonoPcm
        );
    }

    // =========================================================================
    // SSML Generation Tests
    // =========================================================================

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("Hello world"), "Hello world");
        assert_eq!(escape_xml("Hello & goodbye"), "Hello &amp; goodbye");
        assert_eq!(escape_xml("<script>"), "&lt;script&gt;");
        assert_eq!(escape_xml("He said \"hi\""), "He said &quot;hi&quot;");
        assert_eq!(escape_xml("It's nice"), "It&apos;s nice");
        assert_eq!(
            escape_xml("<a href=\"test\">link</a>"),
            "&lt;a href=&quot;test&quot;&gt;link&lt;/a&gt;"
        );
        assert_eq!(escape_xml(""), "");
    }

    #[test]
    fn test_build_ssml_basic() {
        let ssml = build_ssml("Hello world!", "en-US-JennyNeural", "en-US", None);

        assert!(ssml.contains("<speak"));
        assert!(ssml.contains("version='1.0'"));
        assert!(ssml.contains("xmlns='http://www.w3.org/2001/10/synthesis'"));
        assert!(ssml.contains("xml:lang='en-US'"));
        assert!(ssml.contains("<voice name='en-US-JennyNeural'>"));
        assert!(ssml.contains("Hello world!"));
        assert!(ssml.contains("</voice>"));
        assert!(ssml.contains("</speak>"));
        assert!(!ssml.contains("<prosody"));
    }

    #[test]
    fn test_build_ssml_with_speaking_rate() {
        let ssml = build_ssml("Fast speech", "en-US-JennyNeural", "en-US", Some(1.5));

        assert!(ssml.contains("<prosody rate=\"150%\">"));
        assert!(ssml.contains("Fast speech"));
        assert!(ssml.contains("</prosody>"));
    }

    #[test]
    fn test_build_ssml_with_slow_rate() {
        let ssml = build_ssml("Slow speech", "en-US-JennyNeural", "en-US", Some(0.75));

        assert!(ssml.contains("<prosody rate=\"75%\">"));
        assert!(ssml.contains("Slow speech"));
    }

    #[test]
    fn test_build_ssml_with_normal_rate() {
        // Rate of exactly 1.0 should not add prosody
        let ssml = build_ssml("Normal speech", "en-US-JennyNeural", "en-US", Some(1.0));

        assert!(!ssml.contains("<prosody"));
        assert!(ssml.contains("Normal speech"));
    }

    #[test]
    fn test_build_ssml_escapes_special_chars() {
        let ssml = build_ssml(
            "Hello <user> & welcome!",
            "en-US-JennyNeural",
            "en-US",
            None,
        );

        assert!(ssml.contains("Hello &lt;user&gt; &amp; welcome!"));
        assert!(!ssml.contains("<user>"));
    }

    #[test]
    fn test_build_ssml_different_language() {
        let ssml = build_ssml("Guten Tag!", "de-DE-KatjaNeural", "de-DE", None);

        assert!(ssml.contains("xml:lang='de-DE'"));
        assert!(ssml.contains("<voice name='de-DE-KatjaNeural'>"));
    }

    // =========================================================================
    // AzureTTSConfig Tests
    // =========================================================================

    #[test]
    fn test_azure_tts_config_default() {
        let config = AzureTTSConfig::default();

        assert_eq!(config.region, AzureRegion::EastUS);
        assert_eq!(
            config.output_format,
            AzureAudioEncoding::Raw24Khz16BitMonoPcm
        );
        assert!(config.use_ssml);
    }

    #[test]
    fn test_azure_tts_config_from_base() {
        let base = TTSConfig {
            voice_id: Some("en-US-JennyNeural".to_string()),
            audio_format: Some("mp3".to_string()),
            sample_rate: Some(24000),
            speaking_rate: Some(1.2),
            ..Default::default()
        };

        let config = AzureTTSConfig::from_base(base);

        assert_eq!(
            config.output_format,
            AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3
        );
        assert_eq!(config.region, AzureRegion::EastUS);
        assert!(config.use_ssml);
    }

    #[test]
    fn test_azure_tts_config_with_region() {
        let base = TTSConfig::default();
        let config = AzureTTSConfig::with_region(base, AzureRegion::WestEurope);

        assert_eq!(config.region, AzureRegion::WestEurope);
    }

    #[test]
    fn test_azure_tts_config_build_url() {
        let config = AzureTTSConfig {
            region: AzureRegion::WestEurope,
            ..Default::default()
        };

        assert_eq!(
            config.build_tts_url(),
            "https://westeurope.tts.speech.microsoft.com/cognitiveservices/v1"
        );
    }

    #[test]
    fn test_azure_tts_config_build_url_various_regions() {
        let test_cases = vec![
            (AzureRegion::EastUS, "eastus"),
            (AzureRegion::WestEurope, "westeurope"),
            (AzureRegion::JapanEast, "japaneast"),
            (AzureRegion::SoutheastAsia, "southeastasia"),
        ];

        for (region, region_str) in test_cases {
            let config = AzureTTSConfig {
                region,
                ..Default::default()
            };

            let expected = format!(
                "https://{}.tts.speech.microsoft.com/cognitiveservices/v1",
                region_str
            );
            assert_eq!(config.build_tts_url(), expected);
        }
    }

    #[test]
    fn test_azure_tts_config_language_code() {
        // Standard voice name
        let config = AzureTTSConfig {
            base: TTSConfig {
                voice_id: Some("en-US-JennyNeural".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.language_code(), "en-US");

        // German voice
        let config = AzureTTSConfig {
            base: TTSConfig {
                voice_id: Some("de-DE-KatjaNeural".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.language_code(), "de-DE");

        // Empty voice_id defaults to en-US
        let config = AzureTTSConfig {
            base: TTSConfig {
                voice_id: None,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.language_code(), "en-US");

        // Invalid format defaults to en-US
        let config = AzureTTSConfig {
            base: TTSConfig {
                voice_id: Some("invalid".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.language_code(), "en-US");
    }

    #[test]
    fn test_azure_tts_config_voice_name() {
        // Has voice_id
        let config = AzureTTSConfig {
            base: TTSConfig {
                voice_id: Some("en-US-JennyNeural".to_string()),
                model: "fallback-model".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.voice_name(), "en-US-JennyNeural");

        // Empty voice_id falls back to model
        let config = AzureTTSConfig {
            base: TTSConfig {
                voice_id: Some(String::new()),
                model: "en-US-AriaNeural".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.voice_name(), "en-US-AriaNeural");

        // None voice_id falls back to model
        let config = AzureTTSConfig {
            base: TTSConfig {
                voice_id: None,
                model: "en-US-AriaNeural".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.voice_name(), "en-US-AriaNeural");

        // Both empty defaults to JennyNeural
        let config = AzureTTSConfig {
            base: TTSConfig {
                voice_id: None,
                model: String::new(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.voice_name(), "en-US-JennyNeural");
    }

    #[test]
    fn test_azure_tts_config_build_ssml_for_text() {
        let config = AzureTTSConfig {
            base: TTSConfig {
                voice_id: Some("en-US-JennyNeural".to_string()),
                speaking_rate: Some(1.2),
                ..Default::default()
            },
            use_ssml: true,
            ..Default::default()
        };

        let ssml = config.build_ssml_for_text("Hello world!");

        assert!(ssml.contains("<speak"));
        assert!(ssml.contains("en-US-JennyNeural"));
        assert!(ssml.contains("xml:lang='en-US'"));
        assert!(ssml.contains("rate=\"120%\""));
        assert!(ssml.contains("Hello world!"));
    }

    #[test]
    fn test_azure_tts_config_build_ssml_disabled() {
        let config = AzureTTSConfig {
            base: TTSConfig {
                voice_id: Some("en-US-JennyNeural".to_string()),
                ..Default::default()
            },
            use_ssml: false,
            ..Default::default()
        };

        let result = config.build_ssml_for_text("Hello world!");

        assert_eq!(result, "Hello world!");
        assert!(!result.contains("<speak"));
    }

    #[test]
    fn test_azure_tts_config_serialization() {
        let config = AzureTTSConfig {
            base: TTSConfig::default(),
            region: AzureRegion::WestEurope,
            output_format: AzureAudioEncoding::Audio24Khz96KbitrateMonoMp3,
            use_ssml: true,
        };

        // The output_format should be serializable
        let json = serde_json::to_string(&config.output_format).expect("Failed to serialize");
        assert!(json.contains("Audio24Khz96KbitrateMonoMp3"));
    }
}
