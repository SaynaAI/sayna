//! LiveKit participant management handler
//!
//! This module provides REST API endpoints for managing participants in LiveKit rooms,
//! with tenant isolation through auth.id room name prefixing.

use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::auth::Auth;
use crate::state::AppState;

/// Request body for removing a participant from a LiveKit room
///
/// # Example
/// ```json
/// {
///   "room_name": "conversation-room-123",
///   "participant_identity": "user-alice-456"
/// }
/// ```
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RemoveParticipantRequest {
    /// The LiveKit room name where the participant is connected
    #[cfg_attr(feature = "openapi", schema(example = "conversation-room-123"))]
    pub room_name: String,

    /// The identity of the participant to remove
    #[cfg_attr(feature = "openapi", schema(example = "user-alice-456"))]
    pub participant_identity: String,
}

/// Response for a successful participant removal
///
/// # Example
/// ```json
/// {
///   "status": "removed",
///   "room_name": "project1_conversation-room-123",
///   "participant_identity": "user-alice-456"
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RemoveParticipantResponse {
    /// Status of the removal operation
    #[cfg_attr(feature = "openapi", schema(example = "removed"))]
    pub status: String,

    /// The normalized room name (with tenant prefix)
    #[cfg_attr(
        feature = "openapi",
        schema(example = "project1_conversation-room-123")
    )]
    pub room_name: String,

    /// The identity of the removed participant
    #[cfg_attr(feature = "openapi", schema(example = "user-alice-456"))]
    pub participant_identity: String,
}

/// Error response for participant removal failures
///
/// # Example
/// ```json
/// {
///   "error": "Participant 'user-123' not found in room",
///   "code": "PARTICIPANT_NOT_FOUND"
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RemoveParticipantErrorResponse {
    /// Human-readable error message
    #[cfg_attr(feature = "openapi", schema(example = "Participant not found in room"))]
    pub error: String,

    /// Machine-readable error code
    #[cfg_attr(feature = "openapi", schema(example = "PARTICIPANT_NOT_FOUND"))]
    pub code: String,
}

/// Error codes for participant removal operations
#[derive(Debug)]
enum RemoveParticipantErrorCode {
    InvalidRequest,
    ParticipantNotFound,
    LiveKitNotConfigured,
    RemovalFailed,
}

impl RemoveParticipantErrorCode {
    fn as_str(&self) -> &'static str {
        match self {
            RemoveParticipantErrorCode::InvalidRequest => "INVALID_REQUEST",
            RemoveParticipantErrorCode::ParticipantNotFound => "PARTICIPANT_NOT_FOUND",
            RemoveParticipantErrorCode::LiveKitNotConfigured => "LIVEKIT_NOT_CONFIGURED",
            RemoveParticipantErrorCode::RemovalFailed => "REMOVAL_FAILED",
        }
    }
}

fn error_response(status: StatusCode, message: &str, code: RemoveParticipantErrorCode) -> Response {
    (
        status,
        Json(RemoveParticipantErrorResponse {
            error: message.to_string(),
            code: code.as_str().to_string(),
        }),
    )
        .into_response()
}

/// Handler for DELETE /livekit/participant endpoint
///
/// Removes a participant from a LiveKit room, forcibly disconnecting them.
/// The room name is normalized with the auth.id prefix for tenant isolation,
/// ensuring users can only remove participants from their own rooms.
///
/// Note: This does not invalidate the participant's token. To prevent rejoining,
/// use short-lived tokens and avoid issuing new tokens to removed participants.
///
/// # Arguments
/// * `state` - Shared application state containing LiveKit configuration
/// * `auth` - Authentication context from middleware
/// * `request` - Request with room name and participant identity
///
/// # Returns
/// * `Response` - JSON response with removal status or error
///
/// # Errors
/// * 400 Bad Request - Empty room name or participant identity
/// * 404 Not Found - Participant not found in the room
/// * 500 Internal Server Error - LiveKit not configured or removal failed
///
/// # Flow
/// 1. Validate request fields are not empty
/// 2. Normalize room name with auth.id prefix
/// 3. Verify participant exists in the room
/// 4. Remove the participant
/// 5. Return success or appropriate error
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/livekit/participant",
        request_body = RemoveParticipantRequest,
        responses(
            (status = 200, description = "Participant removed successfully", body = RemoveParticipantResponse),
            (status = 400, description = "Invalid request (empty fields)", body = RemoveParticipantErrorResponse),
            (status = 404, description = "Participant not found in room", body = RemoveParticipantErrorResponse),
            (status = 500, description = "LiveKit not configured or removal failed", body = RemoveParticipantErrorResponse)
        ),
        security(
            ("bearer_auth" = [])
        ),
        tag = "livekit"
    )
)]
pub async fn remove_participant(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Auth>,
    Json(request): Json<RemoveParticipantRequest>,
) -> Response {
    // Step 1: Validate request fields
    if request.room_name.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Room name cannot be empty",
            RemoveParticipantErrorCode::InvalidRequest,
        );
    }

    if request.participant_identity.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Participant identity cannot be empty",
            RemoveParticipantErrorCode::InvalidRequest,
        );
    }

    // Step 2: Normalize room name with auth prefix for tenant isolation
    let room_name = auth.normalize_room_name(&request.room_name);

    info!(
        auth_id = ?auth.id,
        room = %request.room_name,
        normalized_room = %room_name,
        participant = %request.participant_identity,
        "Remove participant request"
    );

    // Step 3: Check LiveKit room handler is configured
    let room_handler = match &state.livekit_room_handler {
        Some(handler) => handler,
        None => {
            error!("LiveKit service not configured");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "LiveKit service not configured",
                RemoveParticipantErrorCode::LiveKitNotConfigured,
            );
        }
    };

    // Step 4: Verify participant exists in the room
    let participants = match room_handler.list_participants(&room_name).await {
        Ok(p) => p,
        Err(e) => {
            // If we can't list participants, the room likely doesn't exist
            warn!(
                room = %room_name,
                error = %e,
                "Failed to list participants - room may not exist"
            );
            return error_response(
                StatusCode::NOT_FOUND,
                &format!("Room '{}' not found or not accessible", request.room_name),
                RemoveParticipantErrorCode::ParticipantNotFound,
            );
        }
    };

    let participant_exists = participants
        .iter()
        .any(|p| p.identity == request.participant_identity);

    if !participant_exists {
        warn!(
            room = %room_name,
            participant = %request.participant_identity,
            "Participant not found in room"
        );
        return error_response(
            StatusCode::NOT_FOUND,
            &format!(
                "Participant '{}' not found in room '{}'",
                request.participant_identity, request.room_name
            ),
            RemoveParticipantErrorCode::ParticipantNotFound,
        );
    }

    // Step 5: Remove the participant
    match room_handler
        .remove_participant(&room_name, &request.participant_identity)
        .await
    {
        Ok(()) => {
            info!(
                room = %room_name,
                participant = %request.participant_identity,
                "Participant removed successfully"
            );
            (
                StatusCode::OK,
                Json(RemoveParticipantResponse {
                    status: "removed".to_string(),
                    room_name,
                    participant_identity: request.participant_identity,
                }),
            )
                .into_response()
        }
        Err(e) => {
            error!(
                room = %room_name,
                participant = %request.participant_identity,
                error = %e,
                "Failed to remove participant"
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to remove participant: {}", e),
                RemoveParticipantErrorCode::RemovalFailed,
            )
        }
    }
}

/// Request body for muting/unmuting a participant's track
///
/// # Example
/// ```json
/// {
///   "room_name": "conversation-room-123",
///   "participant_identity": "user-alice-456",
///   "track_sid": "TR_abc123",
///   "muted": true
/// }
/// ```
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MuteParticipantRequest {
    /// The LiveKit room name where the participant is connected
    #[cfg_attr(feature = "openapi", schema(example = "conversation-room-123"))]
    pub room_name: String,

    /// The identity of the participant whose track to mute
    #[cfg_attr(feature = "openapi", schema(example = "user-alice-456"))]
    pub participant_identity: String,

    /// The session ID of the track to mute/unmute
    #[cfg_attr(feature = "openapi", schema(example = "TR_abc123"))]
    pub track_sid: String,

    /// True to mute, false to unmute
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub muted: bool,
}

/// Response for a successful mute/unmute operation
///
/// # Example
/// ```json
/// {
///   "room_name": "project1_conversation-room-123",
///   "participant_identity": "user-alice-456",
///   "track_sid": "TR_abc123",
///   "muted": true
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MuteParticipantResponse {
    /// The normalized room name (with tenant prefix)
    #[cfg_attr(
        feature = "openapi",
        schema(example = "project1_conversation-room-123")
    )]
    pub room_name: String,

    /// The identity of the participant
    #[cfg_attr(feature = "openapi", schema(example = "user-alice-456"))]
    pub participant_identity: String,

    /// The session ID of the track
    #[cfg_attr(feature = "openapi", schema(example = "TR_abc123"))]
    pub track_sid: String,

    /// Current muted state
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub muted: bool,
}

/// Handler for POST /livekit/participant/mute endpoint
///
/// Mutes or unmutes a participant's published track. The room name is
/// normalized with the auth.id prefix for tenant isolation.
///
/// # Arguments
/// * `state` - Shared application state containing LiveKit configuration
/// * `auth` - Authentication context from middleware
/// * `request` - Request with room name, participant identity, track_sid, and muted state
///
/// # Returns
/// * `Response` - JSON response with mute status or error
///
/// # Errors
/// * 400 Bad Request - Empty fields in request
/// * 404 Not Found - Room or participant not found
/// * 500 Internal Server Error - LiveKit not configured or mute operation failed
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/livekit/participant/mute",
        request_body = MuteParticipantRequest,
        responses(
            (status = 200, description = "Track muted/unmuted successfully", body = MuteParticipantResponse),
            (status = 400, description = "Invalid request (empty fields)", body = RemoveParticipantErrorResponse),
            (status = 404, description = "Room or participant not found", body = RemoveParticipantErrorResponse),
            (status = 500, description = "LiveKit not configured or mute failed", body = RemoveParticipantErrorResponse)
        ),
        security(
            ("bearer_auth" = [])
        ),
        tag = "livekit"
    )
)]
pub async fn mute_participant(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Auth>,
    Json(request): Json<MuteParticipantRequest>,
) -> Response {
    // Validate request fields
    if request.room_name.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Room name cannot be empty",
            RemoveParticipantErrorCode::InvalidRequest,
        );
    }

    if request.participant_identity.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Participant identity cannot be empty",
            RemoveParticipantErrorCode::InvalidRequest,
        );
    }

    if request.track_sid.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "Track SID cannot be empty",
            RemoveParticipantErrorCode::InvalidRequest,
        );
    }

    // Normalize room name with auth prefix for tenant isolation
    let room_name = auth.normalize_room_name(&request.room_name);

    info!(
        auth_id = ?auth.id,
        room = %request.room_name,
        normalized_room = %room_name,
        participant = %request.participant_identity,
        track_sid = %request.track_sid,
        muted = %request.muted,
        "Mute participant track request"
    );

    // Check LiveKit room handler is configured
    let room_handler = match &state.livekit_room_handler {
        Some(handler) => handler,
        None => {
            error!("LiveKit service not configured");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "LiveKit service not configured",
                RemoveParticipantErrorCode::LiveKitNotConfigured,
            );
        }
    };

    // Mute/unmute the track
    match room_handler
        .mute_participant_track(
            &room_name,
            &request.participant_identity,
            &request.track_sid,
            request.muted,
        )
        .await
    {
        Ok(track_info) => {
            info!(
                room = %room_name,
                participant = %request.participant_identity,
                track_sid = %request.track_sid,
                muted = %track_info.muted,
                "Track mute state updated"
            );
            (
                StatusCode::OK,
                Json(MuteParticipantResponse {
                    room_name,
                    participant_identity: request.participant_identity,
                    track_sid: request.track_sid,
                    muted: track_info.muted,
                }),
            )
                .into_response()
        }
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("not found") {
                warn!(
                    room = %room_name,
                    participant = %request.participant_identity,
                    track_sid = %request.track_sid,
                    "Track or participant not found"
                );
                return error_response(
                    StatusCode::NOT_FOUND,
                    &format!(
                        "Track '{}' or participant '{}' not found in room '{}'",
                        request.track_sid, request.participant_identity, request.room_name
                    ),
                    RemoveParticipantErrorCode::ParticipantNotFound,
                );
            }
            error!(
                room = %room_name,
                participant = %request.participant_identity,
                track_sid = %request.track_sid,
                error = %e,
                "Failed to mute track"
            );
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Failed to mute track: {}", e),
                RemoveParticipantErrorCode::RemovalFailed,
            )
        }
    }
}
