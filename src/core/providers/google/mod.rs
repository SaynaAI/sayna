//! Google Cloud provider infrastructure.
//!
//! This module provides generic authentication and gRPC client functionality
//! for Google Cloud APIs. It is designed to be reused across different Google
//! Cloud services (Speech-to-Text, Text-to-Speech, etc.).
//!
//! # Architecture
//!
//! The module is organized into three main components:
//!
//! - **auth**: Credential management and token acquisition
//! - **client**: gRPC channel creation and authentication
//! - **error**: Common error types for Google Cloud operations
//!
//! # Credential Sources
//!
//! Google Cloud authentication supports three credential sources:
//!
//! 1. **Application Default Credentials (ADC)**: Used when no explicit credentials
//!    are provided. Reads from `GOOGLE_APPLICATION_CREDENTIALS` environment variable,
//!    default service account on GCP, or `gcloud auth application-default login`.
//!
//! 2. **JSON Content**: Service account credentials provided directly as a JSON string.
//!    Detected when the credential string starts with `{`.
//!
//! 3. **File Path**: Path to a service account JSON file or user credentials file.
//!
//! # Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use sayna::core::providers::google::{
//!     CredentialSource, GoogleAuthClient, TokenProvider,
//!     create_authenticated_channel, GoogleError,
//! };
//!
//! async fn example() -> Result<(), GoogleError> {
//!     // Create credentials from ADC
//!     let source = CredentialSource::from_api_key("");
//!     let scopes = &["https://www.googleapis.com/auth/cloud-platform"];
//!     let auth_client = Arc::new(GoogleAuthClient::new(source, scopes)?);
//!
//!     // Create an authenticated channel
//!     let authenticated = create_authenticated_channel(
//!         "https://speech.googleapis.com",
//!         auth_client,
//!     ).await?;
//!
//!     // Get authorization header for manual use
//!     let auth_header = authenticated.get_authorization_header().await?;
//!
//!     // Or create an interceptor for use with gRPC clients
//!     let interceptor = authenticated.create_interceptor().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Common Scopes
//!
//! Different Google Cloud services require different OAuth2 scopes:
//!
//! - **Cloud Platform**: `https://www.googleapis.com/auth/cloud-platform`
//! - **Speech-to-Text**: `https://www.googleapis.com/auth/cloud-platform`
//! - **Text-to-Speech**: `https://www.googleapis.com/auth/cloud-platform`
//!
//! The `cloud-platform` scope provides access to all Google Cloud APIs.

pub mod auth;
pub mod client;
pub mod error;

// Re-export commonly used types
pub use auth::{CredentialSource, GoogleAuthClient, TokenProvider};
pub use client::{AuthenticatedChannel, create_authenticated_channel, create_grpc_channel};
pub use error::GoogleError;

// Re-export MockTokenProvider for use in tests across the codebase
#[cfg(test)]
pub use auth::MockTokenProvider;

/// The default OAuth2 scope for Google Cloud APIs.
///
/// This scope provides access to all Google Cloud services.
pub const GOOGLE_CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

/// Google Cloud Speech-to-Text API endpoint.
pub const GOOGLE_SPEECH_ENDPOINT: &str = "https://speech.googleapis.com";

/// Google Cloud Text-to-Speech API endpoint.
pub const GOOGLE_TTS_ENDPOINT: &str = "https://texttospeech.googleapis.com";
