# Authentication System

## Overview

Sayna supports two authentication methods for protecting API endpoints:

1. **API Secret Authentication** (Simple): Direct bearer token comparison against a configured secret. Ideal for single-tenant deployments or simple use cases.

2. **JWT-Based Authentication** (Advanced): Delegates token validation to an external authentication service with JWT-signed requests. Provides maximum flexibility for multi-tenant systems and complex authorization logic.

Both methods can be configured independently, and you can choose the approach that best fits your deployment requirements.

## Quick Start

### API Secret (Simplest)
```bash
# 1. Generate a secret
openssl rand -base64 32

# 2. Configure
AUTH_REQUIRED=true
AUTH_API_SECRET=your-generated-secret

# 3. Use
curl -H "Authorization: Bearer your-generated-secret" http://localhost:3001/speak
```

### JWT-Based (Advanced)
```bash
# 1. Generate keys
openssl genrsa -out private.pem 2048
openssl rsa -in private.pem -pubout -out public.pem

# 2. Configure
AUTH_REQUIRED=true
AUTH_SERVICE_URL=https://your-auth.com/validate
AUTH_SIGNING_KEY_PATH=/path/to/private.pem

# 3. Implement auth service (see detailed setup below)
```

## Architecture

### Method 1: API Secret Authentication Flow

Simple bearer token comparison - no external service required.

```
┌─────────┐                 ┌───────┐
│ Client  │                 │ Sayna │
└────┬────┘                 └───┬───┘
     │                          │
     │ POST /speak              │
     │ Authorization: Bearer tk │
     ├─────────────────────────>│
     │                          │
     │                          │ Compare token "tk"
     │                          │ with AUTH_API_SECRET
     │                          │
     │     Allow/Deny request   │
     │<─────────────────────────┤
     │                          │
```

**When to use:**
- Single-tenant deployments
- Simple authentication requirements
- No need for per-request authorization logic
- Lowest latency (no external service call)

### Method 2: JWT-Based Authentication Flow

Delegated validation with external auth service for advanced use cases.

```
┌─────────┐                 ┌───────┐                ┌──────────────┐
│ Client  │                 │ Sayna │                │ Auth Service │
└────┬────┘                 └───┬───┘                └──────┬───────┘
     │                          │                           │
     │ POST /speak              │                           │
     │ Authorization: Bearer tk │                           │
     ├─────────────────────────>│                           │
     │                          │                           │
     │                          │ Create JWT payload:       │
     │                          │ {token, body, headers}    │
     │                          │                           │
     │                          │ Sign with PRIVATE key     │
     │                          │                           │
     │                          │ POST /auth                │
     │                          │ Body: signed JWT          │
     │                          ├──────────────────────────>│
     │                          │                           │
     │                          │                           │ Verify JWT
     │                          │                           │ with PUBLIC key
     │                          │                           │
     │                          │                           │ Validate bearer
     │                          │                           │ token "tk"
     │                          │                           │
     │                          │      200 OK / 401         │
     │                          │<──────────────────────────┤
     │                          │                           │
     │     Allow/Deny request   │                           │
     │<─────────────────────────┤                           │
     │                          │                           │
```

**When to use:**
- Multi-tenant deployments
- Complex authorization logic (permissions, roles, quotas)
- Need to validate against external user database
- Context-aware authorization (different permissions per endpoint)
- Centralized authentication service

### Components

1. **Authentication Middleware** (`src/middleware/auth.rs`)
   - Intercepts HTTP requests to protected endpoints
   - Extracts and validates Authorization header format
   - **API Secret Mode**: Direct token comparison with configured secret
   - **JWT Mode**: Buffers request body/headers and calls AuthClient
   - Priority: API secret checked first if configured

2. **Server Config** (`src/config.rs`)
   - Loads authentication configuration from environment variables
   - Validates that at least one auth method is configured when `AUTH_REQUIRED=true`
   - Helper methods: `has_jwt_auth()`, `has_api_secret_auth()`

3. **Auth Client** (`src/auth/client.rs`) - JWT Mode Only
   - HTTP client for communicating with external auth service
   - Signs auth payloads using JWT
   - Handles connection pooling and timeouts

4. **JWT Signing Module** (`src/auth/jwt.rs`) - JWT Mode Only
   - Signs auth request payloads with private key
   - Supports RSA and ECDSA keys in PEM format
   - Includes timestamp and 5-minute expiration

5. **Error Handling** (`src/errors/auth_error.rs`)
   - Comprehensive error types for auth failures
   - Proper HTTP status code mapping

## Setup

Choose one of the two authentication methods below based on your requirements.

### Option A: API Secret Authentication (Simple)

For simple deployments where you just need a shared secret.

#### 1. Generate a Secure Secret

```bash
# Generate a random 32-character secret
openssl rand -base64 32

# Or use any secure string
# Example: sk_live_abc123xyz789...
```

#### 2. Configure Environment Variables

Add to your `.env` file:

```bash
# Enable authentication
AUTH_REQUIRED=true

# Set your API secret
AUTH_API_SECRET=your-secure-secret-here
```

#### 3. Use the Token

Clients authenticate by sending the secret as a bearer token:

```bash
curl -X POST http://localhost:3001/speak \
  -H "Authorization: Bearer your-secure-secret-here" \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world"}'
```

**That's it!** No external service or key generation needed.

**Security Notes:**
- Use a long, random secret (32+ characters)
- Rotate the secret periodically
- Never commit the secret to version control
- Use HTTPS in production to protect the token in transit

### Option B: JWT-Based Authentication (Advanced)

For multi-tenant or complex authorization requirements.

#### 1. Generate Signing Keys

Generate an RSA key pair for JWT signing:

```bash
# Generate private key (keep this secret!)
openssl genrsa -out auth_private_key.pem 2048

# Extract public key (share with auth service)
openssl rsa -in auth_private_key.pem -pubout -out auth_public_key.pem

# Set proper permissions
chmod 600 auth_private_key.pem
chmod 644 auth_public_key.pem
```

Alternatively, use ECDSA keys (smaller and faster):

```bash
# Generate private key
openssl ecparam -genkey -name prime256v1 -noout -out auth_private_key.pem

# Extract public key
openssl ec -in auth_private_key.pem -pubout -out auth_public_key.pem

# Set proper permissions
chmod 600 auth_private_key.pem
chmod 644 auth_public_key.pem
```

#### 2. Configure Environment Variables

Add to your `.env` file:

```bash
# Enable authentication
AUTH_REQUIRED=true

# JWT-based auth configuration
AUTH_SERVICE_URL=https://your-auth-service.com/auth
AUTH_SIGNING_KEY_PATH=/path/to/auth_private_key.pem
AUTH_TIMEOUT_SECONDS=5
```

#### 3. Implement Auth Service

Your external authentication service must:

1. Accept POST requests with JWT in the body
2. Verify JWT signature using the public key
3. Validate the bearer token in the JWT payload
4. Return:
   - `200 OK` if token is valid
   - `401 Unauthorized` if token is invalid
   - Any other status code for errors

##### Example Auth Service (Node.js + Express)

```javascript
const express = require('express');
const jwt = require('jsonwebtoken');
const fs = require('fs');

const app = express();
const publicKey = fs.readFileSync('auth_public_key.pem');

app.post('/auth', express.text(), (req, res) => {
  try {
    // Verify and decode JWT
    const payload = jwt.verify(req.body, publicKey, {
      algorithms: ['RS256', 'ES256']
    });

    // Validate the bearer token
    const isValid = validateToken(payload.token);

    if (isValid) {
      res.status(200).send('OK');
    } else {
      res.status(401).send('Invalid token');
    }
  } catch (error) {
    res.status(401).send('JWT verification failed');
  }
});

function validateToken(token) {
  // Implement your token validation logic here
  // e.g., check database, verify JWT claims, etc.
  return true; // placeholder
}

app.listen(3000);
```

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `AUTH_REQUIRED` | No | `false` | Enable/disable authentication |
| `AUTH_API_SECRET` | Conditional* | - | Secret token for API Secret authentication |
| `AUTH_SERVICE_URL` | Conditional** | - | External auth service endpoint (JWT mode) |
| `AUTH_SIGNING_KEY_PATH` | Conditional** | - | Path to RSA/ECDSA private key (JWT mode) |
| `AUTH_TIMEOUT_SECONDS` | No | `5` | Auth request timeout in seconds (JWT mode only) |

**Configuration Requirements:**

When `AUTH_REQUIRED=true`, you must configure **at least one** of:
- **Option A (Simple)**: Set `AUTH_API_SECRET`
- **Option B (Advanced)**: Set both `AUTH_SERVICE_URL` and `AUTH_SIGNING_KEY_PATH`

**Both methods can coexist**: If both are configured, API Secret is checked first (takes priority).

## JWT Payload Specification (JWT Mode Only)

This section applies only to JWT-based authentication (Option B). Skip this if you're using API Secret authentication.

The JWT signed by Sayna contains the following claims structure:

```json
{
  "sub": "sayna-auth",
  "iat": 1234567890,
  "exp": 1234568190,
  "auth_data": {
    "token": "bearer-token-from-auth-header",
    "request_body": {"text": "request body as JSON"},
    "request_headers": {
      "content-type": "application/json",
      "user-agent": "client-agent"
    },
    "request_path": "/speak",
    "request_method": "POST"
  }
}
```

### Standard JWT Claims

- `sub` (Subject): Always set to `"sayna-auth"` - identifies the JWT issuer
- `iat` (Issued At): Unix timestamp when the JWT was created
- `exp` (Expiration): Unix timestamp when JWT expires (5 minutes from creation)

### Auth Data Fields (Nested under `auth_data`)

- `token`: The bearer token extracted from the `Authorization` header
- `request_body`: The complete request body parsed as JSON
- `request_headers`: Filtered request headers (see below for exclusions)
- `request_path`: The HTTP request path
- `request_method`: The HTTP method (GET, POST, etc.)

### Header Filtering

The following headers are excluded from `request_headers` for security:
- `authorization` (already in `token` field)
- `cookie` (sensitive)
- `x-forwarded-*` (internal proxy headers)
- `x-sayna-*` (internal application headers)
- `host` (infrastructure)
- `x-real-ip` (infrastructure)

## Protected Endpoints

The following API endpoints require authentication when `AUTH_REQUIRED=true`:

- `POST /speak` - Text-to-speech generation
- `GET /voices` - List available voices
- `POST /livekit/token` - Generate LiveKit participant token

### Public Endpoints (No Auth Required)

- `GET /` - Health check endpoint
- `GET /ws` - WebSocket endpoint (see WebSocket Auth section)

## Usage Examples

### Making Authenticated Requests

The client authentication flow is identical for both methods - just send a bearer token in the Authorization header.

#### With API Secret Authentication

```bash
# Use your configured AUTH_API_SECRET as the bearer token
curl -X POST http://localhost:3001/speak \
  -H "Authorization: Bearer your-configured-secret" \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world", "voice": "en-US-JennyNeural"}'

# List available voices
curl -X GET http://localhost:3001/voices \
  -H "Authorization: Bearer your-configured-secret"
```

#### With JWT-Based Authentication

```bash
# Use your user's token (validated by your auth service)
curl -X POST http://localhost:3001/speak \
  -H "Authorization: Bearer user-jwt-token-abc123" \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world", "voice": "en-US-JennyNeural"}'

# The token can be different for each user
curl -X POST http://localhost:3001/speak \
  -H "Authorization: Bearer user-jwt-token-xyz789" \
  -H "Content-Type: application/json" \
  -d '{"text": "Different user"}'
```

#### Without Authentication (will fail if auth is enabled)

```bash
curl -X POST http://localhost:3001/speak \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world"}'
# Returns: 401 Unauthorized - Missing Authorization header
```

## Error Responses

Authentication errors return JSON responses with the following structure:

```json
{
  "error": "error_code",
  "message": "Human-readable error message"
}
```

### Error Codes and Status Codes

| Error Code | HTTP Status | Description |
|------------|-------------|-------------|
| `missing_auth_header` | 401 Unauthorized | Authorization header is missing from the request |
| `invalid_auth_header` | 401 Unauthorized | Authorization header format is invalid (not "Bearer {token}") |
| `unauthorized` | 401 Unauthorized | Token validation failed (auth service returned 401) |
| `auth_service_error` | 401 or 502 | Auth service returned an error (see below) |
| `auth_service_unavailable` | 503 Service Unavailable | Auth service is unreachable or timed out |
| `config_error` | 500 Internal Server Error | Auth configuration error (e.g., missing signing key) |
| `jwt_signing_error` | 500 Internal Server Error | Failed to sign JWT payload |

### Auth Service Response Handling

Sayna maps auth service responses to HTTP status codes as follows:

| Auth Service Status | Sayna Response Status | Description |
|---------------------|----------------------|-------------|
| 200 OK | 200 OK (request allowed) | Token is valid, request proceeds |
| 401 Unauthorized | 401 Unauthorized | Invalid token, client should not retry |
| 4xx (other) | 401 Unauthorized | Client error mapped to unauthorized |
| 5xx | 502 Bad Gateway | Auth service error, temporary issue |
| Timeout/Network Error | 503 Service Unavailable | Auth service unreachable |

### Example Error Responses

**Missing Authorization Header:**
```json
HTTP/1.1 401 Unauthorized
Content-Type: application/json

{
  "error": "missing_auth_header",
  "message": "Missing Authorization header"
}
```

**Invalid Token:**
```json
HTTP/1.1 401 Unauthorized
Content-Type: application/json

{
  "error": "unauthorized",
  "message": "Unauthorized: Invalid token signature"
}
```

**Auth Service Unavailable:**
```json
HTTP/1.1 503 Service Unavailable
Content-Type: application/json

{
  "error": "auth_service_unavailable",
  "message": "Auth service unavailable: Connection timeout"
}
```

**Auth Service Error (500):**
```json
HTTP/1.1 502 Bad Gateway
Content-Type: application/json

{
  "error": "auth_service_error",
  "message": "Auth service error (500 Internal Server Error): Database connection failed"
}
```

Note: Error bodies from the auth service are capped at 500 characters to prevent DoS attacks.

## Auth Service Implementation Contract

Your external authentication service MUST implement the following contract:

### Request Format

- **Method**: POST
- **Content-Type**: `application/jwt`
- **Body**: JWT string (signed with RS256 or ES256)

### Response Format

The auth service should return:

1. **Success (200 OK)**
   - Status: `200 OK`
   - Body: Optional (ignored by Sayna)
   - Meaning: Token is valid, allow the request

2. **Unauthorized (401)**
   - Status: `401 Unauthorized`
   - Body: Optional error message (capped at 500 chars)
   - Meaning: Token is invalid, deny the request

3. **Server Errors (5xx)**
   - Status: `500`, `503`, etc.
   - Body: Optional error message (capped at 500 chars)
   - Meaning: Temporary auth service issue, Sayna returns 502

### JWT Verification

Your auth service MUST:
1. Verify JWT signature using the public key
2. Check `exp` claim (JWT expiration)
3. Validate `iat` claim is not too old (prevent replay attacks)
4. Extract and validate the bearer token from `auth_data.token`
5. Optionally use `auth_data.request_body`, `auth_data.request_headers`, etc. for context-aware authorization

### Example Verification (Pseudo-code)

```python
def verify_auth_request(jwt_string, public_key):
    try:
        # Verify JWT signature and expiration
        payload = jwt.decode(jwt_string, public_key, algorithms=['RS256', 'ES256'])

        # Check standard claims
        if payload['sub'] != 'sayna-auth':
            return 401, "Invalid JWT subject"

        # Check issued-at time (prevent replay attacks)
        now = int(time.time())
        if abs(now - payload['iat']) > 60:  # Allow 60 second window
            return 401, "JWT too old"

        # Extract auth data
        auth_data = payload['auth_data']
        bearer_token = auth_data['token']

        # Validate bearer token (your custom logic)
        if not is_valid_token(bearer_token):
            return 401, "Invalid bearer token"

        # Optionally check request context
        if auth_data['request_path'] == '/admin' and not is_admin(bearer_token):
            return 401, "Insufficient permissions"

        return 200, "OK"

    except jwt.InvalidSignatureError:
        return 401, "Invalid JWT signature"
    except jwt.ExpiredSignatureError:
        return 401, "JWT expired"
    except Exception as e:
        return 500, f"Internal error: {str(e)}"
```

## WebSocket Authentication

WebSocket authentication is currently not implemented (see action plan task 8). Options for future implementation:

### Option A: Authenticate During HTTP Upgrade
Extract token from query parameters or headers during the WebSocket upgrade handshake.

```javascript
const ws = new WebSocket('ws://localhost:3001/ws?token=bearer-token');
```

### Option B: First Message Authentication
Require authentication in the first WebSocket message (Config message).

### Option C: No WebSocket Auth
Keep WebSocket open and only protect REST endpoints.

**Current Implementation**: Option C (no WebSocket auth)

## Security Considerations

### Best Practices

1. **Private Key Security**
   - Store private key with `chmod 600` permissions
   - Never commit private key to version control
   - Use environment variables for key path
   - Rotate keys periodically

2. **Network Security**
   - Use HTTPS for auth service in production
   - Use TLS for Sayna in production
   - Consider mutual TLS between Sayna and auth service

3. **Token Validation**
   - Implement replay attack protection in auth service
   - Check JWT timestamp to reject old requests
   - Validate JWT expiration
   - Implement rate limiting

4. **Error Handling**
   - Don't leak sensitive information in error messages
   - Log authentication failures for security monitoring
   - Monitor auth service availability

### Why JWT Signing?

JWT signing ensures:
- **Request Integrity**: Auth service receives unmodified request data
- **Source Authentication**: Auth service knows request comes from legitimate Sayna instance
- **Context-Aware Authorization**: Auth service can make decisions based on full request context

### Replay Attack Protection

The auth service should reject requests with old `iat` (Issued At) timestamps to prevent replay attacks:

```javascript
const MAX_AGE = 60; // seconds
const now = Math.floor(Date.now() / 1000);
if (Math.abs(now - payload.iat) > MAX_AGE) {
  return res.status(401).send('Request too old');
}
```

The `iat` claim is a standard JWT field that indicates when the JWT was created. Combined with the `exp` (expiration) claim, this provides robust protection against replay attacks.

## Troubleshooting

### Authentication Not Working

#### For API Secret Authentication

1. **Check configuration**:
   ```bash
   # Verify environment variables are set
   echo $AUTH_REQUIRED
   echo $AUTH_API_SECRET
   ```

2. **Verify token matches**:
   ```bash
   # Make sure you're sending the exact same string
   # API secret comparison is case-sensitive
   curl -v -X POST http://localhost:3001/speak \
     -H "Authorization: Bearer $AUTH_API_SECRET" \
     -H "Content-Type: application/json" \
     -d '{"text": "test"}'
   ```

3. **Review logs**:
   ```bash
   # Look for: "API secret authentication enabled"
   # Or: "API secret authentication failed: token mismatch"
   ```

#### For JWT-Based Authentication

1. **Check configuration**:
   ```bash
   # Verify environment variables are set
   echo $AUTH_REQUIRED
   echo $AUTH_SERVICE_URL
   echo $AUTH_SIGNING_KEY_PATH
   ```

2. **Verify key file**:
   ```bash
   # Check file exists and has correct permissions
   ls -la /path/to/auth_private_key.pem

   # Verify key format
   openssl rsa -in auth_private_key.pem -check  # for RSA
   openssl ec -in auth_private_key.pem -check   # for ECDSA
   ```

3. **Check auth service**:
   ```bash
   # Test auth service is reachable
   curl -X POST $AUTH_SERVICE_URL -d "test"
   ```

4. **Review logs**:
   ```bash
   # Look for: "JWT authentication enabled"
   # Or: "JWT authentication failed"
   ```

### Common Errors

| Error | Cause | Solution |
|-------|-------|----------|
| 401 Unauthorized | Missing or invalid token | Include valid `Authorization: Bearer {token}` header |
| 401 "Invalid API secret" | Token doesn't match AUTH_API_SECRET | Check token is exactly the same as configured secret (case-sensitive) |
| 500 "Auth required but no method configured" | AUTH_REQUIRED=true but no auth method set | Set either AUTH_API_SECRET or (AUTH_SERVICE_URL + AUTH_SIGNING_KEY_PATH) |
| 503 Service Unavailable | Auth service unreachable (JWT mode) | Verify auth service is running and reachable |

## Performance Considerations

### API Secret Authentication
- **Fastest option**: No external service calls, just string comparison
- **Zero latency overhead**: Authentication happens in microseconds
- **No network dependencies**: No risk of auth service downtime
- **Best for**: High-throughput, latency-sensitive applications

### JWT-Based Authentication
- **External service latency**: Adds network round-trip time to each request
- **Connection Pooling**: HTTP client pools connections to minimize overhead
- **Timeout**: Configurable timeout prevents hanging requests (default: 5s)
- **Async**: All auth operations are non-blocking
- **Caching**: Consider implementing token caching in future (not currently implemented)

### Optimization Tips

1. **For JWT mode - Increase timeout** for slow auth services:
   ```bash
   AUTH_TIMEOUT_SECONDS=10
   ```

2. **For JWT mode - Deploy auth service close to Sayna** to reduce network latency

3. **For JWT mode - Monitor auth service performance** and scale as needed

4. **Consider API Secret** if you don't need per-user authorization logic

## Development and Testing

### Disable Auth for Development

```bash
# In .env file
AUTH_REQUIRED=false
```

### Test Mode

For testing without a real auth service, keep `AUTH_REQUIRED=false` or implement a mock auth service.

### Integration Tests

See `tests/auth_integration_test.rs` for examples of testing authentication middleware.

## Choosing Between Authentication Methods

Use this decision guide to select the right authentication method:

### Use API Secret Authentication When:
✅ Single tenant or small number of known clients
✅ Same authorization level for all authenticated requests
✅ Performance/latency is critical
✅ Simple deployment without external services
✅ Quick setup and minimal configuration needed

### Use JWT-Based Authentication When:
✅ Multi-tenant application
✅ Different users need different permissions
✅ Need context-aware authorization (different permissions per endpoint)
✅ Integration with existing user management system
✅ Audit trail and detailed access logs required
✅ Token validation logic changes frequently

### Can I Use Both?
Yes! Both methods can coexist:
- Configure both `AUTH_API_SECRET` and `AUTH_SERVICE_URL`
- API Secret is checked first (takes priority)
- Useful for: admin access via secret + user access via JWT

## Alternative Approaches Comparison

| Approach | Pros | Cons | Use Case |
|----------|------|------|----------|
| **API Secret** (Current) | Simple, fast, no external deps | Single shared secret, less flexible | Simple deployments |
| **JWT Validation** (Current) | Flexible, per-user auth, context-aware | Requires external service, latency overhead | Complex auth requirements |
| **OAuth2 Proxy** | Standardized, battle-tested | Additional infrastructure, not integrated | Enterprise deployments |
| **Embedded JWT** | Fast, no external service | Tightly coupled, hard to update logic | Standalone apps |

Sayna's dual authentication approach combines the simplicity of API secrets with the flexibility of JWT validation, giving you the best of both worlds.
