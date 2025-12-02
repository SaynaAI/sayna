# SIP Routing Architecture

## Overview

Sayna provides intelligent SIP call routing by leveraging custom SIP headers to determine the appropriate downstream webhook destination. This document explains the multi-tenant SIP infrastructure where a single LiveKit SIP deployment can serve multiple customers, with Sayna routing incoming call notifications to the correct webhook endpoint based on the original SIP request destination.

### The Multi-Tenant Challenge

In a typical multi-tenant voice platform, multiple customers share the same SIP infrastructure. When an inbound call arrives, the system must determine which customer should handle it. The challenge is that LiveKit SIP, by itself, doesn't provide a built-in mechanism to route webhook notifications to different endpoints based on the call's destination.

Sayna solves this by:
1. Capturing the original SIP destination through a proxy layer
2. Preserving that information through the LiveKit SIP pipeline
3. Using domain-based routing to forward webhooks to the appropriate customer endpoint

---

## Architecture Overview

### Infrastructure Components

```
                                    ┌─────────────────────────────────────────────────────────┐
                                    │                    Your Infrastructure                   │
                                    │                                                          │
┌──────────────┐                    │  ┌─────────────────┐      ┌─────────────────────────┐  │
│   PSTN       │                    │  │                 │      │                         │  │
│   Carrier    │──SIP INVITE────────┼─▶│    Kamailio     │─────▶│     LiveKit SIP        │  │
│              │  To: customer-a.com│  │   SIP Proxy     │      │                         │  │
└──────────────┘                    │  │                 │      │  Adds SIP attributes    │  │
                                    │  │  Adds header:   │      │  to participant:        │  │
                                    │  │  X-To-IP:       │      │  sip.h.x-to-ip          │  │
                                    │  │  customer-a.com │      │  sip.h.to               │  │
                                    │  └─────────────────┘      └───────────┬─────────────┘  │
                                    │                                       │                 │
                                    │                           Webhook     │                 │
                                    │                           Event       ▼                 │
                                    │                           ┌─────────────────────────┐  │
                                    │                           │                         │  │
                                    │                           │        Sayna            │  │
                                    │                           │                         │  │
                                    │                           │  1. Receives webhook    │  │
                                    │                           │  2. Extracts SIP domain │  │
                                    │                           │  3. Looks up hook config│  │
                                    │                           │  4. Forwards to customer│  │
                                    │                           └───────────┬─────────────┘  │
                                    └───────────────────────────────────────┼──────────────────┘
                                                                            │
                                        ┌───────────────────────────────────┼───────────────────┐
                                        │                                   │                   │
                                        ▼                                   ▼                   ▼
                                ┌───────────────┐                   ┌───────────────┐   ┌───────────────┐
                                │  Customer A   │                   │  Customer B   │   │  Customer C   │
                                │   Webhook     │                   │   Webhook     │   │   Webhook     │
                                │               │                   │               │   │               │
                                │ customer-a.com│                   │ customer-b.com│   │ customer-c.com│
                                └───────────────┘                   └───────────────┘   └───────────────┘
```

### Component Responsibilities

| Component | Role |
|-----------|------|
| **PSTN Carrier** | Routes phone calls to your SIP infrastructure based on DID ownership |
| **Kamailio SIP Proxy** | Intercepts SIP INVITE, extracts destination, adds `X-To-IP` header |
| **LiveKit SIP** | Bridges SIP calls to WebRTC rooms, captures headers as participant attributes |
| **Sayna** | Receives LiveKit webhooks, routes to customer endpoints based on SIP domain |
| **Customer Webhooks** | Downstream services that handle call events for each tenant |

---

## The Routing Problem

### Why Standard SIP Headers Aren't Enough

When a SIP call arrives at LiveKit through a proxy, the original destination information can be lost or modified. Consider this scenario:

1. A caller dials `+1-555-123-4567`, which routes to `customer-a.com`
2. The PSTN carrier sends a SIP INVITE with `To: <sip:+15551234567@customer-a.com>`
3. Kamailio receives the INVITE and needs to forward it to LiveKit
4. When Kamailio forwards to LiveKit, the `To` header gets rewritten to LiveKit's address

**The Problem**: By the time LiveKit receives the call, the original `customer-a.com` destination may be lost because the `To` header now points to LiveKit's SIP endpoint.

### The Solution: Custom SIP Headers

To preserve the original routing information, Kamailio adds a custom `X-To-IP` header before forwarding to LiveKit:

```
Original INVITE:
  To: <sip:+15551234567@customer-a.com>

Forwarded INVITE (with custom header):
  To: <sip:+15551234567@livekit-sip.internal>
  X-To-IP: customer-a.com
```

LiveKit SIP captures all SIP headers and exposes them as participant attributes, making `sip.h.x-to-ip` available for routing decisions.

---

## Kamailio Configuration

### Header Capture and Forwarding

Kamailio serves as the intelligent routing layer that captures the original SIP destination and preserves it for downstream processing. The key operations are:

1. **Extract the original host** from the incoming SIP INVITE's Request-URI or `To` header
2. **Add a custom header** (`X-To-IP`) containing the original destination
3. **Forward the call** to LiveKit SIP

### Conceptual Kamailio Logic

The Kamailio configuration performs these logical steps:

```
On incoming INVITE:
  1. Parse the Request-URI: sip:+15551234567@customer-a.com
  2. Extract the host portion: customer-a.com
  3. Add header: X-To-IP: customer-a.com
  4. Forward to LiveKit SIP endpoint
```

### Why X-To-IP?

The header name `X-To-IP` is a convention that clearly indicates:
- `X-` prefix: Custom/extension header (RFC 3261 compliant)
- `To-IP`: The original intended destination host

This naming makes it immediately clear to anyone inspecting SIP traces that this header contains routing information.

---

## LiveKit SIP Processing

### Header to Attribute Mapping

When LiveKit SIP receives a call, it creates a room and a SIP participant. All SIP headers are captured and exposed as participant attributes with the `sip.h.` prefix:

| SIP Header | Participant Attribute |
|------------|----------------------|
| `X-To-IP: customer-a.com` | `sip.h.x-to-ip: customer-a.com` |
| `To: <sip:user@example.com>` | `sip.h.to: <sip:user@example.com>` |
| `From: <sip:caller@carrier.com>` | `sip.h.from: <sip:caller@carrier.com>` |

### Additional SIP Attributes

LiveKit also provides these useful attributes for call handling:

| Attribute | Description | Example |
|-----------|-------------|---------|
| `sip.phoneNumber` | Caller's phone number | `+15559876543` |
| `sip.trunkPhoneNumber` | Called phone number (your DID) | `+15551234567` |
| `sip.callID` | Unique SIP call identifier | `abc123-def456` |

---

## Sayna Webhook Processing

### Domain Extraction Priority

When Sayna receives a `participant_joined` webhook from LiveKit, it extracts the target domain using this priority order:

1. **`sip.h.x-to-ip`** (highest priority) - Custom header from Kamailio
2. **`sip.h.to`** (fallback) - Standard SIP To header

This priority ensures that the explicit routing override from Kamailio takes precedence over the potentially modified standard headers.

### Domain Parsing

Sayna handles various formats that may appear in SIP headers:

| Input Format | Extracted Domain |
|--------------|------------------|
| `sip:user@example.com` | `example.com` |
| `"Display Name" <sip:user@example.com>` | `example.com` |
| `sip:user@example.com;user=phone;tag=xyz` | `example.com` |
| `sips:user@secure.example.com` | `secure.example.com` |
| `example.com:5060` (plain hostname) | `example.com` |
| `sip-1.example.com` (plain hostname) | `sip-1.example.com` |

All domains are normalized to lowercase for case-insensitive matching against hook configurations.

### Webhook Routing Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Sayna Webhook Processing                          │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  1. Receive LiveKit webhook (participant_joined)                     │
│                          │                                           │
│                          ▼                                           │
│  2. Verify LiveKit signature (JWT validation)                        │
│                          │                                           │
│                          ▼                                           │
│  3. Check if participant has SIP attributes                          │
│     └── If no: Skip forwarding (not a SIP call)                      │
│                          │                                           │
│                          ▼                                           │
│  4. Extract domain from sip.h.x-to-ip (or sip.h.to fallback)        │
│     └── If neither present: Skip forwarding                          │
│                          │                                           │
│                          ▼                                           │
│  5. Look up hook configuration for domain                            │
│     └── If no match: Log warning, skip forwarding                    │
│                          │                                           │
│                          ▼                                           │
│  6. Sign payload with HMAC-SHA256                                    │
│                          │                                           │
│                          ▼                                           │
│  7. Forward to customer webhook (async, non-blocking)                │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Hook Configuration

### Defining Webhook Destinations

SIP hooks map domain names to webhook URLs. Configure hooks to route calls to the appropriate customer endpoint.

**YAML Configuration:**

```yaml
sip:
  room_prefix: "sip-"

  # Global signing secret for webhook authentication
  hook_secret: "your-secret-key-min-32-chars-recommended"

  # Per-domain webhook destinations
  hooks:
    - host: "customer-a.com"
      url: "https://webhook.customer-a.com/sip-events"

    - host: "customer-b.com"
      url: "https://webhook.customer-b.com/sip-events"

    - host: "customer-c.example.org"
      url: "https://api.customer-c.example.org/webhooks/voice"
```

### Runtime Hook Management

Hooks can also be managed at runtime via the REST API:

**List hooks:**
```
GET /sip/hooks
```

**Add or update hooks:**
```
POST /sip/hooks
{
  "hooks": [
    {"host": "new-customer.com", "url": "https://webhook.new-customer.com/events"}
  ]
}
```

**Delete hooks:**
```
DELETE /sip/hooks
{
  "hosts": ["old-customer.com"]
}
```

Runtime changes are persisted and merged with the original configuration on server restart.

---

## Forwarded Webhook Payload

When Sayna forwards an event to a customer webhook, it sends a simplified, consistent payload:

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
  "sip_host": "customer-a.com"
}
```

### Field Descriptions

| Field | Description |
|-------|-------------|
| `participant` | Information about the SIP caller in the LiveKit room |
| `room` | The LiveKit room created for this call |
| `from_phone_number` | The caller's phone number (who is calling) |
| `to_phone_number` | Your DID that received the call |
| `room_prefix` | The configured room name prefix |
| `sip_host` | The domain that was used for routing |

### Webhook Authentication

All forwarded webhooks include HMAC-SHA256 signatures for authenticity verification:

| Header | Description |
|--------|-------------|
| `X-Sayna-Signature` | HMAC-SHA256 signature (format: `v1=<hex>`) |
| `X-Sayna-Timestamp` | Unix timestamp when signature was generated |
| `X-Sayna-Event-Id` | Original LiveKit event ID for correlation |
| `X-Sayna-Signature-Version` | Signature scheme version (`v1`) |

See [LiveKit Integration - Webhook Signing](livekit_integration.md#webhook-signing) for verification examples.

---

## End-to-End Example

### Scenario: Customer A Receives an Inbound Call

1. **Caller Action**: Alice dials `+1-555-123-4567` from her phone

2. **PSTN Routing**: The carrier routes the call to your SIP infrastructure based on DID ownership

3. **Kamailio Processing**:
   ```
   Received INVITE:
     Request-URI: sip:+15551234567@customer-a.com
     From: <sip:+15559876543@carrier.net>

   Added header:
     X-To-IP: customer-a.com

   Forwarded to: LiveKit SIP endpoint
   ```

4. **LiveKit SIP**:
   - Creates room: `sip-+15551234567`
   - Creates SIP participant with attributes:
     - `sip.h.x-to-ip`: `customer-a.com`
     - `sip.phoneNumber`: `+15559876543`
     - `sip.trunkPhoneNumber`: `+15551234567`
   - Sends `participant_joined` webhook to Sayna

5. **Sayna Processing**:
   - Receives webhook, verifies LiveKit signature
   - Extracts domain: `customer-a.com` (from `sip.h.x-to-ip`)
   - Looks up hook: finds `https://webhook.customer-a.com/sip-events`
   - Signs and forwards payload to Customer A's webhook

6. **Customer A's System**:
   - Receives webhook notification
   - Verifies HMAC signature
   - Handles the call (e.g., connects an AI agent, routes to human, plays IVR)

---

## Troubleshooting

### Common Issues

#### "No webhook configured for domain"

**Symptom**: Sayna logs show `SIP forwarding failed: no webhook configured for domain`

**Causes**:
- Missing hook configuration for the domain
- Domain mismatch (check case sensitivity, trailing characters)
- Kamailio not adding `X-To-IP` header

**Solutions**:
1. Add the domain to your hook configuration
2. Verify Kamailio is correctly extracting and forwarding the header
3. Check LiveKit logs to confirm `sip.h.x-to-ip` attribute is present

#### "Malformed sip.h.to header"

**Symptom**: Sayna logs show `Skipping SIP forwarding: malformed sip.h.to header`

**Cause**: The SIP header value cannot be parsed to extract a domain

**Solutions**:
1. Check the SIP header format in LiveKit participant attributes
2. Verify Kamailio is setting the header value correctly
3. Ensure the header contains a valid hostname or SIP URI

#### Webhook Not Received by Customer

**Symptoms**:
- Sayna logs show successful forwarding
- Customer webhook never receives the event

**Possible Causes**:
- Network/firewall blocking outbound requests
- DNS resolution failure
- Customer webhook endpoint is down
- TLS/SSL certificate issues

**Debugging Steps**:
1. Check Sayna logs for HTTP errors
2. Verify the webhook URL is correct and accessible
3. Test the endpoint manually with curl
4. Check for HTTPS certificate validity

---

## Best Practices

### Infrastructure Design

1. **High Availability**: Deploy Kamailio and Sayna in redundant configurations
2. **Monitoring**: Set up alerts for webhook forwarding failures
3. **Logging**: Enable structured logging for SIP attribute tracking

### Security

1. **Webhook Secrets**: Use strong, unique secrets (32+ characters)
2. **HTTPS Only**: Configure hooks to use HTTPS endpoints only
3. **Signature Verification**: Always verify HMAC signatures in downstream webhooks
4. **Network Isolation**: Place SIP infrastructure in a secure network segment

### Performance

1. **Connection Pooling**: Sayna maintains HTTP/2 connection pools for efficient webhook delivery
2. **Async Processing**: Webhook forwarding is non-blocking; LiveKit receives immediate responses
3. **Timeout Handling**: Configure appropriate timeouts for downstream webhooks

---

## Related Documentation

- [LiveKit Integration](livekit_integration.md) - Complete LiveKit webhook and SIP provisioning guide
- [Authentication Configuration](authentication.md) - Securing REST endpoints
- [Configuration Overview](../README.md#configuration) - General configuration guide
