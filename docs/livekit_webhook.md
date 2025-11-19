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
6. [SIP Webhook Forwarding](#sip-webhook-forwarding)
7. [Implementation Checklist](#implementation-checklist)

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
   {"status": "received"}
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
    event_name = %event.event,
    room_name = event.room.as_ref().map(|r| r.name.as_str()),
    participant_identity = event.participant.as_ref().map(|p| p.identity.as_str()),
    participant_name = event.participant.as_ref().map(|p| p.name.as_str()),
    participant_kind = ?event.participant.as_ref().map(|p| p.kind),
    sip_attributes = ?participant.map(extract_sip_attributes),
    sip_domain = participant
        .and_then(|p| p.attributes.get("sip.h.to"))
        .and_then(|header| parse_sip_domain(header)),
    "Received LiveKit webhook event (no side effects applied yet)"
);
```

Instead of logging every SIP attribute individually, the implementation records a structured `HashMap` of keys that start with `sip.` along with the parsed SIP domain (if `sip.h.to` is present). This keeps logs readable while still preserving all SIP metadata for debugging.

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
- A structured map of all attributes whose key starts with `sip.` (recorded as `sip_attributes` in logs)
- The parsed SIP domain derived from `sip.h.to` (recorded as `sip_domain`)

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

## SIP Webhook Forwarding

### Overview

Sayna supports **automatic forwarding of LiveKit webhook events to downstream services** based on the SIP domain in the `sip.h.to` participant attribute. This enables per-domain automation and integration with external systems when SIP calls are received.

**Use Case**: When a SIP participant joins a LiveKit room, Sayna can automatically notify a domain-specific webhook endpoint (e.g., `webhook.example.com`) based on the destination SIP domain from the call's `To` header.

### How It Works

1. **Receive webhook** from LiveKit at `POST /livekit/webhook`
2. **Verify signature** and extract participant attributes
3. **Parse SIP domain** from `sip.h.to` attribute (e.g., `sip:user@example.com` → `example.com`)
4. **Look up hook configuration** for the extracted domain
5. **Forward event** to the configured webhook URL asynchronously (non-blocking)
6. **Return 200 OK** to LiveKit immediately (doesn't wait for downstream hook)

> **Opt-in behavior:** if the `sip` configuration block is absent or contains zero hooks, Step 4 is skipped and no forwarding task is spawned. Non-SIP deployments therefore incur zero overhead from this feature.

### Configuration

SIP webhook forwarding is configured via the `sip` section in `config.yaml`:

```yaml
sip:
  room_prefix: "sip-"
  allowed_addresses:
    - "192.168.1.0/24"
  hooks:
    - host: "example.com"
      url: "https://webhook.example.com/livekit-events"
    - host: "test.org"
      url: "https://webhook.test.org/sip-notifications"
```

**Configuration Notes:**

- **`host`**: The SIP domain to match (case-insensitive, lowercase normalized)
- **`url`**: HTTPS endpoint to forward webhook events to
- Multiple hooks can be configured for different domains
- Hooks are matched case-insensitively against parsed SIP domains

### SIP Domain Parsing

The `parse_sip_domain` helper function extracts the domain from various SIP header formats:

**Supported Formats:**

- Simple: `sip:user@example.com` → `example.com`
- Angle brackets: `"User Name" <sip:user@example.com>` → `example.com`
- URI parameters: `sip:user@example.com;user=phone;tag=xyz` → `example.com`
- Secure SIP: `sips:user@secure.example.com` → `secure.example.com`
- With port: `sip:user@example.com:5060` → `example.com:5060`

**Normalization:**

- Domains are converted to lowercase for case-insensitive matching
- URI parameters (everything after `;`) are stripped before extraction
- Leading/trailing whitespace is trimmed

### HTTP Request Format

When forwarding webhooks, Sayna sends an HTTP POST request with:

**Headers:**
```
Content-Type: application/json
```

**Body:**
- The **exact JSON payload** received from LiveKit (forwarded verbatim)
- This allows downstream services to parse the canonical `WebhookEvent` structure

**Timeout:**
- 5 seconds per request
- Failed requests are logged but don't block LiveKit response

**Example Forwarded Request:**

```bash
POST https://webhook.example.com/livekit-events
Content-Type: application/json

{
  "event": "participant_joined",
  "id": "evt_abc123",
  "createdAt": 1700000000,
  "room": { ... },
  "participant": {
    "sid": "PA_sip123",
    "identity": "sip-caller-456",
    "attributes": {
      "sip.h.to": "sip:customer@example.com",
      "sip.trunkPhoneNumber": "+15551234567",
      "sip.callID": "call-xyz789"
    },
    ...
  }
}
```

### Connection Pooling with ReqManager

Sayna uses the existing **`ReqManager`** infrastructure (from `src/utils/req_manager.rs`) for webhook forwarding:

**Benefits:**

- **Per-domain connection pooling**: Reuses HTTP/2 connections for multiple webhooks to the same host
- **Concurrency limits**: Prevents overwhelming downstream services (default: 3 concurrent requests per host)
- **Automatic retries**: Built-in exponential backoff for transient failures
- **Metrics tracking**: Request counts, success/failure rates, and latencies

**ReqManager Lifecycle:**

1. On first webhook to a domain, create a new `ReqManager` instance
2. Cache it in `AppState.core_state.tts_req_managers` with key `sip-hook-{host}`
3. Reuse for subsequent webhooks to the same host
4. Manager persists for the lifetime of the server

**Implementation Note:**

The `get_or_create_hook_manager` function in `src/handlers/livekit_webhook.rs:307` handles lazy initialization and caching of `ReqManager` instances.

### Logging

**Successful Forwarding (info level):**

```
INFO Successfully forwarded webhook to SIP hook
  event_id="evt_abc123"
  domain="example.com"
  hook_url="https://webhook.example.com/livekit-events"
  status=200
  duration_ms=42
```

**Failed Forwarding (warn level):**

```
WARN SIP forwarding failed: webhook returned non-2xx status
  event_id="evt_abc123"
  room_name=Some("sip-room")
  sip_domain="example.com"
  status=500
  response_body="Internal Server Error: ..."
```

**Skipped Forwarding (debug level):**

```
DEBUG Skipping SIP forwarding: event has no participant
  event_id="evt_room_finished"
  event_type="room_finished"
  room_name=Some("sip-room")

DEBUG Skipping SIP forwarding: participant has no sip.h.to attribute
  event_id="evt_participant_joined"
  room_name=Some("test-room")
```

**Malformed SIP Header (info level):**

```
INFO Skipping SIP forwarding: malformed sip.h.to header (check upstream SIP gateway)
  event_id="evt_abc123"
  room_name=Some("sip-room")
  sip_header="sip:broken@"
```

**Configuration Issues (warn level):**

```
WARN SIP forwarding failed: no webhook configured for domain (add to sip.hooks config)
  event_id="evt_abc123"
  room_name=Some("sip-room")
  sip_domain="unknown.com"
```

**ReqManager Creation (info level):**

```
INFO Created new ReqManager for SIP webhook forwarding
  provider_key="sip-hook-webhook.example.com"
  url="https://webhook.example.com/livekit-events"
```

### Error Handling

Webhook forwarding is **non-blocking** and **best-effort**:

1. **No SIP config**: Forwarding is skipped silently
2. **No participant in event**: Forwarding is skipped (e.g., `room_finished` events)
3. **No `sip.h.to` attribute**: Forwarding is skipped (non-SIP participants)
4. **Malformed SIP domain**: Logged at debug level, forwarding skipped
5. **No matching hook**: Logged at debug level with domain name
6. **HTTP request failure**: Logged at warn level with error details
7. **Non-2xx response**: Logged at warn level with status and body

**Important**: All errors are logged but **do not affect the 200 OK response to LiveKit**. The main webhook handler always returns immediately to prevent LiveKit retries.

### Troubleshooting SIP Forwarding

This section helps operators diagnose and fix SIP webhook forwarding issues using log messages.

#### Log Severity Levels and Actions

**DEBUG Level** (expected, no action needed):
- `Skipping SIP forwarding: event has no participant` - Normal for `room_finished` or other non-participant events
- `Skipping SIP forwarding: participant has no sip.h.to attribute` - Normal for regular (non-SIP) participants

**INFO Level** (upstream issue, not our fault):
- `Skipping SIP forwarding: malformed sip.h.to header` - The SIP gateway sent an invalid SIP URI
  - **Action**: Check your SIP trunk provider's configuration
  - **Example**: Header value is logged for debugging upstream issues

**WARN Level** (requires operator attention):
- `SIP forwarding failed: no webhook configured for domain` - Missing hook configuration
  - **Action**: Add the domain to `sip.hooks` in your config file
  - **Example**: If you see `sip_domain="customer.example.com"`, add:
    ```yaml
    sip:
      hooks:
        - host: "customer.example.com"
          url: "https://webhook.customer.example.com/livekit-events"
    ```

- `SIP forwarding failed: HTTP request error` - Network/connectivity issue
  - **Action**: Check DNS, firewall rules, and webhook endpoint availability
  - **Common causes**: Timeout, connection refused, DNS lookup failed
  - **Debug steps**:
    1. Verify the webhook URL is accessible: `curl -X POST <hook_url>`
    2. Check DNS resolution: `dig <domain>`
    3. Check firewall rules for outbound HTTPS

- `SIP forwarding failed: webhook returned non-2xx status` - Downstream hook rejected the request
  - **Action**: Check the webhook endpoint logs for errors
  - **Example**: If `status=500`, the downstream service has an internal error
  - **Debug steps**:
    1. Check `response_body` field for error details
    2. Review webhook endpoint logs
    3. Verify the endpoint can handle LiveKit's JSON schema

- `SIP forwarding failed: HTTP client error` - ReqManager pool exhaustion or configuration error
  - **Action**: Check server resource limits and connection pool settings
  - **Rare**: Usually indicates high load or misconfiguration

#### Common Scenarios

**Scenario 1: "No webhook configured for domain: unknown.example.com"**
```
WARN SIP forwarding failed: no webhook configured for domain (add to sip.hooks config)
  event_id="evt_abc123"
  sip_domain="unknown.example.com"
```
**Solution**: Add `unknown.example.com` to your SIP hooks configuration:
```yaml
sip:
  hooks:
    - host: "unknown.example.com"
      url: "https://webhook.unknown.example.com/events"
```

**Scenario 2: "malformed sip.h.to header"**
```
INFO Skipping SIP forwarding: malformed sip.h.to header (check upstream SIP gateway)
  sip_header="sip:broken@"
```
**Solution**: This indicates a bug in your SIP trunk provider's header formatting. Contact your SIP provider with the logged `sip_header` value.

**Scenario 3: "HTTP request error: connection timeout"**
```
WARN SIP forwarding failed: HTTP request error (check network/DNS/webhook endpoint)
  sip_domain="example.com"
  error="connection timeout after 5s"
```
**Solution**: The webhook endpoint is unreachable or responding slowly:
1. Test connectivity: `curl -m 5 https://webhook.example.com/events`
2. Check webhook endpoint health/uptime
3. Verify firewall rules allow outbound HTTPS from Sayna server

**Scenario 4: "webhook returned non-2xx status: 503"**
```
WARN SIP forwarding failed: webhook returned non-2xx status
  sip_domain="example.com"
  status=503
  response_body="Service temporarily unavailable"
```
**Solution**: The downstream webhook is having issues:
1. Check webhook service logs for errors
2. Verify webhook service is running and healthy
3. Wait for webhook service to recover (Sayna will retry on next event)

#### Searchable Log Patterns

Use these patterns with log aggregation tools (e.g., grep, Splunk, Datadog):

- **All SIP forwarding failures**: `grep "SIP forwarding failed" logs.txt`
- **Missing hook configs**: `grep "no webhook configured for domain" logs.txt`
- **HTTP errors by domain**: `grep "sip_domain=\"example.com\"" logs.txt | grep "WARN"`
- **Malformed SIP headers**: `grep "malformed sip.h.to header" logs.txt`
- **Successful forwards**: `grep "Successfully forwarded webhook to SIP hook" logs.txt`

#### Metrics and Alerting Recommendations

Set up alerts for these log patterns to detect issues proactively:

1. **Alert on missing hook configs** (WARN level):
   - Pattern: `"no webhook configured for domain"`
   - Severity: Medium
   - Action: Add missing domain to configuration

2. **Alert on HTTP failures** (WARN level):
   - Pattern: `"HTTP request error"` or `"webhook returned non-2xx"`
   - Severity: High
   - Action: Check webhook endpoint health

3. **Monitor successful forwarding rate** (INFO level):
   - Pattern: `"Successfully forwarded webhook to SIP hook"`
   - Metric: Count per minute/hour
   - Alert: If rate drops to zero for extended period

### Background Task Execution

Webhook forwarding runs in a **spawned Tokio task** (`tokio::spawn`) to avoid blocking the main handler:

```rust
// Short-circuit: Only spawn forwarding task if SIP config exists and has hooks
if let Some(sip_config) = &state.config.sip {
    if !sip_config.hooks.is_empty() {
        tokio::spawn(async move {
            if let Err(e) = forward_to_sip_hook(&state, &sip_config, &event, &body_json).await {
                debug!("Webhook forwarding failed: {}", e);
            }
        });
    }
}
```

**Benefits:**

- LiveKit receives 200 OK response in <10ms (signature verification time)
- Downstream webhook forwarding happens asynchronously
- Slow or failing downstream hooks don't delay LiveKit retries
- Multiple hooks can be called concurrently if needed (future enhancement)

**Short-Circuit Behavior:**

When `config.sip` is `None` or the hooks list is empty, the forwarding task is **never spawned**. This ensures:

- Pure non-SIP deployments don't incur unnecessary background tasks
- No "No SIP configuration" debug logs clutter the output
- Zero overhead for users who haven't configured SIP
- The webhook endpoint remains fully functional for all LiveKit events

### Testing Webhook Forwarding

**Unit Tests** (in `tests/livekit_webhook_test.rs`):

- ✅ `parse_sip_domain` helper with various header formats
- ✅ Domain extraction from SIP URIs with parameters, angle brackets, ports
- ✅ Case-insensitive domain matching
- ✅ Malformed SIP URI handling

**Manual Testing:**

Since webhook forwarding is non-blocking and runs in the background, manual testing requires:

1. **Configure SIP hooks** in `config.yaml`
2. **Set up a webhook receiver** (e.g., https://webhook.site or a local HTTP server)
3. **Send test webhook** using the `scripts/test_webhook.sh` script with SIP attributes
4. **Check logs** for forwarding success/failure messages
5. **Verify webhook receiver** received the exact LiveKit JSON payload

**Example Test Setup:**

```yaml
# config.yaml
sip:
  room_prefix: "sip-"
  allowed_addresses: []
  hooks:
    - host: "example.com"
      url: "https://webhook.site/your-unique-url"
```

```bash
# Send test webhook with sip.h.to attribute
cargo run --example sign_webhook
# Copy the curl command and run it
```

**E2E Testing (Future):**

Integration tests with `wiremock` or `mockito` to mock HTTP servers and verify:

- Correct payload forwarding
- Proper header inclusion
- Timeout handling
- Retry behavior

### Security Considerations

1. **No authentication**: Forwarded webhooks do NOT include LiveKit's JWT signature
   - Downstream services should verify requests using IP allowlisting or shared secrets
2. **HTTPS recommended**: Use HTTPS for webhook URLs to prevent eavesdropping
3. **Timeout protection**: 5-second timeout prevents hanging on slow downstream services
4. **Rate limiting**: ReqManager limits concurrent requests per host (default: 3)
5. **Sensitive data**: Be cautious when logging SIP attributes (phone numbers may be PII)

---

## Implementation Checklist

### Phase 1: Basic Webhook Endpoint ✅
- [x] Create `POST /livekit/webhook` endpoint in `src/handlers/livekit_webhook.rs`
- [x] Extract raw body bytes before any parsing
- [x] Extract JWT from `Authorization: Bearer <token>` header
- [x] Build `WebhookReceiver` from `ServerConfig` (on-demand, not stored)
- [x] Call `webhook_receiver.receive(body_str, auth_token)`
- [x] Return appropriate HTTP status codes (200, 401, 503)
- [x] Create dedicated webhook router in `src/routes/webhooks.rs`
- [x] Integrate webhook router in `src/main.rs` without auth middleware

### Phase 2: Event Processing ✅
- [x] Parse successful webhook into `livekit_protocol::WebhookEvent`
- [x] Log event metadata (id, event type, room name)
- [x] Log participant metadata (identity, name, kind)
- [x] Detect SIP participants (`kind == Kind::Sip`)
- [x] Log all `sip.*` attributes for SIP participants

### Phase 3: Error Handling ✅
- [x] Handle missing `Authorization` header → 401
- [x] Handle invalid JWT signature → 401
- [x] Handle body hash mismatch → 401
- [x] Handle missing LiveKit config → 503
- [x] Handle JSON parsing errors → 401
- [x] Log all failure cases at appropriate levels (warn/error)

### Phase 4: Testing ✅
- [x] Unit test: Verify valid webhook with correct signature
- [x] Unit test: Reject webhook with invalid signature
- [x] Unit test: Reject webhook with tampered body
- [x] Unit test: Reject webhook with missing Authorization header
- [x] Unit test: Return 503 when LiveKit not configured
- [x] Integration test: End-to-end webhook delivery (with mocked LiveKit signatures)
- [x] Unit test: SIP attribute extraction
- [x] Unit test: Error status code mapping

### Phase 5: Documentation ✅
- [x] Update `CLAUDE.md` with webhook endpoint documentation
- [x] Add example webhook payload to `docs/` or inline comments
- [x] Document required environment variables (`LIVEKIT_API_KEY`, `LIVEKIT_API_SECRET`)
- [x] Add developer testing tools documentation

### Phase 6: SIP Webhook Forwarding ✅
- [x] Implement `parse_sip_domain` helper function to extract domain from SIP headers
- [x] Add `forward_to_sip_hook` function to route webhooks based on SIP domain
- [x] Integrate ReqManager for per-domain connection pooling
- [x] Add `get_or_create_hook_manager` for lazy ReqManager initialization
- [x] Update `log_webhook_event` to include parsed SIP domain
- [x] Spawn background task for non-blocking webhook forwarding
- [x] Add comprehensive unit tests for `parse_sip_domain` (10+ test cases)
- [x] Update documentation in `docs/livekit_webhook.md` with SIP forwarding section

---

## Developer Testing Guide

### Running the Test Suite

All webhook tests are located in `tests/livekit_webhook_test.rs`. Run them with:

```bash
# Run all webhook tests
cargo test --test livekit_webhook_test

# Run specific test
cargo test --test livekit_webhook_test test_webhook_success

# Run with verbose output
cargo test --test livekit_webhook_test -- --nocapture
```

### Test Coverage

The test suite covers:

**Integration Tests:**
- ✅ Valid webhook with correct signature → 200 OK
- ✅ Valid webhook with SIP attributes → 200 OK (logs SIP data)
- ✅ Missing Authorization header → 401 Unauthorized
- ✅ Empty Authorization token → 401 Unauthorized
- ✅ Invalid signature (wrong secret) → 401 Unauthorized
- ✅ Hash mismatch (tampered body) → 401 Unauthorized
- ✅ Missing LiveKit credentials → 503 Service Unavailable
- ✅ Invalid UTF-8 body → 400 Bad Request
- ✅ Bearer prefix optional (works with and without "Bearer ")

**Unit Tests:**
- ✅ SIP attribute extraction (filters keys starting with "sip.")
- ✅ Error status code mapping (ensures correct LiveKit retry behavior)

### Manual Testing with curl

To test the webhook endpoint locally, you need to:

1. **Generate a signed webhook payload** (see script below)
2. **Send it via curl** with the correct Authorization header

#### Testing Script

Create `scripts/test_webhook.sh`:

```bash
#!/bin/bash

# Configuration
API_KEY="${LIVEKIT_API_KEY:-test-api-key}"
API_SECRET="${LIVEKIT_API_SECRET:-test-api-secret}"
WEBHOOK_URL="${WEBHOOK_URL:-http://localhost:3001/livekit/webhook}"

# Sample webhook payload (participant_joined event)
PAYLOAD=$(cat <<'EOF'
{
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
}
EOF
)

echo "Payload:"
echo "$PAYLOAD" | jq .

# Compute SHA256 hash of payload
HASH=$(echo -n "$PAYLOAD" | sha256sum | awk '{print $1}' | xxd -r -p | base64)

echo ""
echo "SHA256 Hash (base64): $HASH"

# Generate signed JWT using livekit-cli (install with: cargo install livekit-cli)
# Alternatively, use the Rust test helpers to generate the token
TOKEN=$(livekit-cli token create \
  --api-key "$API_KEY" \
  --api-secret "$API_SECRET" \
  --sha256 "$HASH")

echo "JWT Token: $TOKEN"

# Send webhook request
echo ""
echo "Sending webhook to: $WEBHOOK_URL"
curl -X POST "$WEBHOOK_URL" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d "$PAYLOAD" \
  -v

echo ""
```

Make it executable:

```bash
chmod +x scripts/test_webhook.sh
```

#### Simpler Testing with Rust Helper

Alternatively, create a small Rust program in `examples/sign_webhook.rs`:

```rust
use base64::Engine;
use livekit_api::access_token::AccessToken;
use serde_json::json;
use sha2::{Digest, Sha256};

fn main() {
    let api_key = std::env::var("LIVEKIT_API_KEY").unwrap_or("test-api-key".to_string());
    let api_secret = std::env::var("LIVEKIT_API_SECRET").unwrap_or("test-api-secret".to_string());

    // Sample payload
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
    let hash_b64 = base64::engine::general_purpose::STANDARD.encode(&hash);

    // Create signed token
    let token = AccessToken::with_api_key(&api_key, &api_secret)
        .with_sha256(&hash_b64)
        .to_jwt()
        .expect("Failed to create JWT");

    println!("Payload:");
    println!("{}", serde_json::to_string_pretty(&payload).unwrap());
    println!("\nSHA256 Hash (base64): {}", hash_b64);
    println!("\nJWT Token: {}", token);
    println!("\ncurl command:");
    println!("curl -X POST http://localhost:3001/livekit/webhook \\");
    println!("  -H 'Content-Type: application/json' \\");
    println!("  -H 'Authorization: Bearer {}' \\", token);
    println!("  -d '{}'", payload_str);
}
```

Run it:

```bash
cargo run --example sign_webhook
```

### Verifying Logs

When you send a webhook with SIP attributes, check the logs for:

```
INFO  Received LiveKit webhook event (no side effects applied yet)
  event_id="test-event-123"
  event_name="participant_joined"
  room_name=Some("test-room")
  participant_identity=Some("user-123")
  participant_name=Some("Test User")
  participant_kind=Some(0)
  sip_attributes={"sip.callID": "abc123def456", "sip.fromHeader": "Test User <sip:user@example.com>", "sip.trunkPhoneNumber": "+1234567890"}
```

### Common Testing Mistakes

1. **Hash Mismatch**: If you modify the payload after computing the hash, the signature won't verify
   - Solution: Always compute hash from the exact payload you send

2. **Wrong API Secret**: Using different secrets for signing and verification
   - Solution: Ensure `LIVEKIT_API_KEY` and `LIVEKIT_API_SECRET` match in both client and server

3. **Bearer Prefix**: Some tools add "Bearer " automatically, others don't
   - Solution: Our handler strips the prefix if present, so both work

4. **Whitespace in JSON**: Extra whitespace changes the hash
   - Solution: Use compact JSON (no pretty-printing) for production webhooks

### Testing Against Real LiveKit

To test with a real LiveKit server:

1. **Configure LiveKit to send webhooks** to your Sayna instance:
   ```yaml
   # livekit.yaml
   webhook:
     urls:
       - http://your-sayna-server:3001/livekit/webhook
     api_key: your-api-key
   ```

2. **Start Sayna with matching credentials**:
   ```bash
   export LIVEKIT_API_KEY=your-api-key
   export LIVEKIT_API_SECRET=your-api-secret
   cargo run
   ```

3. **Trigger events** by joining a room, and watch Sayna logs for webhook deliveries

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
