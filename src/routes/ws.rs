use axum::{Router, routing::get};
use tower_http::trace::TraceLayer;

use crate::handlers::ws;
use crate::state::AppState;
use std::sync::Arc;

/// Create the WebSocket router
///
/// # WebSocket Authentication Design
///
/// **Design Choice: WebSocket endpoints are intentionally unauthenticated (Option C)**
///
/// The `/ws` endpoint does not require authentication for the following reasons:
///
/// 1. **Complexity**: WebSocket authentication is significantly more complex than REST auth:
///    - Token validation must occur during the HTTP upgrade handshake
///    - Tokens must be passed via query parameters or headers (not ideal for bearer tokens)
///    - Connection lifecycle management becomes more complex with auth failures
///
/// 2. **Use Case**: The primary use case for WebSocket connections is real-time voice
///    processing where:
///    - Connections are typically short-lived
///    - Audio data is ephemeral and not persisted
///    - The service acts as a processing pipeline rather than a data store
///
/// 3. **Alternative Protection**: If WebSocket auth is required, consider:
///    - **Network-level protection**: Use a reverse proxy (nginx, Envoy) with auth
///    - **Application tokens**: Implement custom token validation in the Config message
///    - **Query param tokens**: Validate tokens passed as URL parameters during upgrade
///
/// 4. **REST Endpoints are Protected**: All REST endpoints (`/voices`, `/speak`, `/livekit/token`)
///    require authentication when `AUTH_REQUIRED=true`, protecting the core API surface.
///
/// ## Future Enhancement
///
/// To add WebSocket authentication in the future, implement one of these approaches:
///
/// - **Option A (Upgrade-time auth)**: Validate authorization header during HTTP upgrade
/// - **Option B (Message-based auth)**: Require auth token in the first WebSocket message
/// - **Option C (Current)**: No WebSocket auth, protect via network/proxy layer
///
/// See `docs/authentication.md` for detailed authentication architecture.
pub fn create_ws_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ws", get(ws::ws_voice_handler))
        .layer(TraceLayer::new_for_http())
}
