use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::util::ServiceExt;

use sayna::{ServerConfig, routes, state::AppState};

#[tokio::test]
async fn test_health_check() {
    // Create test configuration
    let config = ServerConfig {
        host: "0.0.0.0".to_string(),
        port: 3001,
        livekit_url: "ws://localhost:7880".to_string(),
        livekit_public_url: "http://localhost:7880".to_string(),
        livekit_api_key: None,
        livekit_api_secret: None,
        deepgram_api_key: None,
        elevenlabs_api_key: None,
        recording_s3_bucket: None,
        recording_s3_region: None,
        recording_s3_endpoint: None,
        recording_s3_access_key: None,
        recording_s3_secret_key: None,
        cache_path: None,
        cache_ttl_seconds: Some(3600),
        auth_service_url: None,
        auth_signing_key_path: None,
        auth_api_secret: None,
        auth_timeout_seconds: 5,
        auth_required: false,
        sip: None,
    };

    // Create app state
    let app_state = AppState::new(config).await;

    // Create router with health check endpoint (public, no auth)
    use axum::{Router, routing::get};
    let app = Router::new()
        .route("/", get(sayna::handlers::api::health_check))
        .with_state(app_state);

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

#[tokio::test]
async fn test_speak_endpoint_missing_api_key() {
    // Create test configuration without API keys
    let config = ServerConfig {
        host: "0.0.0.0".to_string(),
        livekit_api_key: None,
        livekit_api_secret: None,
        port: 3001,
        livekit_url: "ws://localhost:7880".to_string(),
        livekit_public_url: "http://localhost:7880".to_string(),
        deepgram_api_key: None,
        elevenlabs_api_key: None,
        recording_s3_bucket: None,
        recording_s3_region: None,
        recording_s3_endpoint: None,
        recording_s3_access_key: None,
        recording_s3_secret_key: None,
        cache_path: None,
        cache_ttl_seconds: Some(3600),
        auth_service_url: None,
        auth_signing_key_path: None,
        auth_api_secret: None,
        auth_timeout_seconds: 5,
        auth_required: false,
        sip: None,
    };

    // Create app state
    let app_state = AppState::new(config).await;

    // Create router with state
    let app = routes::api::create_api_router().with_state(app_state);

    // Create request body with TTSWebSocketConfig (no API key)
    let request_body = json!({
        "text": "Hello, this is a test.",
        "tts_config": {
            "provider": "deepgram",
            "model": "aura-asteria-en",
            "voice_id": "aura-asteria-en",
            "audio_format": "linear16",
            "sample_rate": 24000,
            "pronunciations": []
        }
    });

    // Create request
    let request = Request::builder()
        .method("POST")
        .uri("/speak")
        .header("content-type", "application/json")
        .body(Body::from(request_body.to_string()))
        .unwrap();

    // Send request
    let response = app.oneshot(request).await.unwrap();

    // Should fail because the API key is invalid/test key
    // The actual provider will reject the request
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_speak_endpoint_empty_text() {
    // Create test configuration
    let config = ServerConfig {
        host: "0.0.0.0".to_string(),
        livekit_api_key: None,
        livekit_api_secret: None,
        port: 3001,
        livekit_url: "ws://localhost:7880".to_string(),
        livekit_public_url: "http://localhost:7880".to_string(),
        deepgram_api_key: Some("test_key".to_string()),
        elevenlabs_api_key: None,
        recording_s3_bucket: None,
        recording_s3_region: None,
        recording_s3_endpoint: None,
        recording_s3_access_key: None,
        recording_s3_secret_key: None,
        cache_path: None,
        cache_ttl_seconds: Some(3600),
        auth_service_url: None,
        auth_signing_key_path: None,
        auth_api_secret: None,
        auth_timeout_seconds: 5,
        auth_required: false,
        sip: None,
    };

    // Create app state
    let app_state = AppState::new(config).await;

    // Create router with state
    let app = routes::api::create_api_router().with_state(app_state);

    // Create request body with empty text
    let request_body = json!({
        "text": "",
        "tts_config": {
            "provider": "deepgram",
            "model": "aura-asteria-en",
            "voice_id": "aura-asteria-en",
            "audio_format": "linear16",
            "sample_rate": 24000,
            "pronunciations": []
        }
    });

    // Create request
    let request = Request::builder()
        .method("POST")
        .uri("/speak")
        .header("content-type", "application/json")
        .body(Body::from(request_body.to_string()))
        .unwrap();

    // Send request
    let response = app.oneshot(request).await.unwrap();

    // Should return bad request for empty text
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // Check response body
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["error"], "Text cannot be empty");
}

#[tokio::test]
async fn test_speak_endpoint_with_pronunciations() {
    // Create test configuration
    let config = ServerConfig {
        host: "0.0.0.0".to_string(),
        port: 3001,
        livekit_api_key: None,
        livekit_api_secret: None,
        livekit_url: "ws://localhost:7880".to_string(),
        livekit_public_url: "http://localhost:7880".to_string(),
        deepgram_api_key: Some("test_key".to_string()),
        elevenlabs_api_key: None,
        recording_s3_bucket: None,
        recording_s3_region: None,
        recording_s3_endpoint: None,
        recording_s3_access_key: None,
        recording_s3_secret_key: None,
        cache_path: None,
        cache_ttl_seconds: Some(3600),
        auth_service_url: None,
        auth_signing_key_path: None,
        auth_api_secret: None,
        auth_timeout_seconds: 5,
        auth_required: false,
        sip: None,
    };

    // Create app state
    let app_state = AppState::new(config).await;

    // Create router with state
    let app = routes::api::create_api_router().with_state(app_state);

    // Create request body with pronunciations
    let request_body = json!({
        "text": "The API and TTS systems are working well.",
        "tts_config": {
            "provider": "deepgram",
            "model": "aura-asteria-en",
            "voice_id": "aura-asteria-en",
            "audio_format": "linear16",
            "sample_rate": 24000,
            "pronunciations": [
                {
                    "word": "API",
                    "pronunciation": "A P I"
                },
                {
                    "word": "TTS",
                    "pronunciation": "text to speech"
                }
            ]
        }
    });

    // Create request
    let request = Request::builder()
        .method("POST")
        .uri("/speak")
        .header("content-type", "application/json")
        .body(Body::from(request_body.to_string()))
        .unwrap();

    // Send request
    let response = app.oneshot(request).await.unwrap();

    // Will fail with test API key, but that's expected
    // We're just testing that the endpoint accepts the pronunciations field
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_speak_endpoint_invalid_provider() {
    // Create test configuration
    let config = ServerConfig {
        host: "0.0.0.0".to_string(),
        port: 3001,
        livekit_api_key: None,
        livekit_api_secret: None,
        livekit_url: "ws://localhost:7880".to_string(),
        livekit_public_url: "http://localhost:7880".to_string(),
        deepgram_api_key: Some("test_key".to_string()),
        elevenlabs_api_key: None,
        recording_s3_bucket: None,
        recording_s3_region: None,
        recording_s3_endpoint: None,
        recording_s3_access_key: None,
        recording_s3_secret_key: None,
        cache_path: None,
        cache_ttl_seconds: Some(3600),
        auth_service_url: None,
        auth_signing_key_path: None,
        auth_api_secret: None,
        auth_timeout_seconds: 5,
        auth_required: false,
        sip: None,
    };

    // Create app state
    let app_state = AppState::new(config).await;

    // Create router with state
    let app = routes::api::create_api_router().with_state(app_state);

    // Create request body with invalid provider
    let request_body = json!({
        "text": "Hello, test.",
        "tts_config": {
            "provider": "invalid_provider",
            "model": "",
            "voice_id": "test_voice",
            "audio_format": "linear16",
            "sample_rate": 24000,
            "pronunciations": []
        }
    });

    // Create request
    let request = Request::builder()
        .method("POST")
        .uri("/speak")
        .header("content-type", "application/json")
        .body(Body::from(request_body.to_string()))
        .unwrap();

    // Send request
    let response = app.oneshot(request).await.unwrap();

    // Should return error for invalid provider
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    // Check response body
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    // The error will be about API key not configured for invalid provider
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("API key not configured")
    );
}
