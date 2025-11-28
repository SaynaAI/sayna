//! Google Cloud Text-to-Speech configuration.
//!
//! This module provides Google-specific TTS configuration that extends the base
//! `TTSConfig` with parameters specific to the Google Cloud Text-to-Speech API.
//!
//! # Architecture
//!
//! The `GoogleTTSConfig` struct embeds the base `TTSConfig` and adds Google-specific
//! fields like audio encoding, pitch, volume gain, and effects profiles. This follows
//! the same pattern as `GoogleSTTConfig` for Speech-to-Text.
//!
//! # Example
//!
//! ```rust,ignore
//! use sayna::core::tts::google::GoogleTTSConfig;
//! use sayna::core::tts::TTSConfig;
//!
//! let base_config = TTSConfig {
//!     voice_id: Some("en-US-Wavenet-D".to_string()),
//!     audio_format: Some("mp3".to_string()),
//!     speaking_rate: Some(1.2),
//!     ..Default::default()
//! };
//!
//! let google_config = GoogleTTSConfig::from_base_config(base_config, "my-project-id".to_string());
//!
//! assert_eq!(google_config.language_code, "en-US");
//! assert_eq!(google_config.audio_encoding.as_str(), "MP3");
//! ```

use crate::core::tts::base::TTSConfig;
use serde::{Deserialize, Serialize};

/// Audio encoding formats supported by Google Cloud Text-to-Speech API.
///
/// This enum provides type-safe representation of Google's supported audio encodings,
/// ensuring compile-time validation of format names.
///
/// # Supported Formats
///
/// | Variant | API Value | Description |
/// |---------|-----------|-------------|
/// | `Linear16` | "LINEAR16" | 16-bit PCM audio |
/// | `Mp3` | "MP3" | MP3 at 32kbps |
/// | `OggOpus` | "OGG_OPUS" | Opus codec in OGG container |
/// | `Mulaw` | "MULAW" | 8-bit mu-law (telephony) |
/// | `Alaw` | "ALAW" | 8-bit A-law (telephony) |
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GoogleAudioEncoding {
    /// 16-bit linear PCM audio
    #[default]
    Linear16,
    /// MP3 audio format
    Mp3,
    /// Opus codec in OGG container
    OggOpus,
    /// 8-bit mu-law encoding (telephony)
    Mulaw,
    /// 8-bit A-law encoding (telephony)
    Alaw,
}

impl GoogleAudioEncoding {
    /// Returns the string value expected by Google's TTS API.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::google::GoogleAudioEncoding;
    ///
    /// assert_eq!(GoogleAudioEncoding::Linear16.as_str(), "LINEAR16");
    /// assert_eq!(GoogleAudioEncoding::Mp3.as_str(), "MP3");
    /// assert_eq!(GoogleAudioEncoding::OggOpus.as_str(), "OGG_OPUS");
    /// ```
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linear16 => "LINEAR16",
            Self::Mp3 => "MP3",
            Self::OggOpus => "OGG_OPUS",
            Self::Mulaw => "MULAW",
            Self::Alaw => "ALAW",
        }
    }

    /// Converts a base config format string to the appropriate Google encoding.
    ///
    /// This method maps common format string variations to the correct enum variant:
    /// - "linear16", "pcm", "wav" → `Linear16`
    /// - "mp3" → `Mp3`
    /// - "ogg_opus", "opus", "ogg" → `OggOpus`
    /// - "mulaw", "ulaw" → `Mulaw`
    /// - "alaw" → `Alaw`
    /// - Unknown formats default to `Linear16`
    ///
    /// # Arguments
    ///
    /// * `format` - The audio format string from base config
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::google::GoogleAudioEncoding;
    ///
    /// assert_eq!(GoogleAudioEncoding::from_format_string("mp3"), GoogleAudioEncoding::Mp3);
    /// assert_eq!(GoogleAudioEncoding::from_format_string("pcm"), GoogleAudioEncoding::Linear16);
    /// assert_eq!(GoogleAudioEncoding::from_format_string("opus"), GoogleAudioEncoding::OggOpus);
    /// ```
    pub fn from_format_string(format: &str) -> Self {
        match format.to_lowercase().as_str() {
            "linear16" | "pcm" | "wav" => Self::Linear16,
            "mp3" => Self::Mp3,
            "ogg_opus" | "ogg-opus" | "opus" | "ogg" => Self::OggOpus,
            "mulaw" | "ulaw" => Self::Mulaw,
            "alaw" => Self::Alaw,
            _ => Self::Linear16,
        }
    }
}

/// Configuration specific to Google Cloud Text-to-Speech API.
///
/// This struct extends the base `TTSConfig` with Google-specific parameters
/// including audio encoding, pitch adjustment, volume gain, and effects profiles.
///
/// # Fields
///
/// | Field | Type | Purpose |
/// |-------|------|---------|
/// | `base` | `TTSConfig` | Base configuration from provider factory |
/// | `project_id` | `String` | Google Cloud project ID |
/// | `language_code` | `String` | BCP-47 language code (e.g., "en-US") |
/// | `audio_encoding` | `GoogleAudioEncoding` | Output audio format |
/// | `pitch` | `Option<f64>` | Pitch adjustment in semitones (-20 to +20) |
/// | `volume_gain_db` | `Option<f64>` | Volume adjustment in dB (-96 to +16) |
/// | `effects_profile_id` | `Vec<String>` | Audio effects profiles for optimization |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleTTSConfig {
    /// Base TTS configuration
    pub base: TTSConfig,
    /// Google Cloud project ID (extracted from credentials)
    pub project_id: String,
    /// BCP-47 language code for voice selection (e.g., "en-US", "es-ES")
    pub language_code: String,
    /// Output audio encoding format
    pub audio_encoding: GoogleAudioEncoding,
    /// Pitch adjustment in semitones (-20.0 to +20.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    /// Volume gain adjustment in dB (-96.0 to +16.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_gain_db: Option<f64>,
    /// Audio effects profiles for playback optimization
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects_profile_id: Vec<String>,
}

impl Default for GoogleTTSConfig {
    fn default() -> Self {
        Self {
            base: TTSConfig::default(),
            project_id: String::new(),
            language_code: "en-US".to_string(),
            audio_encoding: GoogleAudioEncoding::Linear16,
            pitch: None,
            volume_gain_db: None,
            effects_profile_id: Vec::new(),
        }
    }
}

impl GoogleTTSConfig {
    /// Creates a `GoogleTTSConfig` from a base `TTSConfig` and project ID.
    ///
    /// This factory method converts the base configuration to Google-specific settings:
    /// - Maps `audio_format` string to `GoogleAudioEncoding` enum
    /// - Extracts language code from voice name (e.g., "en-US-Wavenet-D" → "en-US")
    /// - Sets Google-specific defaults for optional fields
    ///
    /// # Arguments
    ///
    /// * `config` - The base TTS configuration
    /// * `project_id` - Google Cloud project ID
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use sayna::core::tts::{TTSConfig, google::GoogleTTSConfig};
    ///
    /// let base = TTSConfig {
    ///     voice_id: Some("en-US-Wavenet-D".to_string()),
    ///     audio_format: Some("mp3".to_string()),
    ///     ..Default::default()
    /// };
    ///
    /// let google = GoogleTTSConfig::from_base_config(base, "my-project".to_string());
    /// assert_eq!(google.language_code, "en-US");
    /// ```
    pub fn from_base_config(config: TTSConfig, project_id: String) -> Self {
        // Map audio format to Google encoding
        let audio_encoding = config
            .audio_format
            .as_deref()
            .map(GoogleAudioEncoding::from_format_string)
            .unwrap_or_default();

        // Extract language code from voice name
        let language_code = Self::extract_language_code(config.voice_id.as_deref());

        Self {
            base: config,
            project_id,
            language_code,
            audio_encoding,
            pitch: None,
            volume_gain_db: None,
            effects_profile_id: Vec::new(),
        }
    }

    /// Extracts the BCP-47 language code from a Google voice name.
    ///
    /// Google voice names follow the pattern `{lang}-{region}-{type}-{variant}`,
    /// for example "en-US-Wavenet-D" or "cmn-CN-Standard-A".
    ///
    /// # Arguments
    ///
    /// * `voice_name` - Optional voice name string
    ///
    /// # Returns
    ///
    /// The extracted language code, or "en-US" if extraction fails.
    ///
    /// # Examples
    ///
    /// | Voice Name | Extracted Code |
    /// |------------|----------------|
    /// | `en-US-Wavenet-D` | `en-US` |
    /// | `es-ES-Standard-B` | `es-ES` |
    /// | `cmn-CN-Wavenet-A` | `cmn-CN` |
    /// | `de-DE-Neural2-A` | `de-DE` |
    /// | `invalid` | `en-US` (default) |
    /// | `""` (empty) | `en-US` (default) |
    fn extract_language_code(voice_name: Option<&str>) -> String {
        const DEFAULT_LANGUAGE: &str = "en-US";

        let voice_name = match voice_name {
            Some(name) if !name.is_empty() => name,
            _ => return DEFAULT_LANGUAGE.to_string(),
        };

        // Split by '-' and try to extract language-region
        let parts: Vec<&str> = voice_name.split('-').collect();

        // Need at least 2 parts for language-region (e.g., "en-US")
        // Some languages have 3-letter codes (e.g., "cmn-CN")
        if parts.len() >= 2 {
            // Handle both 2-letter (en) and 3-letter (cmn) language codes
            let first_part = parts[0];
            let second_part = parts[1];

            // If second part looks like a region code (2 uppercase letters), combine them
            if second_part.len() == 2 && second_part.chars().all(|c| c.is_ascii_uppercase()) {
                return format!("{}-{}", first_part, second_part);
            }
            // If second part is lowercase, it might be part of the language code (unlikely but handle gracefully)
            // Fall through to default
        }

        DEFAULT_LANGUAGE.to_string()
    }

    /// Returns the voice name for the API request.
    ///
    /// Checks `voice_id` first, falling back to `model` if `voice_id` is empty.
    /// Returns `None` if both are empty.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = GoogleTTSConfig {
    ///     base: TTSConfig {
    ///         voice_id: Some("en-US-Wavenet-D".to_string()),
    ///         ..Default::default()
    ///     },
    ///     ..Default::default()
    /// };
    /// assert_eq!(config.voice_name(), Some("en-US-Wavenet-D"));
    /// ```
    pub fn voice_name(&self) -> Option<&str> {
        // Check voice_id first
        if let Some(voice_id) = &self.base.voice_id
            && !voice_id.is_empty()
        {
            return Some(voice_id);
        }

        // Fall back to model
        if !self.base.model.is_empty() {
            return Some(&self.base.model);
        }

        None
    }

    /// Returns the speaking rate, clamped to Google's valid range [0.25, 4.0].
    ///
    /// Returns `None` if no speaking rate is configured.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = GoogleTTSConfig {
    ///     base: TTSConfig {
    ///         speaking_rate: Some(5.0), // Above max
    ///         ..Default::default()
    ///     },
    ///     ..Default::default()
    /// };
    /// assert_eq!(config.speaking_rate(), Some(4.0)); // Clamped to max
    /// ```
    pub fn speaking_rate(&self) -> Option<f64> {
        self.base.speaking_rate.map(|rate| {
            let rate = rate as f64;
            rate.clamp(0.25, 4.0)
        })
    }

    /// Returns the sample rate as i32 for the API request.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let config = GoogleTTSConfig {
    ///     base: TTSConfig {
    ///         sample_rate: Some(24000),
    ///         ..Default::default()
    ///     },
    ///     ..Default::default()
    /// };
    /// assert_eq!(config.sample_rate_hertz(), Some(24000));
    /// ```
    pub fn sample_rate_hertz(&self) -> Option<i32> {
        self.base.sample_rate.map(|sr| sr as i32)
    }

    /// Returns the pitch adjustment, clamped to Google's valid range [-20.0, 20.0].
    ///
    /// Returns `None` if no pitch adjustment is configured.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut config = GoogleTTSConfig::default();
    /// config.pitch = Some(25.0); // Above max
    /// assert_eq!(config.clamped_pitch(), Some(20.0)); // Clamped to max
    /// ```
    pub fn clamped_pitch(&self) -> Option<f64> {
        self.pitch.map(|p| p.clamp(-20.0, 20.0))
    }

    /// Returns the volume gain, clamped to Google's valid range [-96.0, 16.0].
    ///
    /// Returns `None` if no volume gain is configured.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut config = GoogleTTSConfig::default();
    /// config.volume_gain_db = Some(-100.0); // Below min
    /// assert_eq!(config.clamped_volume_gain(), Some(-96.0)); // Clamped to min
    /// ```
    pub fn clamped_volume_gain(&self) -> Option<f64> {
        self.volume_gain_db.map(|v| v.clamp(-96.0, 16.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== GoogleAudioEncoding Tests =====

    #[test]
    fn test_audio_encoding_as_str() {
        assert_eq!(GoogleAudioEncoding::Linear16.as_str(), "LINEAR16");
        assert_eq!(GoogleAudioEncoding::Mp3.as_str(), "MP3");
        assert_eq!(GoogleAudioEncoding::OggOpus.as_str(), "OGG_OPUS");
        assert_eq!(GoogleAudioEncoding::Mulaw.as_str(), "MULAW");
        assert_eq!(GoogleAudioEncoding::Alaw.as_str(), "ALAW");
    }

    #[test]
    fn test_audio_encoding_from_format_string() {
        // Linear16 variants
        assert_eq!(
            GoogleAudioEncoding::from_format_string("linear16"),
            GoogleAudioEncoding::Linear16
        );
        assert_eq!(
            GoogleAudioEncoding::from_format_string("pcm"),
            GoogleAudioEncoding::Linear16
        );
        assert_eq!(
            GoogleAudioEncoding::from_format_string("wav"),
            GoogleAudioEncoding::Linear16
        );
        assert_eq!(
            GoogleAudioEncoding::from_format_string("LINEAR16"),
            GoogleAudioEncoding::Linear16
        );

        // MP3
        assert_eq!(
            GoogleAudioEncoding::from_format_string("mp3"),
            GoogleAudioEncoding::Mp3
        );
        assert_eq!(
            GoogleAudioEncoding::from_format_string("MP3"),
            GoogleAudioEncoding::Mp3
        );

        // OGG_OPUS variants
        assert_eq!(
            GoogleAudioEncoding::from_format_string("ogg_opus"),
            GoogleAudioEncoding::OggOpus
        );
        assert_eq!(
            GoogleAudioEncoding::from_format_string("ogg-opus"),
            GoogleAudioEncoding::OggOpus
        );
        assert_eq!(
            GoogleAudioEncoding::from_format_string("opus"),
            GoogleAudioEncoding::OggOpus
        );
        assert_eq!(
            GoogleAudioEncoding::from_format_string("ogg"),
            GoogleAudioEncoding::OggOpus
        );

        // Mulaw variants
        assert_eq!(
            GoogleAudioEncoding::from_format_string("mulaw"),
            GoogleAudioEncoding::Mulaw
        );
        assert_eq!(
            GoogleAudioEncoding::from_format_string("ulaw"),
            GoogleAudioEncoding::Mulaw
        );

        // Alaw
        assert_eq!(
            GoogleAudioEncoding::from_format_string("alaw"),
            GoogleAudioEncoding::Alaw
        );

        // Unknown defaults to Linear16
        assert_eq!(
            GoogleAudioEncoding::from_format_string("unknown"),
            GoogleAudioEncoding::Linear16
        );
        assert_eq!(
            GoogleAudioEncoding::from_format_string(""),
            GoogleAudioEncoding::Linear16
        );
    }

    #[test]
    fn test_audio_encoding_default() {
        assert_eq!(
            GoogleAudioEncoding::default(),
            GoogleAudioEncoding::Linear16
        );
    }

    // ===== GoogleTTSConfig Tests =====

    #[test]
    fn test_google_tts_config_default() {
        let config = GoogleTTSConfig::default();

        assert!(config.project_id.is_empty());
        assert_eq!(config.language_code, "en-US");
        assert_eq!(config.audio_encoding, GoogleAudioEncoding::Linear16);
        assert!(config.pitch.is_none());
        assert!(config.volume_gain_db.is_none());
        assert!(config.effects_profile_id.is_empty());
    }

    #[test]
    fn test_from_base_config() {
        let base = TTSConfig {
            voice_id: Some("en-US-Wavenet-D".to_string()),
            audio_format: Some("mp3".to_string()),
            speaking_rate: Some(1.2),
            sample_rate: Some(24000),
            ..Default::default()
        };

        let google = GoogleTTSConfig::from_base_config(base, "my-project".to_string());

        assert_eq!(google.project_id, "my-project");
        assert_eq!(google.language_code, "en-US");
        assert_eq!(google.audio_encoding, GoogleAudioEncoding::Mp3);
        assert!(google.pitch.is_none());
        assert!(google.volume_gain_db.is_none());
    }

    #[test]
    fn test_extract_language_code() {
        // Standard Google voice names
        assert_eq!(
            GoogleTTSConfig::extract_language_code(Some("en-US-Wavenet-D")),
            "en-US"
        );
        assert_eq!(
            GoogleTTSConfig::extract_language_code(Some("en-GB-Neural2-A")),
            "en-GB"
        );
        assert_eq!(
            GoogleTTSConfig::extract_language_code(Some("es-ES-Standard-B")),
            "es-ES"
        );
        assert_eq!(
            GoogleTTSConfig::extract_language_code(Some("de-DE-Wavenet-A")),
            "de-DE"
        );

        // 3-letter language codes
        assert_eq!(
            GoogleTTSConfig::extract_language_code(Some("cmn-CN-Wavenet-A")),
            "cmn-CN"
        );

        // Edge cases - default to en-US
        assert_eq!(
            GoogleTTSConfig::extract_language_code(Some("invalid")),
            "en-US"
        );
        assert_eq!(GoogleTTSConfig::extract_language_code(Some("")), "en-US");
        assert_eq!(GoogleTTSConfig::extract_language_code(None), "en-US");
    }

    #[test]
    fn test_voice_name_priority() {
        // voice_id takes precedence
        let config = GoogleTTSConfig {
            base: TTSConfig {
                voice_id: Some("en-US-Wavenet-D".to_string()),
                model: "en-US-Standard-A".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.voice_name(), Some("en-US-Wavenet-D"));

        // Falls back to model when voice_id is empty
        let config = GoogleTTSConfig {
            base: TTSConfig {
                voice_id: Some(String::new()),
                model: "en-US-Standard-A".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.voice_name(), Some("en-US-Standard-A"));

        // Falls back to model when voice_id is None
        let config = GoogleTTSConfig {
            base: TTSConfig {
                voice_id: None,
                model: "en-US-Standard-A".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.voice_name(), Some("en-US-Standard-A"));

        // Returns None when both are empty
        let config = GoogleTTSConfig {
            base: TTSConfig {
                voice_id: None,
                model: String::new(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.voice_name(), None);
    }

    #[test]
    fn test_speaking_rate_clamping() {
        // Normal range
        let config = GoogleTTSConfig {
            base: TTSConfig {
                speaking_rate: Some(1.5),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.speaking_rate(), Some(1.5));

        // Below minimum
        let config = GoogleTTSConfig {
            base: TTSConfig {
                speaking_rate: Some(0.1),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.speaking_rate(), Some(0.25));

        // Above maximum
        let config = GoogleTTSConfig {
            base: TTSConfig {
                speaking_rate: Some(5.0),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.speaking_rate(), Some(4.0));

        // None
        let config = GoogleTTSConfig {
            base: TTSConfig {
                speaking_rate: None,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.speaking_rate(), None);
    }

    #[test]
    fn test_sample_rate_hertz() {
        let config = GoogleTTSConfig {
            base: TTSConfig {
                sample_rate: Some(24000),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.sample_rate_hertz(), Some(24000));

        let config = GoogleTTSConfig {
            base: TTSConfig {
                sample_rate: None,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.sample_rate_hertz(), None);
    }

    #[test]
    fn test_clamped_pitch() {
        // Normal range
        let config = GoogleTTSConfig {
            pitch: Some(5.0),
            ..Default::default()
        };
        assert_eq!(config.clamped_pitch(), Some(5.0));

        // Below minimum
        let config = GoogleTTSConfig {
            pitch: Some(-25.0),
            ..Default::default()
        };
        assert_eq!(config.clamped_pitch(), Some(-20.0));

        // Above maximum
        let config = GoogleTTSConfig {
            pitch: Some(25.0),
            ..Default::default()
        };
        assert_eq!(config.clamped_pitch(), Some(20.0));

        // None
        let config = GoogleTTSConfig::default();
        assert_eq!(config.clamped_pitch(), None);
    }

    #[test]
    fn test_clamped_volume_gain() {
        // Normal range
        let config = GoogleTTSConfig {
            volume_gain_db: Some(0.0),
            ..Default::default()
        };
        assert_eq!(config.clamped_volume_gain(), Some(0.0));

        // Below minimum
        let config = GoogleTTSConfig {
            volume_gain_db: Some(-100.0),
            ..Default::default()
        };
        assert_eq!(config.clamped_volume_gain(), Some(-96.0));

        // Above maximum
        let config = GoogleTTSConfig {
            volume_gain_db: Some(20.0),
            ..Default::default()
        };
        assert_eq!(config.clamped_volume_gain(), Some(16.0));

        // None
        let config = GoogleTTSConfig::default();
        assert_eq!(config.clamped_volume_gain(), None);
    }

    #[test]
    fn test_serialization() {
        let config = GoogleTTSConfig {
            base: TTSConfig::default(),
            project_id: "test-project".to_string(),
            language_code: "en-US".to_string(),
            audio_encoding: GoogleAudioEncoding::Mp3,
            pitch: Some(2.5),
            volume_gain_db: None,
            effects_profile_id: vec!["handset-class-device".to_string()],
        };

        let json = serde_json::to_string(&config).expect("Failed to serialize");
        let deserialized: GoogleTTSConfig =
            serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.project_id, "test-project");
        assert_eq!(deserialized.audio_encoding, GoogleAudioEncoding::Mp3);
        assert_eq!(deserialized.pitch, Some(2.5));
        assert!(deserialized.volume_gain_db.is_none());
        assert_eq!(deserialized.effects_profile_id.len(), 1);
    }
}
