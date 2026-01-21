//! SIP outbound call handler
//!
//! This module provides REST API endpoint for initiating outbound SIP calls
//! through LiveKit. It creates or reuses an outbound trunk and places
//! the call to connect to a LiveKit room.

use axum::{
    Extension,
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::auth::Auth;
use crate::handlers::livekit::room_guard::{RoomAccessError, check_room_access};
use crate::livekit::LiveKitError;
use crate::state::AppState;
use crate::utils::validate_phone_number;

/// Per-request SIP configuration overrides
///
/// Allows overriding the global SIP configuration on a per-request basis.
/// When specified, these values take priority over the global server configuration.
///
/// # Priority
/// Request body values > Global config values
///
/// # Example
/// ```json
/// {
///   "outbound_address": "sip.example.com:5060",
///   "auth_username": "user123",
///   "auth_password": "secret456"
/// }
/// ```
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SIPCallSipConfig {
    /// SIP server address override for outbound calls.
    /// When provided, overrides the global `sip.outbound_address` config.
    /// Format: hostname or hostname:port (e.g., "sip.example.com" or "sip.example.com:5060")
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "sip.provider.com:5060"))]
    pub outbound_address: Option<String>,

    /// SIP authentication username override.
    /// When provided, overrides the global `sip.outbound_auth_username` config.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "sip_user_123"))]
    pub auth_username: Option<String>,

    /// SIP authentication password override.
    /// When provided, overrides the global `sip.outbound_auth_password` config.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "secure_password_456"))]
    pub auth_password: Option<String>,
}

/// Request body for initiating an outbound SIP call
///
/// # Example
/// ```json
/// {
///   "room_name": "call-room-123",
///   "participant_name": "John Doe",
///   "participant_identity": "caller-456",
///   "from_phone_number": "+15105550123",
///   "to_phone_number": "+15551234567",
///   "sip": {
///     "outbound_address": "sip.provider.com",
///     "auth_username": "user123",
///     "auth_password": "secret"
///   }
/// }
/// ```
///
/// # SIP Configuration Priority
///
/// The `sip` object is optional. When provided, its fields take priority over global
/// server configuration values. This allows per-request customization of SIP settings.
///
/// **Priority order**: Request body `sip` config > Global server config
///
/// - `outbound_address`: Overrides `sip.outbound_address` from config
/// - `auth_username`: Overrides `sip.outbound_auth_username` from config
/// - `auth_password`: Overrides `sip.outbound_auth_password` from config
///
/// If neither the request body nor global config provides an `outbound_address`,
/// the call will fail with `OUTBOUND_ADDRESS_NOT_CONFIGURED` error.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SIPCallRequest {
    /// The LiveKit room name to connect the call to
    #[cfg_attr(feature = "openapi", schema(example = "call-room-123"))]
    pub room_name: String,

    /// Display name for the SIP participant in the room
    #[cfg_attr(feature = "openapi", schema(example = "John Doe"))]
    pub participant_name: String,

    /// Identity for the SIP participant in the room
    #[cfg_attr(feature = "openapi", schema(example = "caller-456"))]
    pub participant_identity: String,

    /// Phone number the call will originate from.
    /// Must be configured in your SIP provider.
    /// Supports international format (+1234567890).
    #[cfg_attr(feature = "openapi", schema(example = "+15105550123"))]
    pub from_phone_number: String,

    /// Phone number to dial.
    /// Supports international format (+1234567890), national format (07123456789),
    /// or extensions (1234).
    #[cfg_attr(feature = "openapi", schema(example = "+15551234567"))]
    pub to_phone_number: String,

    /// Optional per-request SIP configuration overrides.
    /// When provided, these values take priority over the global server configuration.
    /// This allows using different SIP providers or credentials for specific calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sip: Option<SIPCallSipConfig>,
}

/// Response for a successful SIP call initiation
///
/// # Example
/// ```json
/// {
///   "status": "initiated",
///   "room_name": "call-room-123",
///   "participant_identity": "caller-456",
///   "participant_id": "PA_abc123",
///   "sip_call_id": "SC_xyz789"
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SIPCallResponse {
    /// Status of the call request ("initiated")
    #[cfg_attr(feature = "openapi", schema(example = "initiated"))]
    pub status: String,

    /// The room name where the call was connected
    #[cfg_attr(feature = "openapi", schema(example = "call-room-123"))]
    pub room_name: String,

    /// The identity of the SIP participant in the room
    #[cfg_attr(feature = "openapi", schema(example = "caller-456"))]
    pub participant_identity: String,

    /// The unique participant ID assigned by LiveKit
    #[cfg_attr(feature = "openapi", schema(example = "PA_abc123"))]
    pub participant_id: String,

    /// The unique SIP call ID for tracking
    #[cfg_attr(feature = "openapi", schema(example = "SC_xyz789"))]
    pub sip_call_id: String,
}

/// Error response for SIP call failures
///
/// # Example
/// ```json
/// {
///   "error": "Outbound address not configured",
///   "code": "OUTBOUND_ADDRESS_NOT_CONFIGURED"
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SIPCallErrorResponse {
    /// Human-readable error message
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Outbound address not configured")
    )]
    pub error: String,

    /// Machine-readable error code
    #[cfg_attr(
        feature = "openapi",
        schema(example = "OUTBOUND_ADDRESS_NOT_CONFIGURED")
    )]
    pub code: String,
}

/// Error codes for SIP call operations
#[derive(Debug)]
enum SIPCallErrorCode {
    InvalidPhoneNumber,
    LiveKitNotConfigured,
    OutboundAddressNotConfigured,
    CallFailed,
    RoomAccessDenied,
}

impl SIPCallErrorCode {
    fn as_str(&self) -> &'static str {
        match self {
            SIPCallErrorCode::InvalidPhoneNumber => "INVALID_PHONE_NUMBER",
            SIPCallErrorCode::LiveKitNotConfigured => "LIVEKIT_NOT_CONFIGURED",
            SIPCallErrorCode::OutboundAddressNotConfigured => "OUTBOUND_ADDRESS_NOT_CONFIGURED",
            SIPCallErrorCode::CallFailed => "CALL_FAILED",
            SIPCallErrorCode::RoomAccessDenied => "ROOM_ACCESS_DENIED",
        }
    }
}

fn error_response(status: StatusCode, message: &str, code: SIPCallErrorCode) -> Response {
    (
        status,
        Json(SIPCallErrorResponse {
            error: message.to_string(),
            code: code.as_str().to_string(),
        }),
    )
        .into_response()
}

/// Handler for POST /sip/call endpoint
///
/// Initiates an outbound SIP call through LiveKit.
/// The call is connected to the specified room as a SIP participant.
///
/// # SIP Configuration
///
/// SIP settings can be provided in the request body `sip` object or configured globally
/// in the server configuration. Request body values take priority over global config:
///
/// - `sip.outbound_address` > `config.sip.outbound_address`
/// - `sip.auth_username` > `config.sip.outbound_auth_username`
/// - `sip.auth_password` > `config.sip.outbound_auth_password`
///
/// This allows using different SIP providers or credentials for specific calls.
///
/// # Arguments
/// * `state` - Shared application state containing LiveKit handlers and config
/// * `auth` - Authentication context for room metadata authorization
/// * `request` - Call request with phone numbers, room details, and optional SIP config
///
/// # Returns
/// * `Response` - JSON response with call status or error
///
/// # Errors
/// * 400 Bad Request - Invalid phone number format or empty fields
/// * 403 Forbidden - Room exists with different auth_id (cross-tenant)
/// * 404 Not Found - Room exists with different auth_id (masked as 404)
/// * 500 Internal Server Error - LiveKit not configured, outbound address missing, or call failed
///
/// # Flow
/// 1. Validate non-empty room name and participant identifiers
/// 2. Validate phone number formats
/// 3. Resolve SIP config (request body overrides global config for outbound_address, auth)
/// 4. Verify outbound_address is configured (from request or global config)
/// 5. Check LiveKit SIP handler is available
/// 6. Check room access via metadata auth_id (if room exists)
/// 7. Create outbound call (trunk is created/reused automatically)
/// 8. Ensure room metadata contains auth_id for tenant isolation
/// 9. Return success response with call identifiers
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/sip/call",
        description = "Initiates an outbound SIP call through LiveKit. The optional `sip` object in the request body allows overriding global SIP configuration on a per-request basis. Request body values take priority over global config, enabling different SIP providers or credentials for specific calls.",
        request_body = SIPCallRequest,
        responses(
            (status = 200, description = "Call initiated successfully", body = SIPCallResponse),
            (status = 400, description = "Invalid request (bad phone number or empty fields)", body = SIPCallErrorResponse),
            (status = 404, description = "Room not found or not accessible", body = SIPCallErrorResponse),
            (status = 500, description = "LiveKit not configured, outbound address missing, or call failed", body = SIPCallErrorResponse)
        ),
        security(
            ("bearer_auth" = [])
        ),
        tag = "sip"
    )
)]
pub async fn sip_call(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Auth>,
    Json(request): Json<SIPCallRequest>,
) -> Response {
    // Use clean room name directly (no prefix)
    let room_name = &request.room_name;

    info!(
        room_name = %room_name,
        auth_id = ?auth.id,
        participant_identity = %request.participant_identity,
        to_phone_number = %request.to_phone_number,
        from_phone_number = %request.from_phone_number,
        "SIP call request received"
    );

    // Step 1: Validate room name
    if room_name.trim().is_empty() {
        warn!("SIP call validation failed: empty room_name");
        return error_response(
            StatusCode::BAD_REQUEST,
            "Room name cannot be empty",
            SIPCallErrorCode::InvalidPhoneNumber,
        );
    }

    // Step 2: Validate participant identifiers
    if request.participant_name.trim().is_empty() {
        warn!("SIP call validation failed: empty participant_name");
        return error_response(
            StatusCode::BAD_REQUEST,
            "Participant name cannot be empty",
            SIPCallErrorCode::InvalidPhoneNumber,
        );
    }

    if request.participant_identity.trim().is_empty() {
        warn!("SIP call validation failed: empty participant_identity");
        return error_response(
            StatusCode::BAD_REQUEST,
            "Participant identity cannot be empty",
            SIPCallErrorCode::InvalidPhoneNumber,
        );
    }

    // Step 3: Validate phone numbers
    let validated_to_phone = match validate_phone_number(&request.to_phone_number) {
        Ok(phone) => phone,
        Err(validation_error) => {
            warn!(
                to_phone_number = %request.to_phone_number,
                error = %validation_error,
                "SIP call validation failed: invalid to_phone_number"
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                &format!("Invalid to_phone_number: {}", validation_error),
                SIPCallErrorCode::InvalidPhoneNumber,
            );
        }
    };

    let validated_from_phone = match validate_phone_number(&request.from_phone_number) {
        Ok(phone) => phone,
        Err(validation_error) => {
            warn!(
                from_phone_number = %request.from_phone_number,
                error = %validation_error,
                "SIP call validation failed: invalid from_phone_number"
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                &format!("Invalid from_phone_number: {}", validation_error),
                SIPCallErrorCode::InvalidPhoneNumber,
            );
        }
    };

    // Step 4: Extract SIP config with priority: request body > global config
    // Also extract optional auth credentials for trunk creation
    let global_sip = state.config.sip.as_ref();
    let request_sip = request.sip.as_ref();

    // Resolve outbound_address: request.sip.outbound_address > global.sip.outbound_address
    let (outbound_address, outbound_source) = match request_sip
        .and_then(|s| s.outbound_address.as_ref())
    {
        Some(addr) => (addr.clone(), "request"),
        None => match global_sip.and_then(|s| s.outbound_address.as_ref()) {
            Some(addr) => (addr.clone(), "global"),
            None => {
                warn!(
                    "SIP call failed: outbound_address not configured in request or global config"
                );
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Outbound address not configured",
                    SIPCallErrorCode::OutboundAddressNotConfigured,
                );
            }
        },
    };

    // Resolve auth_username: request.sip.auth_username > global.sip.outbound_auth_username
    let auth_username = request_sip
        .and_then(|s| s.auth_username.clone())
        .or_else(|| global_sip.and_then(|s| s.outbound_auth_username.clone()));

    // Resolve auth_password: request.sip.auth_password > global.sip.outbound_auth_password
    let auth_password = request_sip
        .and_then(|s| s.auth_password.clone())
        .or_else(|| global_sip.and_then(|s| s.outbound_auth_password.clone()));

    // Step 5: Check LiveKit SIP handler is available
    let sip_handler = match &state.livekit_sip_handler {
        Some(handler) => handler.clone(),
        None => {
            warn!("SIP call failed: LiveKit SIP handler not configured");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "LiveKit SIP service not configured",
                SIPCallErrorCode::LiveKitNotConfigured,
            );
        }
    };

    // Step 6: Check LiveKit room handler is available for auth_id management
    let room_handler = match &state.livekit_room_handler {
        Some(handler) => handler.clone(),
        None => {
            warn!("SIP call failed: LiveKit room handler not configured");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "LiveKit room service not configured",
                SIPCallErrorCode::LiveKitNotConfigured,
            );
        }
    };

    // Step 7: Check room access via metadata auth_id (if room exists)
    // For outbound calls, the room might not exist yet. We handle two cases:
    // - Room exists: verify auth_id matches, deny if mismatch
    // - Room doesn't exist: allow (room will be created with call)
    // Note: Empty/whitespace auth.id is treated as unauthenticated mode (skip check)
    if auth.effective_id().is_some() {
        match check_room_access(&auth, room_name, &room_handler).await {
            Ok(()) => {
                // Room exists and access granted - proceed
            }
            Err(RoomAccessError::NotFound(_)) => {
                // Room doesn't exist - that's fine for outbound calls
                // Room will be created when we place the call
            }
            Err(RoomAccessError::AccessDenied { room_name: r, .. }) => {
                warn!(
                    room_name = %r,
                    auth_id = ?auth.id,
                    "SIP call denied: room belongs to different tenant"
                );
                // Return 404 to avoid leaking room existence
                return error_response(
                    StatusCode::NOT_FOUND,
                    "Room not found or not accessible",
                    SIPCallErrorCode::RoomAccessDenied,
                );
            }
            Err(RoomAccessError::LiveKitError(msg)) => {
                error!(
                    room_name = %room_name,
                    error = %msg,
                    "SIP call failed: LiveKit error during room access check"
                );
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("Failed to verify room access: {}", msg),
                    SIPCallErrorCode::CallFailed,
                );
            }
        }
    }

    // Step 8: Create outbound call
    // The create_outbound_call method handles trunk creation/reuse internally
    // Log auth presence without exposing credential values (security best practice)
    info!(
        room_name = %room_name,
        to_phone_number = %validated_to_phone,
        from_phone_number = %validated_from_phone,
        participant_identity = %request.participant_identity,
        outbound_address = %outbound_address,
        outbound_address_source = %outbound_source,
        auth_configured = auth_username.is_some(),
        "Initiating outbound SIP call"
    );

    match sip_handler
        .create_outbound_call(
            room_name,
            &validated_to_phone,
            &validated_from_phone,
            &outbound_address,
            &request.participant_identity,
            &request.participant_name,
            auth_username,
            auth_password,
        )
        .await
    {
        Ok(result) => {
            // Step 9: Ensure room metadata contains auth_id for tenant isolation
            // The call was successful - now associate the room with the tenant
            // Note: Only write auth_id when it's present and non-empty (effective_id returns Some)
            if let Some(auth_id) = auth.effective_id() {
                if let Err(e) = room_handler
                    .ensure_room_auth_id(&result.room_name, auth_id)
                    .await
                {
                    // Log the error but don't fail the call - the call was already placed
                    match &e {
                        LiveKitError::MetadataConflict {
                            existing,
                            attempted,
                        } => {
                            // This shouldn't happen if check_room_access passed, but handle defensively
                            error!(
                                room_name = %result.room_name,
                                existing_auth_id = %existing,
                                attempted_auth_id = %attempted,
                                "Room metadata conflict after call placement - this indicates a race condition"
                            );
                        }
                        _ => {
                            warn!(
                                room_name = %result.room_name,
                                auth_id = %auth_id,
                                error = %e,
                                "Failed to set room auth_id metadata after call placement"
                            );
                        }
                    }
                }
            } else {
                debug!(
                    room_name = %result.room_name,
                    raw_auth_id = ?auth.id,
                    "Skipping room metadata enforcement - unauthenticated mode (no effective auth_id)"
                );
            }

            info!(
                sip_call_id = %result.sip_call_id,
                participant_id = %result.participant_id,
                participant_identity = %result.participant_identity,
                room_name = %result.room_name,
                "Outbound SIP call initiated successfully"
            );
            (
                StatusCode::OK,
                Json(SIPCallResponse {
                    status: "initiated".to_string(),
                    room_name: result.room_name,
                    participant_identity: result.participant_identity,
                    participant_id: result.participant_id,
                    sip_call_id: result.sip_call_id,
                }),
            )
                .into_response()
        }
        Err(e) => {
            error!(
                room_name = %room_name,
                to_phone_number = %validated_to_phone,
                from_phone_number = %validated_from_phone,
                error = %e,
                "Outbound SIP call failed"
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("SIP call failed: {}", e),
                SIPCallErrorCode::CallFailed,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sip_call_request_deserialization() {
        let json = r#"{
            "room_name": "test-room",
            "participant_name": "Test User",
            "participant_identity": "test-identity",
            "from_phone_number": "+15105550123",
            "to_phone_number": "+15551234567"
        }"#;

        let request: SIPCallRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.room_name, "test-room");
        assert_eq!(request.participant_name, "Test User");
        assert_eq!(request.participant_identity, "test-identity");
        assert_eq!(request.from_phone_number, "+15105550123");
        assert_eq!(request.to_phone_number, "+15551234567");
        assert!(request.sip.is_none());
    }

    #[test]
    fn test_sip_call_request_with_sip_config() {
        let json = r#"{
            "room_name": "test-room",
            "participant_name": "Test User",
            "participant_identity": "test-identity",
            "from_phone_number": "+15105550123",
            "to_phone_number": "+15551234567",
            "sip": {
                "outbound_address": "sip.example.com:5060",
                "auth_username": "user123",
                "auth_password": "pass456"
            }
        }"#;

        let request: SIPCallRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.room_name, "test-room");
        assert!(request.sip.is_some());

        let sip_config = request.sip.unwrap();
        assert_eq!(
            sip_config.outbound_address,
            Some("sip.example.com:5060".to_string())
        );
        assert_eq!(sip_config.auth_username, Some("user123".to_string()));
        assert_eq!(sip_config.auth_password, Some("pass456".to_string()));
    }

    #[test]
    fn test_sip_call_request_with_partial_sip_config() {
        let json = r#"{
            "room_name": "test-room",
            "participant_name": "Test User",
            "participant_identity": "test-identity",
            "from_phone_number": "+15105550123",
            "to_phone_number": "+15551234567",
            "sip": {
                "outbound_address": "sip.example.com"
            }
        }"#;

        let request: SIPCallRequest = serde_json::from_str(json).unwrap();
        assert!(request.sip.is_some());

        let sip_config = request.sip.unwrap();
        assert_eq!(
            sip_config.outbound_address,
            Some("sip.example.com".to_string())
        );
        assert!(sip_config.auth_username.is_none());
        assert!(sip_config.auth_password.is_none());
    }

    #[test]
    fn test_sip_call_request_with_empty_sip_config() {
        let json = r#"{
            "room_name": "test-room",
            "participant_name": "Test User",
            "participant_identity": "test-identity",
            "from_phone_number": "+15105550123",
            "to_phone_number": "+15551234567",
            "sip": {}
        }"#;

        let request: SIPCallRequest = serde_json::from_str(json).unwrap();
        assert!(request.sip.is_some());

        let sip_config = request.sip.unwrap();
        assert!(sip_config.outbound_address.is_none());
        assert!(sip_config.auth_username.is_none());
        assert!(sip_config.auth_password.is_none());
    }

    #[test]
    fn test_sip_call_response_serialization() {
        let response = SIPCallResponse {
            status: "initiated".to_string(),
            room_name: "test-room".to_string(),
            participant_identity: "test-identity".to_string(),
            participant_id: "PA_abc123".to_string(),
            sip_call_id: "SC_xyz789".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"status\":\"initiated\""));
        assert!(json.contains("\"room_name\":\"test-room\""));
        assert!(json.contains("\"participant_identity\":\"test-identity\""));
        assert!(json.contains("\"participant_id\":\"PA_abc123\""));
        assert!(json.contains("\"sip_call_id\":\"SC_xyz789\""));
    }

    #[test]
    fn test_sip_call_error_response_serialization() {
        let response = SIPCallErrorResponse {
            error: "Test error".to_string(),
            code: "TEST_CODE".to_string(),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"error\":\"Test error\""));
        assert!(json.contains("\"code\":\"TEST_CODE\""));
    }

    #[test]
    fn test_sip_call_error_codes() {
        assert_eq!(
            SIPCallErrorCode::InvalidPhoneNumber.as_str(),
            "INVALID_PHONE_NUMBER"
        );
        assert_eq!(
            SIPCallErrorCode::LiveKitNotConfigured.as_str(),
            "LIVEKIT_NOT_CONFIGURED"
        );
        assert_eq!(
            SIPCallErrorCode::OutboundAddressNotConfigured.as_str(),
            "OUTBOUND_ADDRESS_NOT_CONFIGURED"
        );
        assert_eq!(SIPCallErrorCode::CallFailed.as_str(), "CALL_FAILED");
    }

    #[test]
    fn test_sip_call_request_minimal() {
        // Test that all fields are required
        let json = r#"{
            "room_name": "r",
            "participant_name": "n",
            "participant_identity": "i",
            "from_phone_number": "+1",
            "to_phone_number": "+2"
        }"#;

        let result: Result<SIPCallRequest, _> = serde_json::from_str(json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_sip_call_request_missing_field() {
        // Missing to_phone_number
        let json = r#"{
            "room_name": "test-room",
            "participant_name": "Test User",
            "participant_identity": "test-identity",
            "from_phone_number": "+15105550123"
        }"#;

        let result: Result<SIPCallRequest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    // =========================================================================
    // SIPCallSipConfig deserialization tests
    // =========================================================================

    #[test]
    fn test_sip_config_deserialization_all_fields() {
        let json = r#"{
            "outbound_address": "sip.provider.com:5060",
            "auth_username": "user123",
            "auth_password": "pass456"
        }"#;

        let config: SIPCallSipConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.outbound_address,
            Some("sip.provider.com:5060".to_string())
        );
        assert_eq!(config.auth_username, Some("user123".to_string()));
        assert_eq!(config.auth_password, Some("pass456".to_string()));
    }

    #[test]
    fn test_sip_config_deserialization_partial_outbound_only() {
        let json = r#"{
            "outbound_address": "sip.example.com"
        }"#;

        let config: SIPCallSipConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.outbound_address, Some("sip.example.com".to_string()));
        assert!(config.auth_username.is_none());
        assert!(config.auth_password.is_none());
    }

    #[test]
    fn test_sip_config_deserialization_partial_auth_only() {
        let json = r#"{
            "auth_username": "user123",
            "auth_password": "pass456"
        }"#;

        let config: SIPCallSipConfig = serde_json::from_str(json).unwrap();
        assert!(config.outbound_address.is_none());
        assert_eq!(config.auth_username, Some("user123".to_string()));
        assert_eq!(config.auth_password, Some("pass456".to_string()));
    }

    #[test]
    fn test_sip_config_deserialization_empty_object() {
        // Empty sip object is valid - all fields are optional
        let json = r#"{}"#;

        let config: SIPCallSipConfig = serde_json::from_str(json).unwrap();
        assert!(config.outbound_address.is_none());
        assert!(config.auth_username.is_none());
        assert!(config.auth_password.is_none());
    }

    #[test]
    fn test_sip_config_deserialization_username_only() {
        // Only username, no password - valid for some SIP providers
        let json = r#"{
            "auth_username": "user_only"
        }"#;

        let config: SIPCallSipConfig = serde_json::from_str(json).unwrap();
        assert!(config.outbound_address.is_none());
        assert_eq!(config.auth_username, Some("user_only".to_string()));
        assert!(config.auth_password.is_none());
    }

    // =========================================================================
    // SIPCallRequest backward compatibility tests
    // =========================================================================

    #[test]
    fn test_sip_call_request_backward_compatibility_no_sip_field() {
        // Verify that requests without the sip field continue to work
        // This ensures backward compatibility with existing API clients
        let json = r#"{
            "room_name": "legacy-room",
            "participant_name": "Legacy User",
            "participant_identity": "legacy-id",
            "from_phone_number": "+15105550123",
            "to_phone_number": "+15551234567"
        }"#;

        let request: SIPCallRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.room_name, "legacy-room");
        assert!(
            request.sip.is_none(),
            "sip field should be None for backward compatibility"
        );
    }

    // =========================================================================
    // Config priority behavior documentation tests
    // =========================================================================

    /// Tests demonstrating the expected config priority behavior:
    ///
    /// # Priority Order
    /// 1. Request body `sip` config values (highest priority)
    /// 2. Global server config values (fallback)
    ///
    /// # Resolution Logic
    /// For each SIP config field (outbound_address, auth_username, auth_password):
    /// - If request.sip.{field} is Some, use that value
    /// - Else if global_config.sip.{field} is Some, use that value
    /// - Else the field is not configured (may result in error for required fields)
    #[test]
    fn test_config_priority_documentation() {
        // This test documents the expected priority behavior through assertions
        // The actual priority logic is in the sip_call handler

        // Scenario 1: Request provides all SIP config - should use request values
        let request_with_full_sip = r#"{
            "room_name": "test-room",
            "participant_name": "Test User",
            "participant_identity": "test-id",
            "from_phone_number": "+15105550123",
            "to_phone_number": "+15551234567",
            "sip": {
                "outbound_address": "request.sip.com:5060",
                "auth_username": "request_user",
                "auth_password": "request_pass"
            }
        }"#;
        let req: SIPCallRequest = serde_json::from_str(request_with_full_sip).unwrap();
        let sip = req.sip.unwrap();
        assert_eq!(
            sip.outbound_address,
            Some("request.sip.com:5060".to_string())
        );
        assert_eq!(sip.auth_username, Some("request_user".to_string()));
        assert_eq!(sip.auth_password, Some("request_pass".to_string()));

        // Scenario 2: Request provides partial SIP config - should use request for provided,
        // and fall back to global for missing (tested at runtime in handler)
        let request_with_partial_sip = r#"{
            "room_name": "test-room",
            "participant_name": "Test User",
            "participant_identity": "test-id",
            "from_phone_number": "+15105550123",
            "to_phone_number": "+15551234567",
            "sip": {
                "outbound_address": "request.sip.com:5060"
            }
        }"#;
        let req: SIPCallRequest = serde_json::from_str(request_with_partial_sip).unwrap();
        let sip = req.sip.unwrap();
        assert_eq!(
            sip.outbound_address,
            Some("request.sip.com:5060".to_string())
        );
        assert!(
            sip.auth_username.is_none(),
            "auth_username should fall back to global config at runtime"
        );
        assert!(
            sip.auth_password.is_none(),
            "auth_password should fall back to global config at runtime"
        );

        // Scenario 3: No SIP config in request - should use global config (tested at runtime)
        let request_without_sip = r#"{
            "room_name": "test-room",
            "participant_name": "Test User",
            "participant_identity": "test-id",
            "from_phone_number": "+15105550123",
            "to_phone_number": "+15551234567"
        }"#;
        let req: SIPCallRequest = serde_json::from_str(request_without_sip).unwrap();
        assert!(
            req.sip.is_none(),
            "Without sip field, all config should come from global at runtime"
        );
    }
}
