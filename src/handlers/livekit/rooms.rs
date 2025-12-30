//! LiveKit rooms listing handler
//!
//! This module provides REST API endpoints for listing LiveKit rooms
//! and retrieving detailed room information with participants,
//! filtered by the authenticated client's ID prefix for tenant isolation.

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use livekit_protocol::participant_info;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info, warn};

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

/// Detailed information about a participant in a LiveKit room
///
/// # Example
/// ```json
/// {
///   "sid": "PA_abc123",
///   "identity": "user-alice-456",
///   "name": "Alice Smith",
///   "state": "ACTIVE",
///   "kind": "STANDARD",
///   "joined_at": 1703123456,
///   "metadata": "{\"role\": \"host\"}",
///   "attributes": {}
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ParticipantInfo {
    /// Unique session ID for this participant (generated by LiveKit)
    #[cfg_attr(feature = "openapi", schema(example = "PA_abc123"))]
    pub sid: String,

    /// Unique identifier provided when connecting
    #[cfg_attr(feature = "openapi", schema(example = "user-alice-456"))]
    pub identity: String,

    /// Display name of the participant
    #[cfg_attr(feature = "openapi", schema(example = "Alice Smith"))]
    pub name: String,

    /// Participant state: JOINING, JOINED, ACTIVE, or DISCONNECTED
    #[cfg_attr(feature = "openapi", schema(example = "ACTIVE"))]
    pub state: String,

    /// Participant kind: STANDARD, AGENT, SIP, EGRESS, or INGRESS
    #[cfg_attr(feature = "openapi", schema(example = "STANDARD"))]
    pub kind: String,

    /// Timestamp when participant joined (Unix timestamp in seconds)
    #[cfg_attr(feature = "openapi", schema(example = 1703123456))]
    pub joined_at: i64,

    /// User-specified metadata for the participant
    #[cfg_attr(feature = "openapi", schema(example = json!({"role": "host"})))]
    pub metadata: String,

    /// User-specified attributes for the participant
    pub attributes: HashMap<String, String>,

    /// Whether the participant is currently publishing audio/video
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub is_publisher: bool,
}

/// Detailed information about a LiveKit room including participants
///
/// # Example
/// ```json
/// {
///   "sid": "RM_xyz789",
///   "name": "project1_conversation-room-123",
///   "num_participants": 2,
///   "max_participants": 10,
///   "creation_time": 1703123456,
///   "metadata": "",
///   "active_recording": false,
///   "participants": [...]
/// }
/// ```
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoomDetailsResponse {
    /// Unique session ID for the room (generated by LiveKit)
    #[cfg_attr(feature = "openapi", schema(example = "RM_xyz789"))]
    pub sid: String,

    /// The full room name (includes tenant prefix)
    #[cfg_attr(
        feature = "openapi",
        schema(example = "project1_conversation-room-123")
    )]
    pub name: String,

    /// Number of current participants in the room
    #[cfg_attr(feature = "openapi", schema(example = 2))]
    pub num_participants: u32,

    /// Maximum allowed participants (0 = no limit)
    #[cfg_attr(feature = "openapi", schema(example = 10))]
    pub max_participants: u32,

    /// Room creation time (Unix timestamp in seconds)
    #[cfg_attr(feature = "openapi", schema(example = 1703123456))]
    pub creation_time: i64,

    /// User-specified metadata for the room
    #[cfg_attr(feature = "openapi", schema(example = ""))]
    pub metadata: String,

    /// Whether a recording is currently active
    #[cfg_attr(feature = "openapi", schema(example = false))]
    pub active_recording: bool,

    /// List of participants currently in the room
    pub participants: Vec<ParticipantInfo>,
}

/// Convert participant state enum to string
fn participant_state_to_string(state: i32) -> String {
    match participant_info::State::try_from(state) {
        Ok(participant_info::State::Joining) => "JOINING".to_string(),
        Ok(participant_info::State::Joined) => "JOINED".to_string(),
        Ok(participant_info::State::Active) => "ACTIVE".to_string(),
        Ok(participant_info::State::Disconnected) => "DISCONNECTED".to_string(),
        Err(_) => "UNKNOWN".to_string(),
    }
}

/// Convert participant kind enum to string
fn participant_kind_to_string(kind: i32) -> String {
    match participant_info::Kind::try_from(kind) {
        Ok(participant_info::Kind::Standard) => "STANDARD".to_string(),
        Ok(participant_info::Kind::Ingress) => "INGRESS".to_string(),
        Ok(participant_info::Kind::Egress) => "EGRESS".to_string(),
        Ok(participant_info::Kind::Sip) => "SIP".to_string(),
        Ok(participant_info::Kind::Agent) => "AGENT".to_string(),
        Err(_) => "UNKNOWN".to_string(),
    }
}

/// Handler for GET /livekit/rooms/{room_name} endpoint
///
/// Returns detailed information about a specific LiveKit room including
/// all current participants. The room name is normalized with the auth.id
/// prefix for tenant isolation.
///
/// # Arguments
/// * `state` - Shared application state containing LiveKit configuration
/// * `auth` - Authentication context from middleware
/// * `room_name` - Name of the room to retrieve (from path parameter)
///
/// # Returns
/// * `Response` - JSON response with room details or error status
///
/// # Errors
/// * 404 Not Found - Room not found or not accessible
/// * 500 Internal Server Error - LiveKit service not configured or API call failed
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/livekit/rooms/{room_name}",
        params(
            ("room_name" = String, Path, description = "Name of the room to retrieve")
        ),
        responses(
            (status = 200, description = "Room details retrieved successfully", body = RoomDetailsResponse),
            (status = 404, description = "Room not found or not accessible"),
            (status = 500, description = "LiveKit service not configured or failed to get room details")
        ),
        security(
            ("bearer_auth" = [])
        ),
        tag = "livekit"
    )
)]
pub async fn get_room_details(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<Auth>,
    Path(room_name): Path<String>,
) -> Response {
    // Validate room name is not empty
    if room_name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Room name cannot be empty"
            })),
        )
            .into_response();
    }

    // Normalize room name with auth prefix for tenant isolation
    let normalized_room_name = auth.normalize_room_name(&room_name);

    info!(
        auth_id = ?auth.id,
        room = %room_name,
        normalized_room = %normalized_room_name,
        "Getting LiveKit room details"
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

    // Get room details with participants
    let (room, participants) = match room_handler.get_room_details(&normalized_room_name).await {
        Ok(result) => result,
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("not found") {
                warn!(
                    room = %normalized_room_name,
                    "Room not found"
                );
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": format!("Room '{}' not found", room_name)
                    })),
                )
                    .into_response();
            }
            error!("Failed to get room details: {:?}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": format!("Failed to get room details: {}", e)
                })),
            )
                .into_response();
        }
    };

    info!(
        auth_id = ?auth.id,
        room = %normalized_room_name,
        num_participants = participants.len(),
        "Successfully retrieved room details"
    );

    // Convert participants to response format
    let participant_infos: Vec<ParticipantInfo> = participants
        .into_iter()
        .map(|p| ParticipantInfo {
            sid: p.sid,
            identity: p.identity,
            name: p.name,
            state: participant_state_to_string(p.state),
            kind: participant_kind_to_string(p.kind),
            joined_at: p.joined_at,
            metadata: p.metadata,
            attributes: p.attributes,
            is_publisher: p.is_publisher,
        })
        .collect();

    let response = RoomDetailsResponse {
        sid: room.sid,
        name: room.name,
        num_participants: room.num_participants,
        max_participants: room.max_participants,
        creation_time: room.creation_time,
        metadata: room.metadata,
        active_recording: room.active_recording,
        participants: participant_infos,
    };

    (StatusCode::OK, Json(response)).into_response()
}
