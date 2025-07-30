use axum::{Router, routing::get};
use tower_http::trace::TraceLayer;

use crate::handlers::{api, voices};
use crate::state::AppState;
use std::sync::Arc;

pub fn create_api_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(api::health_check))
        .route("/voices", get(voices::list_voices))
        .layer(TraceLayer::new_for_http())
}
