# LiveKit Integration

## Overview

Sayna provides comprehensive LiveKit integration with automatic SIP infrastructure provisioning, bidirectional webhook handling, and secure event forwarding. This document covers everything you need to know about integrating Sayna with LiveKit for real-time voice processing and SIP telephony.

### Key Features

- **Inbound Webhook Verification**: Validates webhooks from LiveKit using JWT signature verification
- **SIP Auto-Provisioning**: Automatically creates LiveKit SIP trunks and dispatch rules on startup
- **Outbound Webhook Forwarding**: Routes SIP events to downstream services based on domain matching
- **Cryptographic Signing**: Signs outbound webhooks with HMAC-SHA256 for authenticity verification
- **Connection Pooling**: Efficient HTTP/2 connection management for webhook forwarding

---

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Configuration](#configuration)
3. [Inbound Webhooks (LiveKit → Sayna)](#inbound-webhooks-livekit--sayna)
4. [SIP Configuration & Auto-Provisioning](#sip-configuration--auto-provisioning)
5. [Outbound Webhooks (Sayna → Downstream)](#outbound-webhooks-sayna--downstream)
6. [Webhook Signing](#webhook-signing)
7. [Testing & Development](#testing--development)
8. [Operations & Troubleshooting](#operations--troubleshooting)
9. [Security Considerations](#security-considerations)

---

## Architecture Overview

### Component Diagram

```
                    ┌─────────────────┐
                    │  LiveKit Server │
                    └────────┬────────┘
                             │
                ┌────────────┼────────────┐
                │            │            │
        Webhooks│      Audio │      Token │
                │      Stream│    Generate│
                ▼            ▼            ▼
        ┌───────────────────────────────────┐
        │         Sayna Server              │
        │                                   │
        │  ┌─────────────────────────────┐ │
        │  │  Webhook Handler            │ │
        │  │  - Verify LiveKit signature │ │
        │  │  - Log event details        │ │
        │  │  - Extract SIP attributes   │ │
        │  └──────────┬──────────────────┘ │
        │             │                     │
        │  ┌──────────▼──────────────────┐ │
        │  │  SIP Event Forwarder        │ │
        │  │  - Parse SIP domain         │ │
        │  │  - Sign payload (HMAC)      │ │
        │  │  - Forward to downstream    │ │
        │  └──────────┬──────────────────┘ │
        │             │                     │
        │  ┌──────────▼──────────────────┐ │
        │  │  ReqManager (HTTP Pool)     │ │
        │  │  - Connection pooling       │ │
        │  │  - Retry with backoff       │ │
        │  └─────────────────────────────┘ │
        └───────────────┬───────────────────┘
                        │
            ┌───────────┼───────────┐
            │           │           │
            ▼           ▼           ▼
      ┌─────────┐ ┌─────────┐ ┌─────────┐
      │Customer │ │Customer │ │Customer │
      │   A     │ │   B     │ │   C     │
      │Webhook  │ │Webhook  │ │Webhook  │
      └─────────┘ └─────────┘ └─────────┘
```

### Data Flow

1. **SIP Call Initiated**: Caller dials into LiveKit SIP trunk
2. **LiveKit Creates Room**: Room named `{room_prefix}{phone_number}` (e.g., `sip-+15551234567`)
3. **Webhook Sent to Sayna**: LiveKit posts `participant_joined` event with SIP headers
4. **Signature Verification**: Sayna validates JWT signature from LiveKit
5. **Domain Extraction**: Parses SIP `To` header to extract target domain (e.g., `customer-a.com`)
6. **Hook Lookup**: Matches domain to configured webhook URL
7. **Payload Signing**: Generates HMAC-SHA256 signature with timestamp and event ID
8. **Asynchronous Forwarding**: Spawns background task to POST to customer webhook
9. **Connection Reuse**: Uses cached HTTP/2 connection pool for efficient delivery

---

## Configuration

### Required Environment Variables

```bash
# LiveKit Connection
LIVEKIT_URL=ws://localhost:7880           # WebSocket URL for audio streaming
LIVEKIT_PUBLIC_URL=http://localhost:7880  # Public HTTP URL
LIVEKIT_API_KEY=your-api-key              # Required for webhooks & provisioning
LIVEKIT_API_SECRET=your-api-secret        # Required for webhooks & provisioning
```

### SIP Configuration (Optional)

**YAML Configuration** (`config.yaml`):

```yaml
sip:
  # Room prefix for SIP calls (required)
  room_prefix: "sip-"

  # IP allowlist for SIP connections (required)
  allowed_addresses:
    - "192.168.1.0/24"   # CIDR ranges
    - "203.0.113.10"     # Individual IPs

  # Global signing secret for all webhooks (recommended)
  hook_secret: "your-secret-key-min-16-chars-recommended-32"

  # Downstream webhook targets (optional)
  hooks:
    - host: "customer-a.com"
      url: "https://webhook.customer-a.com/livekit-events"
      # Optional: per-hook secret override
      secret: "customer-a-specific-secret"

    - host: "customer-b.com"
      url: "https://webhook.customer-b.com/livekit-events"
      # Uses global hook_secret
```

**Environment Variables** (alternative):

```bash
# SIP Configuration
SIP_ROOM_PREFIX="sip-"
SIP_ALLOWED_ADDRESSES="192.168.1.0/24,203.0.113.10"
SIP_HOOK_SECRET="your-secret-key"

# Hooks (JSON array)
SIP_HOOKS_JSON='[
  {"host": "customer-a.com", "url": "https://webhook.customer-a.com/events"},
  {"host": "customer-b.com", "url": "https://webhook.customer-b.com/events"}
]'
```

### Configuration Priority

When both YAML and environment variables are present:

```
YAML Configuration > Environment Variables > Defaults
```

- YAML values take precedence over environment variables
- Per-hook secrets override global `hook_secret`
- Missing SIP config → SIP features disabled (webhook endpoint still works for non-SIP events)

### Validation Rules

**Room Prefix**:
- Cannot be empty when SIP config present
- Must contain only: alphanumeric, `-`, `_`
- Valid: `sip-`, `call_`, `room123`
- Invalid: `sip@`, `call#`, `room/name`

**Allowed Addresses**:
- Cannot be empty when SIP config present
- Must be valid IPv4 addresses or CIDR ranges
- Format: `X.X.X.X` or `X.X.X.X/Y`
- IPv6 not currently supported

**Hooks**:
- No duplicate hosts (case-insensitive)
- HTTPS required (HTTP rejected for security)
- Must have effective signing secret (per-hook or global)

**Signing Secrets**:
- Minimum 16 characters (32+ recommended)
- Automatically trimmed of whitespace
- Never logged or exposed in errors

---

## Inbound Webhooks (LiveKit → Sayna)

### Webhook Endpoint

**URL**: `POST /livekit/webhook`

**Authentication**: Unauthenticated endpoint (uses LiveKit's built-in JWT signature verification)

### Verification Mechanism

LiveKit uses **JWT-based webhook signing** with a **double verification** mechanism:

1. **JWT Signature**: The `Authorization` header contains a JWT signed with your LiveKit API secret using HMAC-SHA256
2. **Body Hash**: The JWT's `sha256` claim contains a SHA256 hash of the raw request body

This dual-layer approach ensures both authenticity (webhook came from LiveKit) and integrity (body wasn't tampered with).

### Request Format

**Headers**:
```
Content-Type: application/json
Authorization: Bearer <JWT_TOKEN>
```

**Body**: JSON-encoded `WebhookEvent` structure from `livekit-protocol`

### Response Codes

| Status Code | Meaning | LiveKit Behavior |
|-------------|---------|------------------|
| `200 OK` | Successfully processed | No retry |
| `401 Unauthorized` | Invalid signature or missing auth | No retry (auth issue) |
| `400 Bad Request` | Invalid data format | No retry (client error) |
| `503 Service Unavailable` | LiveKit not configured | Retry with backoff |
| `5xx Other` | Server error | Retry with backoff |

### Event Types

LiveKit sends the following webhook events:

**Room Events**:
- `room_started`: New room session created
- `room_finished`: Room session ended

**Participant Events**:
- `participant_joined`: Participant connected to room
- `participant_left`: Participant disconnected from room

**Track Events**:
- `track_published`: Participant published a new media track
- `track_unpublished`: Participant unpublished a media track

**Egress/Ingress Events**:
- `egress_started`, `egress_updated`, `egress_ended`: Recording/streaming events
- `ingress_started`, `ingress_ended`: Ingress stream events

### Event Data Structure

**WebhookEvent Structure**:

```json
{
  "event": "participant_joined",
  "id": "EVT_abc123",
  "createdAt": 1700000000,
  "room": {
    "sid": "RM_xyz789",
    "name": "sip-+15551234567",
    "metadata": "",
    "numParticipants": 2,
    "activeRecording": false
  },
  "participant": {
    "sid": "PA_sip123",
    "identity": "sip-caller-456",
    "name": "SIP User",
    "kind": 3,
    "state": 0,
    "attributes": {
      "sip.h.to": "sip:customer@example.com",
      "sip.trunkPhoneNumber": "+15551234567",
      "sip.phoneNumber": "+15559876543",
      "sip.callID": "abc123-def456"
    }
  }
}
```

**Participant Kind Values**:
- `0`: Standard (web clients)
- `1`: Ingress (stream ingest)
- `2`: Egress (stream consumer)
- `3`: **SIP participant** (phone calls)
- `4`: Agent (LiveKit agents)

### SIP Attributes

When a participant connects via SIP, their `attributes` map contains SIP metadata:

| Attribute Key | Description | Example |
|---------------|-------------|---------|
| `sip.h.x-to-ip` | Custom `X-To-IP` header for routing override (**priority 1**) | `sip:customer@custom-host.com` |
| `sip.h.to` | Full SIP `To` header (**priority 2**, fallback) | `sip:customer@example.com` |
| `sip.h.from` | Full SIP `From` header | `"Caller Name" <sip:user@provider.com>` |
| `sip.trunkPhoneNumber` | Phone number from trunk | `+15551234567` |
| `sip.phoneNumber` | Caller's phone number | `+15559876543` |
| `sip.callID` | SIP call identifier | `abc123-def456` |
| `sip.fromUser` | SIP From header user part | `alice` |
| `sip.toUser` | SIP To header user part | `bob` |

**SIP Host Header Priority**: When determining the target domain for webhook routing, Sayna checks attributes in this order:
1. `sip.h.x-to-ip` - Custom header that allows operators to override the routing domain
2. `sip.h.to` - Standard SIP To header (fallback if `sip.h.x-to-ip` is not present)

This priority allows SIP gateways to send a custom `X-To-IP` header when the standard `To` header doesn't contain the desired routing target.

### Logging

Sayna logs all webhook events with structured fields:

```
INFO Received LiveKit webhook event
  event_id="EVT_abc123"
  event_name="participant_joined"
  room_name=Some("sip-+15551234567")
  participant_identity=Some("sip-caller-456")
  participant_name=Some("SIP User")
  participant_kind=Some(3)
  sip_attributes={"sip.h.to": "sip:customer@example.com", ...}
  sip_domain=Some("example.com")
```

---

## SIP Configuration & Auto-Provisioning

### Overview

When SIP configuration is enabled and LiveKit API credentials are available, Sayna **automatically provisions** the required LiveKit infrastructure during startup. This ensures your LiveKit server is ready to receive and route SIP calls without manual setup.

### What Gets Provisioned

**SIP Inbound Trunk**:
- Name: `sayna-{room_prefix}-trunk` (e.g., `sayna-sip--trunk`)
- Allowed addresses: Configured IP allowlist from `sip.allowed_addresses`
- Header capture: Enabled (`SipAllHeaders`) to populate participant attributes

**SIP Dispatch Rule**:
- Name: `sayna-{room_prefix}-dispatch` (e.g., `sayna-sip--dispatch`)
- Type: Individual dispatch (creates rooms per caller)
- Room naming: `{room_prefix}{phone_number}` (e.g., `sip-+15551234567`)
- Max participants: Default 3 (caller + Sayna + optional third party)

### Provisioning Behavior

**Idempotent**: Running Sayna multiple times won't recreate existing resources. The provisioning code checks for existing trunks/dispatch rules by name and skips creation if they already exist.

**Fail-Fast**: If provisioning fails, Sayna refuses to start and logs a detailed error message. This prevents accepting SIP calls when infrastructure is incomplete.

**Deterministic Naming**: Resource names are generated from your `room_prefix`, making them predictable and easy to identify in the LiveKit dashboard.

### Required Credentials

To enable auto-provisioning:

```bash
export LIVEKIT_API_KEY="your-api-key"
export LIVEKIT_API_SECRET="your-api-secret"
export SIP_ROOM_PREFIX="sip-"
export SIP_ALLOWED_ADDRESSES="192.168.1.0/24"
```

### Monitoring Provisioning

**Success**:
```
INFO SIP configuration detected, provisioning LiveKit SIP trunk and dispatch rules
INFO Successfully provisioned SIP resources
  trunk="sayna-sip--trunk"
  dispatch="sayna-sip--dispatch"
  livekit_url="ws://localhost:7880"
```

**Skipped** (missing credentials):
```
INFO SIP configuration present but LiveKit API credentials missing
  message="Skipping SIP provisioning. Set LIVEKIT_API_KEY and LIVEKIT_API_SECRET to enable."
```

**Failed**:
```
ERROR Failed to provision SIP resources
  trunk="sayna-sip--trunk"
  dispatch="sayna-sip--dispatch"
  error="ConnectionFailed(...)"

PANIC SIP provisioning failed: ... Cannot start server with SIP enabled.
       Please check LiveKit API credentials and server availability.
```

### Verifying in LiveKit

After successful provisioning, verify in your LiveKit dashboard:

1. Navigate to **SIP** → **Inbound Trunks**
2. Find trunk: `sayna-{your-room-prefix}-trunk`
3. Navigate to **SIP** → **Dispatch Rules**
4. Find dispatch: `sayna-{your-room-prefix}-dispatch`

---

## Outbound Webhooks (Sayna → Downstream)

### Overview

Sayna automatically forwards LiveKit webhook events to downstream services based on the SIP domain extracted from participant attributes. The `sip.h.x-to-ip` attribute takes priority over `sip.h.to`, allowing operators to override the routing domain when needed. This enables per-domain automation and integration with external systems.

### How It Works

1. **Receive webhook** from LiveKit at `POST /livekit/webhook`
2. **Verify signature** using LiveKit's JWT mechanism
3. **Parse SIP domain** from SIP header attributes (`sip.h.x-to-ip` takes priority, falls back to `sip.h.to`)
4. **Look up hook configuration** for the extracted domain (case-insensitive)
5. **Forward event** to the configured webhook URL asynchronously (non-blocking)
6. **Return 200 OK** to LiveKit immediately (doesn't wait for downstream hook)

**Opt-in behavior**: If the `sip` configuration block is absent or contains zero hooks, forwarding is completely skipped. Non-SIP deployments incur zero overhead.

### SIP Domain Parsing

The domain parser handles various SIP header formats:

**Supported Formats**:
- Simple: `sip:user@example.com` → `example.com`
- Angle brackets: `"User Name" <sip:user@example.com>` → `example.com`
- URI parameters: `sip:user@example.com;user=phone;tag=xyz` → `example.com`
- Secure SIP: `sips:user@secure.example.com` → `secure.example.com`
- With port: `sip:user@example.com:5060` → `example.com:5060`
- Plain hostname: `sip-1.example.com:5060` → `sip-1.example.com` (for `sip.h.x-to-ip`)
- Plain hostname without port: `example.com` → `example.com`

**Normalization**:
- Domains converted to lowercase (case-insensitive matching)
- URI parameters (after `;`) stripped before extraction
- Port numbers stripped from plain hostnames
- Leading/trailing whitespace trimmed

### Forwarded Payload

**HTTP Method**: `POST`

**Headers**:
```
Content-Type: application/json
X-Sayna-Signature: v1=<hex_signature>
X-Sayna-Timestamp: <unix_seconds>
X-Sayna-Event-Id: <livekit_event_id>
X-Sayna-Signature-Version: v1
```

**Body** (JSON):
```json
{
  "participant": {
    "name": "SIP User",
    "identity": "sip-caller-123",
    "sid": "PA_abc123"
  },
  "room": {
    "name": "sip-+15551234567",
    "sid": "RM_xyz789"
  },
  "from_phone_number": "+15559876543",
  "to_phone_number": "+15551234567",
  "room_prefix": "sip-",
  "sip_host": "example.com"
}
```

**Field Descriptions**:
- `from_phone_number`: The phone number that initiated the call (from `sip.phoneNumber` attribute)
- `to_phone_number`: The phone number that received the call (from `sip.trunkPhoneNumber` attribute)

### Connection Pooling

Sayna uses the `ReqManager` infrastructure for webhook forwarding:

**Benefits**:
- **Per-domain connection pooling**: Reuses HTTP/2 connections for multiple webhooks to the same host
- **Concurrency limits**: Prevents overwhelming downstream services (default: 3 concurrent requests per host)
- **Automatic retries**: Built-in exponential backoff for transient failures
- **Metrics tracking**: Request counts, success/failure rates, and latencies

**Settings**:
- Timeout: 5 seconds per request
- Max concurrent: 3 requests per host
- HTTP/2 multiplexing enabled

### Error Handling

Webhook forwarding is **non-blocking** and **best-effort**:

| Scenario | Severity | Logged | Action |
|----------|----------|--------|--------|
| No SIP config | N/A | Not logged | Skipped silently |
| Event has no participant | Debug | `Skipping SIP forwarding: event has no participant` | Skipped (expected) |
| Missing SIP host header | Debug | `Skipping SIP forwarding: participant has no sip.h.to attribute` | Skipped (expected, neither `sip.h.x-to-ip` nor `sip.h.to` present) |
| Malformed SIP header | Info | `Skipping SIP forwarding: malformed sip.h.to header` | Check upstream gateway |
| No matching hook | Warn | `SIP forwarding failed: no webhook configured for domain` | Add to config |
| HTTP request failure | Warn | `HTTP request error` with details | Check connectivity |
| Non-2xx response | Warn | `webhook returned non-2xx status` with body | Check endpoint logs |

**Important**: All errors are logged but **do not affect the 200 OK response to LiveKit**. The webhook handler always returns immediately to prevent LiveKit retries.

### Logging

**Success** (info level):
```
INFO Successfully forwarded webhook to SIP hook
  event_id="EVT_abc123"
  domain="example.com"
  hook_url="https://webhook.example.com/events"
  status=200
  duration_ms=42
```

**Failure** (warn level):
```
WARN SIP forwarding failed: webhook returned non-2xx status
  event_id="EVT_abc123"
  sip_domain="example.com"
  status=500
  response_body="Internal Server Error: ..."
```

---

## Webhook Signing

### Overview

All outbound SIP webhooks from Sayna are cryptographically signed using HMAC-SHA256. This ensures downstream services can verify authenticity and detect tampering.

### Signing Contract

**Algorithm**: HMAC-SHA256 with hex-encoded signature

**Canonical String Format**:
```
v1:{timestamp}:{event_id}:{json_payload}
```

**Example**:
```
v1:1700000000:EVT_abc123:{"participant":{"name":"SIP User",...}}
```

### HTTP Headers

Every forwarded webhook includes these headers:

| Header | Format | Description |
|--------|--------|-------------|
| `X-Sayna-Signature` | `v1=<hex>` | HMAC-SHA256 signature (64 hex chars) |
| `X-Sayna-Timestamp` | `<unix_seconds>` | Request timestamp (seconds since epoch) |
| `X-Sayna-Event-Id` | `<event_id>` | LiveKit event ID (for correlation/deduplication) |
| `X-Sayna-Signature-Version` | `v1` | Signature scheme version (future-proofing) |
| `Content-Type` | `application/json` | Payload format |

### Configuration

**Global Secret** (recommended for single-tenant):

```yaml
sip:
  hook_secret: "your-secret-key-min-16-chars-recommended-32"
  hooks:
    - host: "example.com"
      url: "https://webhook.example.com/events"
```

**Per-Hook Secret Override** (recommended for multi-tenant):

```yaml
sip:
  hook_secret: "default-fallback-secret"
  hooks:
    - host: "customer-a.com"
      url: "https://webhook.customer-a.com/events"
      secret: "customer-a-specific-secret"

    - host: "customer-b.com"
      url: "https://webhook.customer-b.com/events"
      secret: "customer-b-specific-secret"
```

**Secret Requirements**:
- Minimum 16 characters (32+ recommended)
- Generate with: `openssl rand -hex 32`
- Store in environment variables or secret management systems
- **Never commit to version control**

### Verification Examples

#### Rust

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

fn verify_sayna_webhook(
    secret: &str,
    signature_header: &str,  // "v1=abc123..."
    timestamp_header: &str,  // "1700000000"
    event_id_header: &str,   // "EVT_abc123"
    body: &str,              // Exact JSON string
) -> Result<(), String> {
    // 1. Parse signature header
    let signature_hex = signature_header
        .strip_prefix("v1=")
        .ok_or("Invalid signature format")?;

    let expected_sig = hex::decode(signature_hex)
        .map_err(|_| "Invalid hex in signature")?;

    // 2. Validate timestamp (5-minute window)
    let timestamp: u64 = timestamp_header.parse()
        .map_err(|_| "Invalid timestamp")?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if now.abs_diff(timestamp) > 300 {
        return Err("Timestamp outside replay window".into());
    }

    // 3. Build canonical string
    let canonical = format!("v1:{}:{}:{}", timestamp, event_id_header, body);

    // 4. Compute HMAC-SHA256
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| format!("HMAC init failed: {}", e))?;
    mac.update(canonical.as_bytes());

    // 5. Constant-time comparison
    mac.verify_slice(&expected_sig)
        .map_err(|_| "Signature verification failed".into())?;

    Ok(())
}
```

#### Node.js (Express)

```javascript
const crypto = require('crypto');

function verifySaynaWebhook(req, res, next) {
  const secret = process.env.SIP_HOOK_SECRET;
  const signature = req.headers['x-sayna-signature']; // "v1=abc123..."
  const timestamp = req.headers['x-sayna-timestamp'];
  const eventId = req.headers['x-sayna-event-id'];

  // CRITICAL: Use raw body, not parsed JSON
  const body = req.rawBody; // Requires bodyParser({ verify: saveRawBody })

  // 1. Parse signature
  if (!signature || !signature.startsWith('v1=')) {
    return res.status(401).json({ error: 'Invalid signature format' });
  }
  const signatureHex = signature.substring(3);

  // 2. Validate timestamp (5 minutes)
  const now = Math.floor(Date.now() / 1000);
  if (Math.abs(now - parseInt(timestamp)) > 300) {
    return res.status(401).json({ error: 'Timestamp outside replay window' });
  }

  // 3. Build canonical string
  const canonical = `v1:${timestamp}:${eventId}:${body}`;

  // 4. Compute HMAC-SHA256
  const hmac = crypto.createHmac('sha256', secret);
  hmac.update(canonical);
  const expectedSignature = hmac.digest('hex');

  // 5. Constant-time comparison
  if (!crypto.timingSafeEqual(
    Buffer.from(signatureHex),
    Buffer.from(expectedSignature)
  )) {
    return res.status(401).json({ error: 'Signature verification failed' });
  }

  next(); // Valid
}

// Usage with raw body preservation
app.post('/webhook',
  express.json({ verify: (req, res, buf) => { req.rawBody = buf.toString(); } }),
  verifySaynaWebhook,
  (req, res) => {
    const event = req.body;
    console.log('Verified webhook:', event);
    res.status(200).json({ received: true });
  }
);
```

#### Python (Flask)

```python
import hmac
import hashlib
import time
from flask import Flask, request, jsonify

app = Flask(__name__)
SECRET = "your-secret-key"

@app.route('/webhook', methods=['POST'])
def webhook():
    # 1. Extract headers
    signature = request.headers.get('X-Sayna-Signature', '')
    timestamp = request.headers.get('X-Sayna-Timestamp', '')
    event_id = request.headers.get('X-Sayna-Event-Id', '')

    if not signature.startswith('v1='):
        return jsonify({"error": "Invalid signature format"}), 401

    signature_hex = signature[3:]

    # 2. Validate timestamp (5 minutes)
    try:
        ts = int(timestamp)
    except ValueError:
        return jsonify({"error": "Invalid timestamp"}), 401

    now = int(time.time())
    if abs(now - ts) > 300:
        return jsonify({"error": "Timestamp outside replay window"}), 401

    # 3. Get raw body (CRITICAL: exact bytes)
    body = request.get_data(as_text=True)

    # 4. Build canonical string
    canonical = f"v1:{timestamp}:{event_id}:{body}"

    # 5. Compute HMAC-SHA256
    expected_sig = hmac.new(
        SECRET.encode('utf-8'),
        canonical.encode('utf-8'),
        hashlib.sha256
    ).hexdigest()

    # 6. Constant-time comparison
    if not hmac.compare_digest(signature_hex, expected_sig):
        return jsonify({"error": "Signature verification failed"}), 401

    # Process webhook
    event = request.get_json()
    print(f"Verified webhook: {event}")
    return jsonify({"received": True}), 200
```

### Replay Protection

**Timestamp Window**: Reject webhooks with timestamps outside a **5-minute window** from current time. This prevents both replay attacks and clock skew issues.

**Why 5 minutes?**
- Balances security (short window limits replay) with reliability (allows network delays)
- Accommodates typical NTP clock drift (≤2 minutes)
- Industry standard (Stripe, Slack use 5 minutes)

**Optional Event ID Deduplication**:
- Store `event_id` in a TTL-based cache (e.g., Redis with 10-minute expiration)
- Reject webhooks with previously seen `event_id` values
- Cache expiration should be 2× timestamp window

### Secret Rotation

**Rotation Procedure**:

1. **Generate new secret**:
   ```bash
   openssl rand -hex 32
   ```

2. **Update Sayna configuration**:
   ```yaml
   sip:
     hook_secret: "new-secret-key"
   ```

3. **Deploy webhook consumer changes** (dual-verification during transition):
   ```rust
   fn verify_dual(old_secret: &str, new_secret: &str, ...) -> Result<(), String> {
       // Try new secret first
       if verify_webhook(new_secret, ...).is_ok() {
           return Ok(());
       }
       // Fall back to old secret during transition
       verify_webhook(old_secret, ...)
   }
   ```

4. **Restart Sayna** with new secret

5. **Monitor logs** for signature failures

6. **Remove fallback** after all consumers updated (24-48 hours)

**Rotation Frequency**:
- **Recommended**: Every 90 days
- **Required**: After suspected compromise
- **Automated**: Use secret management systems (AWS Secrets Manager, HashiCorp Vault)

---

## Testing & Development

### Running Tests

All webhook tests are in `tests/livekit_webhook_test.rs`:

```bash
# Run all webhook tests
cargo test --test livekit_webhook_test

# Run specific test
cargo test --test livekit_webhook_test test_webhook_success

# Run with verbose output
cargo test --test livekit_webhook_test -- --nocapture
```

### Test Coverage

**Integration Tests**:
- ✅ Valid webhook with correct signature → 200 OK
- ✅ Valid webhook with SIP attributes → 200 OK (logs SIP data)
- ✅ Missing Authorization header → 401 Unauthorized
- ✅ Invalid signature (wrong secret) → 401 Unauthorized
- ✅ Hash mismatch (tampered body) → 401 Unauthorized
- ✅ Missing LiveKit credentials → 503 Service Unavailable

**Unit Tests**:
- ✅ SIP domain parsing (10+ test cases)
- ✅ SIP attribute extraction
- ✅ Error status code mapping

### Manual Testing with curl

Create `examples/sign_webhook.rs`:

```rust
use base64::Engine;
use livekit_api::access_token::AccessToken;
use serde_json::json;
use sha2::{Digest, Sha256};

fn main() {
    let api_key = std::env::var("LIVEKIT_API_KEY")
        .unwrap_or("test-api-key".into());
    let api_secret = std::env::var("LIVEKIT_API_SECRET")
        .unwrap_or("test-api-secret".into());

    let payload = json!({
        "event": "participant_joined",
        "id": "test-event-123",
        "createdAt": 1700000000,
        "room": {
            "sid": "RM_test123",
            "name": "test-room"
        },
        "participant": {
            "sid": "PA_test456",
            "identity": "user-123",
            "name": "Test User",
            "kind": 3,
            "attributes": {
                "sip.h.to": "sip:customer@example.com",
                "sip.trunkPhoneNumber": "+1234567890"
            }
        }
    });

    let payload_str = payload.to_string();

    // Compute SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(payload_str.as_bytes());
    let hash_b64 = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());

    // Create signed token
    let token = AccessToken::with_api_key(&api_key, &api_secret)
        .with_sha256(&hash_b64)
        .to_jwt()
        .expect("Failed to create JWT");

    println!("curl -X POST http://localhost:3001/livekit/webhook \\");
    println!("  -H 'Content-Type: application/json' \\");
    println!("  -H 'Authorization: Bearer {}' \\", token);
    println!("  -d '{}'", payload_str);
}
```

Run:
```bash
cargo run --example sign_webhook
# Copy and run the curl command
```

### Testing Against Real LiveKit

Configure LiveKit to send webhooks to your Sayna instance:

```yaml
# livekit.yaml
webhook:
  urls:
    - http://your-sayna-server:3001/livekit/webhook
  api_key: your-api-key
```

Start Sayna with matching credentials:
```bash
export LIVEKIT_API_KEY=your-api-key
export LIVEKIT_API_SECRET=your-api-secret
cargo run
```

Trigger events by joining a room and watch Sayna logs.

---

## Operations & Troubleshooting

### Deployment Checklist

**Prerequisites**:
- [ ] Sayna server with SIP webhook forwarding configured
- [ ] Downstream webhook consumer endpoint (HTTPS recommended)
- [ ] Access to modify Sayna configuration
- [ ] Access to modify webhook consumer code
- [ ] Monitoring/logging access for both systems

**Step 1: Generate Signing Secret**

```bash
# OpenSSL (recommended)
openssl rand -hex 32

# Python
python3 -c "import secrets; print(secrets.token_hex(32))"
```

**Step 2: Update Sayna Configuration**

```yaml
# config.yaml
sip:
  hook_secret: "your-generated-secret-here"
  hooks:
    - host: "example.com"
      url: "https://webhook.example.com/events"
```

**Step 3: Implement Signature Verification**

See [Verification Examples](#verification-examples) above.

**Step 4: Deploy**

1. Deploy webhook consumer changes first (with verification)
2. Update Sayna configuration (add `hook_secret`)
3. Restart Sayna
4. Monitor for 24-48 hours

**Step 5: Validation**

```bash
# Check Sayna logs
grep "Successfully forwarded webhook" /var/log/sayna.log

# Check webhook consumer logs
grep "Verified webhook" /var/log/webhook.log
grep "Signature verification failed" /var/log/webhook.log
```

### Common Issues

#### "No webhooks forwarded"

**Symptoms**: Zero log entries for "Successfully forwarded webhook"

**Causes**:
- Missing `sip.hooks` config
- No SIP participants joining
- LiveKit not configured

**Solutions**:
1. Verify SIP configuration in YAML/ENV
2. Check LiveKit SIP trunk setup
3. Test with a real SIP call

#### "Missing secret error"

**Symptoms**: `ERROR: no signing secret configured for domain: example.com`

**Cause**: `hook_secret` not set and no per-hook `secret`

**Solution**: Add global secret:
```bash
export SIP_HOOK_SECRET="your-secret-key"
```

#### "Signature verification failed"

**Symptoms**: Webhook consumer returns 401, Sayna logs show 401 response

**Causes**:
- Secret mismatch between Sayna and consumer
- Using parsed JSON instead of raw body string
- Clock skew between servers

**Solutions**:
1. Verify secrets match exactly (no whitespace)
2. Use raw body for verification (before JSON parsing)
3. Enable NTP on both servers
4. Check canonical string format: `v1:{timestamp}:{event_id}:{body}`

#### "Timestamp rejected"

**Symptoms**: `"Timestamp outside replay window"` in logs

**Cause**: Clock skew > 5 minutes between servers

**Solutions**:
1. Enable NTP: `sudo timedatectl set-ntp true`
2. Check sync status: `timedatectl status`
3. Verify time on both servers: `date +%s`

#### "HTTP errors (503, timeout)"

**Symptoms**: `WARN: HTTP request error` in Sayna logs

**Causes**:
- Webhook endpoint down
- Network/firewall issue
- DNS resolution failed

**Solutions**:
1. Test endpoint: `curl -X POST <hook_url>`
2. Check DNS: `dig <domain>`
3. Verify firewall rules for outbound HTTPS

### Monitoring & Alerts

**Metrics to Track**:
- Webhook forwarding success rate
- Signature verification success rate
- Average webhook latency
- Timestamp skew

**Recommended Alerts**:

1. **Missing secret configuration**:
   - Pattern: `"no signing secret configured"`
   - Severity: **Critical**
   - Action: Add `hook_secret` immediately

2. **High signature failure rate**:
   - Metric: `signature_failures > 5%`
   - Severity: **High**
   - Action: Check secret sync, clock drift

3. **Clock skew detected**:
   - Pattern: `"Timestamp outside replay window"`
   - Severity: **Medium**
   - Action: Enable NTP, check time sync

4. **Webhook forwarding failures**:
   - Pattern: `"HTTP request error"` or `"webhook returned non-2xx"`
   - Severity: **High**
   - Action: Check endpoint health, network

### Log Patterns

Search for issues:

```bash
# All SIP forwarding failures
grep "SIP forwarding failed" logs.txt

# Missing hook configs
grep "no webhook configured for domain" logs.txt

# HTTP errors by domain
grep 'sip_domain="example.com"' logs.txt | grep "WARN"

# Malformed SIP headers
grep "malformed sip.h.to header" logs.txt

# Successful forwards
grep "Successfully forwarded webhook to SIP hook" logs.txt
```

---

## Security Considerations

### Threat Model

**Threats Mitigated**:
1. **Webhook spoofing**: Attackers cannot forge valid signatures without the secret
2. **Payload tampering**: Any modification invalidates the signature
3. **Replay attacks**: Timestamp window + event ID deduplication prevent reuse

**Threats NOT Mitigated**:
1. **TLS interception**: Signing doesn't protect against MITM if TLS compromised
   - **Solution**: Use valid TLS certs, certificate pinning
2. **Secret leakage**: Stolen secrets allow forging signatures
   - **Solution**: Rotate secrets, use per-hook secrets to limit blast radius
3. **DoS via signature computation**: High-volume attacks can cause CPU load
   - **Solution**: Rate limiting, DDoS protection

### Best Practices

**Configuration**:
- [ ] Use HTTPS for all webhook URLs (prevents secret exposure)
- [ ] Store secrets in environment variables (not hardcoded)
- [ ] Use per-hook secrets for multi-tenant deployments
- [ ] Minimum 32-character secrets (better entropy)

**Operations**:
- [ ] Rotate secrets every 90 days
- [ ] Monitor signature verification logs
- [ ] Never log secrets (only signatures are safe)
- [ ] Implement rate limiting on webhook endpoints
- [ ] Enable NTP on all servers

**Secret Compromise Response**:
1. **Immediate**: Rotate the secret + restart
2. **Coordinate**: Notify downstream services
3. **Audit**: Check logs for suspicious activity
4. **Investigate**: Determine leak source
5. **Per-hook isolation**: Only rotate compromised hook's secret

### Constant-Time Comparison

Always use constant-time comparison when verifying signatures to prevent timing attacks:

**Good**:
- Python: `hmac.compare_digest(received, expected)`
- Node.js: `crypto.timingSafeEqual(received, expected)`
- Rust: `mac.verify_slice(&expected_sig)` (constant-time by default)

**Bad** (vulnerable):
```python
# ❌ NEVER DO THIS
if received_signature == expected_signature:
    return True
```

### HTTPS Requirement

While signatures prevent tampering, they **do NOT encrypt** the payload. Webhook URLs **must use HTTPS** to protect:

1. **Secrets in transit**: Signature headers (though safe to expose, TLS prevents eavesdropping)
2. **PII in payload**: Phone numbers are sensitive
3. **Metadata leakage**: Room names, participant identities may be confidential

**Enforcement** (optional):
```rust
// In configuration validation
if !hook.url.starts_with("https://") {
    return Err("SIP webhook URLs must use HTTPS".into());
}
```

---

## References

### Industry Standards

- **Stripe Webhooks**: https://stripe.com/docs/webhooks/signatures
- **GitHub Webhooks**: https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries
- **Shopify Webhooks**: https://shopify.dev/docs/apps/build/webhooks/subscribe/https#step-5-verify-the-webhook

### Cryptography

- **HMAC RFC 2104**: https://datatracker.ietf.org/doc/html/rfc2104
- **SHA-256 FIPS 180-4**: https://nvlpubs.nist.gov/nistpubs/FIPS/NIST.FIPS.180-4.pdf

### LiveKit Documentation

- **LiveKit Webhooks**: https://docs.livekit.io/home/server/webhooks/
- **LiveKit SIP**: https://docs.livekit.io/sip/

### Related Sayna Documentation

- [CLAUDE.md](../CLAUDE.md) - Project overview and development guidelines
- [Authentication Configuration](authentication.md) - Securing REST endpoints
- [Configuration Overview](../README.md#configuration) - General configuration guide
