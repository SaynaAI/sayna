use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode},
    middleware,
    routing::get,
};
use sayna::{config::ServerConfig, middleware::auth::auth_middleware, state::AppState};
use std::sync::Arc;
use tower::ServiceExt;

/// Helper to create a test AppState with auth disabled
async fn create_test_state_auth_disabled() -> Arc<AppState> {
    let config = ServerConfig {
        host: "localhost".to_string(),
        port: 3001,
        livekit_url: "ws://localhost:7880".to_string(),
        livekit_public_url: "http://localhost:7880".to_string(),
        livekit_api_key: None,
        livekit_api_secret: None,
        deepgram_api_key: None,
        elevenlabs_api_key: None,
        google_credentials: None,
        azure_speech_subscription_key: None,
        azure_speech_region: None,
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
        auth_required: false, // Auth disabled
        sip: None,
    };

    AppState::new(config).await
}

/// Helper handler that returns 200 OK
async fn test_handler() -> &'static str {
    "OK"
}

#[tokio::test]
async fn test_auth_disabled_allows_requests_without_auth_header() {
    let state = create_test_state_auth_disabled().await;

    let app = Router::new()
        .route("/test", get(test_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);

    let request = Request::builder()
        .method(Method::GET)
        .uri("/test")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_auth_disabled_allows_requests_with_auth_header() {
    let state = create_test_state_auth_disabled().await;

    let app = Router::new()
        .route("/test", get(test_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);

    let request = Request::builder()
        .method(Method::GET)
        .uri("/test")
        .header("authorization", "Bearer some-token")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

// Note: Tests for misconfigured auth (AUTH_REQUIRED=true but missing signing key)
// are not included here because the server now implements fail-fast behavior.
// When auth is required but misconfigured, AppState::new() will panic on startup,
// preventing the server from running at all. This is by design to ensure the server
// never runs without proper authentication when AUTH_REQUIRED=true.

// Integration tests with mocked auth service
mod with_auth_service {
    use super::*;
    use std::fs;
    use tempfile::TempDir;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    // Test RSA private key (for signing auth requests)
    const TEST_RSA_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCMrnQRHYGWk8lv
FGwO/Dr6HVPxIc5FONC8Bz2E24pmUIEgRhvOaUsdlak1PZNiJU23eLcsa8/UtlGx
B56i0PNXliiEpU9tCS5sCKf9PTwD5PkJowep4xBdcQMK1MdcDMMULpfJYc08o9rE
zGHtoX4mnT2yk+/hAn9wCdvH0hceNFgxUhXtiJBToIjiC+ANqd3IjlCmcCZH1DhT
Uwgzern6nmo0GetFydjeXFuPUiZdIprY2yRZtZ20j2PkowBroIGIiXTr7/X6fIRA
9Jg4rrSDdoTsv15KBNcpcwoRqJ55/r4mevz3tWea2Mv1hxyJ5ywGnqoMC5X5G3Ur
aoAriQHXAgMBAAECggEACD3qN9x6LJew6+yO3hvh2qhgNBbObljDRdjIvmFcTN03
i2wAEgoyJ+QOOzvFyDC2SmLsnFIepXAe/hebsB88umtmKUtECXfJu/OP3/K38uR1
wJ5IAyh123uU+Yv4uAhZX3PRWa98pipVVUVCEXluGhYJOM6Y9Z4/WBGDykOhLhg9
tr3zmHJFGp0OXmmT4b/WTlwF6hvafaDb9mjAsL0fhK1r9+/U75q0QltJiLlIg5Gb
q0kmQcG60/5Z2NZEizlH1lDB+gCC2s5H9CxXorLsZEpEDcHKkMT5sCM5tic5EMH2
HNXXG+tC3ph/XvmK7Ow+rJEi3mMXwZFs95NJXQBBBQKBgQDGFrklHAUEQZL4Izgh
GIDeL2caUSenXMMmpnD4kLs6P9F8UeQfErLZ2J3D5HayuAKAnj0wiwLOlrGTxC9N
RaVsrVVidTn599b1ICD+fPidpfXhaye3wezrzVeVf3IOYa2LVHsiZZ9nVsHEbBKz
i0yYw3aIlv/PTsSWtf8JumUVowKBgQC1z0bOuIUCuiGApTo00V/NR1IQ8+tX+yU7
dn61SoyKtgmHA1Po+WgEDWi4zehFWU6xUog/KCTzm0VWTGIjh70Rv9w1N7QX/iug
O8bpX8uekZUEt6ShddJYsyhd+e5WwWdxgjm7UOLXgOggJFjpx8CBzlIj9LtbJAJ9
LeS/zItePQKBgQC3EEzuZKSmOEuwkivPOivuKfSot5Nj8jBPycXhkS/WNyBMOgoO
RWOQO8YhQUQJClEVuCdocy+W6GEX5FiqmtC0TMP6B8gaoNbBFn4ncir41mUTe8nq
4ocnrE9i07L+Y3rUprBdK3lTMTRFaHMoBnY1P36N4K5sUakQdwVJYj8E7QKBgQCA
P8T9EeCR+eakLumOVJu13LehSc8b8wdimMXs8LePKbYyzUAlubmMEkFrC6TrNoJy
R3vgwVq/lSomJB+eXKQcnzChQbgCrMLtdv1rpq2mH5/1Ae5aDxjghRDWqfVcsXVc
9rXu0rIRvtb/xWQLFWNQrc/3mS2IrzAqSXNxcMJnKQKBgBiooJ/PPRuwg9ZFJYoc
Sn8fw7fRDvRsslnHus1GgXZYWBbMwbNxGGO0Gy68i2SDu30gPlSVLzb+a8GD6qD/
8unmgG3V7uLXrpNIu5to6SggvL2mt5F036GFEnR2pnPH08ooqQc4u7u2k9P79pS8
V/reoL3Jcy/mQ9MrmJx+K1VC
-----END PRIVATE KEY-----"#;

    /// Helper to create a test AppState with auth enabled
    async fn create_test_state_with_auth(auth_service_url: &str) -> (Arc<AppState>, TempDir) {
        // Create a temporary directory for the key file
        let temp_dir = TempDir::new().unwrap();
        let key_path = temp_dir.path().join("test_key.pem");
        fs::write(&key_path, TEST_RSA_PRIVATE_KEY).unwrap();

        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_public_url: "http://localhost:7880".to_string(),
            livekit_api_key: None,
            livekit_api_secret: None,
            deepgram_api_key: None,
            elevenlabs_api_key: None,
            google_credentials: None,
            azure_speech_subscription_key: None,
            azure_speech_region: None,
            recording_s3_bucket: None,
            recording_s3_region: None,
            recording_s3_endpoint: None,
            recording_s3_access_key: None,
            recording_s3_secret_key: None,
            cache_path: None,
            cache_ttl_seconds: Some(3600),
            auth_service_url: Some(auth_service_url.to_string()),
            auth_signing_key_path: Some(key_path),
            auth_api_secret: None,
            auth_timeout_seconds: 5,
            auth_required: true,
            sip: None,
        };

        let state = AppState::new(config).await;
        (state, temp_dir)
    }

    #[tokio::test]
    async fn test_auth_required_missing_header_returns_401() {
        let mock_server = MockServer::start().await;
        let (state, _temp_dir) = create_test_state_with_auth(&mock_server.uri()).await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // Verify error response body
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "missing_auth_header");
        assert_eq!(json["message"], "Missing Authorization header");
    }

    #[tokio::test]
    async fn test_auth_required_invalid_header_format_returns_401() {
        let mock_server = MockServer::start().await;
        let (state, _temp_dir) = create_test_state_with_auth(&mock_server.uri()).await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("authorization", "InvalidFormat")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "invalid_auth_header");
        assert_eq!(json["message"], "Invalid Authorization header format");
    }

    #[tokio::test]
    async fn test_auth_service_returns_200_allows_request() {
        let mock_server = MockServer::start().await;

        // Mock successful auth response
        Mock::given(method("POST"))
            .and(path("/auth"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

        let (state, _temp_dir) =
            create_test_state_with_auth(&format!("{}/auth", mock_server.uri())).await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("authorization", "Bearer valid-token")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_service_returns_401_denies_request() {
        let mock_server = MockServer::start().await;

        // Mock auth service rejection
        Mock::given(method("POST"))
            .and(path("/auth"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid token"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let (state, _temp_dir) =
            create_test_state_with_auth(&format!("{}/auth", mock_server.uri())).await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("authorization", "Bearer invalid-token")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "unauthorized");
        assert!(json["message"].as_str().unwrap().contains("Invalid token"));
    }

    #[tokio::test]
    async fn test_auth_service_returns_500_returns_bad_gateway() {
        let mock_server = MockServer::start().await;

        // Mock auth service internal error
        Mock::given(method("POST"))
            .and(path("/auth"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal server error"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let (state, _temp_dir) =
            create_test_state_with_auth(&format!("{}/auth", mock_server.uri())).await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("authorization", "Bearer some-token")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // 5xx from auth service should map to 502 Bad Gateway
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "auth_service_error");
        assert!(json["message"].as_str().unwrap().contains("500"));
    }

    #[tokio::test]
    async fn test_auth_service_timeout_returns_service_unavailable() {
        let mock_server = MockServer::start().await;

        // Mock slow auth service (timeout)
        use std::time::Duration;
        Mock::given(method("POST"))
            .and(path("/auth"))
            .respond_with(
                ResponseTemplate::new(200).set_delay(Duration::from_secs(10)), // Longer than timeout
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let (state, _temp_dir) =
            create_test_state_with_auth(&format!("{}/auth", mock_server.uri())).await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("authorization", "Bearer some-token")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Timeout should return 503 Service Unavailable
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "auth_service_unavailable");
    }
}

// Integration tests for API secret authentication
mod with_api_secret {
    use super::*;

    /// Helper to create a test AppState with API secret auth enabled
    async fn create_test_state_with_api_secret(api_secret: &str) -> Arc<AppState> {
        let config = ServerConfig {
            host: "localhost".to_string(),
            port: 3001,
            livekit_url: "ws://localhost:7880".to_string(),
            livekit_public_url: "http://localhost:7880".to_string(),
            livekit_api_key: None,
            livekit_api_secret: None,
            deepgram_api_key: None,
            elevenlabs_api_key: None,
            google_credentials: None,
            azure_speech_subscription_key: None,
            azure_speech_region: None,
            recording_s3_bucket: None,
            recording_s3_region: None,
            recording_s3_endpoint: None,
            recording_s3_access_key: None,
            recording_s3_secret_key: None,
            cache_path: None,
            cache_ttl_seconds: Some(3600),
            auth_service_url: None,
            auth_signing_key_path: None,
            auth_api_secret: Some(api_secret.to_string()),
            auth_timeout_seconds: 5,
            auth_required: true,
            sip: None,
        };

        AppState::new(config).await
    }

    #[tokio::test]
    async fn test_api_secret_auth_missing_header_returns_401() {
        let state = create_test_state_with_api_secret("my-secret-token").await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "missing_auth_header");
        assert_eq!(json["message"], "Missing Authorization header");
    }

    #[tokio::test]
    async fn test_api_secret_auth_invalid_header_format_returns_401() {
        let state = create_test_state_with_api_secret("my-secret-token").await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("authorization", "InvalidFormat")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "invalid_auth_header");
        assert_eq!(json["message"], "Invalid Authorization header format");
    }

    #[tokio::test]
    async fn test_api_secret_auth_valid_token_allows_request() {
        let state = create_test_state_with_api_secret("my-secret-token").await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("authorization", "Bearer my-secret-token")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(body_str, "OK");
    }

    #[tokio::test]
    async fn test_api_secret_auth_invalid_token_denies_request() {
        let state = create_test_state_with_api_secret("my-secret-token").await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("authorization", "Bearer wrong-token")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "unauthorized");
        assert!(
            json["message"]
                .as_str()
                .unwrap()
                .contains("Invalid API secret")
        );
    }

    #[tokio::test]
    async fn test_api_secret_auth_with_post_request_and_body() {
        let state = create_test_state_with_api_secret("my-secret-token").await;

        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::POST)
            .uri("/test")
            .header("authorization", "Bearer my-secret-token")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"test": "data"}"#))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        // Should succeed with valid token, even with body
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED); // GET only handler
    }

    #[tokio::test]
    async fn test_api_secret_auth_case_sensitive_token() {
        let state = create_test_state_with_api_secret("MySecretToken").await;

        // Test with different case - should fail
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state.clone());

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("authorization", "Bearer mysecrettoken")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // Test with exact case - should succeed
        let app = Router::new()
            .route("/test", get(test_handler))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state);

        let request = Request::builder()
            .method(Method::GET)
            .uri("/test")
            .header("authorization", "Bearer MySecretToken")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
