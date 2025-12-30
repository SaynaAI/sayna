//! SIP integration handlers
//!
//! This module provides REST API endpoints for SIP-related operations:
//! - Webhook hook management for SIP event forwarding
//! - SIP call transfer initiation
//!
//! # Endpoints
//!
//! - `GET /sip/hooks` - List configured SIP webhooks
//! - `POST /sip/hooks` - Add or update SIP webhooks
//! - `DELETE /sip/hooks` - Remove SIP webhooks
//! - `POST /sip/transfer` - Initiate a SIP call transfer

mod hooks;
mod transfer;

// Re-export handlers for clean API access
pub use hooks::{
    DeleteSipHooksRequest, SipHookEntry, SipHooksErrorResponse, SipHooksRequest, SipHooksResponse,
    delete_sip_hooks, list_sip_hooks, update_sip_hooks,
};
pub use transfer::{
    SIPTransferErrorResponse, SIPTransferRequest, SIPTransferResponse, sip_transfer,
};

// Re-export utoipa-generated path types for OpenAPI spec generation
#[cfg(feature = "openapi")]
pub use hooks::{__path_delete_sip_hooks, __path_list_sip_hooks, __path_update_sip_hooks};
#[cfg(feature = "openapi")]
pub use transfer::__path_sip_transfer;
