//! Microsoft Azure Speech Service region configuration.
//!
//! This module provides region configuration for Azure Speech Services,
//! supporting both Speech-to-Text and Text-to-Speech endpoints.
//!
//! # Region Selection
//!
//! Choose the region closest to your users for optimal latency,
//! or use specific regions for data residency requirements.
//!
//! # Example
//!
//! ```rust
//! use sayna::core::providers::azure::AzureRegion;
//!
//! let region = AzureRegion::WestEurope;
//!
//! // STT endpoint
//! assert_eq!(region.stt_hostname(), "westeurope.stt.speech.microsoft.com");
//!
//! // TTS endpoint
//! assert_eq!(region.tts_hostname(), "westeurope.tts.speech.microsoft.com");
//!
//! // Voices list endpoint
//! assert!(region.voices_list_url().contains("cognitiveservices/voices/list"));
//! ```
//!
//! See: <https://learn.microsoft.com/en-us/azure/ai-services/speech-service/regions>

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
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::providers::azure::AzureRegion;
    ///
    /// assert_eq!(AzureRegion::EastUS.as_str(), "eastus");
    /// assert_eq!(AzureRegion::WestEurope.as_str(), "westeurope");
    /// ```
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

    // =========================================================================
    // STT (Speech-to-Text) Endpoints
    // =========================================================================

    /// Get the STT hostname for the Azure Speech Service in this region.
    ///
    /// Format: `<region>.stt.speech.microsoft.com`
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::providers::azure::AzureRegion;
    ///
    /// let region = AzureRegion::WestEurope;
    /// assert_eq!(region.stt_hostname(), "westeurope.stt.speech.microsoft.com");
    /// ```
    #[inline]
    pub fn stt_hostname(&self) -> String {
        format!("{}.stt.speech.microsoft.com", self.as_str())
    }

    /// Get the base WebSocket URL for the Azure Speech-to-Text Service in this region.
    ///
    /// Format: `wss://<region>.stt.speech.microsoft.com`
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::providers::azure::AzureRegion;
    ///
    /// let region = AzureRegion::SoutheastAsia;
    /// assert_eq!(
    ///     region.stt_websocket_base_url(),
    ///     "wss://southeastasia.stt.speech.microsoft.com"
    /// );
    /// ```
    #[inline]
    pub fn stt_websocket_base_url(&self) -> String {
        format!("wss://{}.stt.speech.microsoft.com", self.as_str())
    }

    // =========================================================================
    // TTS (Text-to-Speech) Endpoints
    // =========================================================================

    /// Get the TTS hostname for the Azure Speech Service in this region.
    ///
    /// Format: `<region>.tts.speech.microsoft.com`
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::providers::azure::AzureRegion;
    ///
    /// let region = AzureRegion::WestEurope;
    /// assert_eq!(region.tts_hostname(), "westeurope.tts.speech.microsoft.com");
    /// ```
    #[inline]
    pub fn tts_hostname(&self) -> String {
        format!("{}.tts.speech.microsoft.com", self.as_str())
    }

    /// Get the base REST URL for the Azure Text-to-Speech Service in this region.
    ///
    /// Format: `https://<region>.tts.speech.microsoft.com/cognitiveservices/v1`
    ///
    /// This is the endpoint for synthesizing speech from text.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::providers::azure::AzureRegion;
    ///
    /// let region = AzureRegion::EastUS;
    /// assert_eq!(
    ///     region.tts_rest_url(),
    ///     "https://eastus.tts.speech.microsoft.com/cognitiveservices/v1"
    /// );
    /// ```
    #[inline]
    pub fn tts_rest_url(&self) -> String {
        format!(
            "https://{}.tts.speech.microsoft.com/cognitiveservices/v1",
            self.as_str()
        )
    }

    /// Get the voices list endpoint URL for the Azure Text-to-Speech Service.
    ///
    /// Format: `https://<region>.tts.speech.microsoft.com/cognitiveservices/voices/list`
    ///
    /// Use this endpoint to retrieve the list of available voices for the region.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::providers::azure::AzureRegion;
    ///
    /// let region = AzureRegion::WestEurope;
    /// assert_eq!(
    ///     region.voices_list_url(),
    ///     "https://westeurope.tts.speech.microsoft.com/cognitiveservices/voices/list"
    /// );
    /// ```
    #[inline]
    pub fn voices_list_url(&self) -> String {
        format!(
            "https://{}.tts.speech.microsoft.com/cognitiveservices/voices/list",
            self.as_str()
        )
    }

    // =========================================================================
    // Authentication Endpoints
    // =========================================================================

    /// Get the token issuing endpoint URL for obtaining authentication tokens.
    ///
    /// Format: `https://<region>.api.cognitive.microsoft.com/sts/v1.0/issueToken`
    ///
    /// Use this endpoint to exchange your subscription key for an access token.
    /// The access token is valid for 10 minutes.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::providers::azure::AzureRegion;
    ///
    /// let region = AzureRegion::EastUS;
    /// assert_eq!(
    ///     region.token_endpoint(),
    ///     "https://eastus.api.cognitive.microsoft.com/sts/v1.0/issueToken"
    /// );
    /// ```
    #[inline]
    pub fn token_endpoint(&self) -> String {
        format!(
            "https://{}.api.cognitive.microsoft.com/sts/v1.0/issueToken",
            self.as_str()
        )
    }

    // =========================================================================
    // Backwards Compatibility - Deprecated Methods
    // =========================================================================

    /// Get the hostname for the Azure Speech Service in this region.
    ///
    /// **Deprecated**: Use [`stt_hostname`](Self::stt_hostname) instead for clarity.
    ///
    /// Format: `<region>.stt.speech.microsoft.com`
    #[deprecated(since = "0.2.0", note = "Use `stt_hostname` instead for clarity")]
    #[inline]
    pub fn hostname(&self) -> String {
        self.stt_hostname()
    }

    /// Get the base WebSocket URL for the Azure Speech Service in this region.
    ///
    /// **Deprecated**: Use [`stt_websocket_base_url`](Self::stt_websocket_base_url) instead for clarity.
    ///
    /// Format: `wss://<region>.stt.speech.microsoft.com`
    #[deprecated(
        since = "0.2.0",
        note = "Use `stt_websocket_base_url` instead for clarity"
    )]
    #[inline]
    pub fn websocket_base_url(&self) -> String {
        self.stt_websocket_base_url()
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
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::providers::azure::AzureRegion;
    ///
    /// let region: AzureRegion = "westeurope".parse().unwrap();
    /// assert_eq!(region, AzureRegion::WestEurope);
    ///
    /// // Unknown regions become Custom
    /// let custom: AzureRegion = "newregion".parse().unwrap();
    /// assert_eq!(custom, AzureRegion::Custom("newregion".to_string()));
    /// ```
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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Basic Region Tests
    // =========================================================================

    #[test]
    fn test_azure_region_default() {
        let region = AzureRegion::default();
        assert_eq!(region, AzureRegion::EastUS);
        assert_eq!(region.as_str(), "eastus");
    }

    #[test]
    fn test_azure_region_custom() {
        let region = AzureRegion::Custom("newregion".to_string());
        assert_eq!(region.as_str(), "newregion");
        assert_eq!(region.stt_hostname(), "newregion.stt.speech.microsoft.com");
        assert_eq!(region.tts_hostname(), "newregion.tts.speech.microsoft.com");
    }

    // =========================================================================
    // STT Endpoint Tests
    // =========================================================================

    #[test]
    fn test_azure_region_stt_hostname() {
        let region = AzureRegion::WestEurope;
        assert_eq!(region.stt_hostname(), "westeurope.stt.speech.microsoft.com");
    }

    #[test]
    fn test_azure_region_stt_websocket_url() {
        let region = AzureRegion::SoutheastAsia;
        assert_eq!(
            region.stt_websocket_base_url(),
            "wss://southeastasia.stt.speech.microsoft.com"
        );
    }

    // =========================================================================
    // TTS Endpoint Tests
    // =========================================================================

    #[test]
    fn test_azure_region_tts_hostname() {
        let region = AzureRegion::WestEurope;
        assert_eq!(region.tts_hostname(), "westeurope.tts.speech.microsoft.com");
    }

    #[test]
    fn test_azure_region_tts_rest_url() {
        let region = AzureRegion::EastUS;
        assert_eq!(
            region.tts_rest_url(),
            "https://eastus.tts.speech.microsoft.com/cognitiveservices/v1"
        );
    }

    #[test]
    fn test_azure_region_voices_list_url() {
        let region = AzureRegion::WestEurope;
        assert_eq!(
            region.voices_list_url(),
            "https://westeurope.tts.speech.microsoft.com/cognitiveservices/voices/list"
        );
    }

    #[test]
    fn test_azure_region_tts_endpoints_for_all_regions() {
        // Test a sampling of regions to ensure TTS endpoints are correctly formed
        let test_cases = vec![
            (AzureRegion::EastUS, "eastus"),
            (AzureRegion::WestEurope, "westeurope"),
            (AzureRegion::JapanEast, "japaneast"),
            (AzureRegion::AustraliaEast, "australiaeast"),
        ];

        for (region, region_str) in test_cases {
            assert_eq!(
                region.tts_hostname(),
                format!("{}.tts.speech.microsoft.com", region_str)
            );
            assert_eq!(
                region.tts_rest_url(),
                format!(
                    "https://{}.tts.speech.microsoft.com/cognitiveservices/v1",
                    region_str
                )
            );
            assert_eq!(
                region.voices_list_url(),
                format!(
                    "https://{}.tts.speech.microsoft.com/cognitiveservices/voices/list",
                    region_str
                )
            );
        }
    }

    // =========================================================================
    // Authentication Endpoint Tests
    // =========================================================================

    #[test]
    fn test_azure_region_token_endpoint() {
        let region = AzureRegion::EastUS;
        assert_eq!(
            region.token_endpoint(),
            "https://eastus.api.cognitive.microsoft.com/sts/v1.0/issueToken"
        );
    }

    // =========================================================================
    // FromStr Tests
    // =========================================================================

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
    fn test_azure_region_from_str_case_insensitive() {
        assert_eq!(
            "EASTUS".parse::<AzureRegion>().unwrap(),
            AzureRegion::EastUS
        );
        assert_eq!(
            "EastUs".parse::<AzureRegion>().unwrap(),
            AzureRegion::EastUS
        );
        assert_eq!(
            "WestEurope".parse::<AzureRegion>().unwrap(),
            AzureRegion::WestEurope
        );
    }

    #[test]
    fn test_region_from_str_all_known_regions() {
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

    // =========================================================================
    // All Regions as_str Tests
    // =========================================================================

    #[test]
    fn test_all_regions_as_str() {
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

    // =========================================================================
    // Backwards Compatibility Tests
    // =========================================================================

    #[test]
    #[allow(deprecated)]
    fn test_deprecated_hostname_method() {
        let region = AzureRegion::WestEurope;
        assert_eq!(region.hostname(), region.stt_hostname());
    }

    #[test]
    #[allow(deprecated)]
    fn test_deprecated_websocket_base_url_method() {
        let region = AzureRegion::SoutheastAsia;
        assert_eq!(region.websocket_base_url(), region.stt_websocket_base_url());
    }
}
