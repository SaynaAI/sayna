//! Azure Speech Services authentication helpers.
//!
//! This module provides authentication utilities for Azure Cognitive Services
//! Speech APIs, supporting both direct subscription key authentication and
//! bearer token authentication.
//!
//! # Authentication Methods
//!
//! Azure Speech Services supports two authentication methods:
//!
//! ## 1. Subscription Key (Recommended for server-side applications)
//!
//! Pass your subscription key directly via the `Ocp-Apim-Subscription-Key` header.
//! This is the simplest method and recommended for server-side applications where
//! the key can be kept secure.
//!
//! ```rust
//! use sayna::core::providers::azure::{AZURE_SUBSCRIPTION_KEY_HEADER, build_subscription_key_header};
//!
//! let api_key = "your-subscription-key";
//! let headers = build_subscription_key_header(api_key);
//! // Use in HTTP request: headers.insert(AZURE_SUBSCRIPTION_KEY_HEADER, api_key);
//! ```
//!
//! ## 2. Bearer Token (For enhanced security)
//!
//! Exchange your subscription key for a short-lived access token (valid for 10 minutes).
//! This is useful when you want to avoid exposing your subscription key in requests.
//!
//! To obtain a token:
//! 1. POST to the token endpoint with your subscription key
//! 2. Use the returned token in the `Authorization: Bearer {token}` header
//!
//! See: <https://learn.microsoft.com/en-us/azure/ai-services/speech-service/rest-speech-to-text#authentication>

use super::AzureRegion;

/// The HTTP header name for Azure subscription key authentication.
///
/// Use this header to authenticate with your Azure subscription key directly.
///
/// # Example
///
/// ```rust
/// use sayna::core::providers::azure::AZURE_SUBSCRIPTION_KEY_HEADER;
///
/// assert_eq!(AZURE_SUBSCRIPTION_KEY_HEADER, "Ocp-Apim-Subscription-Key");
/// ```
pub const AZURE_SUBSCRIPTION_KEY_HEADER: &str = "Ocp-Apim-Subscription-Key";

/// The HTTP header name for bearer token authentication.
///
/// Use this header when authenticating with an access token obtained from
/// the token endpoint.
///
/// # Example
///
/// ```rust
/// use sayna::core::providers::azure::AZURE_AUTHORIZATION_HEADER;
///
/// assert_eq!(AZURE_AUTHORIZATION_HEADER, "Authorization");
/// ```
pub const AZURE_AUTHORIZATION_HEADER: &str = "Authorization";

/// Build the subscription key header value.
///
/// This is a simple helper that returns the API key as-is, since the
/// subscription key header value is just the key itself (not prefixed).
///
/// # Arguments
///
/// * `api_key` - The Azure subscription key
///
/// # Returns
///
/// The API key string, ready to be used as the header value.
///
/// # Example
///
/// ```rust
/// use sayna::core::providers::azure::build_subscription_key_header;
///
/// let api_key = "your-subscription-key";
/// let header_value = build_subscription_key_header(api_key);
/// assert_eq!(header_value, "your-subscription-key");
/// ```
#[inline]
pub fn build_subscription_key_header(api_key: &str) -> &str {
    api_key
}

/// Build the bearer token authorization header value.
///
/// Formats the access token as a Bearer token for use in the Authorization header.
///
/// # Arguments
///
/// * `token` - The access token obtained from the token endpoint
///
/// # Returns
///
/// The formatted bearer token string: `Bearer {token}`
///
/// # Example
///
/// ```rust
/// use sayna::core::providers::azure::build_bearer_token_header;
///
/// let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...";
/// let header_value = build_bearer_token_header(token);
/// assert!(header_value.starts_with("Bearer "));
/// ```
#[inline]
pub fn build_bearer_token_header(token: &str) -> String {
    format!("Bearer {}", token)
}

/// Build the token request URL for a given region.
///
/// Returns the URL to POST to for obtaining an access token.
///
/// # Arguments
///
/// * `region` - The Azure region
///
/// # Returns
///
/// The token endpoint URL for the specified region.
///
/// # Example
///
/// ```rust
/// use sayna::core::providers::azure::{AzureRegion, build_token_request_url};
///
/// let region = AzureRegion::EastUS;
/// let url = build_token_request_url(&region);
/// assert!(url.contains("eastus"));
/// assert!(url.contains("issueToken"));
/// ```
#[inline]
pub fn build_token_request_url(region: &AzureRegion) -> String {
    region.token_endpoint()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subscription_key_header_constant() {
        assert_eq!(AZURE_SUBSCRIPTION_KEY_HEADER, "Ocp-Apim-Subscription-Key");
    }

    #[test]
    fn test_authorization_header_constant() {
        assert_eq!(AZURE_AUTHORIZATION_HEADER, "Authorization");
    }

    #[test]
    fn test_build_subscription_key_header() {
        let api_key = "test-subscription-key-12345";
        let result = build_subscription_key_header(api_key);
        assert_eq!(result, api_key);
    }

    #[test]
    fn test_build_bearer_token_header() {
        let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test";
        let result = build_bearer_token_header(token);
        assert_eq!(result, "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test");
    }

    #[test]
    fn test_build_bearer_token_header_empty() {
        let result = build_bearer_token_header("");
        assert_eq!(result, "Bearer ");
    }

    #[test]
    fn test_build_token_request_url() {
        let region = AzureRegion::EastUS;
        let url = build_token_request_url(&region);
        assert_eq!(
            url,
            "https://eastus.api.cognitive.microsoft.com/sts/v1.0/issueToken"
        );
    }

    #[test]
    fn test_build_token_request_url_different_regions() {
        let test_cases = vec![
            (
                AzureRegion::WestEurope,
                "https://westeurope.api.cognitive.microsoft.com/sts/v1.0/issueToken",
            ),
            (
                AzureRegion::JapanEast,
                "https://japaneast.api.cognitive.microsoft.com/sts/v1.0/issueToken",
            ),
            (
                AzureRegion::Custom("customregion".to_string()),
                "https://customregion.api.cognitive.microsoft.com/sts/v1.0/issueToken",
            ),
        ];

        for (region, expected_url) in test_cases {
            assert_eq!(build_token_request_url(&region), expected_url);
        }
    }
}
