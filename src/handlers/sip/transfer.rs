//! SIP call transfer handler
//!
//! This module provides REST API endpoint for initiating SIP call transfers
//! through LiveKit. It mirrors the functionality available through WebSocket
//! commands but exposes it as a standard REST API.

use axum::{
    Extension,
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use livekit_protocol::participant_info;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::auth::Auth;
use crate::livekit::LiveKitError;
use crate::state::AppState;
use crate::utils::validate_phone_number;

/// Request body for initiating a SIP call transfer
///
/// # Example
/// ```json
/// {
///   "room_name": "call-room-123",
///   "participant_identity": "sip_participant_456",
///   "transfer_to": "+15551234567"
/// }
/// ```
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SIPTransferRequest {
    /// The LiveKit room name where the SIP participant is connected
    #[cfg_attr(feature = "openapi", schema(example = "call-room-123"))]
    pub room_name: String,

    /// The identity of the SIP participant to transfer.
    /// This can be obtained by listing participants in the room via LiveKit API.
    #[cfg_attr(feature = "openapi", schema(example = "sip_participant_456"))]
    pub participant_identity: String,

    /// The phone number to transfer the call to.
    /// Supports international format (+1234567890), national format (07123456789),
    /// or internal extensions (1234).
    #[cfg_attr(feature = "openapi", schema(example = "+15551234567"))]
    pub transfer_to: String,
}

/// Response for a successful SIP transfer initiation
///
/// Note: A successful response indicates the transfer has been initiated,
/// not that it has completed. The actual transfer may take several seconds.
///
/// # Example
/// ```json
/// {
///   "status": "initiated",
///   "room_name": "call-room-123",
///   "participant_identity": "sip_participant_456",
///   "transfer_to": "tel:+15551234567"
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SIPTransferResponse {
    /// Status of the transfer request ("initiated" or "completed")
    #[cfg_attr(feature = "openapi", schema(example = "initiated"))]
    pub status: String,

    /// The room name where the transfer was initiated
    #[cfg_attr(feature = "openapi", schema(example = "call-room-123"))]
    pub room_name: String,

    /// The identity of the participant being transferred
    #[cfg_attr(feature = "openapi", schema(example = "sip_participant_456"))]
    pub participant_identity: String,

    /// The normalized phone number with tel: prefix
    #[cfg_attr(feature = "openapi", schema(example = "tel:+15551234567"))]
    pub transfer_to: String,
}

/// Error response for SIP transfer failures
///
/// # Example
/// ```json
/// {
///   "error": "Participant 'sip_123' not found or is not a SIP participant",
///   "code": "PARTICIPANT_NOT_FOUND"
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SIPTransferErrorResponse {
    /// Human-readable error message
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Participant not found or is not a SIP participant")
    )]
    pub error: String,

    /// Machine-readable error code
    #[cfg_attr(feature = "openapi", schema(example = "PARTICIPANT_NOT_FOUND"))]
    pub code: String,
}

/// Error codes for SIP transfer operations
#[derive(Debug)]
enum SIPTransferErrorCode {
    InvalidPhoneNumber,
    ParticipantNotFound,
    LiveKitNotConfigured,
    TransferFailed,
}

impl SIPTransferErrorCode {
    fn as_str(&self) -> &'static str {
        match self {
            SIPTransferErrorCode::InvalidPhoneNumber => "INVALID_PHONE_NUMBER",
            SIPTransferErrorCode::ParticipantNotFound => "PARTICIPANT_NOT_FOUND",
            SIPTransferErrorCode::LiveKitNotConfigured => "LIVEKIT_NOT_CONFIGURED",
            SIPTransferErrorCode::TransferFailed => "TRANSFER_FAILED",
        }
    }
}

fn error_response(status: StatusCode, message: &str, code: SIPTransferErrorCode) -> Response {
    (
        status,
        Json(SIPTransferErrorResponse {
            error: message.to_string(),
            code: code.as_str().to_string(),
        }),
    )
        .into_response()
}

/// Handler for POST /sip/transfer endpoint
///
/// Initiates a SIP REFER transfer for a participant in a LiveKit room.
/// The transfer moves an ongoing SIP call to a different phone number.
///
/// # Arguments
/// * `state` - Shared application state containing LiveKit handlers
/// * `request` - Transfer request with room name, participant identity, and destination
///
/// # Returns
/// * `Response` - JSON response with transfer status or error
///
/// # Errors
/// * 400 Bad Request - Invalid phone number format or empty fields
/// * 404 Not Found - Specified participant not found or not a SIP participant
/// * 500 Internal Server Error - LiveKit not configured or transfer operation failed
///
/// # Flow
/// 1. Validate the phone number format
/// 2. Check LiveKit handlers are configured
/// 3. Verify the participant exists and is a SIP participant
/// 4. Execute the SIP transfer
/// 5. Return success or appropriate error
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/sip/transfer",
        request_body = SIPTransferRequest,
        responses(
            (status = 200, description = "Transfer initiated successfully", body = SIPTransferResponse),
            (status = 400, description = "Invalid request (bad phone number or empty fields)", body = SIPTransferErrorResponse),
            (status = 404, description = "Participant not found or not a SIP participant", body = SIPTransferErrorResponse),
            (status = 500, description = "LiveKit not configured or transfer failed", body = SIPTransferErrorResponse)
        ),
        security(
            ("bearer_auth" = [])
        ),
        tag = "sip"
    )
)]
pub async fn sip_transfer(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Auth>,
    Json(request): Json<SIPTransferRequest>,
) -> Response {
    // Normalize room name with auth prefix for tenant isolation
    let room_name = auth.normalize_room_name(&request.room_name);

    info!(
        "SIP transfer request - room: {} (normalized: {}), participant: {:?}, transfer_to: {}",
        request.room_name, room_name, request.participant_identity, request.transfer_to
    );

    // Step 1: Validate phone number format
    let validated_phone = match validate_phone_number(&request.transfer_to) {
        Ok(phone) => phone,
        Err(validation_error) => {
            warn!("SIP transfer validation failed: {}", validation_error);
            return error_response(
                StatusCode::BAD_REQUEST,
                &format!("Invalid phone number: {}", validation_error),
                SIPTransferErrorCode::InvalidPhoneNumber,
            );
        }
    };

    // Step 2: Validate room name
    if request.room_name.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Room name cannot be empty",
            SIPTransferErrorCode::InvalidPhoneNumber,
        );
    }

    // Step 3: Check SIP handler is available
    let sip_handler = match &state.livekit_sip_handler {
        Some(handler) => handler.clone(),
        None => {
            warn!("SIP transfer failed: SIP handler not configured");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "LiveKit SIP service not configured",
                SIPTransferErrorCode::LiveKitNotConfigured,
            );
        }
    };

    // Step 4: Check room handler is available for listing participants
    let room_handler = match &state.livekit_room_handler {
        Some(handler) => handler.clone(),
        None => {
            warn!("SIP transfer failed: Room handler not configured");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "LiveKit room service not configured",
                SIPTransferErrorCode::LiveKitNotConfigured,
            );
        }
    };

    // Step 5: Validate participant identity
    if request.participant_identity.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Participant identity cannot be empty",
            SIPTransferErrorCode::ParticipantNotFound,
        );
    }

    // Verify the participant exists and is a SIP participant
    let participant_identity = match room_handler.list_participants(&room_name).await {
        Ok(participants) => {
            let found = participants.iter().any(|p| {
                p.identity == request.participant_identity
                    && participant_info::Kind::try_from(p.kind)
                        .ok()
                        .is_some_and(|k| k == participant_info::Kind::Sip)
            });
            if !found {
                warn!(
                    "SIP transfer failed: participant '{}' not found or not a SIP participant in room '{}'",
                    request.participant_identity, room_name
                );
                return error_response(
                    StatusCode::NOT_FOUND,
                    &format!(
                        "Participant '{}' not found or is not a SIP participant",
                        request.participant_identity
                    ),
                    SIPTransferErrorCode::ParticipantNotFound,
                );
            }
            request.participant_identity.clone()
        }
        Err(e) => {
            error!("Failed to list participants: {}", e);
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to verify participant: {}", e),
                SIPTransferErrorCode::TransferFailed,
            );
        }
    };

    // Step 6: Execute the SIP transfer
    info!(
        "Initiating SIP transfer: room={}, participant={}, transfer_to={}",
        room_name, participant_identity, validated_phone
    );

    match sip_handler
        .transfer_call(&participant_identity, &room_name, &validated_phone)
        .await
    {
        Ok(()) => {
            info!(
                "SIP transfer completed: room={}, participant={}, transfer_to={}",
                room_name, participant_identity, validated_phone
            );
            (
                StatusCode::OK,
                Json(SIPTransferResponse {
                    status: "completed".to_string(),
                    room_name: room_name.clone(),
                    participant_identity,
                    transfer_to: validated_phone,
                }),
            )
                .into_response()
        }
        Err(LiveKitError::SIPTransferRequestTimeout) => {
            // Timeout means the request was sent but we didn't get a response within 2 seconds.
            // Real errors (permission denied, not found, etc.) respond quickly.
            // Timeout likely means the transfer was initiated and the room was cleaned up.
            info!(
                "SIP transfer initiated (timeout, transfer likely succeeded): room={}, participant={}, transfer_to={}",
                room_name, participant_identity, validated_phone
            );
            (
                StatusCode::OK,
                Json(SIPTransferResponse {
                    status: "initiated".to_string(),
                    room_name,
                    participant_identity,
                    transfer_to: validated_phone,
                }),
            )
                .into_response()
        }
        Err(e) => {
            error!(
                "SIP transfer failed: room={}, participant={}, transfer_to={}, error={}",
                room_name, participant_identity, validated_phone, e
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("SIP transfer failed: {}", e),
                SIPTransferErrorCode::TransferFailed,
            )
        }
    }
}
