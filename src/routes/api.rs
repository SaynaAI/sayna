use axum::{Router, routing::get};
use tower_http::trace::TraceLayer;

use crate::handlers::api;
use crate::state::AppState;
use std::sync::Arc;

pub fn create_api_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(api::health_check))
        .layer(TraceLayer::new_for_http())
}
