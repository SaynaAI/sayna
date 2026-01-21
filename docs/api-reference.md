# Sayna Architecture & API Guide

Sayna is a high-performance, real-time voice server built with Rust, Axum, and Tokio. It unifies multiple speech providers, streams audio/text over WebSockets, exposes a concise REST surface, and can mirror audio into LiveKit rooms for WebRTC participants.

> Authentication note: Optional authentication and authorization strategies are documented separately in `docs/authentication.md`.

## Platform Overview

- Unified STT/TTS pipeline that polyfills provider differences and enforces the same message schema across transports.
- Bidirectional WebSocket endpoint (`/ws`) for low-latency audio streaming, TTS commands, LiveKit coordination, and control signals.
- REST endpoints for health checks, voice discovery, one-shot TTS synthesis, and LiveKit token issuance.
- Pluggable provider layer with Deepgram (STT + TTS), ElevenLabs (STT + TTS), Google (STT + TTS), and Microsoft Azure (STT) adapters; adding providers requires implementing the trait in `src/core/stt` or `src/core/tts`.
- Optional DSP layers: Silero-VAD voice activity detection with ONNX-based turn detection (`stt-vad` feature) and DeepFilterNet noise suppression (`noise-filter` feature).
- Request pooling, adaptive retry logic, and binary audio caching so repeated prompts replay instantly while respecting provider rate limits.

## Core Architecture

### End-to-End Flow
1. A client connects to `/ws` and sends a `config` message describing STT/TTS providers (and optional LiveKit settings).
2. `AppState` (`src/state/mod.rs`) injects provider credentials, cache handles, request managers, and LiveKit helpers needed for the session.
3. `VoiceManager` (`src/core/voice_manager/manager.rs`) spins up the requested providers, registers callbacks, and emits `ready` once both legs are online.
4. Binary audio frames from the WebSocket or LiveKit participants flow into the speech-to-text provider; interim and final transcripts are surfaced as `stt_result`.
5. `speak` messages (or LiveKit data topics) queue TTS jobs; results are streamed back over the socket, optionally cached, and, when configured, piped to LiveKit tracks.
6. REST endpoints reuse the same building blocks: `/voices` fans out to provider APIs, `/speak` instantiates a transient TTS provider, and `/livekit/token` delegates to `LiveKitRoomHandler`.

### Major Components

| Component | Location | Description |
| --- | --- | --- |
| VoiceManager | `src/core/voice_manager/manager.rs` | Owns STT/TTS providers, debounces speech-final events, and routes callbacks to WebSocket + LiveKit sinks. |
| Provider layer | `src/core/stt/*`, `src/core/tts/*` | Trait-based adapters that normalize provider-specific options, handle retries, and expose metrics. |
| WebSocket stack | `src/handlers/ws/*` | Parses messages, validates configs, orchestrates LiveKit setup, and streams audio/data. |
| LiveKit integration | `src/livekit/*` | Token creation, room management, recording hooks, and participant filtering for mirrored audio. |
| Noise & turn detection | `src/utils/noise_filter.rs`, `src/core/turn_detect/*` | Optional DSP stages for cleaner audio and better speech-final timing. |
| Request & cache utilities | `src/utils/req_manager.rs`, `src/core/cache/*` | Shared HTTP/2 pools plus filesystem/in-memory caches for previously synthesized audio. |
| Routing layer | `src/main.rs`, `src/routes/*`, `src/handlers/*` | Axum routers, REST handlers, and middleware wiring. |

### Project Layout

| Path | Purpose |
| --- | --- |
| `src/main.rs` | Server bootstrap, router wiring, feature flag hooks, and graceful shutdown. |
| `src/config.rs` | Environment variable parsing plus validation for LiveKit, cache, and optional security settings. |
| `src/handlers/` | REST and WebSocket handlers (`api.rs`, `voices.rs`, `speak.rs`, `livekit.rs`, `ws/`). |
| `src/core/` | Provider implementations, cache infrastructure, voice manager, turn detection, and shared state. |
| `src/livekit/` | Client + manager used to join rooms, publish audio, and start/stop recording egress. |
| `src/utils/` | Reusable utilities such as the HTTP request manager and noise filtering helpers. |
| `tests/` | Integration tests covering REST, WebSocket flows, and LiveKit/token handling. |
| `docs/` | Human-facing documentation (this file, authentication guide, API reference assets). |

### Build-Time Feature Flags

| Feature | Default | Effect |
| --- | --- | --- |
| `stt-vad` | Disabled | Enables Silero-VAD voice activity detection with integrated ONNX-based Smart Turn detection. This is the **exclusive source** of `is_speech_final` in STT responses. VAD detects silence and Smart Turn confirms turn completion. |
| `noise-filter` | Disabled | Activates DeepFilterNet-based denoising before STT ingestion and LiveKit playback. Disable for lower CPU usage. |
| `openapi` | Disabled | Compiles utoipa annotations and exposes the CLI generator (`cargo run --features openapi -- openapi`). |

### Configuration & Environment

| Variable | Description | Default |
| --- | --- | --- |
| `HOST`, `PORT` | Bind address and port for the Axum server. | `0.0.0.0`, `3001` |
| `DEEPGRAM_API_KEY` | API key for Deepgram STT/TTS. Required when Deepgram is selected. | – |
| `ELEVENLABS_API_KEY` | API key for ElevenLabs STT/TTS. Required when ElevenLabs voices or synthesis are used. | – |
| `AZURE_SPEECH_SUBSCRIPTION_KEY` | Subscription key for Microsoft Azure Speech Services. Required when Azure STT is selected. | – |
| `AZURE_SPEECH_REGION` | Azure region where the Speech resource was created (e.g., `eastus`, `westeurope`). | `eastus` |
| `LIVEKIT_URL` | Internal LiveKit WebSocket URL (used by the server). | `ws://localhost:7880` |
| `LIVEKIT_PUBLIC_URL` | URL the client should dial; returned in `/livekit/token` responses and WebSocket `ready`. | `http://localhost:7880` |
| `LIVEKIT_API_KEY`, `LIVEKIT_API_SECRET` | Credentials for generating LiveKit tokens on the server side. | – |
| `RECORDING_S3_*` | Bucket, region, endpoint, access key, and secret for LiveKit recording egress. Recording is skipped if any are missing. | – |
| `CACHE_PATH` | Filesystem path for persisted audio cache. Falls back to in-memory cache when unset. | In-memory |
| `CACHE_TTL_SECONDS` | TTL for cached TTS payloads. | 30 days |
| `AUTH_*` | Optional authentication variables; see `docs/authentication.md`. | – |

## API Surface

All responses use JSON unless otherwise noted. Errors follow the shape `{ "error": "<message>" }`.

### REST Endpoints

#### `GET /`
- **Purpose**: Liveness/health check.
- **Response** `200 OK`:
  ```json
  { "status": "OK" }
  ```

#### `GET /voices`
- **Purpose**: Aggregate available TTS voices per provider by querying external APIs at request time.
- **Success** `200 OK`: JSON object keyed by provider (`"deepgram"`, `"elevenlabs"`, ...). Each value is an array of descriptors:

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Provider-specific identifier or canonical name. |
| `sample` | string | Preview audio URL (may be empty). |
| `name` | string | Human-friendly name. |
| `accent` | string | Accent or dialect (falls back to `"Unknown"`). |
| `gender` | string | Derived gender label when metadata is available. |
| `language` | string | Primary language; Deepgram codes are converted to readable names when possible. |

- **Failure** `500 Internal Server Error`: Emitted when credentials are missing or an upstream call fails.

#### `POST /speak`
- **Purpose**: One-shot text-to-speech synthesis that returns binary audio.
- **Request Body** (`application/json`):

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `text` | string | Yes | Text to synthesize; trimmed text must be non-empty. |
| `tts_config` | object | Yes | Provider configuration with no API keys (schema below). |

**TTS configuration object**

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `provider` | string | Yes | `deepgram` or `elevenlabs`. |
| `model` | string | Yes | Provider model identifier. |
| `voice_id` | string | No | Voice/model variant (provider default when omitted). |
| `speaking_rate` | number | No | Multiplier applied to playback speed (default `1.0`). |
| `audio_format` | string | No | Output codec such as `linear16`, `mp3`, `ogg`, `wav`. |
| `sample_rate` | integer | No | Target sample rate in Hz (default `24000`). |
| `connection_timeout` | integer | No | Seconds to wait while connecting to provider (default `30`). |
| `request_timeout` | integer | No | Seconds to wait for synthesis (default `60`). |
| `pronunciations` | array | No | Replacement rules applied before synthesis. Each entry contains `word` and `pronunciation`. |

- **Success** `200 OK`: Binary audio payload with headers:
  - `Content-Type`: Derived from the provider format (PCM, WAV, MP3, OGG…).
  - `Content-Length`: Byte length.
  - `x-audio-format`: Raw format identifier from the provider.
  - `x-sample-rate`: Sample rate used to render the clip.

- **Failure**:
  - `400 Bad Request` when `text` is empty.
  - `500 Internal Server Error` for credential issues or synthesis errors.

#### `POST /livekit/token`
- **Purpose**: Issue a LiveKit access token for a participant so clients can join a room.
- **Room Creation**: When authentication is enabled (`auth.id` is present), this endpoint **creates the room if it doesn't exist** and sets `room.metadata.auth_id` to the authenticated tenant's ID before issuing any token. This ensures room ownership is established before participants can join.
- **Tenant Isolation**: If the room already exists with a different `auth_id`, the request is rejected with `403 Forbidden`. This prevents cross-tenant access to rooms.
- **Token Timing**: The token is only issued **after** the room exists and metadata is verified/set, ensuring no participant can join before ownership is established.
- **Request Body**:

| Field | Type | Description |
| --- | --- | --- |
| `room_name` | string | Room to join/create. |
| `participant_name` | string | Display name shown inside LiveKit. |
| `participant_identity` | string | Stable identity string for LiveKit permissions. |

- **Success** `200 OK`:

| Field | Type | Description |
| --- | --- | --- |
| `token` | string | Signed LiveKit JWT for the participant. |
| `room_name` | string | The room name (unchanged from request). |
| `participant_identity` | string | Echo of the requested identity. |
| `livekit_url` | string | Client-facing LiveKit URL pulled from server config. |

- **Failure**:
  - `400 Bad Request` when any field is empty.
  - `403 Forbidden` if the room exists with a different tenant's `auth_id`.
  - `500 Internal Server Error` if LiveKit credentials are not configured, room creation fails, or token generation fails.

#### `GET /livekit/rooms`
- **Purpose**: List all LiveKit rooms belonging to the authenticated tenant. Rooms are filtered by `room.metadata.auth_id` for tenant isolation.
- **Tenant Isolation**: When `auth.id` is present, only rooms where `metadata.auth_id` matches are returned. When `auth.id` is absent, all rooms are returned (backward-compatible mode).
- **Success** `200 OK`:
  ```json
  {
    "rooms": [
      {
        "name": "conversation-room-123",
        "num_participants": 2,
        "creation_time": 1703123456
      }
    ]
  }
  ```

| Field | Type | Description |
| --- | --- | --- |
| `rooms` | array | List of rooms belonging to the authenticated client (filtered by metadata.auth_id). |
| `rooms[].name` | string | The room name. |
| `rooms[].num_participants` | integer | Number of current participants in the room. |
| `rooms[].creation_time` | integer | Room creation time (Unix timestamp in seconds). |

- **Failure**:
  - `500 Internal Server Error` if LiveKit credentials are not configured or listing fails.

#### `GET /livekit/rooms/{room_name}`
- **Purpose**: Get detailed information about a specific LiveKit room including all current participants. Access is authorized via `room.metadata.auth_id` check.
- **Tenant Isolation**: When `auth.id` is present, access requires `room.metadata.auth_id == auth.id`. When `auth.id` is absent, access is allowed (backward-compatible mode).
- **Path Parameters**:

| Parameter | Type | Description |
| --- | --- | --- |
| `room_name` | string | Name of the room to retrieve. |

- **Success** `200 OK`:
  ```json
  {
    "sid": "RM_xyz789",
    "name": "conversation-room-123",
    "num_participants": 2,
    "max_participants": 10,
    "creation_time": 1703123456,
    "metadata": "{\"auth_id\": \"tenant-123\"}",
    "active_recording": false,
    "participants": [
      {
        "sid": "PA_abc123",
        "identity": "user-alice-456",
        "name": "Alice Smith",
        "state": "ACTIVE",
        "kind": "STANDARD",
        "joined_at": 1703123456,
        "metadata": "",
        "attributes": {},
        "is_publisher": true
      }
    ]
  }
  ```

| Field | Type | Description |
| --- | --- | --- |
| `sid` | string | Unique session ID for the room. |
| `name` | string | The room name. |
| `num_participants` | integer | Number of current participants. |
| `max_participants` | integer | Maximum allowed participants (0 = no limit). |
| `creation_time` | integer | Room creation time (Unix timestamp in seconds). |
| `metadata` | string | Room metadata (JSON). Contains `auth_id` for tenant isolation. |
| `active_recording` | boolean | Whether a recording is currently active. |
| `participants` | array | List of participants currently in the room. |
| `participants[].sid` | string | Unique session ID for the participant. |
| `participants[].identity` | string | Unique identifier provided when connecting. |
| `participants[].name` | string | Display name of the participant. |
| `participants[].state` | string | Participant state: `JOINING`, `JOINED`, `ACTIVE`, `DISCONNECTED`, or `UNKNOWN`. |
| `participants[].kind` | string | Participant kind: `STANDARD`, `AGENT`, `SIP`, `EGRESS`, `INGRESS`, or `UNKNOWN`. |
| `participants[].joined_at` | integer | Timestamp when participant joined (Unix timestamp in seconds). |
| `participants[].metadata` | string | User-specified metadata for the participant. |
| `participants[].attributes` | object | User-specified attributes (key-value pairs). |
| `participants[].is_publisher` | boolean | Whether the participant is publishing audio/video. |

- **Failure**:
  - `400 Bad Request` when room name is empty.
  - `404 Not Found` when the room does not exist or access is denied (masked as not found).
  - `500 Internal Server Error` if LiveKit credentials are not configured.

#### `DELETE /livekit/participant`
- **Purpose**: Remove a participant from a LiveKit room, forcibly disconnecting them. Access is authorized via `room.metadata.auth_id` check.
- **Tenant Isolation**: When `auth.id` is present, requires `room.metadata.auth_id == auth.id`. When `auth.id` is absent, access is allowed (backward-compatible mode).
- **Note**: This does not invalidate the participant's token. To prevent rejoining, use short-lived tokens.
- **Request Body**:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `room_name` | string | Yes | The LiveKit room name. |
| `participant_identity` | string | Yes | The identity of the participant to remove. |

- **Success** `200 OK`:
  ```json
  {
    "status": "removed",
    "room_name": "conversation-room-123",
    "participant_identity": "user-alice-456"
  }
  ```

- **Failure**:
  - `400 Bad Request` when room name or participant identity is empty.
  - `404 Not Found` when the room or participant does not exist or access is denied (masked as not found).
  - `500 Internal Server Error` if LiveKit credentials are not configured or removal fails.

**Error response format**:
```json
{
  "error": "Participant 'user-123' not found in room",
  "code": "PARTICIPANT_NOT_FOUND"
}
```

| Error Code | Description |
| --- | --- |
| `INVALID_REQUEST` | Empty room name or participant identity. |
| `PARTICIPANT_NOT_FOUND` | Room or participant not found. |
| `LIVEKIT_NOT_CONFIGURED` | LiveKit service not configured. |
| `REMOVAL_FAILED` | Failed to remove participant. |

#### `POST /livekit/participant/mute`
- **Purpose**: Mute or unmute a participant's published track. Access is authorized via `room.metadata.auth_id` check.
- **Tenant Isolation**: When `auth.id` is present, requires `room.metadata.auth_id == auth.id`. When `auth.id` is absent, access is allowed (backward-compatible mode).
- **Request Body**:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `room_name` | string | Yes | The LiveKit room name. |
| `participant_identity` | string | Yes | The identity of the participant whose track to mute. |
| `track_sid` | string | Yes | The session ID of the track to mute/unmute. |
| `muted` | boolean | Yes | `true` to mute, `false` to unmute. |

- **Success** `200 OK`:
  ```json
  {
    "room_name": "conversation-room-123",
    "participant_identity": "user-alice-456",
    "track_sid": "TR_abc123",
    "muted": true
  }
  ```

- **Failure**:
  - `400 Bad Request` when any required field is empty.
  - `404 Not Found` when the room, participant, or track does not exist or access is denied (masked as not found).
  - `500 Internal Server Error` if LiveKit credentials are not configured or mute operation fails.

**Error response format**: Same as `DELETE /livekit/participant`.

#### `POST /sip/transfer`
- **Purpose**: Initiate a SIP REFER transfer for a participant in a LiveKit room. The transfer moves an ongoing SIP call to a different phone number.
- **Note**: Only SIP participants can be transferred. A successful response indicates the transfer has been **initiated**, not necessarily completed.
- **Tenant Isolation**: When `auth.id` is present, access requires `room.metadata.auth_id == auth.id`. When `auth.id` is absent, access is allowed (backward-compatible mode).
- **Request Body**:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `room_name` | string | Yes | The LiveKit room name. |
| `participant_identity` | string | Yes | The identity of the SIP participant to transfer. |
| `transfer_to` | string | Yes | Phone number to transfer to. Supports international (+1234567890), national (07123456789), or extensions (1234). |

- **Success** `200 OK`:
  ```json
  {
    "status": "initiated",
    "room_name": "call-room-123",
    "participant_identity": "sip_participant_456",
    "transfer_to": "tel:+15551234567"
  }
  ```

| Field | Type | Description |
| --- | --- | --- |
| `status` | string | `initiated` or `completed`. |
| `room_name` | string | The room name. |
| `participant_identity` | string | The identity of the transferred participant. |
| `transfer_to` | string | The normalized phone number with `tel:` prefix. |

- **Failure**:
  - `400 Bad Request` when phone number is invalid or fields are empty.
  - `404 Not Found` when room is not accessible (masked), or participant is not found or is not a SIP participant.
  - `500 Internal Server Error` if LiveKit SIP service is not configured or transfer fails.

**Error response format**:
```json
{
  "error": "Participant 'sip_123' not found or is not a SIP participant",
  "code": "PARTICIPANT_NOT_FOUND"
}
```

| Error Code | Description |
| --- | --- |
| `INVALID_PHONE_NUMBER` | Invalid phone number format. |
| `PARTICIPANT_NOT_FOUND` | Participant not found or not a SIP participant. |
| `LIVEKIT_NOT_CONFIGURED` | LiveKit SIP service not configured. |
| `TRANSFER_FAILED` | Transfer operation failed. |
| `ROOM_ACCESS_DENIED` | Room not accessible (belongs to different tenant). |

#### `POST /sip/call`
- **Purpose**: Initiate an outbound SIP call through LiveKit. The call connects to a specified LiveKit room as a SIP participant.
- **Note**: This endpoint requires either the global `sip.outbound_address` configuration or a per-request `sip.outbound_address` in the request body. Outbound trunks are automatically created or reused based on the `from_phone_number`.
- **Per-Request SIP Config**: The optional `sip` object in the request body allows overriding global SIP configuration on a per-request basis. Request body values take priority over global config, enabling different SIP providers or credentials for specific calls.
- **Tenant Isolation**: When `auth.id` is present:
  - If the room already exists, access requires `room.metadata.auth_id == auth.id`.
  - If the room doesn't exist, it will be created and `metadata.auth_id` will be set to the caller's `auth.id`.
  - When `auth.id` is absent, access is allowed (backward-compatible mode).
- **Request Body**:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `room_name` | string | Yes | The LiveKit room name to connect the call to. |
| `participant_name` | string | Yes | Display name for the SIP participant in the room. |
| `participant_identity` | string | Yes | Identity for the SIP participant in the room. |
| `from_phone_number` | string | Yes | Phone number the call will originate from. Must be configured in your SIP provider. Supports international format (+1234567890). |
| `to_phone_number` | string | Yes | Phone number to dial. Supports international format (+1234567890), national format (07123456789), or extensions (1234). |
| `sip` | object | No | Optional per-request SIP configuration overrides. See `SIPCallSipConfig` below. |

**`SIPCallSipConfig` object** (optional `sip` field):

| Field | Type | Description |
| --- | --- | --- |
| `outbound_address` | string\|null | SIP server address override. Format: `hostname` or `hostname:port`. Overrides global `sip.outbound_address`. |
| `auth_username` | string\|null | SIP authentication username override. Overrides global `sip.outbound_auth_username`. |
| `auth_password` | string\|null | SIP authentication password override. Overrides global `sip.outbound_auth_password`. |

**SIP Configuration Priority**: Request body `sip` config > Global server config

- **Success** `200 OK`:
  ```json
  {
    "status": "initiated",
    "room_name": "call-room-123",
    "participant_identity": "caller-456",
    "participant_id": "PA_abc123",
    "sip_call_id": "SC_xyz789"
  }
  ```

| Field | Type | Description |
| --- | --- | --- |
| `status` | string | Always `initiated` for successful requests. |
| `room_name` | string | The room name. |
| `participant_identity` | string | The identity of the SIP participant in the room. |
| `participant_id` | string | The unique participant ID assigned by LiveKit. |
| `sip_call_id` | string | The unique SIP call ID for tracking. |

- **Failure**:
  - `400 Bad Request` when phone numbers are invalid or required fields are empty.
  - `404 Not Found` when room exists with a different `auth_id` (masked as not found).
  - `500 Internal Server Error` if LiveKit is not configured, outbound address is missing, or call fails.

**Error response format**:
```json
{
  "error": "Outbound address not configured",
  "code": "OUTBOUND_ADDRESS_NOT_CONFIGURED"
}
```

| Error Code | Description |
| --- | --- |
| `INVALID_PHONE_NUMBER` | Invalid phone number format or empty required field. |
| `OUTBOUND_ADDRESS_NOT_CONFIGURED` | The `sip.outbound_address` configuration is not set. |
| `LIVEKIT_NOT_CONFIGURED` | LiveKit SIP service not configured. |
| `CALL_FAILED` | Call initiation failed (includes error details). |
| `ROOM_ACCESS_DENIED` | Room not accessible (belongs to different tenant). |

**Trunk Reuse Behavior**: When making outbound calls, Sayna automatically manages SIP outbound trunks:
1. Searches for an existing outbound trunk that contains the `from_phone_number`.
2. If found, reuses that trunk for the call.
3. If not found, creates a new trunk named `sayna-outbound-{from_phone_number}`.

This enables efficient trunk management without manual provisioning.

**Authentication Configuration**: Outbound trunk authentication is configured at the server level via SIP configuration, not per-request. If your SIP provider requires authentication, set `sip.outbound_auth_username` and `sip.outbound_auth_password` in your server configuration (YAML or environment variables). See [Outbound Trunk Authentication](livekit_integration.md#outbound-trunk-authentication) for details.

#### `GET /sip/hooks`
- **Purpose**: List all configured SIP webhook hooks from the runtime cache.
- **Behavior**: Hosts defined in the application configuration always appear and override any cached host with the same name.
- **Success** `200 OK`:
  ```json
  {
    "hooks": [
      {
        "host": "example.com",
        "url": "https://webhook.example.com/events",
        "auth_id": "tenant-123"
      }
    ]
  }
  ```

| Field | Type | Description |
| --- | --- | --- |
| `hooks` | array | List of configured SIP hooks. |
| `hooks[].host` | string | SIP domain pattern (case-insensitive). |
| `hooks[].url` | string | HTTPS URL to forward webhook events to. |
| `hooks[].auth_id` | string | Tenant identifier for room metadata. Required when `AUTH_REQUIRED=true`; optional when `AUTH_REQUIRED=false`. |

- **Failure**:
  - `500 Internal Server Error` if reading the cache fails.

#### `POST /sip/hooks`
- **Purpose**: Add or replace SIP webhook hooks at runtime. Changes persist across server restarts via the cache file (`<cache_path>/sip_hooks.json`).
- **Behavior**: Hosts defined in the application configuration are immutable and override any cached or runtime updates.
- **Request Body**:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `hooks` | array | Yes | List of hooks to add or replace. |
| `hooks[].host` | string | Yes | SIP domain pattern (case-insensitive). Existing hooks with matching hosts are replaced. |
| `hooks[].url` | string | Yes | HTTPS URL to forward webhook events to. |
| `hooks[].auth_id` | string | Conditional | Tenant identifier for room metadata. **Required** when `AUTH_REQUIRED=true`; may be empty when `AUTH_REQUIRED=false`. |

- **Success** `200 OK`: Returns the merged list of all hooks (existing + new).
  ```json
  {
    "hooks": [
      {
        "host": "example.com",
        "url": "https://webhook.example.com/events",
        "auth_id": "tenant-123"
      },
      {
        "host": "another.com",
        "url": "https://webhook.another.com/events",
        "auth_id": "tenant-456"
      }
    ]
  }
  ```

- **Failure**:
  - `400 Bad Request` when duplicate hosts are detected in the request, or when `AUTH_REQUIRED=true` and a hook has an empty or whitespace-only `auth_id`.
  - `405 Method Not Allowed` when trying to add or update a host defined in the application configuration.
  - `500 Internal Server Error` if no cache path is configured or writing fails.

**Note**: Secrets are NOT stored in the runtime cache. Hooks added via this endpoint will use the global `hook_secret` from the server configuration for webhook signing.

#### `DELETE /sip/hooks`
- **Purpose**: Remove SIP webhook hooks by host name. Changes persist across server restarts. If a deleted host exists in the original server configuration, it will revert to its config value after deletion.
- **Behavior**: Hosts defined in the application configuration are immutable and cannot be removed.
- **Request Body**:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `hosts` | array | Yes | List of host names to remove (case-insensitive). Must contain at least one host. |

- **Success** `200 OK`: Returns the updated list of hooks after deletion.
  ```json
  {
    "hooks": [
      {
        "host": "remaining.com",
        "url": "https://webhook.remaining.com/events"
      }
    ]
  }
  ```

- **Failure**:
  - `400 Bad Request` when the `hosts` array is empty.
  - `405 Method Not Allowed` when trying to delete a host defined in the application configuration.
  - `500 Internal Server Error` if no cache path is configured or writing fails.

**Note**: Deleting a host that exists in the original server configuration will cause it to revert to its config value (and config-defined hosts themselves cannot be removed). The runtime state is always a merge of config + cache, so removing a host from cache only affects runtime-added entries or cached overrides.

### WebSocket Endpoint (`GET /ws`)

#### Connection Lifecycle
1. Connect via WebSocket and immediately send a `config` message.
2. The server initializes providers (and LiveKit, if requested) and replies with `ready`.
3. Stream audio frames as binary messages that match the declared STT sample rate/encoding.
4. Use `speak`, `clear`, or `send_message` commands to drive TTS and LiveKit data.
5. Close the socket when finished; the server also closes when a fatal `error` is emitted.

#### Incoming Messages

##### `config`
Configures audio processing and optional LiveKit mirroring. Must be the first message.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `type` | string | Yes | Always `config`. |
| `stream_id` | string | No | Session identifier used for recording path; server generates a UUID when omitted. |
| `audio` | boolean | No | Defaults to `true`. When `false`, STT/TTS are skipped but LiveKit messaging can still be used. |
| `stt_config` | object | Conditional | Required when `audio=true`. See table below. |
| `tts_config` | object | Conditional | Required when `audio=true`. Same schema as the REST `tts_config`. |
| `livekit` | object | No | LiveKit options; omitted for WebSocket-only sessions. |

**STT configuration**

| Field | Type | Description |
| --- | --- | --- |
| `provider` | string | Provider identifier (`deepgram`, `google`, `elevenlabs`, or `microsoft-azure`). |
| `language` | string | Locale such as `en-US`. |
| `sample_rate` | integer | Expected sample rate for inbound audio. |
| `channels` | integer | Channel count (1 = mono). |
| `punctuation` | boolean | Enables/disables punctuation in transcripts. |
| `encoding` | string | Audio encoding label (e.g., `linear16`). |
| `model` | string | Provider model name. |

**LiveKit configuration**

| Field | Type | Description |
| --- | --- | --- |
| `room_name` | string | Room to join or create. |
| `enable_recording` | boolean | Starts a room composite recording to `{server_prefix}/{stream_id}/audio.ogg` when `true`. |
| `sayna_participant_identity` | string | Override for the agent identity (default `sayna-ai`). |
| `sayna_participant_name` | string | Override for the agent display name (default `Sayna AI`). |
| `listen_participants` | array<string> | Restrict audio/data processing to specific participant identities (empty list listens to all). |

##### `speak`
Queues text for synthesis.

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | `speak`. |
| `text` | string | Text to synthesize. |
| `flush` | boolean | When `true`, drops any buffered audio before enqueuing the new request (default `true`). |
| `allow_interruption` | boolean | When `false`, blocks subsequent `speak`/`clear` until playback finishes (default `true`). |

##### `clear`
Immediately clears queued audio and LiveKit buffers. Useful for interruptions.

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | `clear`. |

##### `send_message`
Publishes a LiveKit data message (if LiveKit is configured) and also feeds the VoiceManager message bus.

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | `send_message`. |
| `message` | string | Payload string. |
| `role` | string | Application-specific role label (`user`, `assistant`, etc.). |
| `topic` | string | Optional LiveKit topic (defaults to `messages`). |
| `debug` | object | Optional JSON metadata that downstream consumers can inspect. |

##### Binary audio frames
Send raw audio bytes that match `sample_rate`, `channels`, and `encoding` supplied in `stt_config`. Frames are forwarded to the STT provider and, if LiveKit mirroring is active, also injected into the room.

#### Outgoing Messages

##### `ready`

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | `ready`. |
| `livekit_room_name` | string | Present when LiveKit is configured. |
| `livekit_url` | string | Public LiveKit URL (mirrors `LIVEKIT_PUBLIC_URL`). |
| `sayna_participant_identity` | string | Agent identity used inside LiveKit (if applicable). |
| `sayna_participant_name` | string | Agent display name (if applicable). |

##### `stt_result`

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | `stt_result`. |
| `transcript` | string | Recognized text. |
| `is_final` | boolean | `true` when no more updates are expected for the utterance. |
| `is_speech_final` | boolean | `true` when the speaker's turn is complete. Determined exclusively by VAD + Smart Turn detection (when `stt-vad` feature is enabled), not by STT providers. |
| `confidence` | number | Provider-supplied confidence score. |

**Speech Final Detection**

The `is_speech_final` flag is set exclusively by Sayna's VAD + Smart Turn detection system (when the `stt-vad` feature is enabled). STT providers do not set this flag. The detection process:

1. **Silero-VAD** monitors audio for silence
2. When silence exceeds the configured threshold (default 200ms), **Smart Turn** is triggered
3. Smart Turn analyzes accumulated audio to confirm turn completion
4. If confirmed, `is_speech_final=true` is emitted

**Speech Final Timing Behavior**

When `stt-vad` is enabled, Sayna implements a fallback system to ensure every utterance receives a `speech_final` event:

1. **Primary path (VAD + Smart Turn)**: Silero-VAD detects silence, then Smart Turn confirms turn completion. This is the normal path for speech final detection.

2. **Hard timeout guarantee (5s)**: If the Smart Turn detector doesn't fire within 5 seconds of the first `is_final` result, the system **automatically forces** a `speech_final` event. This prevents utterances from hanging indefinitely.

The hard timeout is measured from the **first** `is_final` result in a speech segment and is **not restarted** by subsequent `is_final` results (continuous speech). This ensures that even long utterances are bounded by the 5-second maximum wait time.

**Observability**: When the hard timeout fires, a `WARN`-level log is emitted:
```
Hard timeout fired after Xms - forcing speech_final (no turn detection confirmation received)
```
This allows SREs to monitor fallback frequency and tune VAD or Smart Turn thresholds.

##### `message`

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | `message`. |
| `message` | object | Unified payload described below. |

**Unified message payload**

| Field | Type | Description |
| --- | --- | --- |
| `message` | string? | Text content when the source was UTF-8. |
| `data` | string? | Base64 data for binary payloads. |
| `identity` | string | Origin participant identity. |
| `topic` | string | Topic or channel (`messages` by default). |
| `room` | string | LiveKit room identifier. |
| `timestamp` | integer | Milliseconds since Unix epoch. |

##### `participant_disconnected`

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | `participant_disconnected`. |
| `participant` | object | Contains `identity`, optional `name`, `room`, and `timestamp`. |

##### `tts_playback_complete`

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | `tts_playback_complete`. |
| `timestamp` | integer | When playback finished (milliseconds since epoch). |

##### `error`

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | `error`. |
| `message` | string | Human-readable explanation. |

##### Binary audio frames
Synthesized audio is streamed as binary frames using the format returned by the provider (`x-audio-format`, `x-sample-rate` in the REST API mirror these values).

#### LiveKit Integration Notes
- The server manages the agent-side LiveKit participant; clients only need a `/livekit/token` for their user identity.
- `listen_participants` filters prevent the VoiceManager from processing audio/data for unwanted identities.
- When `enable_recording=true`, the server starts/stops composite recording via the configured S3 target during connection lifecycle events.
- `clear` commands flush both WebSocket and LiveKit buffers to keep playback synchronized across transports.

## Operational Notes & Best Practices
- Reuse the same `tts_config` when possible; the VoiceManager hashes the configuration and caches rendered audio to eliminate provider round-trips.
- If you disable `audio` in the `config` message, you can still use LiveKit data relaying and the `/livekit/token` flow for text-only experiences.
- The `noise-filter` feature improves transcription quality but increases CPU usage; disable it for ultra-low-latency or resource-constrained deployments.
- The `stt-vad` feature enables integrated VAD and Smart Turn detection, which is the exclusive source of `is_speech_final` in STT responses.
- Use integration tests in `tests/` (for example `tests/ws_tests.rs`) as references when extending message formats or LiveKit behavior.
- Generate refreshed machine-readable docs with `cargo run --features openapi -- openapi -o docs/openapi.yaml` whenever request/response structures change.
