# LiveKit Webhook Integration Design

## Overview

This document provides a comprehensive guide to implementing LiveKit webhook support in Sayna. It explains how LiveKit webhook signing works in the Rust ecosystem using the existing `livekit-api` and `livekit-protocol` crates, and specifies exactly what information we need from each webhook event.

This design brief serves as training material for engineers who have never worked with LiveKit before and provides a clear specification for implementation.

## Table of Contents

1. [Webhook Verification Mechanism](#webhook-verification-mechanism)
2. [HTTP Contract](#http-contract)
3. [Event Data Structures](#event-data-structures)
4. [Configuration Requirements](#configuration-requirements)
5. [Logging and Telemetry](#logging-and-telemetry)
6. [Implementation Checklist](#implementation-checklist)

---

## Webhook Verification Mechanism

### How LiveKit Signs Webhooks

LiveKit uses **JWT-based webhook signing** with a **double verification** mechanism to ensure webhook authenticity and integrity:

1. **JWT Signature**: The webhook request includes a JWT token in the `Authorization` header, signed with the LiveKit API secret using HMAC-SHA256
2. **Body Hash**: The JWT contains a SHA256 hash of the raw request body in the `sha256` claim

This dual-layer approach ensures both that the webhook came from LiveKit (JWT signature) and that the body wasn't tampered with (body hash).

### WebhookReceiver Implementation

The `livekit-api` crate (v0.4.6) provides `WebhookReceiver` in `src/webhooks.rs` that handles verification:

```rust
pub struct WebhookReceiver {
    token_verifier: TokenVerifier,
}

impl WebhookReceiver {
    pub fn receive(
        &self,
        body: &str,
        auth_token: &str,
    ) -> Result<proto::WebhookEvent, WebhookError>
}
```

**Key implementation details:**

1. **TokenVerifier** (`livekit-api/src/access_token.rs`):
   - Verifies the JWT using the LiveKit API secret (HMAC-SHA256)
   - Validates the following JWT claims:
     - `iss` (issuer): Must match the LiveKit API key
     - `exp` (expiration): Token must not be expired
     - `nbf` (not before): Token must be valid at current time
     - `sha256`: Contains base64-encoded SHA256 hash of the request body

2. **Body Hash Verification** (`livekit-api/src/webhooks.rs:51-57`):
   ```rust
   let mut hasher = Sha256::new();
   hasher.update(body);
   let hash = hasher.finalize();

   let claim_hash = base64::engine::general_purpose::STANDARD.decode(claims.sha256)?;
   if claim_hash[..] != hash[..] {
       return Err(WebhookError::InvalidSignature);
   }
   ```

3. **Why Both Are Required**:
   - **JWT signature** proves the webhook came from LiveKit (only they know the API secret)
   - **Body hash** proves the JSON body matches what LiveKit signed (prevents tampering between signing and delivery)
   - Without the body hash, an attacker could replay a valid JWT with modified body data

### Error Types

The `WebhookError` enum from `livekit-api/src/webhooks.rs` defines possible failure modes:

```rust
pub enum WebhookError {
    InvalidSignature,        // Body hash doesn't match sha256 claim
    InvalidBase64,           // sha256 claim isn't valid base64
    InvalidAuth,             // JWT verification failed (wrong secret, expired, wrong issuer)
    InvalidData,             // JSON parsing failed
}
```

### Critical Implementation Note

**The raw request body bytes must be read BEFORE parsing JSON** because the SHA256 hash is computed on the exact byte sequence. Once the body is consumed by a JSON parser, you cannot recreate the exact original bytes for verification.

**Correct approach (Axum):**
```rust
// Extract raw body bytes first
let body_bytes = axum::body::to_bytes(request.into_body(), usize::MAX).await?;
let body_str = std::str::from_utf8(&body_bytes)?;

// Verify with raw string
let event = webhook_receiver.receive(body_str, auth_token)?;
```

**Incorrect approach:**
```rust
// ❌ WRONG: Don't parse JSON before verification
let Json(data) = axum::extract::Json::<WebhookEvent>::from_request(req).await?;
// Now we've lost the raw body bytes and can't verify the hash!
```

---

## HTTP Contract

### Request Format

LiveKit delivers webhook events as HTTP POST requests with the following characteristics:

**Headers:**
- `Content-Type: application/json`
- `Authorization: Bearer <JWT_TOKEN>` - Contains the signed JWT token

**Body:**
- JSON-encoded `WebhookEvent` structure (from `livekit-protocol`)
- Must be verified against the SHA256 hash in the JWT before parsing

### Expected Response Codes

LiveKit interprets response codes as follows:

- **2xx (Success)**: Webhook processed successfully, no retry
- **Any other status (4xx, 5xx)**: Webhook delivery failed, LiveKit will retry with exponential backoff

**Implementation requirements:**

1. **Missing Authorization header**: Return `401 Unauthorized`
   ```json
   {"error": "Missing Authorization header"}
   ```

2. **Invalid JWT signature or expired token**: Return `401 Unauthorized`
   ```json
   {"error": "Invalid webhook signature"}
   ```

3. **Body hash mismatch**: Return `401 Unauthorized`
   ```json
   {"error": "Invalid webhook signature"}
   ```

4. **LiveKit not configured**: Return `503 Service Unavailable`
   ```json
   {"error": "LiveKit webhooks not configured"}
   ```
   (This allows LiveKit to retry in case the service is still starting up)

5. **Successful processing**: Return `200 OK`
   ```json
   {"status": "ok"}
   ```

### Open Questions

**Q: Does LiveKit always supply the Authorization header on retries?**

A: Yes, based on the LiveKit server source code, the JWT is regenerated for each retry attempt with a fresh timestamp and body hash. However, we should handle the missing header case gracefully as a defensive measure.

---

## Event Data Structures

### WebhookEvent Structure

From `livekit-protocol-0.4.0/src/livekit.rs:4753`:

```rust
pub struct WebhookEvent {
    /// Event type: one of room_started, room_finished, participant_joined,
    /// participant_left, track_published, track_unpublished, egress_started,
    /// egress_updated, egress_ended, ingress_started, ingress_ended
    pub event: String,

    /// Room information (always present for room_* and participant_* events)
    pub room: Option<Room>,

    /// Participant information (present for participant_* and track_* events)
    pub participant: Option<ParticipantInfo>,

    /// Track information (present for track_* events)
    pub track: Option<TrackInfo>,

    /// Egress information (present for egress_* events)
    pub egress_info: Option<EgressInfo>,

    /// Ingress information (present for ingress_* events)
    pub ingress_info: Option<IngressInfo>,

    /// Unique event UUID (always present)
    pub id: String,

    /// Event creation timestamp in seconds since Unix epoch (always present)
    pub created_at: i64,
}
```

### Room Structure

From `livekit-protocol-0.4.0/src/livekit.rs:212`:

```rust
pub struct Room {
    /// Unique room session ID (changes each time room is created)
    pub sid: String,

    /// Room name (stable identifier)
    pub name: String,

    /// User-defined metadata (JSON string, often contains custom application data)
    pub metadata: String,

    /// Room creation timestamp (seconds since Unix epoch)
    pub creation_time: i64,

    /// Room creation timestamp (milliseconds since Unix epoch)
    pub creation_time_ms: i64,

    /// Current number of participants
    pub num_participants: u32,

    /// Current number of publishers (participants publishing tracks)
    pub num_publishers: u32,

    /// Whether active recording is in progress
    pub active_recording: bool,

    // Other fields omitted (empty_timeout, max_participants, enabled_codecs, etc.)
}
```

**Fields we care about for logging:**
- `name`: The room identifier
- `metadata`: May contain application-specific context (should be logged for debugging)
- `num_participants`, `num_publishers`: Useful for understanding room state

### ParticipantInfo Structure

From `livekit-protocol-0.4.0/src/livekit.rs:297`:

```rust
pub struct ParticipantInfo {
    /// Unique participant session ID
    pub sid: String,

    /// Participant identity (stable identifier across reconnections)
    pub identity: String,

    /// Participant display name
    pub name: String,

    /// User-defined metadata (JSON string)
    pub metadata: String,

    /// Key-value attributes map (THIS IS WHERE SIP HEADERS LIVE)
    pub attributes: HashMap<String, String>,

    /// Participant kind (Standard, Ingress, Egress, Sip, Agent)
    pub kind: i32,  // See participant_info::Kind enum

    /// Connection state (Joining, Joined, Active, Disconnected)
    pub state: i32,  // See participant_info::State enum

    /// Timestamp when participant joined (seconds)
    pub joined_at: i64,

    /// Timestamp when participant joined (milliseconds)
    pub joined_at_ms: i64,

    // Other fields omitted (tracks, permission, region, etc.)
}
```

**Participant Kind Enum** (from `livekit.rs:375`):
```rust
pub enum Kind {
    Standard = 0,  // Web clients
    Ingress = 1,   // Only ingests streams
    Egress = 2,    // Only consumes streams
    Sip = 3,       // SIP participants (IMPORTANT for us!)
    Agent = 4,     // LiveKit agents
}
```

**Critical Field: `attributes` HashMap**

This is where **SIP-related metadata** is stored when a participant connects via SIP trunk:

- `sip.callID`: SIP call identifier
- `sip.trunkPhoneNumber`: Phone number from the SIP trunk
- `sip.phoneNumber`: Caller's phone number
- `sip.fromUser`: SIP From header user part
- `sip.toUser`: SIP To header user part
- `sip.fromHost`: SIP From header host part
- `sip.toHost`: SIP To header host part

**Example attributes for SIP participant:**
```rust
{
    "sip.callID": "abc123-def456",
    "sip.trunkPhoneNumber": "+15551234567",
    "sip.phoneNumber": "+15559876543",
    "sip.fromUser": "alice",
    "sip.toUser": "bob"
}
```

---

## Configuration Requirements

### ServerConfig Fields

From `src/config/mod.rs:40-82`, the following `ServerConfig` fields are required for webhook verification:

```rust
pub struct ServerConfig {
    // LiveKit settings
    pub livekit_url: String,              // e.g., "ws://localhost:7880"
    pub livekit_public_url: String,       // e.g., "http://localhost:7880"
    pub livekit_api_key: Option<String>,  // Required for webhooks
    pub livekit_api_secret: Option<String>, // Required for webhooks

    // ... other fields ...
}
```

### Building WebhookReceiver

The `WebhookReceiver` can be constructed on-demand from the config (no persistent state needed):

```rust
use livekit_api::access_token::TokenVerifier;
use livekit_api::webhooks::WebhookReceiver;

fn create_webhook_receiver(config: &ServerConfig) -> Result<WebhookReceiver, String> {
    let api_key = config.livekit_api_key.as_ref()
        .ok_or("LiveKit API key not configured")?;
    let api_secret = config.livekit_api_secret.as_ref()
        .ok_or("LiveKit API secret not configured")?;

    let verifier = TokenVerifier::with_api_key(api_key, api_secret);
    Ok(WebhookReceiver::new(verifier))
}
```

### Configuration Validation

The webhook endpoint should:

1. **Return 503 Service Unavailable** if `livekit_api_key` or `livekit_api_secret` are not configured
2. **Log a warning** at startup if LiveKit credentials are missing (webhook endpoint won't work)
3. **Not require** LiveKit config for other endpoints (webhooks are optional feature)

---

## Logging and Telemetry

### Logging Guidelines

Per `.cursor/rules/axum.mdc` and `.cursor/rules/rust.mdc`, use the `tracing` crate with appropriate levels:

**Success Case (info level):**
```rust
tracing::info!(
    event_id = %event.id,
    event_type = %event.event,
    room_name = ?event.room.as_ref().map(|r| &r.name),
    participant_identity = ?event.participant.as_ref().map(|p| &p.identity),
    participant_kind = ?event.participant.as_ref().map(|p| p.kind),
    "Received LiveKit webhook event"
);
```

**SIP Attributes (info level):**

For events with `participant.kind == Kind::Sip`, log all SIP-related attributes:

```rust
if let Some(participant) = &event.participant {
    if participant.kind == livekit_protocol::participant_info::Kind::Sip as i32 {
        // Log SIP-specific attributes
        for (key, value) in &participant.attributes {
            if key.starts_with("sip.") {
                tracing::info!(
                    event_id = %event.id,
                    participant_identity = %participant.identity,
                    attribute_key = %key,
                    attribute_value = %value,
                    "SIP attribute"
                );
            }
        }
    }
}
```

**Validation Failures (warn level):**
```rust
tracing::warn!(
    error = %err,
    "Webhook signature verification failed"
);
```

**Configuration Errors (error level):**
```rust
tracing::error!(
    "LiveKit webhooks disabled: API key or secret not configured"
);
```

### What to Log

#### Always Log (for all events):
- `event.id`: Unique event identifier
- `event.event`: Event type string (e.g., "participant_joined")
- `event.created_at`: Event timestamp
- `room.name`: Room identifier (if present)
- `room.metadata`: Room custom metadata (if present)
- `participant.identity`: Participant stable identifier (if present)
- `participant.name`: Participant display name (if present)
- `participant.kind`: Participant type enum value (if present)

#### Additionally Log for SIP Participants:
- All `participant.attributes` entries where key starts with `sip.`
- Common keys: `sip.callID`, `sip.trunkPhoneNumber`, `sip.phoneNumber`, `sip.fromUser`, `sip.toUser`

#### Log Validation Failures:
- Missing `Authorization` header
- JWT verification failures (invalid signature, expired, wrong issuer)
- Body hash mismatches
- JSON parsing errors

### Structured Logging Format

Use `tracing` field syntax for structured logs:

```rust
tracing::info!(
    event_id = %event.id,                    // Display formatting
    event_type = %event.event,
    room_name = ?event.room.as_ref().map(|r| &r.name),  // Debug formatting for Option
    participant_identity = ?event.participant.as_ref().map(|p| &p.identity),
    sip_trunk_number = ?event.participant.as_ref()
        .and_then(|p| p.attributes.get("sip.trunkPhoneNumber")),
    "Processed webhook event successfully"
);
```

---

## Implementation Checklist

### Phase 1: Basic Webhook Endpoint
- [ ] Create `POST /livekit/webhook` endpoint in `src/handlers/livekit.rs`
- [ ] Extract raw body bytes before any parsing
- [ ] Extract JWT from `Authorization: Bearer <token>` header
- [ ] Build `WebhookReceiver` from `ServerConfig` (on-demand, not stored)
- [ ] Call `webhook_receiver.receive(body_str, auth_token)`
- [ ] Return appropriate HTTP status codes (200, 401, 503)

### Phase 2: Event Processing
- [ ] Parse successful webhook into `livekit_protocol::WebhookEvent`
- [ ] Log event metadata (id, event type, room name)
- [ ] Log participant metadata (identity, name, kind)
- [ ] Detect SIP participants (`kind == Kind::Sip`)
- [ ] Log all `sip.*` attributes for SIP participants

### Phase 3: Error Handling
- [ ] Handle missing `Authorization` header → 401
- [ ] Handle invalid JWT signature → 401
- [ ] Handle body hash mismatch → 401
- [ ] Handle missing LiveKit config → 503
- [ ] Handle JSON parsing errors → 401
- [ ] Log all failure cases at appropriate levels (warn/error)

### Phase 4: Testing
- [ ] Unit test: Verify valid webhook with correct signature
- [ ] Unit test: Reject webhook with invalid signature
- [ ] Unit test: Reject webhook with tampered body
- [ ] Unit test: Reject webhook with missing Authorization header
- [ ] Unit test: Return 503 when LiveKit not configured
- [ ] Integration test: End-to-end webhook delivery (may require LiveKit test server)

### Phase 5: Documentation
- [ ] Update `CLAUDE.md` with webhook endpoint documentation
- [ ] Add example webhook payload to `docs/` or inline comments
- [ ] Document required environment variables (`LIVEKIT_API_KEY`, `LIVEKIT_API_SECRET`)

---

## Event Types Reference

From `livekit-protocol/src/livekit.rs:4754-4756`, LiveKit sends the following event types:

**Room Events:**
- `room_started`: New room session created
- `room_finished`: Room session ended

**Participant Events:**
- `participant_joined`: Participant connected to room
- `participant_left`: Participant disconnected from room

**Track Events:**
- `track_published`: Participant published a new media track
- `track_unpublished`: Participant unpublished a media track

**Egress Events:**
- `egress_started`: Recording/streaming egress started
- `egress_updated`: Egress status changed
- `egress_ended`: Recording/streaming completed

**Ingress Events:**
- `ingress_started`: Ingress stream started
- `ingress_ended`: Ingress stream ended

---

## Security Considerations

1. **Always verify webhooks**: Never trust webhook data without signature verification
2. **Use constant-time comparison**: The `livekit-api` crate already does this for hash comparison
3. **Rate limiting**: Consider adding rate limiting to the webhook endpoint to prevent abuse
4. **Log security events**: Log all authentication failures for security monitoring
5. **Don't expose internal errors**: Return generic "Invalid webhook signature" messages to external callers

---

## Example Usage

### Creating WebhookReceiver

```rust
use livekit_api::access_token::TokenVerifier;
use livekit_api::webhooks::WebhookReceiver;

let verifier = TokenVerifier::with_api_key(
    &config.livekit_api_key.unwrap(),
    &config.livekit_api_secret.unwrap()
);
let receiver = WebhookReceiver::new(verifier);
```

### Processing Webhook

```rust
match receiver.receive(body_str, auth_token) {
    Ok(event) => {
        tracing::info!(
            event_id = %event.id,
            event_type = %event.event,
            "Webhook verified successfully"
        );
        // Process event...
        StatusCode::OK
    }
    Err(WebhookError::InvalidAuth(_)) => {
        tracing::warn!("Invalid webhook signature");
        StatusCode::UNAUTHORIZED
    }
    Err(e) => {
        tracing::error!(error = ?e, "Webhook processing failed");
        StatusCode::UNAUTHORIZED
    }
}
```

---

## References

- `livekit-api` v0.4.6: `/home/tigran/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/livekit-api-0.4.6/`
- `livekit-protocol` v0.4.0: `/home/tigran/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/livekit-protocol-0.4.0/`
- LiveKit Documentation: https://docs.livekit.io/home/server/webhooks/
- Sayna Configuration: `src/config/mod.rs`
- Axum Best Practices: `.cursor/rules/axum.mdc`
- Rust Best Practices: `.cursor/rules/rust.mdc`
