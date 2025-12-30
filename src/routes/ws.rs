use axum::{Router, routing::get};
use tower_http::trace::TraceLayer;

use crate::handlers::ws;
use crate::state::AppState;
use std::sync::Arc;

/// Create the WebSocket router
///
/// # WebSocket Authentication Design
///
/// The WebSocket endpoint uses the same auth middleware as REST endpoints for tenant isolation.
/// The auth middleware provides `Auth` context that is used to prefix LiveKit room names with
/// the authenticated client's ID, ensuring different tenants cannot access each other's rooms.
///
/// ## Behavior
///
/// - **When `AUTH_REQUIRED=true`**: Requires valid authentication token in the Authorization header
/// - **When `AUTH_REQUIRED=false`**: Inserts an empty `Auth` context (no room name prefixing)
///
/// ## Room Name Isolation
///
/// When a client connects with an authenticated context (e.g., `auth.id = "project1"`),
/// any LiveKit room names are automatically prefixed with the auth ID:
/// - Client requests room `my-room` → Actual room becomes `project1_my-room`
/// - This prevents tenant A from accessing tenant B's rooms
///
/// See `docs/authentication.md` for detailed authentication architecture.
pub fn create_ws_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ws", get(ws::ws_voice_handler))
        .layer(TraceLayer::new_for_http())
}
