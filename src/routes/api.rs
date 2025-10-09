use axum::{
    Router,
    routing::{get, post},
};
use tower_http::trace::TraceLayer;

use crate::handlers::{speak, voices};
use crate::state::AppState;
use std::sync::Arc;

pub fn create_api_router() -> Router<Arc<AppState>> {
    Router::new()
        // Protected routes (auth required)
        .route("/voices", get(voices::list_voices))
        .route("/speak", post(speak::speak_handler))
        .layer(TraceLayer::new_for_http())
}
