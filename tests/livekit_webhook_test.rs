use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use bytes::Bytes;
use livekit_api::access_token::AccessToken;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::util::ServiceExt;

use sayna::{ServerConfig, routes, state::AppState};

/// Helper to create a minimal test configuration with LiveKit credentials
fn create_test_config_with_livekit() -> ServerConfig {
    ServerConfig {
        host: "0.0.0.0".to_string(),
        port: 3001,
        livekit_url: "ws://localhost:7880".to_string(),
        livekit_public_url: "http://localhost:7880".to_string(),
        livekit_api_key: Some("test-api-key".to_string()),
        livekit_api_secret: Some("test-api-secret".to_string()),
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
    }
}

/// Helper to create a test configuration without LiveKit credentials
fn create_test_config_without_livekit() -> ServerConfig {
    ServerConfig {
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
    }
}

/// Helper to create a signed webhook payload
///
/// This mimics LiveKit's webhook signing flow:
/// 1. Create JSON payload
/// 2. Compute SHA256 hash of payload
/// 3. Base64-encode the hash
/// 4. Sign with AccessToken using api_key and api_secret
fn create_signed_webhook(api_key: &str, api_secret: &str, payload: Value) -> (String, String) {
    // Convert payload to string
    let payload_str = payload.to_string();

    // Compute SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(payload_str.as_bytes());
    let hash = hasher.finalize();

    // Base64-encode the hash
    let hash_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &hash);

    // Create signed token
    let token = AccessToken::with_api_key(api_key, api_secret)
        .with_sha256(&hash_b64)
        .to_jwt()
        .expect("Failed to create JWT token");

    (payload_str, token)
}

/// Helper to create a participant_joined webhook event
fn create_participant_joined_event() -> Value {
    json!({
        "event": "participant_joined",
        "id": "test-event-123",
        "createdAt": 1700000000,
        "room": {
            "sid": "RM_test123",
            "name": "test-room",
            "emptyTimeout": 300,
            "maxParticipants": 10,
            "creationTime": 1700000000,
            "turnPassword": "",
            "enabledCodecs": [],
            "metadata": "",
            "numParticipants": 1,
            "numPublishers": 0,
            "activeRecording": false
        },
        "participant": {
            "sid": "PA_test456",
            "identity": "user-123",
            "state": 0,
            "name": "Test User",
            "metadata": "",
            "joinedAt": 1700000000,
            "permission": {
                "canSubscribe": true,
                "canPublish": true,
                "canPublishData": true,
                "hidden": false,
                "recorder": false
            },
            "region": "us-west-2",
            "isPublisher": false,
            "kind": 0,
            "attributes": {},
            "tracks": []
        }
    })
}

/// Helper to create a participant_joined event with SIP attributes
fn create_participant_joined_event_with_sip() -> Value {
    json!({
        "event": "participant_joined",
        "id": "test-event-456",
        "createdAt": 1700000000,
        "room": {
            "sid": "RM_test789",
            "name": "sip-test-room",
            "emptyTimeout": 300,
            "maxParticipants": 10,
            "creationTime": 1700000000,
            "turnPassword": "",
            "enabledCodecs": [],
            "metadata": "",
            "numParticipants": 1,
            "numPublishers": 0,
            "activeRecording": false
        },
        "participant": {
            "sid": "PA_sip123",
            "identity": "sip-caller-456",
            "state": 0,
            "name": "SIP Caller",
            "metadata": "",
            "joinedAt": 1700000000,
            "permission": {
                "canSubscribe": true,
                "canPublish": true,
                "canPublishData": true,
                "hidden": false,
                "recorder": false
            },
            "region": "us-west-2",
            "isPublisher": false,
            "kind": 0,
            "attributes": {
                "sip.trunkPhoneNumber": "+1234567890",
                "sip.fromHeader": "Test User <sip:user@example.com>",
                "sip.callID": "abc123def456",
                "other.attribute": "should-be-ignored"
            },
            "tracks": []
        }
    })
}

#[tokio::test]
async fn test_webhook_success() {
    // Arrange: Create app with LiveKit credentials
    let config = create_test_config_with_livekit();
    let app_state = AppState::new(config).await;
    let app = routes::webhooks::create_webhook_router().with_state(app_state);

    // Create signed webhook payload
    let payload = create_participant_joined_event();
    let (payload_str, token) = create_signed_webhook("test-api-key", "test-api-secret", payload);

    // Act: Send webhook request
    let request = Request::builder()
        .method("POST")
        .uri("/livekit/webhook")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", token))
        .body(Body::from(payload_str))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Assert: Check response status and body
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected 200 OK for valid webhook"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "received");
}

#[tokio::test]
async fn test_webhook_success_with_sip_attributes() {
    // Arrange: Create app with LiveKit credentials
    let config = create_test_config_with_livekit();
    let app_state = AppState::new(config).await;
    let app = routes::webhooks::create_webhook_router().with_state(app_state);

    // Create signed webhook payload with SIP attributes
    let payload = create_participant_joined_event_with_sip();
    let (payload_str, token) = create_signed_webhook("test-api-key", "test-api-secret", payload);

    // Act: Send webhook request
    let request = Request::builder()
        .method("POST")
        .uri("/livekit/webhook")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", token))
        .body(Body::from(payload_str))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Assert: Check response status and body
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected 200 OK for valid webhook with SIP attributes"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "received");

    // Note: SIP attributes are logged (tested in unit tests below)
    // but not returned in the response, so we don't assert on them here
}

#[tokio::test]
async fn test_webhook_missing_authorization_header() {
    // Arrange: Create app with LiveKit credentials
    let config = create_test_config_with_livekit();
    let app_state = AppState::new(config).await;
    let app = routes::webhooks::create_webhook_router().with_state(app_state);

    // Create payload without signing it
    let payload = create_participant_joined_event();

    // Act: Send request without Authorization header
    let request = Request::builder()
        .method("POST")
        .uri("/livekit/webhook")
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Assert: Expect 401 Unauthorized (4xx = no retry in LiveKit)
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Expected 401 for missing Authorization header"
    );
}

#[tokio::test]
async fn test_webhook_empty_authorization_token() {
    // Arrange: Create app with LiveKit credentials
    let config = create_test_config_with_livekit();
    let app_state = AppState::new(config).await;
    let app = routes::webhooks::create_webhook_router().with_state(app_state);

    let payload = create_participant_joined_event();

    // Act: Send request with empty Authorization header
    let request = Request::builder()
        .method("POST")
        .uri("/livekit/webhook")
        .header("content-type", "application/json")
        .header("authorization", "")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Assert: Expect 401 Unauthorized
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Expected 401 for empty Authorization token"
    );
}

#[tokio::test]
async fn test_webhook_invalid_signature() {
    // Arrange: Create app with LiveKit credentials
    let config = create_test_config_with_livekit();
    let app_state = AppState::new(config).await;
    let app = routes::webhooks::create_webhook_router().with_state(app_state);

    // Create payload signed with WRONG secret
    let payload = create_participant_joined_event();
    let (payload_str, _) = create_signed_webhook("test-api-key", "wrong-secret", payload);

    // Create a fake token that won't verify
    let bad_token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.invalid";

    // Act: Send request with invalid token
    let request = Request::builder()
        .method("POST")
        .uri("/livekit/webhook")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", bad_token))
        .body(Body::from(payload_str))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Assert: Expect 401 Unauthorized (4xx = no retry)
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Expected 401 for invalid signature"
    );
}

#[tokio::test]
async fn test_webhook_hash_mismatch() {
    // Arrange: Create app with LiveKit credentials
    let config = create_test_config_with_livekit();
    let app_state = AppState::new(config).await;
    let app = routes::webhooks::create_webhook_router().with_state(app_state);

    // Create payload and sign it
    let original_payload = create_participant_joined_event();
    let (_, token) = create_signed_webhook("test-api-key", "test-api-secret", original_payload);

    // But send a DIFFERENT payload (hash won't match)
    let tampered_payload = json!({
        "event": "participant_left",
        "id": "tampered-event-999"
    });

    // Act: Send request with mismatched hash
    let request = Request::builder()
        .method("POST")
        .uri("/livekit/webhook")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {}", token))
        .body(Body::from(tampered_payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Assert: Expect 401 Unauthorized (signature verification will fail)
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Expected 401 for hash mismatch"
    );
}

#[tokio::test]
async fn test_webhook_no_livekit_credentials() {
    // Arrange: Create app WITHOUT LiveKit credentials
    let config = create_test_config_without_livekit();
    let app_state = AppState::new(config).await;
    let app = routes::webhooks::create_webhook_router().with_state(app_state);

    // Create a valid-looking payload (doesn't matter since we'll fail early)
    let payload = create_participant_joined_event();

    // Act: Send request
    let request = Request::builder()
        .method("POST")
        .uri("/livekit/webhook")
        .header("content-type", "application/json")
        .header("authorization", "Bearer fake-token")
        .body(Body::from(payload.to_string()))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Assert: Expect 503 Service Unavailable (5xx = LiveKit will retry)
    // This tells LiveKit "we're not configured yet, try again later"
    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "Expected 503 when LiveKit credentials not configured"
    );
}

#[tokio::test]
async fn test_webhook_invalid_utf8_body() {
    // Arrange: Create app with LiveKit credentials
    let config = create_test_config_with_livekit();
    let app_state = AppState::new(config).await;
    let app = routes::webhooks::create_webhook_router().with_state(app_state);

    // Create invalid UTF-8 bytes
    let invalid_utf8 = vec![0xFF, 0xFE, 0xFD];

    // Act: Send request with invalid UTF-8
    let request = Request::builder()
        .method("POST")
        .uri("/livekit/webhook")
        .header("content-type", "application/json")
        .header("authorization", "Bearer fake-token")
        .body(Body::from(Bytes::from(invalid_utf8)))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Assert: Expect 400 Bad Request (client error, no retry)
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Expected 400 for invalid UTF-8 body"
    );
}

#[tokio::test]
async fn test_webhook_bearer_prefix_optional() {
    // Arrange: Create app with LiveKit credentials
    let config = create_test_config_with_livekit();
    let app_state = AppState::new(config).await;
    let app = routes::webhooks::create_webhook_router().with_state(app_state);

    // Create signed webhook payload
    let payload = create_participant_joined_event();
    let (payload_str, token) = create_signed_webhook("test-api-key", "test-api-secret", payload);

    // Act: Send request WITHOUT "Bearer " prefix
    let request = Request::builder()
        .method("POST")
        .uri("/livekit/webhook")
        .header("content-type", "application/json")
        .header("authorization", token) // No "Bearer " prefix
        .body(Body::from(payload_str))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // Assert: Should still succeed (handler strips "Bearer " if present)
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "Expected 200 OK even without 'Bearer ' prefix"
    );
}

// ============================================================================
// Unit tests for helper functions
// ============================================================================

#[cfg(test)]
mod unit_tests {
    use super::*;
    use livekit_api::webhooks::WebhookError;
    use livekit_protocol::ParticipantInfo;
    use sayna::handlers::livekit_webhook::{extract_sip_attributes, webhook_error_to_status};

    #[test]
    fn test_extract_sip_attributes_with_sip_keys() {
        // Arrange: Create participant with SIP attributes
        let mut participant = ParticipantInfo::default();
        participant.attributes.insert(
            "sip.trunkPhoneNumber".to_string(),
            "+1234567890".to_string(),
        );
        participant.attributes.insert(
            "sip.fromHeader".to_string(),
            "User <sip:user@example.com>".to_string(),
        );
        participant
            .attributes
            .insert("sip.callID".to_string(), "abc123".to_string());
        participant
            .attributes
            .insert("other.attribute".to_string(), "ignored".to_string());
        participant
            .attributes
            .insert("notSipAttr".to_string(), "also-ignored".to_string());

        // Act: Extract SIP attributes
        let sip_attrs = extract_sip_attributes(&participant);

        // Assert: Only SIP attributes are extracted
        assert_eq!(sip_attrs.len(), 3);
        assert_eq!(
            sip_attrs.get("sip.trunkPhoneNumber"),
            Some(&"+1234567890".to_string())
        );
        assert_eq!(
            sip_attrs.get("sip.fromHeader"),
            Some(&"User <sip:user@example.com>".to_string())
        );
        assert_eq!(sip_attrs.get("sip.callID"), Some(&"abc123".to_string()));
        assert_eq!(sip_attrs.get("other.attribute"), None);
        assert_eq!(sip_attrs.get("notSipAttr"), None);
    }

    #[test]
    fn test_extract_sip_attributes_no_sip_keys() {
        // Arrange: Create participant without SIP attributes
        let mut participant = ParticipantInfo::default();
        participant
            .attributes
            .insert("other.attribute".to_string(), "value".to_string());
        participant
            .attributes
            .insert("another".to_string(), "value2".to_string());

        // Act: Extract SIP attributes
        let sip_attrs = extract_sip_attributes(&participant);

        // Assert: Empty map
        assert!(sip_attrs.is_empty());
    }

    #[test]
    fn test_extract_sip_attributes_empty_participant() {
        // Arrange: Create participant with no attributes
        let participant = ParticipantInfo::default();

        // Act: Extract SIP attributes
        let sip_attrs = extract_sip_attributes(&participant);

        // Assert: Empty map
        assert!(sip_attrs.is_empty());
    }

    #[test]
    fn test_webhook_error_to_status_invalid_signature() {
        // Arrange
        let error = WebhookError::InvalidSignature;

        // Act
        let status = webhook_error_to_status(&error);

        // Assert: Returns 401 Unauthorized (4xx = no retry)
        // This is correct because invalid signatures should not trigger retries
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // Note: WebhookError variants wrap specific error types from dependencies.
    // The error mapping logic is simple and tested via integration tests above.
    // Testing every error variant would require constructing complex dependency types,
    // which adds maintenance burden without significant value.
    //
    // Key invariants tested:
    // - InvalidSignature -> 401 (tested above)
    // - All error paths are covered in integration tests (invalid auth, bad JSON, etc.)
    // - The mapping function itself is trivial (simple match statement)
}
