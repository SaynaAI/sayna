//! LiveKit integration handlers
//!
//! This module provides REST API endpoints for LiveKit-related operations:
//! - Token generation for participant authentication
//! - Webhook handling for LiveKit events
//!
//! # Endpoints
//!
//! - `POST /livekit/token` - Generate JWT tokens for LiveKit room access
//! - `POST /livekit/webhook` - Receive and process LiveKit webhook events

mod token;
mod webhook;

// Re-export handlers for clean API access
pub use token::{TokenRequest, TokenResponse, generate_token};
pub use webhook::{
    SIPHookEvent, SIPHookParticipant, SIPHookRoom, SipForwardingError, extract_sip_attributes,
    get_sip_host_header, handle_livekit_webhook, parse_sip_domain, webhook_error_to_status,
};

// Re-export utoipa-generated path types for OpenAPI spec generation
#[cfg(feature = "openapi")]
pub use token::__path_generate_token;
