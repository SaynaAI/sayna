use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::Value;
use tower::util::ServiceExt;

use sayna::{ServerConfig, routes, state::AppState};

#[tokio::test]
async fn test_health_check() {
    // Create test configuration
    let config = ServerConfig {
        host: "0.0.0.0".to_string(),
        port: 3001,
    };

    // Create app state
    let app_state = AppState::new(config);

    // Create router with state
    let app = routes::api::create_api_router().with_state(app_state);

    // Create request
    let request = Request::builder().uri("/").body(Body::empty()).unwrap();

    // Send request
    let response = app.oneshot(request).await.unwrap();

    // Check response status
    assert_eq!(response.status(), StatusCode::OK);

    // Check response body
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "OK");
}
