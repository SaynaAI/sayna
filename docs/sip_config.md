# SIP Configuration

Sayna supports first-class SIP configuration for managing SIP-specific settings such as room prefixes, allowed IP addresses, and downstream webhook targets for LiveKit SIP events.

## Overview

SIP configuration is **optional** and can be provided via YAML configuration file or environment variables. When configured, Sayna enforces the following:

1. **Room Prefix**: All SIP-related LiveKit rooms must use a specific prefix
2. **IP Filtering**: Only connections from specified IP addresses/CIDRs are allowed
3. **Event Routing**: LiveKit webhook events are forwarded to downstream services based on host matching

## Configuration Methods

### YAML Configuration

Add a `sip` block to your `config.yaml`:

```yaml
sip:
  room_prefix: "sip-"
  allowed_addresses:
    - "192.168.1.0/24"
    - "10.0.0.1"
  hooks:
    - host: "example.com"
      url: "https://webhook.example.com/events"
    - host: "another.com"
      url: "https://webhook2.example.com/events"
```

### Environment Variables

Set the following environment variables:

- **`SIP_ROOM_PREFIX`** (required): Room name prefix for SIP calls
  - Example: `sip-`
  - Must contain only alphanumeric characters, `-`, or `_`

- **`SIP_ALLOWED_ADDRESSES`** (optional): Comma-separated list of IP addresses/CIDRs
  - Example: `192.168.1.0/24,10.0.0.1`
  - Whitespace around entries is automatically trimmed

- **`SIP_HOOKS_JSON`** (optional): JSON array of hook configurations
  - Example: `[{"host": "example.com", "url": "https://webhook.example.com/events"}]`
  - Each hook must have a `host` and `url` field
  - URLs **must** use HTTPS (HTTP is rejected for security)

**Example:**

```bash
export SIP_ROOM_PREFIX="sip-"
export SIP_ALLOWED_ADDRESSES="192.168.1.0/24,10.0.0.1"
export SIP_HOOKS_JSON='[{"host": "example.com", "url": "https://webhook.example.com/events"}]'
```

## Configuration Priority

When both YAML and environment variables are provided:

```
Environment Variables > YAML Values > No SIP Config
```

- **Room Prefix**: ENV overrides YAML
- **Allowed Addresses**: ENV completely replaces YAML list
- **Hooks**: ENV completely replaces YAML list

## Validation Rules

Sayna validates SIP configuration at startup and rejects invalid configurations with descriptive error messages:

### Room Prefix Validation

- **Cannot be empty**
- **Must contain only**: alphanumeric characters, hyphens (`-`), and underscores (`_`)
- **Valid examples**: `sip-`, `sip_call`, `room-123`
- **Invalid examples**: `sip@test`, `call#`, `room/name`

### Allowed Addresses Validation

- **Cannot be empty** when SIP config is present
- **Each entry must** look like a valid IPv4 address or CIDR range
- **Format**: `X.X.X.X` or `X.X.X.X/Y` (basic regex validation)
- **Valid examples**: `192.168.1.0/24`, `10.0.0.1`, `172.16.0.0/12`
- **Invalid examples**: `not-an-ip`, `example.com`, `2001:db8::/32` (IPv6 not supported yet)

### Hooks Validation

- **No duplicate hosts**: Each host can only appear once (case-insensitive)
- **HTTPS required**: All webhook URLs must start with `https://`
- **Rationale**: LiveKit events may include phone numbers and sensitive data

**Valid:**
```yaml
hooks:
  - host: "example.com"
    url: "https://webhook.example.com/events"
  - host: "another.com"
    url: "https://webhook2.example.com/events"
```

**Invalid (duplicate host):**
```yaml
hooks:
  - host: "example.com"
    url: "https://webhook1.example.com/events"
  - host: "Example.COM"  # Duplicate (case-insensitive)
    url: "https://webhook2.example.com/events"
```

**Invalid (HTTP URL):**
```yaml
hooks:
  - host: "example.com"
    url: "http://webhook.example.com/events"  # Must be HTTPS
```

## Auto-Provisioning

When SIP configuration is enabled and LiveKit API credentials are available, Sayna automatically provisions the required LiveKit SIP trunk and dispatch rule during application startup. This ensures LiveKit is ready to receive and route SIP calls without manual setup.

### What Gets Provisioned

1. **SIP Inbound Trunk**: Creates a trunk with the configured allowed IP addresses
   - Name: `sayna-{room_prefix}-trunk` (e.g., `sayna-sip--trunk`)
   - Allowed addresses: Uses the `allowed_addresses` from SIP config

2. **SIP Dispatch Rule**: Creates a dispatch rule to route calls to rooms
   - Name: `sayna-{room_prefix}-dispatch` (e.g., `sayna-sip--dispatch`)
   - Room prefix: Uses the `room_prefix` from SIP config
   - Max participants: Defaults to 3 (caller + Sayna + optional third party)

### Provisioning Behavior

- **Idempotent**: Running Sayna multiple times won't recreate existing resources
- **Fail-fast**: If provisioning fails, Sayna refuses to start and logs the error
- **Predictable naming**: Resource names are deterministic based on `room_prefix`

### Required Credentials

To enable auto-provisioning, you must configure:

- **`LIVEKIT_API_KEY`**: LiveKit API key
- **`LIVEKIT_API_SECRET`**: LiveKit API secret
- **SIP configuration**: Either via YAML or environment variables

**Example:**

```bash
export LIVEKIT_API_KEY="your-api-key"
export LIVEKIT_API_SECRET="your-api-secret"
export SIP_ROOM_PREFIX="sip-"
export SIP_ALLOWED_ADDRESSES="192.168.1.0/24,10.0.0.1"
```

### Monitoring Provisioning

Sayna logs detailed information about the provisioning process:

**Success:**
```
INFO SIP configuration detected, provisioning LiveKit SIP trunk and dispatch rules
INFO Successfully provisioned SIP resources: trunk=sayna-sip--trunk, dispatch=sayna-sip--dispatch, livekit_url=ws://localhost:7880
```

**Skipped (missing credentials):**
```
INFO SIP configuration present but LiveKit API credentials missing. Skipping SIP provisioning. Set LIVEKIT_API_KEY and LIVEKIT_API_SECRET to enable.
```

**Failed:**
```
ERROR Failed to provision SIP resources: trunk=sayna-sip--trunk, dispatch=sayna-sip--dispatch, livekit_url=ws://localhost:7880, error=ConnectionFailed(...)
PANIC SIP provisioning failed for trunk=sayna-sip--trunk, dispatch=sayna-sip--dispatch: ... Cannot start server with SIP enabled. Please check LiveKit API credentials and server availability.
```

### Verifying in LiveKit UI

After Sayna starts successfully, you can verify the provisioned resources in the LiveKit Cloud dashboard or self-hosted admin UI:

1. Navigate to **SIP** → **Inbound Trunks**
2. Look for trunk named `sayna-{your-room-prefix}-trunk`
3. Navigate to **SIP** → **Dispatch Rules**
4. Look for dispatch rule named `sayna-{your-room-prefix}-dispatch`

### Manual Configuration (Not Recommended)

If you need to customize settings beyond what auto-provisioning provides (e.g., different max_participants), you can:

1. Let Sayna provision the basic resources
2. Manually adjust settings in the LiveKit dashboard
3. Future restarts won't overwrite your changes (provisioning is idempotent)

**Note**: Future versions of Sayna may expose more provisioning options in the configuration file.

## How It Works

### Room Prefix Matching

When a LiveKit room name starts with the configured `room_prefix`, Sayna treats it as a SIP-related room. This allows runtime components to:

1. Filter and identify SIP calls
2. Apply SIP-specific processing logic
3. Route events to the appropriate downstream webhooks

### IP Address Filtering

The `allowed_addresses` list controls which source IPs can establish SIP connections. This is useful for:

- Restricting SIP traffic to known providers
- Implementing allowlists for trusted networks
- Preventing unauthorized SIP access

### Webhook Event Routing

When LiveKit sends a webhook event, Sayna:

1. Extracts the host from the event (implementation-specific)
2. Matches it against configured hook hosts (case-insensitive)
3. Forwards the event to the corresponding webhook URL via HTTPS POST

## Use Cases

### Single SIP Provider

```yaml
sip:
  room_prefix: "sip-"
  allowed_addresses:
    - "203.0.113.0/24"  # Provider's IP range
  hooks:
    - host: "sip-provider.example.com"
      url: "https://backend.myapp.com/sip/events"
```

### Multiple SIP Providers

```yaml
sip:
  room_prefix: "sip-"
  allowed_addresses:
    - "203.0.113.0/24"   # Provider A
    - "198.51.100.0/24"  # Provider B
  hooks:
    - host: "provider-a.example.com"
      url: "https://backend.myapp.com/sip/provider-a"
    - host: "provider-b.example.com"
      url: "https://backend.myapp.com/sip/provider-b"
```

### Development/Testing

For local development, you can use private IP ranges:

```yaml
sip:
  room_prefix: "test-sip-"
  allowed_addresses:
    - "127.0.0.1"
    - "192.168.1.0/24"
  hooks:
    - host: "localhost"
      url: "https://localhost:8443/webhooks"  # Still requires HTTPS
```

## Accessing Configuration at Runtime

In your Rust code, access SIP configuration via `ServerConfig`:

```rust
use sayna::config::ServerConfig;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ServerConfig::from_env()?;

    if let Some(sip_config) = &config.sip {
        println!("SIP room prefix: {}", sip_config.room_prefix);
        println!("Allowed addresses: {:?}", sip_config.allowed_addresses);
        println!("Configured {} webhook hooks", sip_config.hooks.len());

        for hook in &sip_config.hooks {
            println!("  {} -> {}", hook.host, hook.url);
        }
    } else {
        println!("SIP configuration not enabled");
    }

    Ok(())
}
```

## Troubleshooting

### Error: "SIP room_prefix is required when SIP configuration is present"

**Cause**: You've set `SIP_ALLOWED_ADDRESSES` or `SIP_HOOKS_JSON` but forgot `SIP_ROOM_PREFIX`.

**Solution**: Always set `SIP_ROOM_PREFIX` when using any SIP configuration:

```bash
export SIP_ROOM_PREFIX="sip-"
```

### Error: "SIP hook URL must be HTTPS"

**Cause**: One of your webhook URLs uses `http://` instead of `https://`.

**Solution**: Update all hook URLs to use HTTPS:

```yaml
hooks:
  - host: "example.com"
    url: "https://webhook.example.com/events"  # ✓ HTTPS
```

### Error: "Duplicate SIP hook host: example.com"

**Cause**: You've configured the same host multiple times (case-insensitive).

**Solution**: Ensure each host appears only once:

```yaml
hooks:
  - host: "example.com"
    url: "https://webhook.example.com/events"
  # Remove or rename duplicate:
  - host: "another.com"  # Different host
    url: "https://webhook2.example.com/events"
```

### Error: "SIP allowed_address '...' does not look like a valid IPv4 address"

**Cause**: Invalid IP address format.

**Solution**: Use valid IPv4 addresses or CIDR ranges:

```yaml
allowed_addresses:
  - "192.168.1.0/24"  # ✓ Valid CIDR
  - "10.0.0.1"        # ✓ Valid IP
  # - "example.com"   # ✗ Invalid (use IP)
```

## Related Documentation

- [LiveKit Webhook Design](livekit_webhook.md) - Detailed webhook implementation and event handling
- [Authentication Configuration](authentication.md) - Securing webhook endpoints
- [Configuration Overview](../README.md#configuration) - General configuration guide

## Future Enhancements

Potential improvements to SIP configuration:

- IPv6 support for `allowed_addresses`
- Webhook retry configuration (timeout, retries, backoff)
- Dynamic host matching (wildcards, regex patterns)
- Webhook request signing for verification
- Per-hook authentication credentials
