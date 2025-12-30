//! LiveKit token generation handler
//!
//! This module provides the REST API endpoint for LiveKit token generation,
//! allowing multiple participants to join the same LiveKit room.

use axum::{
    Extension,
    extract::{Json, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};

use crate::auth::Auth;
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
/// # Arguments
/// * `state` - Shared application state containing LiveKit configuration
/// * `request` - Token request with room name and participant details
///
/// # Returns
/// * `Response` - JSON response with token or error status
///
/// # Errors
/// * 400 Bad Request - Invalid request data (empty fields)
/// * 500 Internal Server Error - LiveKit service not configured or token generation failed
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/livekit/token",
        request_body = TokenRequest,
        responses(
            (status = 200, description = "Token generated successfully", body = TokenResponse),
            (status = 400, description = "Invalid request (missing or empty fields)"),
            (status = 500, description = "LiveKit service not configured or token generation failed")
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
    // Normalize room name with auth prefix for tenant isolation
    let room_name = auth.normalize_room_name(&request.room_name);

    info!(
        "Token generation request - room: {} (normalized: {}), participant: {} ({})",
        request.room_name, room_name, request.participant_name, request.participant_identity
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

    // Generate token using normalized room name and custom participant identity
    let token = match room_handler.user_token(
        &room_name,
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

    info!("Successfully generated token for room: {}", room_name);

    // Build response with normalized room name so clients know the actual room
    let response = TokenResponse {
        token,
        room_name,
        participant_identity: request.participant_identity,
        livekit_url: state.config.livekit_public_url.clone(),
    };

    (StatusCode::OK, Json(response)).into_response()
}
