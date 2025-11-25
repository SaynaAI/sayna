//! Google Cloud gRPC client infrastructure.
//!
//! This module provides generic gRPC client functionality for Google Cloud APIs,
//! including authenticated channel creation and interceptor support.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use sayna::core::providers::google::{
//!     CredentialSource, GoogleAuthClient, TokenProvider,
//!     create_grpc_channel, AuthenticatedChannel,
//! };
//!
//! async fn example() -> Result<(), Box<dyn std::error::Error>> {
//!     let source = CredentialSource::from_api_key("");
//!     let scopes = &["https://www.googleapis.com/auth/cloud-platform"];
//!     let auth_client = Arc::new(GoogleAuthClient::new(source, scopes)?);
//!
//!     let channel = create_grpc_channel("https://speech.googleapis.com").await?;
//!     let authenticated = AuthenticatedChannel::new(channel, auth_client);
//!
//!     // Use with any Google Cloud gRPC service
//!     let auth_header = authenticated.get_authorization_header().await?;
//!
//!     Ok(())
//! }
//! ```

use std::sync::Arc;

use tracing::{debug, error};

use super::auth::TokenProvider;
use super::error::GoogleError;

/// Creates a gRPC channel to a Google Cloud API endpoint.
///
/// This function establishes a TLS-secured connection to the specified endpoint.
/// The channel can be reused for multiple requests.
///
/// # Arguments
///
/// * `endpoint` - The Google Cloud API endpoint URL (e.g., "https://speech.googleapis.com")
///
/// # Errors
///
/// Returns `GoogleError::ConnectionFailed` if the connection cannot be established.
///
/// # Example
///
/// ```rust,no_run
/// use sayna::core::providers::google::create_grpc_channel;
///
/// async fn example() -> Result<(), Box<dyn std::error::Error>> {
///     let channel = create_grpc_channel("https://speech.googleapis.com").await?;
///     // Use the channel with a gRPC client
///     Ok(())
/// }
/// ```
pub async fn create_grpc_channel(endpoint: &str) -> Result<tonic::transport::Channel, GoogleError> {
    let channel = tonic::transport::Channel::from_shared(endpoint.to_string())
        .map_err(|e| {
            error!(error = %e, endpoint = %endpoint, "Invalid endpoint URL");
            GoogleError::ConfigurationError(format!("Invalid endpoint URL '{endpoint}': {e}"))
        })?
        .tls_config(tonic::transport::ClientTlsConfig::new())
        .map_err(|e| {
            error!(error = %e, "Failed to configure TLS");
            GoogleError::ConnectionFailed(format!("Failed to configure TLS: {e}"))
        })?
        .connect()
        .await
        .map_err(|e| {
            error!(error = %e, endpoint = %endpoint, "Failed to connect to Google API");
            GoogleError::ConnectionFailed(format!("Failed to connect to '{endpoint}': {e}"))
        })?;

    debug!(endpoint = %endpoint, "Connected to Google Cloud API");

    Ok(channel)
}

/// An authenticated gRPC channel wrapper that attaches bearer tokens to requests.
///
/// This struct combines a gRPC channel with a token provider to create
/// authenticated connections to Google Cloud APIs.
pub struct AuthenticatedChannel {
    channel: tonic::transport::Channel,
    token_provider: Arc<dyn TokenProvider>,
}

impl AuthenticatedChannel {
    /// Creates a new authenticated channel.
    ///
    /// # Arguments
    ///
    /// * `channel` - The underlying gRPC channel
    /// * `token_provider` - The token provider for authentication
    pub fn new(channel: tonic::transport::Channel, token_provider: Arc<dyn TokenProvider>) -> Self {
        Self {
            channel,
            token_provider,
        }
    }

    /// Returns a reference to the underlying gRPC channel.
    pub fn channel(&self) -> &tonic::transport::Channel {
        &self.channel
    }

    /// Clones the underlying gRPC channel.
    ///
    /// Channels in tonic are cheap to clone as they use internal Arc.
    pub fn clone_channel(&self) -> tonic::transport::Channel {
        self.channel.clone()
    }

    /// Returns a reference to the token provider.
    pub fn token_provider(&self) -> &Arc<dyn TokenProvider> {
        &self.token_provider
    }

    /// Gets the authorization header value (including "Bearer " prefix).
    ///
    /// # Errors
    ///
    /// Returns `GoogleError::AuthenticationFailed` if token retrieval fails.
    pub async fn get_authorization_header(&self) -> Result<String, GoogleError> {
        let token = self.token_provider.get_token().await?;
        Ok(format!("Bearer {token}"))
    }

    /// Creates a tonic interceptor that adds the authorization header to requests.
    ///
    /// This interceptor fetches a fresh token for each call. For high-frequency
    /// calls, consider caching the header value and refreshing periodically.
    ///
    /// # Errors
    ///
    /// Returns `GoogleError::AuthenticationFailed` if the initial token cannot be fetched.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use sayna::core::providers::google::AuthenticatedChannel;
    ///
    /// async fn example(authenticated: &AuthenticatedChannel) -> Result<(), Box<dyn std::error::Error>> {
    ///     let interceptor = authenticated.create_interceptor().await?;
    ///     // Use with: SomeClient::with_interceptor(channel, interceptor)
    ///     Ok(())
    /// }
    /// ```
    pub async fn create_interceptor(
        &self,
    ) -> Result<
        impl FnMut(tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> + Clone,
        GoogleError,
    > {
        let auth_header = self.get_authorization_header().await?;

        // Pre-parse the header value once to avoid repeated parsing
        let auth_metadata_value: tonic::metadata::MetadataValue<_> =
            auth_header.parse().map_err(|_| {
                GoogleError::AuthenticationFailed(
                    "Failed to parse authorization header".to_string(),
                )
            })?;

        Ok(move |mut request: tonic::Request<()>| {
            request
                .metadata_mut()
                .insert("authorization", auth_metadata_value.clone());
            Ok(request)
        })
    }

    /// Creates a tonic interceptor with a pre-computed authorization header.
    ///
    /// This is useful when you want to reuse the same token for multiple requests
    /// without re-fetching it each time.
    ///
    /// # Arguments
    ///
    /// * `auth_header` - The full authorization header value (e.g., "Bearer token123")
    pub fn create_interceptor_with_header(
        auth_header: String,
    ) -> Result<
        impl FnMut(tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> + Clone,
        GoogleError,
    > {
        let auth_metadata_value: tonic::metadata::MetadataValue<_> =
            auth_header.parse().map_err(|_| {
                GoogleError::AuthenticationFailed(
                    "Failed to parse authorization header".to_string(),
                )
            })?;

        Ok(move |mut request: tonic::Request<()>| {
            request
                .metadata_mut()
                .insert("authorization", auth_metadata_value.clone());
            Ok(request)
        })
    }
}

/// Creates an authenticated gRPC channel for a Google Cloud API.
///
/// This is a convenience function that combines channel creation and authentication setup.
///
/// # Arguments
///
/// * `endpoint` - The Google Cloud API endpoint URL
/// * `token_provider` - The token provider for authentication
///
/// # Errors
///
/// Returns `GoogleError` if connection or authentication fails.
///
/// # Example
///
/// ```rust,no_run
/// use std::sync::Arc;
/// use sayna::core::providers::google::{
///     CredentialSource, GoogleAuthClient,
///     create_authenticated_channel,
/// };
///
/// async fn example() -> Result<(), Box<dyn std::error::Error>> {
///     let source = CredentialSource::from_api_key("");
///     let scopes = &["https://www.googleapis.com/auth/cloud-platform"];
///     let auth_client = Arc::new(GoogleAuthClient::new(source, scopes)?);
///
///     let authenticated = create_authenticated_channel(
///         "https://speech.googleapis.com",
///         auth_client,
///     ).await?;
///
///     Ok(())
/// }
/// ```
pub async fn create_authenticated_channel(
    endpoint: &str,
    token_provider: Arc<dyn TokenProvider>,
) -> Result<AuthenticatedChannel, GoogleError> {
    let channel = create_grpc_channel(endpoint).await?;
    Ok(AuthenticatedChannel::new(channel, token_provider))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::providers::google::auth::MockTokenProvider;

    #[tokio::test]
    async fn test_authenticated_channel_get_authorization_header() {
        let channel = tonic::transport::Channel::from_static("https://example.com").connect_lazy();
        let provider = Arc::new(MockTokenProvider::with_token("test-token"));

        let authenticated = AuthenticatedChannel::new(channel, provider);
        let header = authenticated.get_authorization_header().await.unwrap();

        assert_eq!(header, "Bearer test-token");
    }

    #[tokio::test]
    async fn test_authenticated_channel_get_authorization_header_error() {
        let channel = tonic::transport::Channel::from_static("https://example.com").connect_lazy();
        let provider = Arc::new(MockTokenProvider::with_error("Token fetch failed"));

        let authenticated = AuthenticatedChannel::new(channel, provider);
        let result = authenticated.get_authorization_header().await;

        assert!(result.is_err());
        if let Err(GoogleError::AuthenticationFailed(msg)) = result {
            assert_eq!(msg, "Token fetch failed");
        } else {
            panic!("Expected AuthenticationFailed error");
        }
    }

    #[tokio::test]
    async fn test_create_interceptor() {
        let channel = tonic::transport::Channel::from_static("https://example.com").connect_lazy();
        let provider = Arc::new(MockTokenProvider::with_token("test-token"));

        let authenticated = AuthenticatedChannel::new(channel, provider);
        let mut interceptor = authenticated.create_interceptor().await.unwrap();

        let request = tonic::Request::new(());
        let modified = interceptor(request).unwrap();

        let auth_header = modified.metadata().get("authorization").unwrap();
        assert_eq!(auth_header.to_str().unwrap(), "Bearer test-token");
    }

    #[test]
    fn test_create_interceptor_with_header() {
        let mut interceptor =
            AuthenticatedChannel::create_interceptor_with_header("Bearer my-token".to_string())
                .unwrap();

        let request = tonic::Request::new(());
        let modified = interceptor(request).unwrap();

        let auth_header = modified.metadata().get("authorization").unwrap();
        assert_eq!(auth_header.to_str().unwrap(), "Bearer my-token");
    }

    #[tokio::test]
    async fn test_channel_clone() {
        let channel = tonic::transport::Channel::from_static("https://example.com").connect_lazy();
        let provider = Arc::new(MockTokenProvider::with_token("test"));

        let authenticated = AuthenticatedChannel::new(channel, provider);
        let _cloned = authenticated.clone_channel();
        // Just verify it doesn't panic - channels are opaque
    }
}
