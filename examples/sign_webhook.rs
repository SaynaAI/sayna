use base64::Engine;
use livekit_api::access_token::AccessToken;
use serde_json::json;
use sha2::{Digest, Sha256};

fn main() {
    let api_key = std::env::var("LIVEKIT_API_KEY").unwrap_or("test-api-key".to_string());
    let api_secret = std::env::var("LIVEKIT_API_SECRET").unwrap_or("test-api-secret".to_string());

    // Sample payload with SIP attributes
    let payload = json!({
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
            "attributes": {
                "sip.trunkPhoneNumber": "+1234567890",
                "sip.fromHeader": "Test User <sip:user@example.com>",
                "sip.callID": "abc123def456"
            },
            "tracks": []
        }
    });

    let payload_str = payload.to_string();

    // Compute SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(payload_str.as_bytes());
    let hash = hasher.finalize();

    // Base64-encode the hash
    let hash_b64 = base64::engine::general_purpose::STANDARD.encode(hash);

    // Create signed token
    let token = AccessToken::with_api_key(&api_key, &api_secret)
        .with_sha256(&hash_b64)
        .to_jwt()
        .expect("Failed to create JWT");

    println!("=== LiveKit Webhook Signature Tool ===\n");
    println!("API Key: {}", api_key);
    println!("SHA256 Hash (base64): {}\n", hash_b64);

    println!("Payload:");
    println!("{}\n", serde_json::to_string_pretty(&payload).unwrap());

    println!("JWT Token:");
    println!("{}\n", token);

    println!("curl command:");
    println!("curl -X POST http://localhost:3001/livekit/webhook \\");
    println!("  -H 'Content-Type: application/json' \\");
    println!("  -H 'Authorization: Bearer {}' \\", token);
    println!("  -d '{}'", payload_str);
}
