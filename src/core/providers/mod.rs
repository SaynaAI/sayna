//! Provider infrastructure for external cloud services.
//!
//! This module contains generic infrastructure for interacting with cloud providers
//! like Google Cloud. The infrastructure is designed to be reusable across different
//! services (STT, TTS, etc.) from the same provider.
//!
//! # Available Providers
//!
//! - **google**: Google Cloud authentication and gRPC client infrastructure

pub mod google;

// Re-export Google Cloud types for convenience
pub use google::{
    AuthenticatedChannel, CredentialSource, GOOGLE_CLOUD_PLATFORM_SCOPE, GOOGLE_SPEECH_ENDPOINT,
    GOOGLE_TTS_ENDPOINT, GoogleAuthClient, GoogleError, TokenProvider,
    create_authenticated_channel, create_grpc_channel,
};
