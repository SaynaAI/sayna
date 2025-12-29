//! LiveKit rooms listing handler
//!
//! This module provides the REST API endpoint for listing LiveKit rooms,
//! filtered by the authenticated client's ID prefix for tenant isolation.

use axum::{
    Extension, Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use std::sync::Arc;
use tracing::{error, info};

use crate::auth::Auth;
use crate::state::AppState;

/// Information about a LiveKit room
///
/// # Example
/// ```json
/// {
///   "name": "project1_conversation-room-123",
///   "num_participants": 2,
///   "creation_time": 1703123456
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoomInfo {
    /// The full room name (includes tenant prefix)
    #[cfg_attr(
        feature = "openapi",
        schema(example = "project1_conversation-room-123")
    )]
    pub name: String,
    /// Number of current participants in the room
    #[cfg_attr(feature = "openapi", schema(example = 2))]
    pub num_participants: u32,
    /// Room creation time (Unix timestamp in seconds)
    #[cfg_attr(feature = "openapi", schema(example = 1703123456))]
    pub creation_time: i64,
}

/// Response containing the list of LiveKit rooms
///
/// # Example
/// ```json
/// {
///   "rooms": [
///     {
///       "name": "project1_room-1",
///       "num_participants": 2,
///       "creation_time": 1703123456
///     },
///     {
///       "name": "project1_room-2",
///       "num_participants": 0,
///       "creation_time": 1703123789
///     }
///   ]
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ListRoomsResponse {
    /// List of rooms belonging to the authenticated client
    pub rooms: Vec<RoomInfo>,
}

/// Handler for GET /livekit/rooms endpoint
///
/// Lists all LiveKit rooms belonging to the authenticated client.
/// Rooms are filtered by the auth.id prefix for tenant isolation.
///
/// # Arguments
/// * `state` - Shared application state containing LiveKit configuration
/// * `auth` - Authentication context from middleware
///
/// # Returns
/// * `Response` - JSON response with rooms list or error status
///
/// # Errors
/// * 500 Internal Server Error - LiveKit service not configured or API call failed
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/livekit/rooms",
        responses(
            (status = 200, description = "Rooms listed successfully", body = ListRoomsResponse),
            (status = 500, description = "LiveKit service not configured or failed to list rooms")
        ),
        security(
            ("bearer_auth" = [])
        ),
        tag = "livekit"
    )
)]
pub async fn list_rooms(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Auth>,
) -> Response {
    // Build the prefix for filtering rooms by tenant
    let prefix = auth.id.as_ref().map(|id| format!("{}_", id));

    info!(
        auth_id = ?auth.id,
        prefix = ?prefix,
        "Listing LiveKit rooms"
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

    // List rooms with prefix filter
    let rooms = match room_handler.list_rooms(prefix.as_deref()).await {
        Ok(r) => r,
        Err(e) => {
            error!("Failed to list LiveKit rooms: {:?}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to list rooms: {}", e)
                })),
            )
                .into_response();
        }
    };

    info!(
        auth_id = ?auth.id,
        count = rooms.len(),
        "Successfully listed LiveKit rooms"
    );

    // Convert to response format
    let room_infos: Vec<RoomInfo> = rooms
        .into_iter()
        .map(|r| RoomInfo {
            name: r.name,
            num_participants: r.num_participants,
            creation_time: r.creation_time,
        })
        .collect();

    let response = ListRoomsResponse { rooms: room_infos };

    (StatusCode::OK, Json(response)).into_response()
}
