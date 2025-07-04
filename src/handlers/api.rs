use axum::{http::StatusCode, response::Json};
use serde_json::{Value, json};

/// Health check handler
/// Returns a simple JSON response indicating the server is running
pub async fn health_check() -> Result<Json<Value>, StatusCode> {
    Ok(Json(json!({
        "status": "OK"
    })))
}
