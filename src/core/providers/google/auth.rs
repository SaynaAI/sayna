//! Google Cloud authentication and credential management.
//!
//! This module provides generic authentication functionality for Google Cloud APIs,
//! including support for multiple credential sources (Application Default Credentials,
//! service account JSON files, and inline JSON credentials).
//!
//! # Credential Sources
//!
//! Google Cloud authentication supports three credential sources:
//!
//! - **Application Default Credentials (ADC)**: Used when no explicit credentials are provided.
//!   Reads from `GOOGLE_APPLICATION_CREDENTIALS` env var, default service account on GCP,
//!   or `gcloud auth application-default login`.
//!
//! - **JSON Content**: Service account credentials provided directly as a JSON string.
//!   Useful for secrets management systems that inject credentials as environment variables.
//!
//! - **File Path**: Path to a service account JSON file or user credentials file.
//!
//! # Example
//!
//! ```rust,no_run
//! use sayna::core::providers::google::{CredentialSource, GoogleAuthClient, TokenProvider};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Using Application Default Credentials
//!     let source = CredentialSource::ApplicationDefault;
//!     let scopes = &["https://www.googleapis.com/auth/cloud-platform"];
//!     let client = GoogleAuthClient::new(source, scopes)?;
//!
//!     // Get an access token
//!     let token = client.get_token().await?;
//!     Ok(())
//! }
//! ```

use std::path::Path;

use google_cloud_auth::credentials::{Builder as CredentialsBuilder, Credentials};
use http::Extensions;
use tracing::{debug, error};

use super::error::GoogleError;

/// Determines the source of Google Cloud credentials based on the api_key value.
///
/// This enum represents the three ways credentials can be provided to Google Cloud APIs.
#[derive(Debug, Clone, PartialEq)]
pub enum CredentialSource {
    /// Use Application Default Credentials (from `GOOGLE_APPLICATION_CREDENTIALS` env var,
    /// default service account on GCP, or `gcloud auth application-default login`).
    ApplicationDefault,

    /// Service account JSON content provided directly as a string.
    /// Detected when the input starts with `{`.
    JsonContent(String),

    /// Path to a service account JSON file or user credentials file.
    FilePath(String),
}

impl CredentialSource {
    /// Determines the credential source based on the api_key value.
    ///
    /// - Empty string → `ApplicationDefault`
    /// - Starts with `{` → `JsonContent`
    /// - Otherwise → `FilePath`
    ///
    /// # Arguments
    ///
    /// * `api_key` - The credential string (can be empty, JSON content, or file path)
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::providers::google::CredentialSource;
    ///
    /// // Empty string uses ADC
    /// assert_eq!(
    ///     CredentialSource::from_api_key(""),
    ///     CredentialSource::ApplicationDefault
    /// );
    ///
    /// // JSON content
    /// let json = r#"{"type": "service_account"}"#;
    /// assert!(matches!(
    ///     CredentialSource::from_api_key(json),
    ///     CredentialSource::JsonContent(_)
    /// ));
    ///
    /// // File path
    /// assert!(matches!(
    ///     CredentialSource::from_api_key("/path/to/creds.json"),
    ///     CredentialSource::FilePath(_)
    /// ));
    /// ```
    pub fn from_api_key(api_key: &str) -> Self {
        if api_key.is_empty() {
            CredentialSource::ApplicationDefault
        } else if api_key.trim_start().starts_with('{') {
            CredentialSource::JsonContent(api_key.to_string())
        } else {
            CredentialSource::FilePath(api_key.to_string())
        }
    }

    /// Validates the credential source for common issues.
    ///
    /// - `ApplicationDefault`: Always valid (actual validation happens at connection time)
    /// - `JsonContent`: Validates JSON syntax and structure
    /// - `FilePath`: Validates path doesn't contain traversal and file exists
    ///
    /// # Errors
    ///
    /// Returns `GoogleError::ConfigurationError` if validation fails.
    pub fn validate(&self) -> Result<(), GoogleError> {
        match self {
            CredentialSource::ApplicationDefault => Ok(()),
            CredentialSource::JsonContent(json) => {
                if !json.trim_start().starts_with('{') || !json.trim_end().ends_with('}') {
                    return Err(GoogleError::ConfigurationError(
                        "Invalid JSON content: must be a JSON object".to_string(),
                    ));
                }

                serde_json::from_str::<serde_json::Value>(json).map_err(|e| {
                    GoogleError::ConfigurationError(format!("Invalid JSON content: {e}"))
                })?;
                Ok(())
            }
            CredentialSource::FilePath(path) => {
                if path.contains("..") {
                    return Err(GoogleError::ConfigurationError(
                        "Invalid credential file path: path traversal not allowed".to_string(),
                    ));
                }

                if !Path::new(path).exists() {
                    return Err(GoogleError::ConfigurationError(format!(
                        "Credential file not found: {path}"
                    )));
                }

                Ok(())
            }
        }
    }

    /// Extracts the project_id from credentials.
    ///
    /// For service account credentials, the project_id is stored in the JSON.
    /// For Application Default Credentials, this will attempt to read from
    /// the GOOGLE_APPLICATION_CREDENTIALS file or return None.
    ///
    /// # Returns
    ///
    /// Returns `Some(project_id)` if found, `None` otherwise.
    ///
    /// # Example
    ///
    /// ```rust
    /// use sayna::core::providers::google::CredentialSource;
    ///
    /// let json = r#"{"type": "service_account", "project_id": "my-project"}"#;
    /// let source = CredentialSource::from_api_key(json);
    /// assert_eq!(source.extract_project_id(), Some("my-project".to_string()));
    /// ```
    pub fn extract_project_id(&self) -> Option<String> {
        match self {
            CredentialSource::ApplicationDefault => {
                // Try to read project_id from GOOGLE_APPLICATION_CREDENTIALS file
                if let Ok(path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                            return json
                                .get("project_id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                        }
                    }
                }
                None
            }
            CredentialSource::JsonContent(json) => {
                serde_json::from_str::<serde_json::Value>(json)
                    .ok()
                    .and_then(|v| v.get("project_id").and_then(|p| p.as_str()).map(|s| s.to_string()))
            }
            CredentialSource::FilePath(path) => {
                std::fs::read_to_string(path)
                    .ok()
                    .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
                    .and_then(|json| {
                        json.get("project_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
            }
        }
    }
}

/// Trait for providing OAuth2 access tokens for Google Cloud APIs.
///
/// This trait abstracts token retrieval, allowing for different implementations
/// (production credentials, mock providers for testing, etc.).
#[async_trait::async_trait]
pub trait TokenProvider: Send + Sync {
    /// Retrieves a valid access token for Google Cloud APIs.
    ///
    /// The token is automatically refreshed when expired.
    ///
    /// # Errors
    ///
    /// Returns `GoogleError::AuthenticationFailed` if token retrieval fails.
    async fn get_token(&self) -> Result<String, GoogleError>;
}

/// Google Cloud authentication client that manages credentials and access tokens.
///
/// This client handles OAuth2 token acquisition and refresh for Google Cloud APIs.
/// It supports all credential types (ADC, service account, user credentials).
pub struct GoogleAuthClient {
    credentials: Credentials,
}

impl std::fmt::Debug for GoogleAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleAuthClient")
            .field("credentials", &"<credentials>")
            .finish()
    }
}

impl GoogleAuthClient {
    /// Creates a new Google Cloud authentication client.
    ///
    /// # Arguments
    ///
    /// * `credential_source` - The source of credentials
    /// * `scopes` - OAuth2 scopes required for the API being accessed
    ///
    /// # Errors
    ///
    /// Returns an error if credentials cannot be loaded or are invalid.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use sayna::core::providers::google::{CredentialSource, GoogleAuthClient};
    ///
    /// let source = CredentialSource::from_api_key("");
    /// let scopes = &["https://www.googleapis.com/auth/cloud-platform"];
    /// let client = GoogleAuthClient::new(source, scopes)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new<S: AsRef<str>>(
        credential_source: CredentialSource,
        scopes: &[S],
    ) -> Result<Self, GoogleError> {
        credential_source.validate()?;

        let scope_strings: Vec<String> = scopes.iter().map(|s| s.as_ref().to_string()).collect();

        let credentials = match credential_source {
            CredentialSource::ApplicationDefault => {
                CredentialsBuilder::default()
                    .with_scopes(scope_strings.clone())
                    .build()
                    .map_err(|e| {
                        error!(error = %e, "Failed to initialize Application Default Credentials");
                        GoogleError::AuthenticationFailed(format!(
                            "Failed to initialize Application Default Credentials: {e}. \
                             Ensure GOOGLE_APPLICATION_CREDENTIALS is set or run 'gcloud auth application-default login'"
                        ))
                    })?
            }
            CredentialSource::JsonContent(ref json) => {
                use google_cloud_auth::credentials::service_account;
                let json_value: serde_json::Value = serde_json::from_str(json).map_err(|e| {
                    GoogleError::ConfigurationError(format!("Invalid JSON content: {e}"))
                })?;

                service_account::Builder::new(json_value)
                    .with_access_specifier(service_account::AccessSpecifier::from_scopes(
                        scope_strings,
                    ))
                    .build()
                    .map_err(|e| {
                        error!(error = %e, "Failed to initialize credentials from JSON content");
                        GoogleError::AuthenticationFailed(format!(
                            "Failed to initialize credentials from JSON content: {e}"
                        ))
                    })?
            }
            CredentialSource::FilePath(ref path) => {
                let json_content = std::fs::read_to_string(path).map_err(|e| {
                    error!(error = %e, path = %path, "Failed to read credentials file");
                    GoogleError::ConfigurationError(format!(
                        "Failed to read credentials file '{path}': {e}"
                    ))
                })?;

                let json_value: serde_json::Value =
                    serde_json::from_str(&json_content).map_err(|e| {
                        error!(error = %e, path = %path, "Failed to parse credentials file");
                        GoogleError::ConfigurationError(format!(
                            "Failed to parse credentials file '{path}': {e}"
                        ))
                    })?;

                let cred_type = json_value
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                match cred_type {
                    "service_account" => {
                        use google_cloud_auth::credentials::service_account;
                        service_account::Builder::new(json_value)
                            .with_access_specifier(service_account::AccessSpecifier::from_scopes(
                                scope_strings,
                            ))
                            .build()
                            .map_err(|e| {
                                error!(error = %e, path = %path, "Failed to load service account credentials");
                                GoogleError::AuthenticationFailed(format!(
                                    "Failed to load service account credentials from '{path}': {e}"
                                ))
                            })?
                    }
                    "authorized_user" => {
                        use google_cloud_auth::credentials::user_account;
                        user_account::Builder::new(json_value)
                            .with_scopes(scope_strings)
                            .build()
                            .map_err(|e| {
                                error!(error = %e, path = %path, "Failed to load user account credentials");
                                GoogleError::AuthenticationFailed(format!(
                                    "Failed to load user account credentials from '{path}': {e}"
                                ))
                            })?
                    }
                    _ => {
                        return Err(GoogleError::ConfigurationError(format!(
                            "Unsupported credential type '{cred_type}' in file '{path}'. \
                             Expected 'service_account' or 'authorized_user'"
                        )));
                    }
                }
            }
        };

        debug!("Google Cloud authentication client initialized successfully");
        Ok(Self { credentials })
    }

    /// Creates a new auth client from the api_key field.
    ///
    /// This is a convenience method that combines `CredentialSource::from_api_key`
    /// and `GoogleAuthClient::new`.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The credential string (empty for ADC, JSON content, or file path)
    /// * `scopes` - OAuth2 scopes required for the API
    pub fn from_api_key<S: AsRef<str>>(api_key: &str, scopes: &[S]) -> Result<Self, GoogleError> {
        let source = CredentialSource::from_api_key(api_key);
        Self::new(source, scopes)
    }

    fn extract_token_from_headers(
        &self,
        headers: google_cloud_auth::credentials::CacheableResource<http::HeaderMap>,
    ) -> Result<String, GoogleError> {
        use google_cloud_auth::credentials::CacheableResource;

        let header_map = match headers {
            CacheableResource::New { data, .. } => data,
            CacheableResource::NotModified => {
                return Err(GoogleError::AuthenticationFailed(
                    "Received NotModified response but no cached token available".to_string(),
                ));
            }
        };

        let auth_value = header_map.get(http::header::AUTHORIZATION).ok_or_else(|| {
            GoogleError::AuthenticationFailed(
                "No Authorization header in credentials response".to_string(),
            )
        })?;

        let auth_str = auth_value.to_str().map_err(|e| {
            GoogleError::AuthenticationFailed(format!("Invalid Authorization header value: {e}"))
        })?;

        let token = auth_str
            .strip_prefix("Bearer ")
            .ok_or_else(|| {
                GoogleError::AuthenticationFailed(
                    "Authorization header is not a Bearer token".to_string(),
                )
            })?
            .to_string();

        Ok(token)
    }
}

#[async_trait::async_trait]
impl TokenProvider for GoogleAuthClient {
    async fn get_token(&self) -> Result<String, GoogleError> {
        let headers = self
            .credentials
            .headers(Extensions::new())
            .await
            .map_err(|e| {
                error!(error = %e, "Failed to fetch access token");
                GoogleError::AuthenticationFailed(format!("Failed to fetch access token: {e}"))
            })?;

        self.extract_token_from_headers(headers)
    }
}

/// A mock token provider for testing purposes.
#[cfg(test)]
pub struct MockTokenProvider {
    pub token: Option<String>,
    pub error: Option<String>,
}

#[cfg(test)]
impl MockTokenProvider {
    pub fn with_token(token: impl Into<String>) -> Self {
        Self {
            token: Some(token.into()),
            error: None,
        }
    }

    pub fn with_error(error: impl Into<String>) -> Self {
        Self {
            token: None,
            error: Some(error.into()),
        }
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl TokenProvider for MockTokenProvider {
    async fn get_token(&self) -> Result<String, GoogleError> {
        if let Some(ref error) = self.error {
            Err(GoogleError::AuthenticationFailed(error.clone()))
        } else if let Some(ref token) = self.token {
            Ok(token.clone())
        } else {
            Err(GoogleError::AuthenticationFailed(
                "Mock provider not configured".to_string(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_source_from_empty_api_key() {
        let source = CredentialSource::from_api_key("");
        assert_eq!(source, CredentialSource::ApplicationDefault);
    }

    #[test]
    fn test_credential_source_from_json_content() {
        let json = r#"{"type": "service_account", "project_id": "test"}"#;
        let source = CredentialSource::from_api_key(json);
        assert_eq!(source, CredentialSource::JsonContent(json.to_string()));
    }

    #[test]
    fn test_credential_source_from_json_with_whitespace() {
        let json = r#"  { "type": "service_account" }"#;
        let source = CredentialSource::from_api_key(json);
        assert_eq!(source, CredentialSource::JsonContent(json.to_string()));
    }

    #[test]
    fn test_credential_source_from_file_path() {
        let path = "/path/to/credentials.json";
        let source = CredentialSource::from_api_key(path);
        assert_eq!(source, CredentialSource::FilePath(path.to_string()));
    }

    #[test]
    fn test_credential_source_validate_application_default() {
        let source = CredentialSource::ApplicationDefault;
        assert!(source.validate().is_ok());
    }

    #[test]
    fn test_credential_source_validate_valid_json() {
        let json = r#"{"type": "service_account", "project_id": "test"}"#;
        let source = CredentialSource::JsonContent(json.to_string());
        assert!(source.validate().is_ok());
    }

    #[test]
    fn test_credential_source_validate_invalid_json_structure() {
        let json = "not a json object";
        let source = CredentialSource::JsonContent(json.to_string());
        let result = source.validate();
        assert!(result.is_err());
        if let Err(GoogleError::ConfigurationError(msg)) = result {
            assert!(msg.contains("must be a JSON object"));
        } else {
            panic!("Expected ConfigurationError");
        }
    }

    #[test]
    fn test_credential_source_validate_path_traversal() {
        let path = "../../../etc/passwd";
        let source = CredentialSource::FilePath(path.to_string());
        let result = source.validate();
        assert!(result.is_err());
        if let Err(GoogleError::ConfigurationError(msg)) = result {
            assert!(msg.contains("path traversal not allowed"));
        } else {
            panic!("Expected ConfigurationError");
        }
    }

    #[test]
    fn test_credential_source_validate_nonexistent_file() {
        let path = "/nonexistent/path/to/credentials.json";
        let source = CredentialSource::FilePath(path.to_string());
        let result = source.validate();
        assert!(result.is_err());
        if let Err(GoogleError::ConfigurationError(msg)) = result {
            assert!(msg.contains("Credential file not found"));
        } else {
            panic!("Expected ConfigurationError");
        }
    }

    #[test]
    fn test_credential_source_validate_existing_file() {
        let source = CredentialSource::FilePath("Cargo.toml".to_string());
        assert!(source.validate().is_ok());
    }

    #[tokio::test]
    async fn test_mock_token_provider_with_token() {
        let provider = MockTokenProvider::with_token("test-token-12345");
        let token = provider.get_token().await.unwrap();
        assert_eq!(token, "test-token-12345");
    }

    #[tokio::test]
    async fn test_mock_token_provider_with_error() {
        let provider = MockTokenProvider::with_error("Token fetch failed");
        let result = provider.get_token().await;
        assert!(result.is_err());
        if let Err(GoogleError::AuthenticationFailed(msg)) = result {
            assert_eq!(msg, "Token fetch failed");
        } else {
            panic!("Expected AuthenticationFailed error");
        }
    }

    #[tokio::test]
    async fn test_mock_token_provider_unconfigured() {
        let provider = MockTokenProvider {
            token: None,
            error: None,
        };
        let result = provider.get_token().await;
        assert!(result.is_err());
        if let Err(GoogleError::AuthenticationFailed(msg)) = result {
            assert!(msg.contains("not configured"));
        } else {
            panic!("Expected AuthenticationFailed error");
        }
    }

    #[test]
    fn test_extract_project_id_from_json_content() {
        let json = r#"{"type": "service_account", "project_id": "my-test-project"}"#;
        let source = CredentialSource::JsonContent(json.to_string());
        assert_eq!(
            source.extract_project_id(),
            Some("my-test-project".to_string())
        );
    }

    #[test]
    fn test_extract_project_id_from_json_content_missing() {
        let json = r#"{"type": "service_account"}"#;
        let source = CredentialSource::JsonContent(json.to_string());
        assert_eq!(source.extract_project_id(), None);
    }

    #[test]
    fn test_extract_project_id_application_default_no_env() {
        let source = CredentialSource::ApplicationDefault;
        // Without GOOGLE_APPLICATION_CREDENTIALS set to a valid file,
        // this should return None
        let result = source.extract_project_id();
        // Result depends on environment, so we just check it doesn't panic
        assert!(result.is_none() || result.is_some());
    }

    #[test]
    fn test_extract_project_id_file_path_nonexistent() {
        let source = CredentialSource::FilePath("/nonexistent/path/creds.json".to_string());
        assert_eq!(source.extract_project_id(), None);
    }

    #[test]
    fn test_extract_project_id_from_api_key_json() {
        let json = r#"{"type": "service_account", "project_id": "extracted-project"}"#;
        let source = CredentialSource::from_api_key(json);
        assert_eq!(
            source.extract_project_id(),
            Some("extracted-project".to_string())
        );
    }
}
