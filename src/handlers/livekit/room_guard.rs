//! # Room Access Guard for LiveKit REST Endpoints
//!
//! This module provides metadata-based authorization for LiveKit room operations,
//! replacing the previous approach of prefixing room names with `auth.id`.
//!
//! ## Authorization Policy
//!
//! Access control is enforced via `room.metadata.auth_id`:
//!
//! | auth.id | Room exists | Room has auth_id | auth_id matches | Decision |
//! |---------|-------------|------------------|-----------------|----------|
//! | None    | Any         | Any              | N/A             | **Allow** (backward-compatible) |
//! | Some(X) | No          | N/A              | N/A             | Depends on operation |
//! | Some(X) | Yes         | None             | N/A             | **Deny** (prevent orphan access) |
//! | Some(X) | Yes         | Some(X)          | Yes             | **Allow** |
//! | Some(X) | Yes         | Some(Y)          | No              | **Deny** (cross-tenant) |
//!
//! ### Rationale for denying access to rooms without auth_id:
//!
//! When authentication is enabled (`auth.id` is `Some`), rooms without an `auth_id`
//! in their metadata are considered "orphan" rooms that were either:
//! - Created before authentication was enabled
//! - Created through some other mechanism bypassing the REST API
//!
//! Denying access prevents cross-tenant data leakage. If backward compatibility
//! with legacy rooms is needed, operators should update room metadata to include
//! the appropriate `auth_id`.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use crate::handlers::livekit::room_guard::{check_room_access, RoomAccessError};
//!
//! // In a handler:
//! let room_handler = state.livekit_room_handler.as_ref().unwrap();
//! check_room_access(&auth, "my-room", room_handler).await?;
//! ```

use axum::{Json, http::StatusCode, response::IntoResponse};
use tracing::warn;

use crate::auth::Auth;
use crate::livekit::metadata::get_auth_id;
use crate::livekit::room_handler::LiveKitRoomHandler;

/// Result of a room access check.
#[derive(Debug)]
pub enum RoomAccessError {
    /// Room was not found.
    NotFound(String),
    /// Access denied due to auth_id mismatch or missing auth_id.
    AccessDenied {
        room_name: String,
        reason: AccessDeniedReason,
    },
    /// Failed to fetch room information from LiveKit.
    LiveKitError(String),
}

/// Reason for access denial.
#[derive(Debug)]
pub enum AccessDeniedReason {
    /// Room has a different auth_id than the authenticated client.
    AuthIdMismatch { expected: String, actual: String },
    /// Room exists but has no auth_id when authentication is enabled.
    MissingAuthId,
}

impl RoomAccessError {
    /// Convert to an HTTP response, masking the specific reason as 404 to avoid
    /// leaking room existence information.
    pub fn into_response_masked(self) -> axum::response::Response {
        // Always return 404 to avoid leaking room existence
        let message = match &self {
            RoomAccessError::NotFound(room) => format!("Room '{}' not found", room),
            RoomAccessError::AccessDenied { room_name, .. } => {
                format!("Room '{}' not found or not accessible", room_name)
            }
            RoomAccessError::LiveKitError(msg) => {
                format!("Failed to access room: {}", msg)
            }
        };

        let status = match &self {
            RoomAccessError::NotFound(_) | RoomAccessError::AccessDenied { .. } => {
                StatusCode::NOT_FOUND
            }
            RoomAccessError::LiveKitError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }

    /// Convert to an HTTP response with explicit access denied (403) status.
    /// Use this variant when explicitly signaling authorization failure is acceptable.
    pub fn into_response_explicit(self) -> axum::response::Response {
        let (status, message) = match &self {
            RoomAccessError::NotFound(room) => {
                (StatusCode::NOT_FOUND, format!("Room '{}' not found", room))
            }
            RoomAccessError::AccessDenied { room_name, reason } => {
                let msg = match reason {
                    AccessDeniedReason::AuthIdMismatch { .. } => {
                        format!("Access denied to room '{}'", room_name)
                    }
                    AccessDeniedReason::MissingAuthId => {
                        format!(
                            "Access denied to room '{}': room not associated with tenant",
                            room_name
                        )
                    }
                };
                (StatusCode::FORBIDDEN, msg)
            }
            RoomAccessError::LiveKitError(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to access room: {}", msg),
            ),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

/// Check if the authenticated client has access to a specific room.
///
/// This function implements the metadata-based authorization policy described
/// in the module documentation.
///
/// # Arguments
/// * `auth` - The authenticated client context
/// * `room_name` - The room name to check access for
/// * `room_handler` - LiveKit room handler for fetching room metadata
///
/// # Returns
/// * `Ok(())` - Access is allowed
/// * `Err(RoomAccessError)` - Access is denied or an error occurred
///
/// # Policy
///
/// - **auth.id is None/empty/whitespace**: Access is allowed (backward-compatible mode)
/// - **auth.id is Some (non-empty)**: Access requires `room.metadata.auth_id == auth.id`
///
/// # Example
///
/// ```rust,ignore
/// match check_room_access(&auth, "my-room", room_handler).await {
///     Ok(()) => { /* proceed with operation */ }
///     Err(e) => return e.into_response_masked(),
/// }
/// ```
pub async fn check_room_access(
    auth: &Auth,
    room_name: &str,
    room_handler: &LiveKitRoomHandler,
) -> Result<(), RoomAccessError> {
    // If no auth.id (or empty/whitespace), allow access (backward-compatible mode)
    let auth_id = match auth.effective_id() {
        Some(id) => id,
        None => return Ok(()),
    };

    // Fetch room metadata
    let (room, _participants) = room_handler
        .get_room_details(room_name)
        .await
        .map_err(|e| {
            let err_msg = e.to_string();
            if err_msg.contains("not found") {
                RoomAccessError::NotFound(room_name.to_string())
            } else {
                RoomAccessError::LiveKitError(err_msg)
            }
        })?;

    // Extract auth_id from room metadata
    let room_auth_id = get_auth_id(&room.metadata);

    match room_auth_id {
        Some(rid) if rid == *auth_id => {
            // auth_id matches - allow access
            Ok(())
        }
        Some(rid) => {
            // auth_id mismatch - deny access
            warn!(
                room = %room_name,
                expected = %auth_id,
                actual = %rid,
                "Room access denied: auth_id mismatch"
            );
            Err(RoomAccessError::AccessDenied {
                room_name: room_name.to_string(),
                reason: AccessDeniedReason::AuthIdMismatch {
                    expected: auth_id.to_string(),
                    actual: rid,
                },
            })
        }
        None => {
            // Room exists but has no auth_id - deny when auth is enabled
            warn!(
                room = %room_name,
                auth_id = %auth_id,
                "Room access denied: room has no auth_id in metadata"
            );
            Err(RoomAccessError::AccessDenied {
                room_name: room_name.to_string(),
                reason: AccessDeniedReason::MissingAuthId,
            })
        }
    }
}

/// Check room access and return the room metadata if allowed.
///
/// This is a convenience function that combines the access check with returning
/// the room information, avoiding a second fetch.
///
/// # Returns
/// * `Ok((room, participants))` - Access allowed, returns room data
/// * `Err(RoomAccessError)` - Access denied or error
pub async fn check_room_access_with_data(
    auth: &Auth,
    room_name: &str,
    room_handler: &LiveKitRoomHandler,
) -> Result<
    (
        livekit_protocol::Room,
        Vec<livekit_protocol::ParticipantInfo>,
    ),
    RoomAccessError,
> {
    // If no auth.id (or empty/whitespace), fetch and return directly (backward-compatible mode)
    let auth_id = match auth.effective_id() {
        Some(id) => id,
        None => {
            // No auth - just fetch the room
            return room_handler.get_room_details(room_name).await.map_err(|e| {
                let err_msg = e.to_string();
                if err_msg.contains("not found") {
                    RoomAccessError::NotFound(room_name.to_string())
                } else {
                    RoomAccessError::LiveKitError(err_msg)
                }
            });
        }
    };

    // Fetch room metadata
    let (room, participants) = room_handler
        .get_room_details(room_name)
        .await
        .map_err(|e| {
            let err_msg = e.to_string();
            if err_msg.contains("not found") {
                RoomAccessError::NotFound(room_name.to_string())
            } else {
                RoomAccessError::LiveKitError(err_msg)
            }
        })?;

    // Extract auth_id from room metadata
    let room_auth_id = get_auth_id(&room.metadata);

    match room_auth_id {
        Some(rid) if rid == *auth_id => {
            // auth_id matches - return room data
            Ok((room, participants))
        }
        Some(rid) => {
            // auth_id mismatch - deny access
            warn!(
                room = %room_name,
                expected = %auth_id,
                actual = %rid,
                "Room access denied: auth_id mismatch"
            );
            Err(RoomAccessError::AccessDenied {
                room_name: room_name.to_string(),
                reason: AccessDeniedReason::AuthIdMismatch {
                    expected: auth_id.to_string(),
                    actual: rid,
                },
            })
        }
        None => {
            // Room exists but has no auth_id - deny when auth is enabled
            warn!(
                room = %room_name,
                auth_id = %auth_id,
                "Room access denied: room has no auth_id in metadata"
            );
            Err(RoomAccessError::AccessDenied {
                room_name: room_name.to_string(),
                reason: AccessDeniedReason::MissingAuthId,
            })
        }
    }
}

/// Filter a list of rooms by auth_id.
///
/// This function filters rooms to only include those with matching `auth_id`
/// in their metadata. Used primarily for the `GET /livekit/rooms` endpoint.
///
/// # Arguments
/// * `auth` - The authenticated client context
/// * `rooms` - List of rooms to filter
///
/// # Returns
/// Filtered list of rooms belonging to the authenticated client.
///
/// # Policy
///
/// - **auth.id is None/empty/whitespace**: Returns all rooms (backward-compatible mode)
/// - **auth.id is Some (non-empty)**: Returns only rooms where `metadata.auth_id == auth.id`
pub fn filter_rooms_by_auth(
    auth: &Auth,
    rooms: Vec<livekit_protocol::Room>,
) -> Vec<livekit_protocol::Room> {
    // If no auth.id (or empty/whitespace), return all rooms (backward-compatible mode)
    let auth_id = match auth.effective_id() {
        Some(id) => id,
        None => return rooms,
    };

    rooms
        .into_iter()
        .filter(|room| {
            get_auth_id(&room.metadata)
                .map(|rid| rid == *auth_id)
                .unwrap_or(false)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use livekit_protocol::Room;

    fn make_room(name: &str, metadata: &str) -> Room {
        Room {
            name: name.to_string(),
            metadata: metadata.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn test_filter_rooms_by_auth_no_auth() {
        let auth = Auth::empty();
        let rooms = vec![
            make_room("room1", r#"{"auth_id": "tenant-a"}"#),
            make_room("room2", r#"{"auth_id": "tenant-b"}"#),
            make_room("room3", ""),
        ];

        let filtered = filter_rooms_by_auth(&auth, rooms);
        assert_eq!(filtered.len(), 3); // All rooms returned
    }

    #[test]
    fn test_filter_rooms_by_auth_with_auth() {
        let auth = Auth::new("tenant-a");
        let rooms = vec![
            make_room("room1", r#"{"auth_id": "tenant-a"}"#),
            make_room("room2", r#"{"auth_id": "tenant-b"}"#),
            make_room("room3", r#"{"auth_id": "tenant-a"}"#),
            make_room("room4", ""), // No auth_id
        ];

        let filtered = filter_rooms_by_auth(&auth, rooms);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].name, "room1");
        assert_eq!(filtered[1].name, "room3");
    }

    #[test]
    fn test_filter_rooms_by_auth_empty_auth_id() {
        let auth = Auth {
            id: Some("".to_string()),
        };
        let rooms = vec![
            make_room("room1", r#"{"auth_id": "tenant-a"}"#),
            make_room("room2", ""),
        ];

        let filtered = filter_rooms_by_auth(&auth, rooms);
        assert_eq!(filtered.len(), 2); // Empty auth.id treated as no auth
    }

    #[test]
    fn test_filter_rooms_excludes_rooms_without_metadata() {
        let auth = Auth::new("tenant-a");
        let rooms = vec![
            make_room("room1", r#"{"auth_id": "tenant-a"}"#),
            make_room("room2", "{}"),                   // Empty object
            make_room("room3", r#"{"other": "data"}"#), // No auth_id key
        ];

        let filtered = filter_rooms_by_auth(&auth, rooms);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "room1");
    }
}
