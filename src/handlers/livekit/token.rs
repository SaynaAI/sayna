//! LiveKit token generation handler
//!
//! This module provides the REST API endpoint for LiveKit token generation,
//! allowing multiple participants to join the same LiveKit room.
//!
//! ## Authentication
//!
//! When authentication is enabled (`auth.id` is present), this endpoint:
//! - Creates the room if it doesn't exist
//! - Sets `room.metadata.auth_id` to the authenticated tenant's ID
//! - Rejects token generation if the room exists with a different tenant's `auth_id`
//!
//! This ensures room ownership is established before any token is issued.

use axum::{
    Extension,
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::auth::Auth;
use crate::livekit::LiveKitError;
use crate::state::AppState;

/// Request body for generating a LiveKit token
///
/// # Example
/// ```json
/// {
///   "room_name": "conversation-room-123",
///   "participant_name": "Alice Smith",
///   "participant_identity": "user-alice-456"
/// }
/// ```
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TokenRequest {
    /// The LiveKit room name to generate a token for
    #[cfg_attr(feature = "openapi", schema(example = "conversation-room-123"))]
    pub room_name: String,
    /// Display name for the participant (e.g., "John Doe")
    #[cfg_attr(feature = "openapi", schema(example = "Alice Smith"))]
    pub participant_name: String,
    /// Unique identifier for the participant (e.g., "user-123")
    #[cfg_attr(feature = "openapi", schema(example = "user-alice-456"))]
    pub participant_identity: String,
}

/// Response containing the generated LiveKit token
///
/// # Example
/// ```json
/// {
///   "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
///   "room_name": "conversation-room-123",
///   "participant_identity": "user-alice-456",
///   "livekit_url": "ws://localhost:7880"
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TokenResponse {
    /// The generated JWT token for LiveKit
    #[cfg_attr(
        feature = "openapi",
        schema(example = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...")
    )]
    pub token: String,
    /// Echo back the room name for client confirmation
    #[cfg_attr(feature = "openapi", schema(example = "conversation-room-123"))]
    pub room_name: String,
    /// Echo back the participant identity for client confirmation
    #[cfg_attr(feature = "openapi", schema(example = "user-alice-456"))]
    pub participant_identity: String,
    /// The LiveKit server URL to connect to
    #[cfg_attr(feature = "openapi", schema(example = "ws://localhost:7880"))]
    pub livekit_url: String,
}

/// Handler for POST /livekit/token endpoint
///
/// Generates a LiveKit JWT token for a participant to join a specific room.
///
/// When authentication is enabled (`auth.id` is present), this handler:
/// 1. Creates the room if it doesn't exist
/// 2. Sets `room.metadata.auth_id` to the authenticated tenant's ID
/// 3. Issues the token only after metadata is verified/set
///
/// # Arguments
/// * `state` - Shared application state containing LiveKit configuration
/// * `request` - Token request with room name and participant details
///
/// # Returns
/// * `Response` - JSON response with token or error status
///
/// # Errors
/// * 400 Bad Request - Invalid request data (empty fields)
/// * 403 Forbidden - Room exists with a different tenant's `auth_id`
/// * 500 Internal Server Error - LiveKit service not configured, room creation failed, or token generation failed
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/livekit/token",
        request_body = TokenRequest,
        responses(
            (status = 200, description = "Token generated successfully. Room is created if it doesn't exist and metadata.auth_id is set.", body = TokenResponse),
            (status = 400, description = "Invalid request (missing or empty fields)"),
            (status = 403, description = "Access denied: room exists with a different tenant's auth_id"),
            (status = 500, description = "LiveKit service not configured, room creation failed, or token generation failed")
        ),
        security(
            ("bearer_auth" = [])
        ),
        tag = "livekit"
    )
)]
pub async fn generate_token(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Auth>,
    Json(request): Json<TokenRequest>,
) -> Response {
    // Use room name as-is (no prefixing) - tenant isolation is via metadata
    let room_name = &request.room_name;

    info!(
        auth_id = ?auth.id,
        room = %room_name,
        participant = %request.participant_name,
        identity = %request.participant_identity,
        "Token generation request"
    );

    // Validate that LiveKit handler is configured
    let room_handler = match &state.livekit_room_handler {
        Some(handler) => handler,
        None => {
            error!("LiveKit service not configured");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "LiveKit service not configured"
                })),
            )
                .into_response();
        }
    };

    // Validate request data
    if request.room_name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Invalid request: room_name cannot be empty"
            })),
        )
            .into_response();
    }

    if request.participant_name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Invalid request: participant_name cannot be empty"
            })),
        )
            .into_response();
    }

    if request.participant_identity.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Invalid request: participant_identity cannot be empty"
            })),
        )
            .into_response();
    }

    // If auth.id exists (and is non-empty/non-whitespace), ensure room exists and has
    // correct auth_id in metadata. This creates the room if needed and associates it
    // with the tenant. Token issuance only proceeds after metadata is set.
    // Empty or whitespace-only auth.id is treated as unauthenticated mode.
    if let Some(auth_id) = auth.effective_id() {
        match room_handler
            .ensure_room_with_auth_id(room_name, auth_id)
            .await
        {
            Ok(()) => {
                info!(
                    room = %room_name,
                    auth_id = %auth_id,
                    "Room exists with correct auth_id"
                );
            }
            Err(LiveKitError::MetadataConflict {
                existing,
                attempted,
            }) => {
                warn!(
                    room = %room_name,
                    existing_auth_id = %existing,
                    attempted_auth_id = %attempted,
                    "Token generation denied: room belongs to different tenant"
                );
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "Access denied: room belongs to a different tenant"
                    })),
                )
                    .into_response();
            }
            Err(LiveKitError::RoomNotFound(_)) => {
                // This should not happen after create attempt - indicates a serious issue
                error!(
                    room = %room_name,
                    auth_id = %auth_id,
                    "Room could not be created or found after creation attempt"
                );
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "Failed to create room: room not found after creation attempt"
                    })),
                )
                    .into_response();
            }
            Err(e) => {
                error!(
                    room = %room_name,
                    error = %e,
                    "Failed to ensure room with auth_id"
                );
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": format!("Failed to prepare room: {}", e)
                    })),
                )
                    .into_response();
            }
        }
    }

    // Generate token using raw room name
    let token = match room_handler.user_token(
        room_name,
        &request.participant_identity,
        &request.participant_name,
    ) {
        Ok(t) => t,
        Err(e) => {
            error!("Failed to generate LiveKit token: {:?}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to generate LiveKit token: {}", e)
                })),
            )
                .into_response();
        }
    };

    info!(
        room = %room_name,
        identity = %request.participant_identity,
        "Successfully generated token"
    );

    // Build response with clean room name (no prefixing)
    let response = TokenResponse {
        token,
        room_name: room_name.clone(),
        participant_identity: request.participant_identity,
        livekit_url: state.config.livekit_public_url.clone(),
    };

    (StatusCode::OK, Json(response)).into_response()
}
