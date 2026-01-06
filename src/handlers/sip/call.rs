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

/// Request body for initiating an outbound SIP call
///
/// # Example
/// ```json
/// {
///   "room_name": "call-room-123",
///   "participant_name": "John Doe",
///   "participant_identity": "caller-456",
///   "from_phone_number": "+15105550123",
///   "to_phone_number": "+15551234567"
/// }
/// ```
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
/// # Arguments
/// * `state` - Shared application state containing LiveKit handlers and config
/// * `auth` - Authentication context for room metadata authorization
/// * `request` - Call request with phone numbers and room details
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
/// 3. Check SIP config exists and contains outbound_address
/// 4. Check LiveKit SIP handler is available
/// 5. Check room access via metadata auth_id (if room exists)
/// 6. Create outbound call (trunk is created/reused automatically)
/// 7. Ensure room metadata contains auth_id for tenant isolation
/// 8. Return success response with call identifiers
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/sip/call",
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

    // Step 4: Check SIP config exists and contains outbound_address
    // Also extract optional auth credentials for trunk creation
    let (outbound_address, auth_username, auth_password) = match &state.config.sip {
        Some(sip_config) => match &sip_config.outbound_address {
            Some(addr) => (
                addr.clone(),
                sip_config.outbound_auth_username.clone(),
                sip_config.outbound_auth_password.clone(),
            ),
            None => {
                warn!("SIP call failed: outbound_address not configured in SIP config");
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Outbound address not configured",
                    SIPCallErrorCode::OutboundAddressNotConfigured,
                );
            }
        },
        None => {
            warn!("SIP call failed: SIP config not present");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "SIP configuration not available",
                SIPCallErrorCode::OutboundAddressNotConfigured,
            );
        }
    };

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
}
