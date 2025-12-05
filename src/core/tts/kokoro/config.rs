//! Configuration for Kokoro TTS provider
//!
//! This module provides configuration options for the Kokoro 82M ONNX-based TTS provider.
//! Kokoro TTS runs locally using ONNX Runtime and requires no cloud API.
//!
//! ## Available Voices
//!
//! Voice files are stored in the cache directory under `voices/`. Run `sayna init` to
//! download all available voices from HuggingFace. Use `assets::list_available_voices()`
//! to get the list of downloaded voices.
//!
//! ### Voice Naming Convention
//!
//! Voice IDs follow the pattern: `{accent}{gender}_{name}`
//! - First letter: `a` = American, `b` = British, etc.
//! - Second letter: `f` = Female, `m` = Male
//! - Name: Voice name (e.g., `bella`, `adam`)

use crate::core::tts::{TTSConfig, TTSError, TTSResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::assets;

/// Output sample rate for Kokoro TTS (fixed by model architecture)
#[allow(dead_code)]
pub const KOKORO_SAMPLE_RATE: u32 = 24000;

/// Configuration specific to Kokoro TTS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KokoroConfig {
    /// Voice ID to use (e.g., "af_bella", "am_adam", "bf_emma")
    /// Default: "af_bella" (American Female - Bella)
    #[serde(default = "default_voice_id")]
    pub voice_id: String,

    /// Speaking rate multiplier (0.1 to 5.0, default 1.0)
    /// Values < 1.0 produce slower speech, > 1.0 produce faster speech.
    #[serde(default = "default_speaking_rate")]
    pub speaking_rate: f32,

    /// Cache directory for downloaded assets
    /// If not set, uses XDG_CACHE_HOME/kokoro or ~/.cache/kokoro
    #[serde(default)]
    pub cache_path: Option<PathBuf>,
}

fn default_voice_id() -> String {
    super::DEFAULT_VOICE.to_string()
}

fn default_speaking_rate() -> f32 {
    1.0
}

impl Default for KokoroConfig {
    fn default() -> Self {
        Self {
            voice_id: super::DEFAULT_VOICE.to_string(),
            speaking_rate: 1.0,
            cache_path: None,
        }
    }
}

impl KokoroConfig {
    /// Validate the configuration (basic validation without checking file existence)
    pub fn validate(&self) -> TTSResult<()> {
        // Validate speaking rate
        if !(0.1..=5.0).contains(&self.speaking_rate) {
            return Err(TTSError::InvalidConfiguration(format!(
                "Speaking rate must be between 0.1 and 5.0, got {}",
                self.speaking_rate
            )));
        }

        // Validate voice ID is not empty
        if self.voice_id.is_empty() {
            return Err(TTSError::InvalidConfiguration(
                "Voice ID cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate that the voice file exists in the cache
    pub fn validate_voice(&self, asset_config: &assets::KokoroAssetConfig) -> TTSResult<()> {
        if !assets::is_voice_available(asset_config, &self.voice_id) {
            let available = assets::list_available_voices(asset_config);
            return Err(TTSError::InvalidConfiguration(format!(
                "Voice '{}' not found. Available voices: {:?}. Run `sayna init` to download.",
                self.voice_id, available
            )));
        }
        Ok(())
    }

    /// Get the asset configuration
    pub fn to_asset_config(&self) -> assets::KokoroAssetConfig {
        assets::KokoroAssetConfig {
            cache_path: self
                .cache_path
                .clone()
                .unwrap_or_else(assets::get_default_cache_path),
        }
    }

    /// Create a KokoroConfig from a generic TTSConfig
    pub fn from_tts_config(config: &TTSConfig) -> TTSResult<Self> {
        let mut kokoro_config = KokoroConfig::default();

        // Voice ID
        if let Some(ref voice_id) = config.voice_id {
            kokoro_config.voice_id = voice_id.clone();
        }

        // Speaking rate with validation
        if let Some(rate) = config.speaking_rate {
            if !(0.1..=5.0).contains(&rate) {
                return Err(TTSError::InvalidConfiguration(format!(
                    "Speaking rate must be between 0.1 and 5.0, got {}",
                    rate
                )));
            }
            kokoro_config.speaking_rate = rate;
        }

        Ok(kokoro_config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = KokoroConfig::default();
        assert_eq!(config.voice_id, "af_bella");
        assert_eq!(config.speaking_rate, 1.0);
        assert!(config.cache_path.is_none());
    }

    #[test]
    fn test_default_config_validates() {
        let config = KokoroConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_from_tts_config() {
        let tts_config = TTSConfig {
            provider: "kokoro".to_string(),
            voice_id: Some("am_adam".to_string()),
            speaking_rate: Some(1.5),
            ..Default::default()
        };

        let result = KokoroConfig::from_tts_config(&tts_config);
        assert!(result.is_ok());

        let config = result.unwrap();
        assert_eq!(config.voice_id, "am_adam");
        assert_eq!(config.speaking_rate, 1.5);
    }

    #[test]
    fn test_invalid_speaking_rate_too_high() {
        let tts_config = TTSConfig {
            provider: "kokoro".to_string(),
            speaking_rate: Some(10.0),
            ..Default::default()
        };

        let result = KokoroConfig::from_tts_config(&tts_config);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_speaking_rate_too_low() {
        let tts_config = TTSConfig {
            provider: "kokoro".to_string(),
            speaking_rate: Some(0.05),
            ..Default::default()
        };

        let result = KokoroConfig::from_tts_config(&tts_config);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_invalid_speed() {
        let config = KokoroConfig {
            speaking_rate: 10.0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_voice_id() {
        let config = KokoroConfig {
            voice_id: String::new(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_unknown_voice() {
        // Basic validation no longer checks voice file existence
        // (that happens at connect time via validate_voice)
        let config = KokoroConfig {
            voice_id: "nonexistent_voice".to_string(),
            ..Default::default()
        };
        // Basic validation should pass (voice ID is not empty)
        assert!(config.validate().is_ok());

        // But validate_voice should fail when checked against asset config
        let asset_config = config.to_asset_config();
        assert!(config.validate_voice(&asset_config).is_err());
    }

    #[test]
    fn test_to_asset_config_default() {
        let config = KokoroConfig::default();
        let asset_config = config.to_asset_config();
        assert!(asset_config.cache_path.to_string_lossy().contains("kokoro"));
    }

    #[test]
    fn test_to_asset_config_custom_path() {
        let config = KokoroConfig {
            cache_path: Some(PathBuf::from("/custom/cache")),
            ..Default::default()
        };
        let asset_config = config.to_asset_config();
        assert_eq!(asset_config.cache_path, PathBuf::from("/custom/cache"));
    }

    #[test]
    fn test_serde_serialization() {
        let config = KokoroConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("af_bella"));
    }

    #[test]
    fn test_serde_deserialization() {
        let json = r#"{
            "voice_id": "bf_emma",
            "speaking_rate": 1.2
        }"#;

        let config: KokoroConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.voice_id, "bf_emma");
        assert_eq!(config.speaking_rate, 1.2);
    }

    #[test]
    fn test_serde_deserialization_with_defaults() {
        let json = r#"{}"#;
        let config: KokoroConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.voice_id, "af_bella");
        assert_eq!(config.speaking_rate, 1.0);
    }
}
