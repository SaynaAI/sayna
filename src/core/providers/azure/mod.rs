//! Microsoft Azure Speech Services provider infrastructure.
//!
//! This module provides shared infrastructure for Azure Cognitive Services
//! Speech APIs, including region configuration and authentication helpers.
//! It is designed to be reused across different Azure Speech services
//! (Speech-to-Text, Text-to-Speech, etc.).
//!
//! # Architecture
//!
//! The module is organized into two main components:
//!
//! - **region**: Regional endpoint configuration for all Azure Speech services
//! - **auth**: Authentication helpers for subscription key and bearer token auth
//!
//! # Authentication
//!
//! Azure Speech Services supports two authentication methods:
//!
//! 1. **Subscription Key**: Pass your key via the `Ocp-Apim-Subscription-Key` header.
//!    This is the simplest method for server-side applications.
//!
//! 2. **Bearer Token**: Exchange your subscription key for a short-lived access token
//!    (valid for 10 minutes). Use when you want to avoid exposing your key.
//!
//! # Example
//!
//! ```rust
//! use sayna::core::providers::azure::{
//!     AzureRegion,
//!     AZURE_SUBSCRIPTION_KEY_HEADER,
//!     build_subscription_key_header,
//! };
//!
//! // Select a region
//! let region = AzureRegion::WestEurope;
//!
//! // Get endpoints
//! let stt_url = region.stt_websocket_base_url();
//! let tts_url = region.tts_rest_url();
//! let voices_url = region.voices_list_url();
//!
//! // Build authentication header
//! let api_key = "your-subscription-key";
//! let header_value = build_subscription_key_header(api_key);
//! ```
//!
//! See: <https://learn.microsoft.com/en-us/azure/ai-services/speech-service/>

pub mod auth;
pub mod region;

// Re-export commonly used types
pub use auth::{
    AZURE_AUTHORIZATION_HEADER, AZURE_SUBSCRIPTION_KEY_HEADER, build_bearer_token_header,
    build_subscription_key_header, build_token_request_url,
};
pub use region::AzureRegion;
