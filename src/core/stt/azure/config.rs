//! Configuration types for Microsoft Azure Speech-to-Text API.
//!
//! This module contains all configuration-related types including:
//! - Regional endpoint selection
//! - Output format specifications
//! - Profanity handling options
//! - Provider-specific configuration options

use super::super::base::STTConfig;

// =============================================================================
// Azure Regions
// =============================================================================

/// Microsoft Azure Speech Service regions.
///
/// Choose the region closest to your users for optimal latency,
/// or use specific regions for data residency requirements.
///
/// See: <https://learn.microsoft.com/en-us/azure/ai-services/speech-service/regions>
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AzureRegion {
    /// East US (Virginia)
    #[default]
    EastUS,
    /// East US 2 (Virginia)
    EastUS2,
    /// West US (California)
    WestUS,
    /// West US 2 (Washington)
    WestUS2,
    /// West US 3 (Arizona)
    WestUS3,
    /// Central US (Iowa)
    CentralUS,
    /// North Central US (Illinois)
    NorthCentralUS,
    /// South Central US (Texas)
    SouthCentralUS,
    /// West Europe (Netherlands)
    WestEurope,
    /// North Europe (Ireland)
    NorthEurope,
    /// UK South (London)
    UKSouth,
    /// France Central (Paris)
    FranceCentral,
    /// Germany West Central (Frankfurt)
    GermanyWestCentral,
    /// Switzerland North (Zurich)
    SwitzerlandNorth,
    /// East Asia (Hong Kong)
    EastAsia,
    /// Southeast Asia (Singapore)
    SoutheastAsia,
    /// Japan East (Tokyo)
    JapanEast,
    /// Japan West (Osaka)
    JapanWest,
    /// Korea Central (Seoul)
    KoreaCentral,
    /// Australia East (Sydney)
    AustraliaEast,
    /// Canada Central (Toronto)
    CanadaCentral,
    /// Brazil South (Sao Paulo)
    BrazilSouth,
    /// India Central (Pune)
    IndiaCentral,
    /// Custom region not explicitly listed.
    ///
    /// Use this for new or less common regions without requiring code changes.
    Custom(String),
}

impl AzureRegion {
    /// Get the region identifier string used in Azure URLs.
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            Self::EastUS => "eastus",
            Self::EastUS2 => "eastus2",
            Self::WestUS => "westus",
            Self::WestUS2 => "westus2",
            Self::WestUS3 => "westus3",
            Self::CentralUS => "centralus",
            Self::NorthCentralUS => "northcentralus",
            Self::SouthCentralUS => "southcentralus",
            Self::WestEurope => "westeurope",
            Self::NorthEurope => "northeurope",
            Self::UKSouth => "uksouth",
            Self::FranceCentral => "francecentral",
            Self::GermanyWestCentral => "germanywestcentral",
            Self::SwitzerlandNorth => "switzerlandnorth",
            Self::EastAsia => "eastasia",
            Self::SoutheastAsia => "southeastasia",
            Self::JapanEast => "japaneast",
            Self::JapanWest => "japanwest",
            Self::KoreaCentral => "koreacentral",
            Self::AustraliaEast => "australiaeast",
            Self::CanadaCentral => "canadacentral",
            Self::BrazilSouth => "brazilsouth",
            Self::IndiaCentral => "centralindia",
            Self::Custom(region) => region.as_str(),
        }
    }

    /// Get the hostname for the Azure Speech Service in this region.
    ///
    /// Format: `<region>.stt.speech.microsoft.com`
    #[inline]
    pub fn hostname(&self) -> String {
        format!("{}.stt.speech.microsoft.com", self.as_str())
    }

    /// Get the base WebSocket URL for the Azure Speech Service in this region.
    ///
    /// Format: `wss://<region>.stt.speech.microsoft.com`
    #[inline]
    pub fn websocket_base_url(&self) -> String {
        format!("wss://{}.stt.speech.microsoft.com", self.as_str())
    }

    /// Get the token issuing endpoint URL for obtaining authentication tokens.
    ///
    /// Format: `https://<region>.api.cognitive.microsoft.com/sts/v1.0/issueToken`
    ///
    /// Use this endpoint to exchange your subscription key for an access token.
    #[inline]
    pub fn token_endpoint(&self) -> String {
        format!(
            "https://{}.api.cognitive.microsoft.com/sts/v1.0/issueToken",
            self.as_str()
        )
    }
}

impl std::str::FromStr for AzureRegion {
    type Err = std::convert::Infallible;

    /// Parse a region from a string identifier.
    ///
    /// Known region identifiers are matched to their explicit variants.
    /// Unknown identifiers are wrapped in the `Custom` variant.
    ///
    /// This implementation never fails - unknown regions become `Custom(s)`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let region = match s.to_lowercase().as_str() {
            "eastus" => Self::EastUS,
            "eastus2" => Self::EastUS2,
            "westus" => Self::WestUS,
            "westus2" => Self::WestUS2,
            "westus3" => Self::WestUS3,
            "centralus" => Self::CentralUS,
            "northcentralus" => Self::NorthCentralUS,
            "southcentralus" => Self::SouthCentralUS,
            "westeurope" => Self::WestEurope,
            "northeurope" => Self::NorthEurope,
            "uksouth" => Self::UKSouth,
            "francecentral" => Self::FranceCentral,
            "germanywestcentral" => Self::GermanyWestCentral,
            "switzerlandnorth" => Self::SwitzerlandNorth,
            "eastasia" => Self::EastAsia,
            "southeastasia" => Self::SoutheastAsia,
            "japaneast" => Self::JapanEast,
            "japanwest" => Self::JapanWest,
            "koreacentral" => Self::KoreaCentral,
            "australiaeast" => Self::AustraliaEast,
            "canadacentral" => Self::CanadaCentral,
            "brazilsouth" => Self::BrazilSouth,
            "centralindia" => Self::IndiaCentral,
            _ => Self::Custom(s.to_string()),
        };
        Ok(region)
    }
}

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
        let base_url = self.region.websocket_base_url();

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

    #[test]
    fn test_azure_region_default() {
        let region = AzureRegion::default();
        assert_eq!(region, AzureRegion::EastUS);
        assert_eq!(region.as_str(), "eastus");
    }

    #[test]
    fn test_azure_region_hostname() {
        let region = AzureRegion::WestEurope;
        assert_eq!(region.hostname(), "westeurope.stt.speech.microsoft.com");
    }

    #[test]
    fn test_azure_region_websocket_url() {
        let region = AzureRegion::SoutheastAsia;
        assert_eq!(
            region.websocket_base_url(),
            "wss://southeastasia.stt.speech.microsoft.com"
        );
    }

    #[test]
    fn test_azure_region_token_endpoint() {
        let region = AzureRegion::EastUS;
        assert_eq!(
            region.token_endpoint(),
            "https://eastus.api.cognitive.microsoft.com/sts/v1.0/issueToken"
        );
    }

    #[test]
    fn test_azure_region_custom() {
        let region = AzureRegion::Custom("newregion".to_string());
        assert_eq!(region.as_str(), "newregion");
        assert_eq!(region.hostname(), "newregion.stt.speech.microsoft.com");
    }

    #[test]
    fn test_azure_region_from_str() {
        assert_eq!(
            "eastus".parse::<AzureRegion>().unwrap(),
            AzureRegion::EastUS
        );
        assert_eq!(
            "WESTEUROPE".parse::<AzureRegion>().unwrap(),
            AzureRegion::WestEurope
        );
        assert_eq!(
            "unknown".parse::<AzureRegion>().unwrap(),
            AzureRegion::Custom("unknown".to_string())
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

    // -------------------------------------------------------------------------
    // Additional Region Tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_all_regions_as_str() {
        // Verify all named regions produce expected string values
        let regions = vec![
            (AzureRegion::EastUS, "eastus"),
            (AzureRegion::EastUS2, "eastus2"),
            (AzureRegion::WestUS, "westus"),
            (AzureRegion::WestUS2, "westus2"),
            (AzureRegion::WestUS3, "westus3"),
            (AzureRegion::CentralUS, "centralus"),
            (AzureRegion::NorthCentralUS, "northcentralus"),
            (AzureRegion::SouthCentralUS, "southcentralus"),
            (AzureRegion::WestEurope, "westeurope"),
            (AzureRegion::NorthEurope, "northeurope"),
            (AzureRegion::UKSouth, "uksouth"),
            (AzureRegion::FranceCentral, "francecentral"),
            (AzureRegion::GermanyWestCentral, "germanywestcentral"),
            (AzureRegion::SwitzerlandNorth, "switzerlandnorth"),
            (AzureRegion::EastAsia, "eastasia"),
            (AzureRegion::SoutheastAsia, "southeastasia"),
            (AzureRegion::JapanEast, "japaneast"),
            (AzureRegion::JapanWest, "japanwest"),
            (AzureRegion::KoreaCentral, "koreacentral"),
            (AzureRegion::AustraliaEast, "australiaeast"),
            (AzureRegion::CanadaCentral, "canadacentral"),
            (AzureRegion::BrazilSouth, "brazilsouth"),
            (AzureRegion::IndiaCentral, "centralindia"),
        ];

        for (region, expected) in regions {
            assert_eq!(
                region.as_str(),
                expected,
                "Region {:?} should produce '{}'",
                region,
                expected
            );
        }
    }

    #[test]
    fn test_region_from_str_all_known_regions() {
        // Test all known region string identifiers
        let test_cases = vec![
            ("eastus", AzureRegion::EastUS),
            ("eastus2", AzureRegion::EastUS2),
            ("westus", AzureRegion::WestUS),
            ("westus2", AzureRegion::WestUS2),
            ("westus3", AzureRegion::WestUS3),
            ("centralus", AzureRegion::CentralUS),
            ("northcentralus", AzureRegion::NorthCentralUS),
            ("southcentralus", AzureRegion::SouthCentralUS),
            ("westeurope", AzureRegion::WestEurope),
            ("northeurope", AzureRegion::NorthEurope),
            ("uksouth", AzureRegion::UKSouth),
            ("francecentral", AzureRegion::FranceCentral),
            ("germanywestcentral", AzureRegion::GermanyWestCentral),
            ("switzerlandnorth", AzureRegion::SwitzerlandNorth),
            ("eastasia", AzureRegion::EastAsia),
            ("southeastasia", AzureRegion::SoutheastAsia),
            ("japaneast", AzureRegion::JapanEast),
            ("japanwest", AzureRegion::JapanWest),
            ("koreacentral", AzureRegion::KoreaCentral),
            ("australiaeast", AzureRegion::AustraliaEast),
            ("canadacentral", AzureRegion::CanadaCentral),
            ("brazilsouth", AzureRegion::BrazilSouth),
            ("centralindia", AzureRegion::IndiaCentral),
        ];

        for (input, expected) in test_cases {
            assert_eq!(
                input.parse::<AzureRegion>().unwrap(),
                expected,
                "Parsing '{}' should produce {:?}",
                input,
                expected
            );
        }
    }

    #[test]
    fn test_region_from_str_case_insensitive() {
        // All case variations should work
        assert_eq!(
            "EASTUS".parse::<AzureRegion>().unwrap(),
            AzureRegion::EastUS
        );
        assert_eq!(
            "EastUs".parse::<AzureRegion>().unwrap(),
            AzureRegion::EastUS
        );
        assert_eq!(
            "WESTEUROPE".parse::<AzureRegion>().unwrap(),
            AzureRegion::WestEurope
        );
        assert_eq!(
            "WestEurope".parse::<AzureRegion>().unwrap(),
            AzureRegion::WestEurope
        );
    }

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
