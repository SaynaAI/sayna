//! Provider infrastructure for external cloud services.
//!
//! This module contains generic infrastructure for interacting with cloud providers
//! like Google Cloud and Microsoft Azure. The infrastructure is designed to be
//! reusable across different services (STT, TTS, etc.) from the same provider.
//!
//! # Available Providers
//!
//! - **google**: Google Cloud authentication and gRPC client infrastructure
//! - **azure**: Microsoft Azure Speech Services region and authentication infrastructure

pub mod azure;
pub mod google;

// Re-export Google Cloud types for convenience
pub use google::{
    AuthenticatedChannel, CredentialSource, GOOGLE_CLOUD_PLATFORM_SCOPE, GOOGLE_SPEECH_ENDPOINT,
    GOOGLE_TTS_ENDPOINT, GoogleAuthClient, GoogleError, TokenProvider,
    create_authenticated_channel, create_grpc_channel,
};

// Re-export Azure types for convenience
pub use azure::{
    AZURE_AUTHORIZATION_HEADER, AZURE_SUBSCRIPTION_KEY_HEADER, AzureRegion,
    build_bearer_token_header, build_subscription_key_header, build_token_request_url,
};
