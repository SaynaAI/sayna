use axum::{Router, routing::get};
use tower_http::trace::TraceLayer;

use crate::handlers::ws;
use crate::state::AppState;
use std::sync::Arc;

pub fn create_ws_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/ws", get(ws::ws_voice_handler))
        .layer(TraceLayer::new_for_http())
}
