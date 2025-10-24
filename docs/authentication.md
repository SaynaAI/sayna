# Authentication System

## Overview

Sayna implements a customer-based authentication system that delegates token validation to an external authentication service. This design provides flexibility for implementing custom authorization logic while maintaining security through JWT signing to prevent request tampering.

## Architecture

### Authentication Flow

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

### Components

1. **Authentication Middleware** (`src/middleware/auth.rs`)
   - Intercepts HTTP requests to protected endpoints
   - Extracts and validates Authorization header format
   - Buffers request body and headers for auth validation
   - Calls AuthClient to validate tokens

2. **Auth Client** (`src/auth/client.rs`)
   - HTTP client for communicating with external auth service
   - Signs auth payloads using JWT
   - Handles connection pooling and timeouts

3. **JWT Signing Module** (`src/auth/jwt.rs`)
   - Signs auth request payloads with private key
   - Supports RSA and ECDSA keys in PEM format
   - Includes timestamp and 5-minute expiration

4. **Error Handling** (`src/errors/auth_error.rs`)
   - Comprehensive error types for auth failures
   - Proper HTTP status code mapping

## Setup

### 1. Generate Signing Keys

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

### 2. Configure Environment Variables

Add the following to your `.env` file:

```bash
# Authentication configuration
AUTH_REQUIRED=true
AUTH_SERVICE_URL=https://your-auth-service.com/auth
AUTH_SIGNING_KEY_PATH=/path/to/auth_private_key.pem
AUTH_TIMEOUT_SECONDS=5
```

### 3. Implement Auth Service

Your external authentication service must:

1. Accept POST requests with JWT in the body
2. Verify JWT signature using the public key
3. Validate the bearer token in the JWT payload
4. Return:
   - `200 OK` if token is valid
   - `401 Unauthorized` if token is invalid
   - Any other status code for errors

#### Example Auth Service (Node.js + Express)

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
| `AUTH_SERVICE_URL` | Yes* | - | External auth service endpoint |
| `AUTH_SIGNING_KEY_PATH` | Yes* | - | Path to RSA/ECDSA private key (PEM format) |
| `AUTH_TIMEOUT_SECONDS` | No | `5` | Auth request timeout in seconds |

*Required when `AUTH_REQUIRED=true`

## JWT Payload Specification

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

```bash
# With valid token
curl -X POST http://localhost:3001/speak \
  -H "Authorization: Bearer your-token-here" \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world", "voice": "en-US-JennyNeural"}'

# Without token (will fail if auth is enabled)
curl -X POST http://localhost:3001/speak \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world"}'
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
   # Sayna logs authentication events
   # Look for: "Authentication enabled", "Auth client not initialized", etc.
   ```

### Common Errors

| Error | Cause | Solution |
|-------|-------|----------|
| 401 Unauthorized | Missing or invalid token | Include valid `Authorization: Bearer {token}` header |
| 500 Internal Server Error | Auth misconfigured | Check AUTH_SERVICE_URL and AUTH_SIGNING_KEY_PATH are set |
| 503 Service Unavailable | Auth service unreachable | Verify auth service is running and reachable |

## Performance Considerations

- **Connection Pooling**: HTTP client pools connections to auth service
- **Timeout**: Configurable timeout prevents hanging requests
- **Async**: All auth operations are non-blocking
- **Caching**: Consider implementing token caching in future (not currently implemented)

### Optimization Tips

1. **Increase timeout** for slow auth services:
   ```bash
   AUTH_TIMEOUT_SECONDS=10
   ```

2. **Deploy auth service close to Sayna** to reduce network latency

3. **Monitor auth service performance** and scale as needed

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

## Alternative Approaches

### 1. Direct Token Forwarding
Send bearer token directly to auth service with a shared secret. Simpler but less flexible.

### 2. OAuth2 Proxy Pattern
Use oauth2-proxy or similar in front of Sayna. Standardized but requires additional infrastructure.

### 3. Embedded Token Validation
Validate JWT tokens directly in Sayna (no external service). Faster but less flexible for multi-tenant scenarios.

The current JWT signing approach provides maximum flexibility for complex authorization requirements.
