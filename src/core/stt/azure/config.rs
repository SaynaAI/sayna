//! Configuration types for Microsoft Azure Speech-to-Text API.
//!
//! This module contains all configuration-related types including:
//! - Output format specifications
//! - Profanity handling options
//! - Provider-specific configuration options
//!
//! Note: The `AzureRegion` type is now defined in `crate::core::providers::azure`
//! and re-exported here for backwards compatibility.

use super::super::base::STTConfig;

// Re-export AzureRegion from the shared providers module for backwards compatibility
pub use crate::core::providers::azure::AzureRegion;

// =============================================================================
// Output Format
// =============================================================================

/// Azure Speech-to-Text output format options.
///
/// Controls the level of detail in recognition results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AzureOutputFormat {
    /// Simple format - returns basic recognition results with just the display text.
    ///
    /// Recommended for applications that only need the final transcription text.
    Simple,

    /// Detailed format - returns rich results including NBest alternatives,
    /// confidence scores, and optional word-level timing.
    ///
    /// Use this when you need confidence scores or alternative transcriptions.
    #[default]
    Detailed,
}

impl AzureOutputFormat {
    /// Convert to the API query parameter string value.
    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Detailed => "detailed",
        }
    }
}

// =============================================================================
// Profanity Handling
// =============================================================================

/// Azure Speech-to-Text profanity handling options.
///
/// Controls how profane words are handled in recognition results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AzureProfanityOption {
    /// Masked - replace profane words with asterisks.
    ///
    /// Example: "What the ****" instead of the actual word.
    #[default]
    Masked,

    /// Removed - completely remove profane words from output.
    ///
    /// The words are omitted entirely from the transcription.
    Removed,

    /// Raw - return profane words unchanged.
    ///
    /// Use when you need the exact transcription including profanity.
    Raw,
}

impl AzureProfanityOption {
    /// Convert to the API query parameter string value.
    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Masked => "masked",
            Self::Removed => "removed",
            Self::Raw => "raw",
        }
    }
}

// =============================================================================
// Main Configuration
// =============================================================================

/// Configuration specific to Microsoft Azure Speech-to-Text API.
///
/// This configuration extends the base `STTConfig` with Azure-specific
/// parameters for the WebSocket streaming API.
///
/// # Example
///
/// ```rust
/// use sayna::core::stt::azure::{AzureSTTConfig, AzureRegion, AzureOutputFormat};
/// use sayna::core::stt::STTConfig;
///
/// let config = AzureSTTConfig {
///     base: STTConfig {
///         api_key: "your-subscription-key".to_string(),
///         language: "en-US".to_string(),
///         ..Default::default()
///     },
///     region: AzureRegion::WestEurope,
///     output_format: AzureOutputFormat::Detailed,
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone)]
pub struct AzureSTTConfig {
    /// Base STT configuration (shared across all providers).
    ///
    /// Contains common settings like api_key, language, sample_rate, etc.
    pub base: STTConfig,

    /// Azure region for the Speech Service endpoint.
    ///
    /// Choose the region closest to your users for optimal latency.
    pub region: AzureRegion,

    /// Output format for recognition results.
    ///
    /// - `Simple`: Basic results with just the display text
    /// - `Detailed`: Rich results with NBest alternatives and confidence scores
    pub output_format: AzureOutputFormat,

    /// Profanity handling option.
    ///
    /// Controls how profane words appear in the transcription output.
    pub profanity: AzureProfanityOption,

    /// Enable interim (partial) results during speech recognition.
    ///
    /// When `true`, the service sends `speech.hypothesis` events with
    /// partial transcriptions as audio is being processed. This enables
    /// real-time display of transcription progress.
    pub interim_results: bool,

    /// Request word-level timing information in detailed mode.
    ///
    /// Only effective when `output_format` is `Detailed`.
    /// Provides start and end times for each recognized word.
    pub word_level_timing: bool,

    /// Custom Speech endpoint ID for specialized recognition models.
    ///
    /// Use this to specify a Custom Speech model trained on your specific
    /// domain or vocabulary. Leave as `None` to use the default model.
    pub endpoint_id: Option<String>,

    /// Languages for automatic language detection.
    ///
    /// When specified, Azure will automatically detect which of these
    /// languages is being spoken and switch recognition accordingly.
    /// Leave as `None` to use the language specified in `base.language`.
    ///
    /// Example: `Some(vec!["en-US".to_string(), "es-ES".to_string()])`
    pub auto_detect_languages: Option<Vec<String>>,
}

impl Default for AzureSTTConfig {
    fn default() -> Self {
        Self {
            base: STTConfig::default(),
            region: AzureRegion::default(),
            output_format: AzureOutputFormat::Detailed,
            profanity: AzureProfanityOption::Masked,
            interim_results: true,
            word_level_timing: false,
            endpoint_id: None,
            auto_detect_languages: None,
        }
    }
}

impl AzureSTTConfig {
    /// Build the complete WebSocket URL with all query parameters.
    ///
    /// Constructs the full WebSocket URL for connecting to Azure Speech Service,
    /// including:
    /// - Regional endpoint base URL
    /// - Speech recognition API path
    /// - All configuration query parameters
    ///
    /// # Returns
    ///
    /// The complete WebSocket URL string ready for connection.
    ///
    /// # Example URL
    ///
    /// ```text
    /// wss://eastus.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1
    ///     ?language=en-US
    ///     &format=detailed
    ///     &profanity=masked
    /// ```
    pub fn build_websocket_url(&self) -> String {
        let base_url = self.region.stt_websocket_base_url();

        // Start with the base path and required parameters
        let mut url = format!(
            "{}/speech/recognition/conversation/cognitiveservices/v1?language={}&format={}&profanity={}",
            base_url,
            self.base.language,
            self.output_format.as_str(),
            self.profanity.as_str()
        );

        // Add endpoint ID for Custom Speech models
        if let Some(ref endpoint_id) = self.endpoint_id {
            url.push_str("&cid=");
            url.push_str(endpoint_id);
        }

        // Add auto-detect languages if specified and non-empty
        if let Some(ref languages) = self.auto_detect_languages
            && !languages.is_empty()
        {
            // Azure expects comma-separated language codes for the 'languages' parameter
            url.push_str("&languages=");
            url.push_str(&languages.join(","));
        }

        url
    }

    /// Create a new configuration from base STTConfig.
    ///
    /// Initializes Azure-specific settings with sensible defaults.
    pub fn from_base(base: STTConfig) -> Self {
        Self {
            base,
            ..Default::default()
        }
    }

    /// Create a configuration with a specific region.
    ///
    /// Convenience constructor for common use case of selecting a region.
    pub fn with_region(base: STTConfig, region: AzureRegion) -> Self {
        Self {
            base,
            region,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: Core AzureRegion tests are now in crate::core::providers::azure::region.
    // Tests here focus on STT-specific configuration and re-export verification.

    #[test]
    fn test_azure_region_reexport_works() {
        // Verify that AzureRegion is properly re-exported from the providers module
        let region = AzureRegion::default();
        assert_eq!(region, AzureRegion::EastUS);
        assert_eq!(region.as_str(), "eastus");
    }

    #[test]
    fn test_azure_region_stt_methods_available() {
        // Verify that STT-specific methods are available via re-export
        let region = AzureRegion::WestEurope;
        assert_eq!(region.stt_hostname(), "westeurope.stt.speech.microsoft.com");
        assert_eq!(
            region.stt_websocket_base_url(),
            "wss://westeurope.stt.speech.microsoft.com"
        );
    }

    #[test]
    fn test_azure_output_format() {
        assert_eq!(AzureOutputFormat::Simple.as_str(), "simple");
        assert_eq!(AzureOutputFormat::Detailed.as_str(), "detailed");
        assert_eq!(AzureOutputFormat::default(), AzureOutputFormat::Detailed);
    }

    #[test]
    fn test_azure_profanity_option() {
        assert_eq!(AzureProfanityOption::Masked.as_str(), "masked");
        assert_eq!(AzureProfanityOption::Removed.as_str(), "removed");
        assert_eq!(AzureProfanityOption::Raw.as_str(), "raw");
        assert_eq!(
            AzureProfanityOption::default(),
            AzureProfanityOption::Masked
        );
    }

    #[test]
    fn test_azure_stt_config_default() {
        let config = AzureSTTConfig::default();
        assert_eq!(config.region, AzureRegion::EastUS);
        assert_eq!(config.output_format, AzureOutputFormat::Detailed);
        assert_eq!(config.profanity, AzureProfanityOption::Masked);
        assert!(config.interim_results);
        assert!(!config.word_level_timing);
        assert!(config.endpoint_id.is_none());
        assert!(config.auto_detect_languages.is_none());
    }

    #[test]
    fn test_azure_stt_config_build_url() {
        let config = AzureSTTConfig {
            base: STTConfig {
                language: "en-US".to_string(),
                ..Default::default()
            },
            region: AzureRegion::WestEurope,
            output_format: AzureOutputFormat::Detailed,
            profanity: AzureProfanityOption::Raw,
            ..Default::default()
        };

        let url = config.build_websocket_url();
        assert!(url.starts_with("wss://westeurope.stt.speech.microsoft.com"));
        assert!(url.contains("language=en-US"));
        assert!(url.contains("format=detailed"));
        assert!(url.contains("profanity=raw"));
    }

    #[test]
    fn test_azure_stt_config_build_url_with_endpoint_id() {
        let config = AzureSTTConfig {
            base: STTConfig {
                language: "en-US".to_string(),
                ..Default::default()
            },
            endpoint_id: Some("custom-model-123".to_string()),
            ..Default::default()
        };

        let url = config.build_websocket_url();
        assert!(url.contains("cid=custom-model-123"));
    }

    #[test]
    fn test_azure_stt_config_build_url_with_auto_detect() {
        let config = AzureSTTConfig {
            base: STTConfig {
                language: "en-US".to_string(),
                ..Default::default()
            },
            auto_detect_languages: Some(vec!["en-US".to_string(), "es-ES".to_string()]),
            ..Default::default()
        };

        let url = config.build_websocket_url();
        assert!(url.contains("languages=en-US,es-ES"));
    }

    #[test]
    fn test_azure_stt_config_from_base() {
        let base = STTConfig {
            api_key: "test-key".to_string(),
            language: "de-DE".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let config = AzureSTTConfig::from_base(base.clone());
        assert_eq!(config.base.api_key, "test-key");
        assert_eq!(config.base.language, "de-DE");
        assert_eq!(config.region, AzureRegion::EastUS);
    }

    #[test]
    fn test_azure_stt_config_with_region() {
        let base = STTConfig::default();
        let config = AzureSTTConfig::with_region(base, AzureRegion::JapanEast);
        assert_eq!(config.region, AzureRegion::JapanEast);
    }

    // Note: Additional region tests (as_str, FromStr, case insensitivity) are now
    // in crate::core::providers::azure::region. Tests here focus on STT config.

    #[test]
    fn test_azure_stt_config_build_url_simple_format() {
        let config = AzureSTTConfig {
            base: STTConfig {
                language: "fr-FR".to_string(),
                ..Default::default()
            },
            output_format: AzureOutputFormat::Simple,
            profanity: AzureProfanityOption::Removed,
            ..Default::default()
        };

        let url = config.build_websocket_url();
        assert!(url.contains("format=simple"));
        assert!(url.contains("profanity=removed"));
        assert!(url.contains("language=fr-FR"));
    }

    #[test]
    fn test_azure_stt_config_build_url_empty_auto_detect() {
        // Empty auto_detect_languages should not add the parameter
        let config = AzureSTTConfig {
            base: STTConfig {
                language: "en-US".to_string(),
                ..Default::default()
            },
            auto_detect_languages: Some(vec![]),
            ..Default::default()
        };

        let url = config.build_websocket_url();
        assert!(!url.contains("languages="));
    }

    #[test]
    fn test_azure_stt_config_build_url_full_options() {
        let config = AzureSTTConfig {
            base: STTConfig {
                language: "en-GB".to_string(),
                ..Default::default()
            },
            region: AzureRegion::UKSouth,
            output_format: AzureOutputFormat::Detailed,
            profanity: AzureProfanityOption::Masked,
            interim_results: true,
            word_level_timing: true,
            endpoint_id: Some("custom-endpoint".to_string()),
            auto_detect_languages: Some(vec!["en-GB".to_string(), "en-US".to_string()]),
        };

        let url = config.build_websocket_url();
        assert!(url.starts_with("wss://uksouth.stt.speech.microsoft.com"));
        assert!(url.contains("language=en-GB"));
        assert!(url.contains("format=detailed"));
        assert!(url.contains("profanity=masked"));
        assert!(url.contains("cid=custom-endpoint"));
        assert!(url.contains("languages=en-GB,en-US"));
    }
}
