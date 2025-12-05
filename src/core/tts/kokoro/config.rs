//! Configuration for Kokoro TTS provider
//!
//! This module provides configuration options for the Kokoro 82M ONNX-based TTS provider.
//! Kokoro TTS runs locally using ONNX Runtime and requires no cloud API.
//!
//! ## Available Voices
//!
//! ### American English
//! - **Female**: `af_bella`, `af_jessica`, `af_kailey`, `af_nicole`, `af_nova`, `af_river`,
//!   `af_sarah`, `af_sky`, `af_v0bella`, `af_v0isley`, `af_v0nicole`, `af_v0sarah`
//! - **Male**: `am_adam`, `am_echo`, `am_eric`, `am_fenrir`, `am_liam`, `am_michael`,
//!   `am_v0adam`, `am_v0michael`
//!
//! ### British English
//! - **Female**: `bf_alice`, `bf_emma`, `bf_isabella`, `bf_lily`
//! - **Male**: `bm_daniel`, `bm_fable`, `bm_george`, `bm_lewis`
//!
//! ## Voice Naming Convention
//!
//! Voice IDs follow the pattern: `{accent}{gender}_{name}`
//! - First letter: `a` = American, `b` = British
//! - Second letter: `f` = Female, `m` = Male
//! - Name: Voice name (e.g., `bella`, `adam`)
//!
//! ## Voice Mixing
//!
//! Blend multiple voices using weighted syntax:
//! ```text
//! af_bella.5+am_adam.5  // 50% Bella + 50% Adam
//! ```

use crate::core::tts::{TTSConfig, TTSError, TTSResult};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default model file name
pub const DEFAULT_MODEL_FILE: &str = "kokoro-v1.0.onnx";

/// Timestamped model file name (for word alignment)
#[allow(dead_code)]
pub const DEFAULT_MODEL_TIMESTAMPED_FILE: &str = "kokoro-v1.0-timestamped.onnx";

/// Default voices file name
pub const DEFAULT_VOICES_FILE: &str = "voices-v1.0.bin";

/// Default cache subdirectory name
pub const DEFAULT_CACHE_SUBDIR: &str = "kokoro_tts";

/// Default model URL (non-quantized)
pub const DEFAULT_MODEL_URL: &str = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx";

/// Default quantized model URL
pub const DEFAULT_MODEL_QUANTIZED_URL: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped/resolve/main/model_quantized.onnx";

/// Default timestamped model URL (for word alignment)
pub const DEFAULT_MODEL_TIMESTAMPED_URL: &str = "https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped/resolve/main/model.onnx";

/// Default voices URL
pub const DEFAULT_VOICES_URL: &str = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin";

/// Output sample rate for Kokoro TTS (fixed by model architecture)
#[allow(dead_code)]
pub const KOKORO_SAMPLE_RATE: u32 = 24000;

/// ONNX graph optimization level
///
/// Controls how aggressively ONNX Runtime optimizes the model graph.
/// Higher levels typically result in faster inference but longer initialization time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum GraphOptimizationLevel {
    /// No optimization
    Disabled,
    /// Basic optimizations (constant folding, redundant node elimination)
    Basic,
    /// Extended optimizations (includes basic + more advanced optimizations)
    Extended,
    /// All optimizations enabled (recommended for production)
    #[default]
    Level3,
}

impl GraphOptimizationLevel {
    /// Convert to ONNX Runtime optimization level
    #[cfg(feature = "kokoro-tts")]
    pub fn to_ort_level(self) -> ort::session::builder::GraphOptimizationLevel {
        use ort::session::builder::GraphOptimizationLevel as OrtLevel;
        match self {
            Self::Disabled => OrtLevel::Disable,
            Self::Basic => OrtLevel::Level1,
            Self::Extended => OrtLevel::Level2,
            Self::Level3 => OrtLevel::Level3,
        }
    }
}

/// Configuration specific to Kokoro TTS
///
/// This struct contains all configuration options for the Kokoro TTS provider.
/// It can be constructed directly, from a `TTSConfig`, or deserialized from YAML/JSON.
///
/// # Example
///
/// ```rust,ignore
/// use sayna::core::tts::kokoro::KokoroConfig;
///
/// let config = KokoroConfig {
///     voice_id: Some("bf_emma".to_string()),
///     speaking_rate: Some(1.2),
///     use_quantized: true,
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KokoroConfig {
    /// Voice ID to use (e.g., "af_bella", "am_adam", "bf_emma")
    ///
    /// Default: "af_bella" (American Female - Bella)
    ///
    /// Supports voice mixing with weighted syntax: "af_bella.5+am_adam.5"
    #[serde(default = "default_voice_id")]
    pub voice_id: Option<String>,

    /// Speaking rate multiplier (0.1 to 5.0, default 1.0)
    ///
    /// Values < 1.0 produce slower speech, > 1.0 produce faster speech.
    #[serde(default = "default_speaking_rate")]
    pub speaking_rate: Option<f32>,

    /// Default language for phonemization
    ///
    /// Supported values: "en-us" (American), "en-gb" (British), or eSpeak language codes.
    /// Default: "en-us"
    #[serde(default = "default_language")]
    pub default_language: String,

    /// Local path to the ONNX model file
    ///
    /// If not specified, the model will be loaded from the cache directory.
    #[serde(default)]
    pub model_path: Option<PathBuf>,

    /// URL to download the model from if not present locally
    ///
    /// Default depends on `use_quantized` and `use_timestamps` settings.
    #[serde(default)]
    pub model_url: Option<String>,

    /// Local path to the voices.bin file
    ///
    /// If not specified, voices will be loaded from the cache directory.
    #[serde(default)]
    pub voices_path: Option<PathBuf>,

    /// URL to download voices from if not present locally
    ///
    /// Default: GitHub release URL for voices-v1.0.bin
    #[serde(default)]
    pub voices_url: Option<String>,

    /// Cache directory for downloaded assets
    ///
    /// If specified, assets will be stored in `{cache_path}/kokoro_tts/`.
    /// Required for asset download during `sayna init`.
    #[serde(default)]
    pub cache_path: Option<PathBuf>,

    /// Number of threads for ONNX inference
    ///
    /// Default: 4 threads. Set to None to use ONNX Runtime's default.
    #[serde(default = "default_num_threads")]
    pub num_threads: Option<usize>,

    /// ONNX graph optimization level
    ///
    /// Higher levels provide faster inference but slower initialization.
    /// Default: Level3 (all optimizations)
    #[serde(default)]
    pub graph_optimization_level: GraphOptimizationLevel,

    /// Whether to use the quantized model variant for faster inference
    ///
    /// Default: true (92 MB quantized vs 326 MB full precision)
    #[serde(default = "default_use_quantized")]
    pub use_quantized: bool,

    /// Whether to use the timestamped model variant for word alignment
    ///
    /// Enables word-level timestamps in TTS output.
    /// Default: false
    #[serde(default)]
    pub use_timestamps: bool,
}

fn default_voice_id() -> Option<String> {
    Some(super::DEFAULT_VOICE.to_string())
}

fn default_speaking_rate() -> Option<f32> {
    Some(1.0)
}

fn default_language() -> String {
    "en-us".to_string()
}

fn default_num_threads() -> Option<usize> {
    Some(4)
}

fn default_use_quantized() -> bool {
    true
}

impl Default for KokoroConfig {
    fn default() -> Self {
        Self {
            voice_id: Some(super::DEFAULT_VOICE.to_string()),
            speaking_rate: Some(1.0),
            default_language: "en-us".to_string(),
            model_path: None,
            model_url: None,
            voices_path: None,
            voices_url: None,
            cache_path: None,
            num_threads: Some(4),
            graph_optimization_level: GraphOptimizationLevel::Level3,
            use_quantized: true,
            use_timestamps: false,
        }
    }
}

impl KokoroConfig {
    /// Get the cache directory for Kokoro assets
    ///
    /// Returns `{cache_path}/kokoro_tts` if cache_path is set.
    /// Returns an error if cache_path is not configured.
    ///
    /// Note: This method does NOT create the directory. Directory creation
    /// is handled by the asset download functions during `sayna init`.
    pub fn get_cache_dir(&self) -> Result<PathBuf> {
        if let Some(cache_path) = &self.cache_path {
            Ok(cache_path.join(DEFAULT_CACHE_SUBDIR))
        } else {
            anyhow::bail!("No cache directory specified")
        }
    }

    /// Validate the configuration
    ///
    /// Checks that all configuration values are within valid ranges and
    /// that required fields are properly configured.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Speaking rate is outside the valid range (0.1 to 5.0)
    /// - Voice ID is empty
    /// - Language is empty
    pub fn validate(&self) -> TTSResult<()> {
        // Validate speaking rate
        if let Some(rate) = self.speaking_rate
            && !(0.1..=5.0).contains(&rate)
        {
            return Err(TTSError::InvalidConfiguration(format!(
                "Speaking rate must be between 0.1 and 5.0, got {}",
                rate
            )));
        }

        // Validate voice ID is not empty if provided
        if let Some(ref voice_id) = self.voice_id
            && voice_id.is_empty()
        {
            return Err(TTSError::InvalidConfiguration(
                "Voice ID cannot be empty".to_string(),
            ));
        }

        // Validate language is not empty
        if self.default_language.is_empty() {
            return Err(TTSError::InvalidConfiguration(
                "Default language cannot be empty".to_string(),
            ));
        }

        // Validate model path exists if explicitly specified
        if let Some(ref model_path) = self.model_path
            && !model_path.exists()
        {
            return Err(TTSError::InvalidConfiguration(format!(
                "Model file not found at specified path: {}. Run 'sayna init' to download.",
                model_path.display()
            )));
        }

        // Validate voices path exists if explicitly specified
        if let Some(ref voices_path) = self.voices_path
            && !voices_path.exists()
        {
            return Err(TTSError::InvalidConfiguration(format!(
                "Voices file not found at specified path: {}. Run 'sayna init' to download.",
                voices_path.display()
            )));
        }

        Ok(())
    }

    /// Get the effective model URL based on configuration
    ///
    /// Returns the URL to download the model from, considering
    /// `use_quantized` and `use_timestamps` settings.
    pub fn get_model_url(&self) -> &str {
        if let Some(ref url) = self.model_url {
            url
        } else if self.use_quantized {
            DEFAULT_MODEL_QUANTIZED_URL
        } else if self.use_timestamps {
            DEFAULT_MODEL_TIMESTAMPED_URL
        } else {
            DEFAULT_MODEL_URL
        }
    }

    /// Get the effective voices URL based on configuration
    pub fn get_voices_url(&self) -> &str {
        self.voices_url.as_deref().unwrap_or(DEFAULT_VOICES_URL)
    }

    /// Create a KokoroAssetConfig for path resolution
    ///
    /// This is used by the assets module for downloading and resolving paths.
    pub fn to_asset_config(&self) -> super::assets::KokoroAssetConfig {
        use super::assets::KokoroAssetConfig;

        let cache_path = if let Some(ref cache_path) = self.cache_path {
            cache_path.join(DEFAULT_CACHE_SUBDIR)
        } else if let Some(ref model_path) = self.model_path {
            // Use parent directory of explicit model path as cache
            model_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(super::assets::get_default_cache_path)
        } else {
            super::assets::get_default_cache_path()
        };

        KokoroAssetConfig {
            cache_path,
            model_url: self.get_model_url().to_string(),
            voices_url: self.get_voices_url().to_string(),
            use_timestamped: self.use_timestamps,
            model_path: self.model_path.clone(),
            voices_path: self.voices_path.clone(),
        }
    }

    /// Create a KokoroConfig from a generic TTSConfig
    ///
    /// Maps common TTS configuration fields to Kokoro-specific settings.
    pub fn from_tts_config(config: &TTSConfig) -> TTSResult<Self> {
        let mut kokoro_config = KokoroConfig::default();

        // Voice ID
        if let Some(ref voice_id) = config.voice_id {
            kokoro_config.voice_id = Some(voice_id.clone());
        }

        // Speaking rate with validation
        if let Some(rate) = config.speaking_rate {
            if !(0.1..=5.0).contains(&rate) {
                return Err(TTSError::InvalidConfiguration(format!(
                    "Speaking rate must be between 0.1 and 5.0, got {}",
                    rate
                )));
            }
            kokoro_config.speaking_rate = Some(rate);
        }

        // Model path - look for it in the model field or try to find default
        if !config.model.is_empty() {
            kokoro_config.model_path = Some(PathBuf::from(&config.model));
        } else {
            kokoro_config.model_path = Self::find_default_model_path();
        }

        // Voices path - try to find default based on model path
        kokoro_config.voices_path =
            Self::find_default_voices_path(kokoro_config.model_path.as_ref());

        Ok(kokoro_config)
    }

    /// Try to find the default model path in common locations
    fn find_default_model_path() -> Option<PathBuf> {
        // Check common locations
        let locations = [
            // XDG cache directory with new subdir name
            dirs::cache_dir().map(|p| p.join(DEFAULT_CACHE_SUBDIR).join(DEFAULT_MODEL_FILE)),
            // Legacy kokoro directory
            dirs::cache_dir().map(|p| p.join("kokoro").join(DEFAULT_MODEL_FILE)),
            // Home directory
            dirs::home_dir().map(|p| p.join(".kokoro").join(DEFAULT_MODEL_FILE)),
            // Current directory
            Some(PathBuf::from(DEFAULT_MODEL_FILE)),
        ];

        for location in locations.into_iter().flatten() {
            if location.exists() {
                return Some(location);
            }
        }

        // Return default cache location even if it doesn't exist yet
        dirs::cache_dir().map(|p| p.join(DEFAULT_CACHE_SUBDIR).join(DEFAULT_MODEL_FILE))
    }

    /// Try to find the default voices path based on model path
    fn find_default_voices_path(model_path: Option<&PathBuf>) -> Option<PathBuf> {
        // If model path exists, look for voices in same directory
        if let Some(model_path) = model_path
            && let Some(model_dir) = model_path.parent()
        {
            let voices_path = model_dir.join(DEFAULT_VOICES_FILE);
            if voices_path.exists() {
                return Some(voices_path);
            }
            // Return the path even if it doesn't exist yet
            return Some(voices_path);
        }

        // Check common locations
        let locations = [
            dirs::cache_dir().map(|p| p.join(DEFAULT_CACHE_SUBDIR).join(DEFAULT_VOICES_FILE)),
            dirs::cache_dir().map(|p| p.join("kokoro").join(DEFAULT_VOICES_FILE)),
            dirs::home_dir().map(|p| p.join(".kokoro").join(DEFAULT_VOICES_FILE)),
            Some(PathBuf::from(DEFAULT_VOICES_FILE)),
        ];

        for location in locations.into_iter().flatten() {
            if location.exists() {
                return Some(location);
            }
        }

        // Return default cache location even if it doesn't exist yet
        dirs::cache_dir().map(|p| p.join(DEFAULT_CACHE_SUBDIR).join(DEFAULT_VOICES_FILE))
    }
}

/// Helper module for finding directories (minimal implementation)
mod dirs {
    use std::path::PathBuf;

    pub fn cache_dir() -> Option<PathBuf> {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|h| h.join(".cache")))
    }

    pub fn home_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = KokoroConfig::default();
        assert_eq!(config.voice_id, Some("af_bella".to_string()));
        assert_eq!(config.speaking_rate, Some(1.0));
        assert_eq!(config.default_language, "en-us");
        assert!(config.use_quantized);
        assert!(!config.use_timestamps);
        assert_eq!(config.num_threads, Some(4));
        assert!(matches!(
            config.graph_optimization_level,
            GraphOptimizationLevel::Level3
        ));
    }

    #[test]
    fn test_default_config_validates() {
        let config = KokoroConfig::default();
        // Default config should validate (ignoring file path checks)
        // Note: model_path and voices_path are None by default, so they won't be checked
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
        assert_eq!(config.voice_id, Some("am_adam".to_string()));
        assert_eq!(config.speaking_rate, Some(1.5));
    }

    #[test]
    fn test_invalid_speaking_rate_too_high() {
        let tts_config = TTSConfig {
            provider: "kokoro".to_string(),
            speaking_rate: Some(10.0), // Invalid: > 5.0
            ..Default::default()
        };

        let result = KokoroConfig::from_tts_config(&tts_config);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_speaking_rate_too_low() {
        let tts_config = TTSConfig {
            provider: "kokoro".to_string(),
            speaking_rate: Some(0.05), // Invalid: < 0.1
            ..Default::default()
        };

        let result = KokoroConfig::from_tts_config(&tts_config);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_invalid_speed() {
        let mut config = KokoroConfig::default();
        config.speaking_rate = Some(10.0); // Too fast
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_voice_id() {
        let mut config = KokoroConfig::default();
        config.voice_id = Some(String::new()); // Empty voice ID
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_language() {
        let mut config = KokoroConfig::default();
        config.default_language = String::new(); // Empty language
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_get_cache_dir_with_path() {
        let mut config = KokoroConfig::default();
        config.cache_path = Some(PathBuf::from("/tmp/test_cache"));

        let result = config.get_cache_dir();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PathBuf::from("/tmp/test_cache/kokoro_tts"));
    }

    #[test]
    fn test_get_cache_dir_without_path() {
        let config = KokoroConfig::default();
        let result = config.get_cache_dir();
        assert!(result.is_err());
    }

    #[test]
    fn test_get_model_url_quantized() {
        let config = KokoroConfig {
            use_quantized: true,
            use_timestamps: false,
            ..Default::default()
        };
        assert_eq!(config.get_model_url(), DEFAULT_MODEL_QUANTIZED_URL);
    }

    #[test]
    fn test_get_model_url_timestamped() {
        let config = KokoroConfig {
            use_quantized: false,
            use_timestamps: true,
            ..Default::default()
        };
        assert_eq!(config.get_model_url(), DEFAULT_MODEL_TIMESTAMPED_URL);
    }

    #[test]
    fn test_get_model_url_default() {
        let config = KokoroConfig {
            use_quantized: false,
            use_timestamps: false,
            ..Default::default()
        };
        assert_eq!(config.get_model_url(), DEFAULT_MODEL_URL);
    }

    #[test]
    fn test_get_model_url_custom() {
        let config = KokoroConfig {
            model_url: Some("https://example.com/model.onnx".to_string()),
            use_quantized: true, // Should be ignored when custom URL is set
            ..Default::default()
        };
        assert_eq!(config.get_model_url(), "https://example.com/model.onnx");
    }

    #[test]
    fn test_serde_serialization() {
        let config = KokoroConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("af_bella"));
        assert!(json.contains("en-us"));
    }

    #[test]
    fn test_serde_deserialization() {
        let json = r#"{
            "voice_id": "bf_emma",
            "speaking_rate": 1.2,
            "default_language": "en-gb",
            "use_quantized": false,
            "use_timestamps": true
        }"#;

        let config: KokoroConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.voice_id, Some("bf_emma".to_string()));
        assert_eq!(config.speaking_rate, Some(1.2));
        assert_eq!(config.default_language, "en-gb");
        assert!(!config.use_quantized);
        assert!(config.use_timestamps);
    }

    #[test]
    fn test_serde_deserialization_with_defaults() {
        // Minimal JSON should use defaults for missing fields
        let json = r#"{}"#;
        let config: KokoroConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.voice_id, Some("af_bella".to_string()));
        assert_eq!(config.speaking_rate, Some(1.0));
        assert_eq!(config.default_language, "en-us");
        assert!(config.use_quantized);
        assert!(!config.use_timestamps);
    }

    #[test]
    fn test_graph_optimization_level_default() {
        let level = GraphOptimizationLevel::default();
        assert!(matches!(level, GraphOptimizationLevel::Level3));
    }
}
