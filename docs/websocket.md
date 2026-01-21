# WebSocket API Reference

The Sayna WebSocket API is the foundation of all real-time voice processing features in the platform. Whether you're building a voice assistant, transcription service, or interactive voice application, the WebSocket endpoint at `/ws` provides low-latency, bidirectional communication for streaming audio, receiving transcriptions, and controlling text-to-speech synthesis.

This guide walks you through everything you need to integrate Sayna's WebSocket API into your application, from establishing connections to handling complex multi-party voice scenarios with LiveKit.

---

## Table of Contents

1. [Overview](#overview)
2. [Connection & Authentication](#connection--authentication)
3. [Session Lifecycle](#session-lifecycle)
4. [Message Protocol](#message-protocol)
5. [Configuration](#configuration)
6. [Use Case Patterns](#use-case-patterns)
7. [Audio Processing](#audio-processing)
8. [LiveKit Integration](#livekit-integration)
9. [Error Handling](#error-handling)
10. [Best Practices](#best-practices)
11. [Troubleshooting](#troubleshooting)

---

## Overview

### What is the WebSocket API?

Sayna's WebSocket API provides real-time voice processing capabilities through a persistent bidirectional connection. Unlike traditional REST APIs that require a new request for each operation, WebSockets maintain an open channel allowing continuous streaming of audio data and instant delivery of transcription results.

**Key Capabilities:**
- **Speech-to-Text (STT)**: Stream audio and receive real-time transcriptions with confidence scores
- **Text-to-Speech (TTS)**: Convert text to natural-sounding audio with voice customization
- **LiveKit Integration**: Connect voice processing to WebRTC rooms for multi-party voice applications
- **Low Latency**: Optimized for real-time voice interactions with minimal delay
- **Flexible Modes**: Operate with or without LiveKit, with or without audio processing

### When to Use WebSocket vs REST

**Use WebSocket When:**
- You need continuous audio streaming
- Real-time transcription is required
- Building conversational AI or voice assistants
- Integrating with LiveKit for multi-party voice
- Latency is critical (sub-second response times)

**Use REST When:**
- Processing single audio files
- Batch TTS synthesis
- Simple voice list queries
- Obtaining LiveKit tokens

---

## Connection & Authentication

### Establishing a Connection

Connect to the WebSocket endpoint using standard WebSocket protocols:

```
ws://your-server:3001/ws
```

Or for secure connections:

```
wss://your-server:3001/ws
```

**Connection Flow:**
1. Client initiates WebSocket handshake
2. Server accepts and upgrades HTTP connection
3. Server waits for configuration message
4. Upon valid configuration, server initializes voice providers
5. Server sends `ready` message when initialization completes
6. Bidirectional communication begins

### Authentication Model

By design, WebSocket endpoints are **unauthenticated** to simplify integration and reduce complexity. Audio data is ephemeral and not persisted by the server.

**Security Options:**
- **Network-Level Protection**: Deploy behind reverse proxy (nginx, Envoy) with authentication
- **Application Tokens**: Implement custom token validation in configuration messages
- **VPN/Private Network**: Restrict WebSocket access to trusted networks

**Note:** REST endpoints (`/voices`, `/speak`, `/livekit/token`) support authentication when `AUTH_REQUIRED=true` in server configuration. See authentication documentation for details.

### Connection Stability

The server actively monitors connection health:
- **Ping/Pong**: Automatic WebSocket keep-alive handled by the framework
- **Timeout Detection**: Stale connections detected and closed after 10 seconds of inactivity
- **Graceful Shutdown**: Server sends close frames and cleans up resources on disconnect

---

## Session Lifecycle

Every WebSocket session follows a predictable lifecycle. Understanding this flow is essential for building robust integrations.

### 1. Connection Phase

Client establishes WebSocket connection to `/ws` endpoint.

```
[Client] ---> WebSocket Handshake ---> [Server]
[Server] ---> 101 Switching Protocols ---> [Client]
```

### 2. Configuration Phase

**First message MUST be a configuration message.** This tells Sayna what voice providers to initialize and how to configure them.

```json
{
  "type": "config",
  "audio": true,
  "stt_config": {
    "provider": "deepgram",
    "language": "en-US",
    "sample_rate": 16000,
    "channels": 1,
    "punctuation": true,
    "encoding": "linear16",
    "model": "nova-2"
  },
  "tts_config": {
    "provider": "deepgram",
    "model": "aura-asteria-en",
    "voice_id": "aura-asteria-en",
    "speaking_rate": 1.0,
    "audio_format": "linear16",
    "sample_rate": 24000
  }
}
```

**What Happens During Configuration:**
1. Server validates configuration structure
2. Injects API keys from server environment (clients never send keys)
3. Initializes STT provider connection
4. Initializes TTS provider connection
5. Sets up audio caching system
6. If LiveKit configured, creates/joins room and generates tokens
7. Registers callbacks for transcription and synthesis events
8. Waits for all providers to reach ready state (30-second timeout)

**Configuration Failures:**
If configuration is invalid or providers fail to initialize, server sends an error message but **keeps the connection open**, allowing you to retry with corrected configuration.

```json
{
  "type": "error",
  "message": "STT and TTS configurations required when audio is enabled"
}
```

### 3. Ready Phase

When all systems are initialized, server sends a `ready` message signaling that you can begin streaming audio and issuing commands.

```json
{
  "type": "ready",
  "livekit_room_name": "conversation-room-123",
  "livekit_url": "wss://livekit.example.com",
  "sayna_participant_identity": "sayna-ai",
  "sayna_participant_name": "Sayna AI"
}
```

**LiveKit Token Generation:**
If using LiveKit, clients (other than Sayna) must call `POST /livekit/token` to obtain their own participant tokens:

```json
POST /livekit/token
{
  "room_name": "conversation-room-123",
  "participant_identity": "user-123",
  "participant_name": "John Doe"
}
```

Response:
```json
{
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "room_name": "conversation-room-123",
  "participant_identity": "user-123",
  "livekit_url": "wss://livekit.example.com"
}
```

### 4. Active Phase

This is where your application logic runs. You can:
- Stream binary audio frames for transcription
- Send `speak` commands for text-to-speech
- Send `clear` commands to interrupt TTS playback
- Send `send_message` commands for LiveKit data channels
- Receive STT transcription results
- Receive TTS audio chunks
- Receive LiveKit participant messages
- Receive LiveKit participant events

**Example Active Session:**
```
[Client] ---> Binary Audio (16-bit PCM) ---> [Server]
[Server] ---> { "type": "stt_result", "transcript": "hello", ... } ---> [Client]
[Client] ---> { "type": "speak", "text": "Hello! How are you?" } ---> [Server]
[Server] ---> Binary Audio (TTS synthesized) ---> [Client]
[Server] ---> { "type": "tts_playback_complete", ... } ---> [Client]
```

### 5. Cleanup Phase

When the WebSocket connection closes (client disconnect, network failure, or server shutdown), Sayna automatically:
1. Stops all STT/TTS provider connections
2. Disconnects from LiveKit room (if connected)
3. Stops any active recordings
4. Deletes the LiveKit room (to prevent orphaned rooms)
5. Clears all cached data for the session
6. Releases all resources

**Graceful Disconnect:**
Clients should close the WebSocket connection cleanly when done:

```javascript
// JavaScript example
websocket.close(1000, "Session complete");
```

**Server-Initiated Close:**
Server may close the connection if:
- LiveKit participant disconnect event occurs (100ms grace period for UI updates)
- Fatal errors during provider initialization
- Connection timeout (no activity for 10 seconds)

---

## Message Protocol

All messages are JSON (text) or binary (audio data). The server automatically distinguishes between message types.

### Incoming Messages (Client → Server)

These are messages your application sends to Sayna.

#### 1. Config Message

**Purpose:** Initialize voice providers and optionally configure LiveKit integration.

**Required:** Yes, must be the first message sent.

**Structure:**
```json
{
  "type": "config",
  "stream_id": "optional-session-identifier",
  "audio": true,
  "stt_config": { ... },
  "tts_config": { ... },
  "livekit": { ... }
}
```

**Fields:**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | string | Yes | - | Must be `"config"` |
| `stream_id` | string | No | Auto-generated UUID v4 | Unique session identifier. Used for recording paths and session tracking. |
| `audio` | boolean | No | `true` | Enable audio processing (STT/TTS). Set to `false` for LiveKit control-only mode. |
| `stt_config` | object | Conditional | - | Required when `audio=true`. Speech-to-text configuration. |
| `tts_config` | object | Conditional | - | Required when `audio=true`. Text-to-speech configuration. |
| `livekit` | object | No | - | Optional LiveKit room configuration. |

See [Configuration](#configuration) section for detailed field specifications.

---

#### 2. Speak Message

**Purpose:** Generate speech from text using configured TTS provider.

**Structure:**
```json
{
  "type": "speak",
  "text": "Hello! How can I help you today?",
  "flush": true,
  "allow_interruption": true
}
```

**Fields:**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `type` | string | Yes | - | Must be `"speak"` |
| `text` | string | Yes | - | Text to synthesize into speech |
| `flush` | boolean | No | `true` | If `true`, clears TTS queue before speaking. If `false`, appends to queue. |
| `allow_interruption` | boolean | No | `true` | If `true`, playback can be interrupted by `clear` commands or new audio. If `false`, playback completes regardless of interruption attempts. |

**Behavior:**
- Server validates that audio is enabled
- Text is sent to TTS provider for synthesis
- Synthesized audio is streamed back as binary messages
- If `flush=true`, any pending TTS audio is cleared first
- If `allow_interruption=false`, creates a non-interruptible playback window
- When complete, server sends `tts_playback_complete` message
- If LiveKit is configured, audio is also published to the LiveKit room

**Example Use Cases:**
```json
// Queue multiple phrases
{"type": "speak", "text": "First sentence.", "flush": false}
{"type": "speak", "text": "Second sentence.", "flush": false}

// Interrupt current speech with urgent message
{"type": "speak", "text": "URGENT: System alert!", "flush": true}

// Play important message that cannot be interrupted
{"type": "speak", "text": "Please listen carefully...", "allow_interruption": false}
```

---

#### 3. Binary Audio Message

**Purpose:** Stream audio data for speech-to-text transcription.

**Structure:** Raw binary WebSocket message (not JSON)

**Requirements:**
- Audio format must match `stt_config` parameters:
  - `sample_rate`: Audio sample rate in Hz (e.g., 16000)
  - `channels`: Number of channels (1 for mono, 2 for stereo)
  - `encoding`: Audio encoding format (e.g., "linear16" for 16-bit PCM)
- Audio is streamed continuously, not in complete files
- Server processes audio in real-time as it arrives

**Behavior:**
- Server receives binary frames (zero-copy optimization)
- Audio passed to STT provider for transcription
- Transcription results sent back as `stt_result` messages
- Optional noise filtering applied (if enabled)
- Optional turn detection determines when speaker has finished

**Best Practices:**
- Send audio in small chunks (e.g., 100-200ms worth of audio per message)
- Maintain consistent chunk sizes for optimal latency
- Ensure audio format exactly matches configuration (no automatic resampling)
- For 16kHz mono linear16: send ~3200 bytes per 100ms (16000 samples/sec × 2 bytes/sample × 0.1 sec)

---

#### 4. Clear Message

**Purpose:** Stop current TTS playback and clear all pending audio operations.

**Structure:**
```json
{
  "type": "clear"
}
```

**What Gets Cleared:**
1. **TTS Provider Queue** - All pending text waiting to be synthesized
2. **TTS Provider Audio Buffer** - Any audio chunks waiting to be sent
3. **LiveKit Audio Source Buffer** - NativeAudioSource internal buffer (if LiveKit connected)
4. **Audio Frame Queue** - Pending audio frames not yet sent to WebRTC
5. **Pending Audio Operations** - Queued operations in the operation worker

**Response Behavior:**
- The clear command is **fire-and-forget** - no response message is sent
- Errors during clearing are logged server-side but not returned to client
- Success is assumed unless an error message is received

**Timing Guarantees:**
- Cancellation is **immediate** for queued operations not yet in flight
- Audio operations queued after the clear command will proceed normally
- There may be a brief settling period (~10-50ms) where in-flight audio completes
- **WebRTC transport buffers cannot be cleared retroactively** - audio frames already captured to WebRTC may still be transmitted to remote participants

**Limitations:**
- Does NOT clear STT processing (only affects TTS output)
- Cannot stop audio that has already entered the WebRTC transport layer
- When using LiveKit, participants may hear a small amount of audio after clear due to network latency

**Non-Interruptible Audio Behavior:**
If `allow_interruption: false` was set on the most recent speak command, the clear command will be **silently ignored** until the non-interruptible playback completes. This protects critical audio (alerts, legal disclaimers) from being interrupted.

```javascript
// Non-interruptible audio protection example
{"type": "speak", "text": "Important legal notice...", "allow_interruption": false}
// At this point, clear commands are ignored until playback completes
{"type": "clear"}  // Ignored - no error, just silently skipped
```

**Use Cases:**
```javascript
// User interrupts AI while it's speaking
// Your app detects user started speaking, sends clear to stop AI
{"type": "clear"}

// User clicks "Stop" button in UI
{"type": "clear"}

// Urgent message needs to interrupt current speech
{"type": "clear"}
{"type": "speak", "text": "URGENT: System alert!", "flush": true}
```

**Implementation Notes:**
- If audio processing is disabled (`audio=false` in config), the clear command only clears LiveKit audio buffers (if configured)
- The clear operation acquires locks on audio resources, which may briefly delay concurrent audio operations

---

#### 5. Send Message

**Purpose:** Send custom messages through LiveKit data channel to other participants.

**Structure:**
```json
{
  "type": "send_message",
  "message": "Hello from the AI!",
  "role": "assistant",
  "topic": "chat",
  "debug": {
    "source": "sayna-ai",
    "version": "1.0"
  }
}
```

**Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | Yes | Must be `"send_message"` |
| `message` | string | Yes | Message content to send |
| `role` | string | Yes | Sender role (e.g., "assistant", "user", "system") |
| `topic` | string | No | Message topic/channel for routing (default: empty) |
| `debug` | object | No | Optional debug metadata (any valid JSON) |

**Requirements:**
- LiveKit must be configured in initial config
- Message sent asynchronously via operation queue
- Returns error if LiveKit not available

**Behavior:**
- Message queued for LiveKit data channel delivery
- Sent to all participants in the room (or filtered by `listen_participants`)
- Other participants receive message via their LiveKit data channel subscription
- Does NOT echo back to sender via WebSocket

**Use Cases:**
```json
// Send chat message
{"type": "send_message", "message": "How can I help?", "role": "assistant", "topic": "chat"}

// Send state update
{"type": "send_message", "message": "typing", "role": "assistant", "topic": "status"}

// Send structured data
{"type": "send_message", "message": "{\"action\":\"navigate\",\"url\":\"/home\"}", "role": "system", "topic": "commands"}
```

---

#### 6. SIP Transfer Message

**Purpose:** Transfer an active SIP call to another phone number using SIP REFER.

**Structure:**
```json
{
  "type": "sip_transfer",
  "transfer_to": "+1234567890"
}
```

**Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | Yes | Must be `"sip_transfer"` |
| `transfer_to` | string | Yes | Destination phone number to transfer the call to |

**Phone Number Formats:**
- International format with `+` prefix (e.g., `"+1234567890"`) - recommended
- National format without prefix (e.g., `"1234567890"`)
- Internal extensions (e.g., `"1234"`)

**Requirements:**
- LiveKit must be configured in initial config
- An active SIP participant must exist in the room
- The SIP handler must be configured on the server

**Behavior:**
1. Server validates the phone number format
2. Retrieves the room name from connection state
3. Fetches participants from the room via LiveKit API
4. Initiates a SIP REFER transfer using the first SIP participant found
5. On success, the call is transferred and the SIP participant leaves the room
6. On failure, a `sip_transfer_error` message is sent back to the client

**Error Handling:**
- Invalid phone number format → `sip_transfer_error` message
- No room configured → `sip_transfer_error` message
- No SIP participant in room → `sip_transfer_error` message
- SIP handler not configured → `sip_transfer_error` message
- Transfer API failure → `sip_transfer_error` message

**Use Cases:**
```json
// Transfer to international number
{"type": "sip_transfer", "transfer_to": "+1234567890"}

// Transfer to national number
{"type": "sip_transfer", "transfer_to": "5551234567"}

// Transfer to internal extension
{"type": "sip_transfer", "transfer_to": "1001"}
```

---

### Outgoing Messages (Server → Client)

These are messages Sayna sends to your application.

#### 1. Ready Message

**Purpose:** Notify client that all voice providers are initialized and session is ready for audio streaming.

**Structure:**
```json
{
  "type": "ready",
  "livekit_room_name": "conversation-room-123",
  "livekit_url": "wss://livekit.example.com",
  "sayna_participant_identity": "sayna-ai",
  "sayna_participant_name": "Sayna AI"
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | Always `"ready"` |
| `livekit_room_name` | string | LiveKit room name (only if LiveKit configured) |
| `livekit_url` | string | LiveKit server WebSocket URL (only if LiveKit configured) |
| `sayna_participant_identity` | string | Sayna AI's participant identity in LiveKit room |
| `sayna_participant_name` | string | Sayna AI's display name in LiveKit room |

**When Received:**
- After successful configuration
- All STT/TTS providers are connected and ready
- LiveKit room created and joined (if configured)
- Safe to begin sending audio frames and speak commands

**Next Steps:**
- If using LiveKit, other participants should call `POST /livekit/token` to get their tokens
- Begin streaming audio for transcription
- Send speak commands for TTS synthesis

---

#### 2. STT Result Message

**Purpose:** Deliver speech-to-text transcription results.

**Structure:**
```json
{
  "type": "stt_result",
  "transcript": "Hello, how are you today?",
  "is_final": true,
  "is_speech_final": true,
  "confidence": 0.94
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | Always `"stt_result"` |
| `transcript` | string | Transcribed text |
| `is_final` | boolean | `true` if this is the final version of the transcript (no more updates for this phrase) |
| `is_speech_final` | boolean | `true` if the speaker has stopped speaking (turn detection via VAD + Smart Turn) |
| `confidence` | number | Confidence score from 0.0 to 1.0 (higher is more confident) |

**Understanding Transcript Finality:**

**Interim Results** (`is_final=false`):
```json
{"transcript": "hel", "is_final": false, "is_speech_final": false, "confidence": 0.7}
{"transcript": "hello", "is_final": false, "is_speech_final": false, "confidence": 0.82}
{"transcript": "hello how", "is_final": false, "is_speech_final": false, "confidence": 0.85}
```

**Final Result** (`is_final=true`):
```json
{"transcript": "Hello, how are you?", "is_final": true, "is_speech_final": false, "confidence": 0.94}
```

**Speech Final** (`is_speech_final=true`):
```json
{"transcript": "Hello, how are you?", "is_final": true, "is_speech_final": true, "confidence": 0.94}
```

**How `is_speech_final` Works:**

The `is_speech_final` flag is determined exclusively by the VAD + Smart Turn detection system (when the `stt-vad` feature is enabled), not by STT providers. The detection works as follows:

1. **Silero-VAD** monitors incoming audio for speech activity
2. When silence is detected for the configured duration (default: 200ms), the **Smart Turn model** is triggered
3. The Smart Turn model analyzes the accumulated audio to determine if the speaker has completed their turn
4. If the model confirms turn completion (probability >= threshold), `is_speech_final=true` is emitted

This two-stage approach (VAD + Smart Turn) provides both speed (fast silence detection) and accuracy (semantic turn completion). The `is_speech_final` flag indicates the speaker has finished their thought, which is useful for knowing when to respond in conversational AI.

**Frequency:**
- Interim results arrive frequently during speech (every 100-500ms depending on provider)
- Final results arrive when provider finalizes transcription
- Speech final results arrive when VAD + Smart Turn detection determines the speaker's turn has ended

**Use Cases:**
```javascript
// Show live transcription in UI (interim results)
if (!result.is_final) {
  updateLiveTranscript(result.transcript);
}

// Save final transcription
if (result.is_final) {
  saveFinalTranscript(result.transcript);
}

// Trigger AI response when user finishes speaking
if (result.is_speech_final) {
  generateAIResponse(result.transcript);
}
```

---

#### 3. TTS Playback Complete Message

**Purpose:** Notify when text-to-speech synthesis and playback has finished.

**Structure:**
```json
{
  "type": "tts_playback_complete",
  "timestamp": 1700000000000
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | Always `"tts_playback_complete"` |
| `timestamp` | number | Unix timestamp in milliseconds when playback completed |

**When Received:**
- After all audio chunks for a `speak` command have been sent
- After LiveKit audio publication completes (if configured)
- Useful for latency measurement and state management

**Use Cases:**
```javascript
// Measure TTS latency
const speakTime = Date.now();
// ... receive tts_playback_complete ...
const latency = ttsComplete.timestamp - speakTime;

// Re-enable microphone after AI finishes speaking
if (message.type === "tts_playback_complete") {
  enableMicrophone();
}

// Show "AI is listening" indicator
if (message.type === "tts_playback_complete") {
  showListeningIndicator();
}
```

---

#### 4. Message (Unified Message)

**Purpose:** Deliver messages received from LiveKit participants via data channel.

**Structure:**
```json
{
  "type": "message",
  "message": {
    "message": "Hello from participant!",
    "data": null,
    "identity": "user-123",
    "topic": "chat",
    "room": "conversation-room-123",
    "timestamp": 1700000000000
  }
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | Always `"message"` |
| `message` | object | Unified message envelope (see below) |

**Unified Message Envelope:**

| Field | Type | Description |
|-------|------|-------------|
| `message` | string | Text message content (present if text message) |
| `data` | string | Base64-encoded binary data (present if binary message) |
| `identity` | string | Sender participant identity |
| `topic` | string | Message topic/channel |
| `room` | string | Room name where message originated |
| `timestamp` | number | Unix timestamp in milliseconds when received |

**Message Types:**
- **Text Messages**: `message` field populated, `data` is `null`
- **Binary Messages**: `data` field populated (base64), `message` is `null`

**Filtering:**
- If `listen_participants` configured in LiveKit config, only messages from specified participants are forwarded
- Server-generated messages (identity = server) always forwarded
- Empty `listen_participants` array means all messages forwarded

**Use Cases:**
```javascript
// Handle text chat
if (msg.message.message) {
  displayChatMessage(msg.message.identity, msg.message.message);
}

// Handle binary data
if (msg.message.data) {
  const binary = base64Decode(msg.message.data);
  processBinaryData(binary);
}

// Route by topic
if (msg.message.topic === "chat") {
  handleChatMessage(msg.message);
} else if (msg.message.topic === "commands") {
  handleCommand(msg.message);
}
```

---

#### 5. Participant Disconnected Message

**Purpose:** Notify when a LiveKit participant leaves the room.

**Structure:**
```json
{
  "type": "participant_disconnected",
  "participant": {
    "identity": "user-123",
    "name": "John Doe",
    "room": "conversation-room-123",
    "timestamp": 1700000000000
  }
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | Always `"participant_disconnected"` |
| `participant.identity` | string | Participant's unique identity |
| `participant.name` | string | Participant's display name (optional, may be null) |
| `participant.room` | string | Room name where disconnection occurred |
| `participant.timestamp` | number | Unix timestamp in milliseconds when disconnection occurred |

**Important Behavior:**
- Server waits 100ms after sending this message
- Server then automatically **closes the WebSocket connection**
- This ensures LiveKit session cleanup when participants leave
- Re-establish connection if you need to continue

**Use Cases:**
```javascript
// Update UI when participant leaves
if (message.type === "participant_disconnected") {
  const participant = message.participant;
  console.log(`${participant.identity} left room ${participant.room} at ${new Date(participant.timestamp)}`);
  removeParticipantFromUI(participant.identity);

  // Prepare for connection close
  showReconnectPrompt();
}
```

---

#### 6. Error Message

**Purpose:** Communicate errors without terminating the connection.

**Structure:**
```json
{
  "type": "error",
  "message": "STT and TTS configurations required when audio is enabled"
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | Always `"error"` |
| `message` | string | Human-readable error description |

**Common Errors:**
- Configuration validation failures
- Provider initialization errors
- Missing audio configuration when `audio=true`
- LiveKit not configured for commands requiring it
- Audio processing failures
- Provider API errors

**Connection Behavior:**
- Most errors do NOT close the connection
- Server keeps WebSocket open allowing retry with corrected data
- Fatal errors (rare) followed by connection close

**Use Cases:**
```javascript
// Handle errors gracefully
if (message.type === "error") {
  console.error("Sayna error:", message.message);

  // Retry configuration if it failed
  if (message.message.includes("configuration")) {
    retryConfigurationWithFix();
  }

  // Show error to user
  displayErrorNotification(message.message);
}
```

---

#### 7. SIP Transfer Error Message

**Purpose:** Communicate SIP transfer-specific errors, allowing clients to handle transfer failures separately from general errors.

**Structure:**
```json
{
  "type": "sip_transfer_error",
  "message": "No SIP participant found in room to transfer"
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `type` | string | Always `"sip_transfer_error"` |
| `message` | string | Human-readable error description |

**Common SIP Transfer Errors:**
- `"Invalid phone number: {details}"` - Phone number format validation failed
- `"No SIP participant found in room to transfer"` - No active SIP call in the room
- `"Room name not available. Ensure LiveKit is configured."` - LiveKit room not set up
- `"LiveKit client not configured..."` - SIP handler not available on server
- `"SIP transfer failed: {details}"` - Transfer API call failed

**Use Cases:**
```javascript
// Handle SIP transfer errors specifically
if (message.type === "sip_transfer_error") {
  console.error("SIP Transfer failed:", message.message);

  // Check for specific error conditions
  if (message.message.includes("Invalid phone number")) {
    showPhoneNumberFormatError();
  } else if (message.message.includes("No SIP participant")) {
    showNoActiveCallError();
  } else {
    showGenericTransferError(message.message);
  }
}
```

---

#### 8. Binary Audio Message

**Purpose:** Deliver synthesized TTS audio.

**Structure:** Raw binary WebSocket message (not JSON)

**Content:** Audio data in format specified by `tts_config`:
- Format: As specified in `audio_format` (e.g., "linear16" for 16-bit PCM)
- Sample rate: As specified in `sample_rate` (e.g., 24000 Hz)
- Channels: Typically mono (1 channel)

**Receiving Audio:**
- Multiple binary messages may be sent for a single `speak` command
- Audio streamed as it's generated (low latency)
- Final `tts_playback_complete` message sent after last chunk
- Audio can be played directly or buffered for smoothness

**Use Cases:**
```javascript
websocket.onmessage = (event) => {
  if (event.data instanceof Blob) {
    // Binary audio message
    playAudioChunk(event.data);
  } else {
    // JSON control message
    const message = JSON.parse(event.data);
    handleControlMessage(message);
  }
};
```

---

## Configuration

The configuration message is the most important message in the protocol. It determines what capabilities are available for the session.

### Audio Flag

```json
{
  "audio": true
}
```

| Value | Behavior |
|-------|----------|
| `true` (default) | Full audio processing enabled. STT and TTS providers initialized. Can stream audio, receive transcriptions, and synthesize speech. |
| `false` | Audio processing disabled. No STT/TTS providers started. Useful for LiveKit control-only mode where you only need data channels and room management. |

**Audio Disabled Mode:**
When `audio=false`, you can still:
- Connect to LiveKit rooms
- Send/receive data channel messages
- Manage LiveKit recordings
- Control room state

You CANNOT:
- Stream audio for transcription
- Use `speak` commands
- Receive TTS audio
- Use `clear` commands

Attempting to send audio or speak commands with `audio=false` results in an error message (connection remains open).

---

### STT Configuration

Required when `audio=true`.

```json
{
  "stt_config": {
    "provider": "deepgram",
    "language": "en-US",
    "sample_rate": 16000,
    "channels": 1,
    "punctuation": true,
    "encoding": "linear16",
    "model": "nova-2"
  }
}
```

**Fields:**

| Field | Type | Required | Description | Example |
|-------|------|----------|-------------|---------|
| `provider` | string | Yes | STT provider name. Supported: `"deepgram"`, `"google"`, `"elevenlabs"`, `"microsoft-azure"`, `"cartesia"` | `"deepgram"` |
| `api_key` | string | No | Provider credentials (see below for Google) | `""` |
| `language` | string | Yes | BCP-47 language code for transcription | `"en-US"`, `"es-ES"`, `"fr-FR"` |
| `sample_rate` | number | Yes | Audio sample rate in Hz. Must match binary audio you send. | `16000`, `24000`, `48000` |
| `channels` | number | Yes | Number of audio channels. `1` = mono, `2` = stereo. | `1` |
| `punctuation` | boolean | Yes | Enable automatic punctuation in transcripts | `true`, `false` |
| `encoding` | string | Yes | Audio encoding format. Must match binary audio you send. | `"linear16"`, `"opus"` |
| `model` | string | Yes | Provider-specific model identifier | `"nova-2"` (Deepgram), `"latest_long"` (Google) |

**Provider-specific notes:**

- **Deepgram**: API key injected from server environment (`DEEPGRAM_API_KEY`). Clients should not include `api_key` field.
- **Google**: Supports three credential modes via `api_key` field:
  - Empty string (`""`): Uses Application Default Credentials
  - File path: Path to service account JSON file
  - JSON content: Raw JSON string (starts with `{`)
- **ElevenLabs**: API key injected from server environment (`ELEVENLABS_API_KEY`). Clients should not include `api_key` field.
- **Microsoft Azure**: Subscription key injected from server environment (`AZURE_SPEECH_SUBSCRIPTION_KEY`). Region configured via `AZURE_SPEECH_REGION`. See [Azure STT documentation](azure-stt.md) for details.
- **Cartesia**: API key injected from server environment (`CARTESIA_API_KEY`). Uses raw binary PCM audio (not base64). Model defaults to `"ink-whisper"`. See [Cartesia STT documentation](cartesia-stt.md) for details.

**Critical:** The `sample_rate`, `channels`, and `encoding` must **exactly match** the binary audio frames you send. Sayna does not perform audio format conversion. Mismatches result in garbled transcriptions or errors.

**Common Configurations:**

**Deepgram - High-quality phone calls (16kHz mono):**
```json
{
  "provider": "deepgram",
  "language": "en-US",
  "sample_rate": 16000,
  "channels": 1,
  "punctuation": true,
  "encoding": "linear16",
  "model": "nova-2"
}
```

**Deepgram - High-quality web audio (48kHz stereo):**
```json
{
  "provider": "deepgram",
  "language": "en-US",
  "sample_rate": 48000,
  "channels": 2,
  "punctuation": true,
  "encoding": "linear16",
  "model": "nova-2"
}
```

**Google Cloud STT - Standard configuration:**
```json
{
  "provider": "google",
  "api_key": "",
  "language": "en-US",
  "sample_rate": 16000,
  "channels": 1,
  "punctuation": true,
  "encoding": "linear16",
  "model": "latest_long"
}
```

**Google Cloud STT - Telephony optimized:**
```json
{
  "provider": "google",
  "api_key": "",
  "language": "en-US",
  "sample_rate": 8000,
  "channels": 1,
  "punctuation": true,
  "encoding": "mulaw",
  "model": "telephony"
}
```

**Microsoft Azure STT - Standard configuration:**
```json
{
  "provider": "microsoft-azure",
  "api_key": "",
  "language": "en-US",
  "sample_rate": 16000,
  "channels": 1,
  "punctuation": true,
  "encoding": "linear16",
  "model": ""
}
```

**Cartesia STT - Standard configuration:**
```json
{
  "provider": "cartesia",
  "api_key": "",
  "language": "en",
  "sample_rate": 16000,
  "channels": 1,
  "punctuation": true,
  "encoding": "pcm_s16le",
  "model": "ink-whisper"
}
```

See [Google STT documentation](google-stt.md), [Azure STT documentation](azure-stt.md), and [Cartesia STT documentation](cartesia-stt.md) for detailed configuration options and model selection.

---

### TTS Configuration

Required when `audio=true`.

```json
{
  "tts_config": {
    "provider": "deepgram",
    "model": "aura-asteria-en",
    "voice_id": "aura-asteria-en",
    "speaking_rate": 1.0,
    "audio_format": "linear16",
    "sample_rate": 24000,
    "connection_timeout": 30,
    "request_timeout": 60,
    "pronunciations": []
  }
}
```

**Fields:**

| Field | Type | Required | Description | Example |
|-------|------|----------|-------------|---------|
| `provider` | string | Yes | TTS provider name. Currently supported: `"deepgram"`, `"elevenlabs"`, `"google"`, `"azure"` | `"deepgram"` |
| `model` | string | Yes | Provider-specific model identifier | `"aura-asteria-en"` (Deepgram), `"eleven_multilingual_v2"` (ElevenLabs) |
| `voice_id` | string | No | Voice identifier for synthesis. Provider-specific. | `"aura-asteria-en"`, `"21m00Tcm4TlvDq8ikWAM"` |
| `speaking_rate` | number | No | Speech speed multiplier. Range: 0.25 to 4.0. Default: 1.0 | `0.8` (slower), `1.2` (faster) |
| `audio_format` | string | No | Audio encoding format for output | `"linear16"`, `"mp3"`, `"opus"` |
| `sample_rate` | number | No | Output audio sample rate in Hz | `16000`, `24000`, `48000` |
| `connection_timeout` | number | No | Provider connection timeout in seconds | `30` |
| `request_timeout` | number | No | TTS synthesis request timeout in seconds | `60` |
| `pronunciations` | array | No | Custom pronunciation replacements (see below) | `[{"word": "API", "pronunciation": "A P I"}]` |

**Pronunciations:**

Customize how specific words are pronounced by providing replacement rules:

```json
{
  "pronunciations": [
    {
      "word": "API",
      "pronunciation": "A P I"
    },
    {
      "word": "SQL",
      "pronunciation": "sequel"
    },
    {
      "word": "nginx",
      "pronunciation": "engine X"
    }
  ]
}
```

Before synthesis, Sayna replaces occurrences of `word` with `pronunciation` in the text.

**Audio Caching:**

Sayna automatically caches TTS audio based on a hash of:
- Provider
- Voice ID
- Model
- Audio format
- Sample rate
- Speaking rate

When the same text is synthesized with identical configuration, cached audio is returned instantly (sub-100ms latency). Keep your TTS configuration stable across sessions to maximize cache hits.

**Provider-Specific Settings:**

**Deepgram:**
```json
{
  "provider": "deepgram",
  "model": "aura-asteria-en",
  "voice_id": "aura-asteria-en",
  "speaking_rate": 1.0,
  "audio_format": "linear16",
  "sample_rate": 24000
}
```

**ElevenLabs:**
```json
{
  "provider": "elevenlabs",
  "model": "eleven_multilingual_v2",
  "voice_id": "21m00Tcm4TlvDq8ikWAM",
  "speaking_rate": 1.0,
  "audio_format": "mp3_44100_128",
  "sample_rate": 44100
}
```

**Google Cloud TTS:**
```json
{
  "provider": "google",
  "voice_id": "en-US-Wavenet-D",
  "speaking_rate": 1.0,
  "audio_format": "linear16",
  "sample_rate": 24000
}
```

Voice names follow the pattern `{language}-{region}-{type}-{variant}`. Types include Standard, WaveNet, Neural2, and Studio (ordered by quality). See [Google TTS documentation](google-tts.md) for detailed voice selection and configuration options.

**Microsoft Azure TTS:**
```json
{
  "provider": "azure",
  "voice_id": "en-US-JennyNeural",
  "speaking_rate": 1.0,
  "audio_format": "linear16",
  "sample_rate": 24000
}
```

Voice names follow the pattern `{language}-{region}-{name}Neural` (e.g., `en-US-JennyNeural`, `de-DE-ConradNeural`). See [Azure TTS documentation](azure-tts.md) for detailed voice selection and configuration options.

---

### LiveKit Configuration

Optional. Include to enable LiveKit integration for multi-party voice rooms.

```json
{
  "stream_id": "optional-uuid-or-auto-generated",
  "livekit": {
    "room_name": "conversation-room-123",
    "enable_recording": false,
    "sayna_participant_identity": "sayna-ai",
    "sayna_participant_name": "Sayna AI",
    "listen_participants": []
  }
}
```

**Fields:**

| Field | Type | Required | Description | Example |
|-------|------|----------|-------------|---------|
| `room_name` | string | Yes | Unique room identifier. Room created if doesn't exist. | `"conversation-room-123"` |
| `enable_recording` | boolean | No | Start LiveKit cloud recording. Default: `false` | `true`, `false` |
| `sayna_participant_identity` | string | No | Sayna AI's participant identity. Default: `"sayna-ai"` | `"assistant-bot"` |
| `sayna_participant_name` | string | No | Sayna AI's display name. Default: `"Sayna AI"` | `"Support Assistant"` |
| `listen_participants` | array | No | Participant identity filter. Default: `[]` (all participants) | `["user-123", "user-456"]` |

**Recording path**: When `enable_recording` is true, recordings are saved to `{server_s3_prefix}/{stream_id}/audio.ogg`. `stream_id` is set at the top level of the WebSocket config message; if omitted, the server auto-generates a UUID.

**Room Management:**

When LiveKit is configured:
1. Server creates the room (or joins if it exists)
2. Server generates a JWT token for Sayna AI participant
3. Server connects to the room as `sayna_participant_identity`
4. Server publishes audio tracks for TTS output
5. Server subscribes to audio tracks from participants (filtered by `listen_participants`)
6. Server subscribes to data channel messages (filtered by `listen_participants`)
7. On disconnect, server automatically deletes the room

**Participant Filtering (`listen_participants`):**

Control whose audio and messages Sayna processes:

| Value | Behavior |
|-------|----------|
| `[]` (empty, default) | Process audio and messages from **all** participants |
| `["user-123"]` | Only process audio and messages from `user-123`, ignore others |
| `["user-123", "user-456"]` | Only process audio and messages from `user-123` and `user-456` |

**Use Cases:**
- **All participants** (default): Voice assistant in meeting hears everyone
- **Specific participants**: AI assistant only listens to moderator or host
- **1-on-1 conversations**: AI only processes audio from specific user

**Note:** Server-generated messages (participant identity = server) are always processed regardless of filter.

**Recording:**

Enable cloud recording to S3-compatible storage:

```json
{
  "stream_id": "support-call-789",
  "livekit": {
    "room_name": "support-call-789",
    "enable_recording": true
  }
}
```

**Recording behavior:**
- Recording starts when WebSocket config is received
- Recording includes all room participants (not just filtered ones)
- Recording format: audio-only OGG stored as `audio.ogg`
- Recording automatically stops when WebSocket disconnects
- Requires S3 configuration in server environment
- Recording path: `{server_s3_prefix}/{stream_id}/audio.ogg`

**Getting Participant Tokens:**

After receiving the `ready` message with LiveKit details, other participants must obtain tokens:

```bash
POST /livekit/token
Content-Type: application/json

{
  "room_name": "conversation-room-123",
  "participant_identity": "user-123",
  "participant_name": "John Doe"
}
```

Response:
```json
{
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "room_name": "conversation-room-123",
  "participant_identity": "user-123",
  "livekit_url": "wss://livekit.example.com"
}
```

Use the token to connect to LiveKit room from web browsers or mobile apps using LiveKit SDKs.

---

## Use Case Patterns

Sayna's WebSocket API supports multiple integration patterns. Choose based on your application needs.

### Pattern 1: WebSocket-Only STT/TTS

**Use Case:** Voice assistant within your own application. You handle audio capture/playback.

**Configuration:**
```json
{
  "type": "config",
  "audio": true,
  "stt_config": {
    "provider": "deepgram",
    "language": "en-US",
    "sample_rate": 16000,
    "channels": 1,
    "punctuation": true,
    "encoding": "linear16",
    "model": "nova-2"
  },
  "tts_config": {
    "provider": "deepgram",
    "model": "aura-asteria-en",
    "voice_id": "aura-asteria-en",
    "speaking_rate": 1.0,
    "audio_format": "linear16",
    "sample_rate": 24000
  }
}
```

**Flow:**
```
1. Connect to /ws
2. Send config (no LiveKit)
3. Receive "ready" message
4. Start streaming microphone audio → Sayna
5. Receive STT results
6. When user finishes speaking (is_speech_final from VAD + Smart Turn), generate response
7. Send "speak" command with response text
8. Receive TTS audio chunks
9. Play audio to speaker
10. Receive "tts_playback_complete"
11. Re-enable microphone, repeat
```

**Pros:**
- Simple integration
- Full control over audio capture/playback
- Low latency
- Works in any environment (desktop, mobile, embedded)

**Cons:**
- Must handle audio I/O yourself
- No multi-party voice
- No WebRTC benefits

---

### Pattern 2: WebSocket + LiveKit Hybrid

**Use Case:** AI assistant in a multi-party meeting. WebSocket controls AI behavior, LiveKit provides WebRTC audio distribution to browsers.

**Configuration:**
```json
{
  "type": "config",
  "stream_id": "meeting-456",
  "audio": true,
  "stt_config": { ... },
  "tts_config": { ... },
  "livekit": {
    "room_name": "meeting-room-456",
    "enable_recording": true,
    "sayna_participant_identity": "meeting-assistant",
    "sayna_participant_name": "Meeting Assistant",
    "listen_participants": []
  }
}
```

**Flow:**
```
1. Connect to /ws
2. Send config with LiveKit
3. Receive "ready" with room details
4. Browser participants call POST /livekit/token
5. Browser participants join LiveKit room with tokens
6. Participants speak → LiveKit → Sayna STT → WebSocket STT results
7. Send "speak" command for AI response
8. Sayna TTS → LiveKit room (all participants hear) + WebSocket (for logging)
9. Receive "tts_playback_complete"
10. Repeat
```

**Pros:**
- Best of both worlds: WebSocket control + WebRTC distribution
- Participants use standard browsers (no custom audio code)
- Multi-party voice support
- Cloud recording
- LiveKit handles NAT traversal, quality adaptation, etc.

**Cons:**
- More complex architecture
- Requires LiveKit server
- Additional token management

**Perfect for:**
- Voice bots in meetings
- Customer support AI (agent + customer)
- Virtual receptionists
- Collaborative voice applications

---

### Pattern 3: LiveKit Control-Only Mode

**Use Case:** Use Sayna as LiveKit room manager without audio processing. Useful for room automation, recording control, or data channel messaging.

**Configuration:**
```json
{
  "type": "config",
  "stream_id": "automation-session",
  "audio": false,
  "livekit": {
    "room_name": "automation-room",
    "enable_recording": true
  }
}
```

**Flow:**
```
1. Connect to /ws
2. Send config with audio=false, LiveKit enabled
3. Receive "ready"
4. Room created, recording started
5. Use "send_message" to send data to participants
6. Receive "message" events from participants
7. Control recording, room state via commands
8. Disconnect → room deleted, recording stopped
```

**Pros:**
- No STT/TTS overhead
- Lightweight
- Focus on messaging and control

**Cons:**
- No voice processing
- Must handle audio elsewhere if needed

**Perfect for:**
- Room lifecycle automation
- Recording management
- LiveKit integration testing
- Data-only applications

---

## Audio Processing

Understanding how audio flows through Sayna helps you optimize for latency and quality.

### Audio Format Requirements

**Critical Rule:** Binary audio you send MUST match `stt_config` parameters exactly.

**Example:**
If your `stt_config` specifies:
```json
{
  "sample_rate": 16000,
  "channels": 1,
  "encoding": "linear16"
}
```

Your binary audio must be:
- 16000 samples per second
- Mono (1 channel)
- 16-bit signed PCM (linear16)
- Little-endian byte order

**Calculating Chunk Sizes:**

For optimal streaming, send audio in small chunks (100-200ms worth of audio):

```
Bytes per chunk = sample_rate × channels × bytes_per_sample × duration_seconds

Example (16kHz mono linear16, 100ms chunks):
= 16000 samples/sec × 1 channel × 2 bytes/sample × 0.1 sec
= 3200 bytes per chunk
```

**Common Formats:**

| Format | Sample Rate | Channels | Encoding | Bytes/Chunk (100ms) |
|--------|-------------|----------|----------|---------------------|
| Phone quality | 16000 | 1 | linear16 | 3200 |
| CD quality | 44100 | 2 | linear16 | 17640 |
| High-quality | 48000 | 2 | linear16 | 19200 |

### STT Audio Flow

**Without LiveKit:**
```
Your App → WebSocket Binary → Sayna
                               ↓
                         (Optional) DeepFilterNet noise reduction
                               ↓
                         STT Provider (e.g., Deepgram)
                               ↓
                         STT Results → WebSocket → Your App
```

**With LiveKit:**
```
Browser Participant → LiveKit Room → Sayna
                                      ↓
                                (Optional) DeepFilterNet
                                      ↓
                                STT Provider
                                      ↓
                                STT Results → WebSocket → Your App
                                      ↓
                                (Optional) Send to LiveKit data channel
```

**Noise Filtering:**

If compiled with `noise-filter` feature (disabled by default), Sayna applies DeepFilterNet noise reduction:
- Reduces background noise, keyboard typing, fan noise
- Preserves speech quality
- CPU-intensive, runs on thread pool (non-blocking)
- Conservative blending to avoid over-processing
- Automatically applied to all audio (WebSocket + LiveKit)

**VAD + Turn Detection:**

If compiled with `stt-vad` feature (disabled by default), Sayna uses Silero-VAD for voice activity detection combined with an ONNX Smart Turn detection model:
- Silero-VAD monitors audio for continuous silence
- When silence exceeds threshold (default 200ms), the Smart Turn detection model is triggered
- Smart Turn model analyzes the accumulated audio to confirm if the speaker has completed their turn
- Sets `is_speech_final=true` when the speaker's turn is complete
- This is the **exclusive source** of `is_speech_final` - STT providers do not set this flag
- More accurate than silence-only or text-only detection due to semantic understanding
- Helps conversational AI know when to respond
- VAD and turn detection are always bundled together under the `stt-vad` feature

### TTS Audio Flow

**Without LiveKit:**
```
Your App → "speak" command → Sayna
                              ↓
                         TTS Provider
                              ↓
                         (Check cache)
                              ↓
                    Binary Audio → WebSocket → Your App
                              ↓
                    "tts_playback_complete" → Your App
```

**With LiveKit (Dual Routing):**
```
Your App → "speak" command → Sayna
                              ↓
                         TTS Provider
                              ↓
                         (Check cache)
                              ↓
                    ┌─────────┴──────────┐
                    ↓                    ↓
           Binary → LiveKit      Binary → WebSocket
           (primary, all participants hear)  (fallback, logging)
                    ↓
           "tts_playback_complete" → Your App
```

**Audio Caching:**

Sayna caches TTS audio for faster repeated synthesis:
- Cache key: hash of (provider, voice, model, format, rate, text)
- Cache hit: audio returned in <100ms
- Cache miss: audio generated by provider (~500-2000ms)
- Cache shared across all sessions with same TTS config
- Maximize cache hits by keeping TTS config stable

**Example Cache Benefit:**
```
First synthesis: "Hello!" → 1200ms (provider synthesis time)
Second synthesis: "Hello!" → 80ms (cache hit)
Third synthesis: "Hello!" → 75ms (cache hit)
```

### Interruption Handling

Sayna provides intelligent audio interruption:

**Interruptible Playback (default):**
```json
{"type": "speak", "text": "This can be interrupted", "allow_interruption": true}
```

While playing:
- `clear` command stops playback immediately
- New `speak` command with `flush=true` stops playback
- User starts speaking → your app can send `clear` to stop AI

**Non-Interruptible Playback:**
```json
{"type": "speak", "text": "Important message!", "allow_interruption": false}
```

While playing:
- `clear` command ignored (returns silently)
- New `speak` commands queued (do not pre-empt)
- Playback completes fully, then queue processes

**Use Cases:**
- Interruptible: conversational responses, long descriptions
- Non-interruptible: critical alerts, legal disclaimers, error messages

---

## LiveKit Integration

LiveKit integration transforms Sayna into a multi-party voice platform.

### Architecture

```
┌─────────────────────────────────────────────────┐
│                  LiveKit Room                   │
│                                                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐     │
│  │ Browser  │  │ Browser  │  │  Sayna   │     │
│  │  User A  │  │  User B  │  │    AI    │     │
│  └──────────┘  └──────────┘  └──────────┘     │
│       ↑              ↑              ↑          │
│       └──────────────┴──────────────┘          │
│          WebRTC Audio + Data Channels          │
└─────────────────────────────────────────────────┘
                       ↕
              WebSocket Control Plane
                       ↕
              ┌─────────────────┐
              │   Your Backend  │
              │  (Orchestrator) │
              └─────────────────┘
```

### Dual-Transport Model

Sayna uses LiveKit for audio distribution, WebSocket for control:

**LiveKit (WebRTC):**
- Participant audio tracks (STT input)
- Sayna TTS audio tracks (output to participants)
- Data channel messages (participant chat, events)
- Handled by LiveKit SDK (NAT traversal, quality adaptation)

**WebSocket:**
- Control messages (speak, clear, send_message)
- STT transcription results
- TTS completion notifications
- Error messages
- Participant events

**Why Both?**
- LiveKit excels at audio distribution (WebRTC, TURN servers, quality adaptation)
- WebSocket excels at control messages and structured data
- Separating concerns allows optimizations for each

### Room Lifecycle

**Creation:**
```
1. WebSocket sends config with livekit block
2. Sayna creates room (or joins existing)
3. Sayna generates JWT token for itself
4. Sayna joins room as sayna_participant_identity
5. Sayna publishes audio track (for TTS output)
6. Sayna subscribes to participant tracks (for STT input)
7. Sayna subscribes to data channel
8. Sayna sends "ready" with room details
```

**Active:**
```
- Participants join with their own tokens (from POST /livekit/token)
- Participants speak → Sayna STT → WebSocket
- WebSocket sends "speak" → Sayna TTS → LiveKit (all hear)
- Participants send data → Sayna → WebSocket "message" event
- WebSocket sends "send_message" → Sayna → LiveKit data channel
```

**Cleanup:**
```
1. WebSocket disconnects (client or server initiated)
2. Sayna stops STT/TTS providers
3. Sayna disconnects from LiveKit room (100ms timeout)
4. Sayna stops recording (if enabled)
5. Sayna deletes LiveKit room
```

**Room Deletion:**
Sayna automatically deletes rooms on disconnect to prevent orphaned rooms consuming resources. If you need persistent rooms, manage room lifecycle separately using LiveKit APIs.

### Participant Management

**Participant Tokens:**

Each participant needs their own JWT token:

```bash
# After receiving ready message with room_name
POST /livekit/token
{
  "room_name": "conversation-room-123",
  "participant_identity": "user-456",
  "participant_name": "Alice Smith"
}
```

Response includes token for LiveKit SDK:
```json
{
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "room_name": "conversation-room-123",
  "participant_identity": "user-456",
  "livekit_url": "wss://livekit.example.com"
}
```

**Participant Filtering:**

Control which participants Sayna processes:

```json
{
  "livekit": {
    "room_name": "moderated-meeting",
    "listen_participants": ["moderator-1", "moderator-2"]
  }
}
```

**Effect:**
- Sayna only transcribes audio from `moderator-1` and `moderator-2`
- Other participants' audio ignored (not sent to STT)
- Only data messages from filtered participants forwarded
- Sayna still publishes TTS audio to all participants

**Use Cases:**
- **Support Bot**: Only listen to customer (not support agents)
- **Meeting Assistant**: Only transcribe host/moderator
- **Voice Commands**: Only process specific user's voice

**Disconnect Handling:**

When a participant leaves:
```json
{
  "type": "participant_disconnected",
  "participant": {
    "identity": "user-456",
    "name": "Alice Smith"
  }
}
```

**Important:** 100ms after this message, Sayna closes the WebSocket. This ensures clean LiveKit session teardown. Re-establish connection if needed.

### Data Channel Messaging

LiveKit provides a data channel for custom messages between participants.

**Sending Messages:**
```json
{
  "type": "send_message",
  "message": "Hello participants!",
  "role": "assistant",
  "topic": "chat"
}
```

Sayna publishes to LiveKit data channel → all participants receive (via their LiveKit SDK subscriptions).

**Receiving Messages:**
```json
{
  "type": "message",
  "message": {
    "message": "Hello AI!",
    "identity": "user-456",
    "topic": "chat",
    "room": "conversation-room-123",
    "timestamp": 1700000000000
  }
}
```

**Message Routing by Topic:**

Use `topic` field for application-level routing:

```javascript
// Participant sends with topic
livekitRoom.localParticipant.publishData(
  JSON.stringify({message: "typing...", topic: "status"}),
  DataPacket_Kind.RELIABLE
);

// Your backend receives
{
  "type": "message",
  "message": {
    "message": "typing...",
    "topic": "status",
    "identity": "user-456",
    ...
  }
}

// Route based on topic
if (message.message.topic === "status") {
  updateParticipantStatus(message.message.identity, message.message.message);
} else if (message.message.topic === "chat") {
  handleChatMessage(message.message);
}
```

**Binary Data:**

Data channel supports binary data (base64-encoded in WebSocket message):

```json
{
  "type": "message",
  "message": {
    "data": "SGVsbG8gd29ybGQh",
    "identity": "user-456",
    "topic": "file-transfer",
    ...
  }
}
```

Decode base64 to get original binary data.

### Recording

Enable cloud recording to S3-compatible storage:

```json
{
  "stream_id": "support-call-123",
  "livekit": {
    "room_name": "support-call-123",
    "enable_recording": true
  }
}
```

**Behavior:**
- Recording starts when config received
- Records all room audio/video (not just Sayna)
- Format: audio-only OGG saved as `audio.ogg`
- Storage: S3-compatible (configured in server environment)
- Stops automatically on WebSocket disconnect
- Recording path: `{server_s3_prefix}/{stream_id}/audio.ogg`

**Requirements:**
- LiveKit egress service configured
- S3 credentials in server environment

**Access Recording:**
After session ends, download from `{server_s3_prefix}/{stream_id}/audio.ogg`.

---

## Error Handling

Sayna communicates errors without abruptly closing connections, allowing recovery.

### Error Message Format

```json
{
  "type": "error",
  "message": "Human-readable error description"
}
```

### Common Errors

**Configuration Errors:**
```json
{"type": "error", "message": "STT and TTS configurations required when audio is enabled"}
```

**Solution:** Send config with both `stt_config` and `tts_config` when `audio=true`.

---

**Provider Initialization Errors:**
```json
{"type": "error", "message": "Failed to initialize STT provider: connection timeout"}
```

**Solution:** Check provider API credentials in server environment. Retry configuration.

---

**Audio Processing Errors:**
```json
{"type": "error", "message": "Audio processing failed: invalid audio format"}
```

**Solution:** Ensure binary audio matches `stt_config` (sample rate, channels, encoding).

---

**LiveKit Errors:**
```json
{"type": "error", "message": "LiveKit not configured"}
```

**Solution:** Include `livekit` block in config message if using LiveKit features.

---

**TTS Synthesis Errors:**
```json
{"type": "error", "message": "TTS synthesis failed: provider returned error"}
```

**Solution:** Check TTS config parameters (voice_id, model). Provider may have rate limits.

### Error Recovery Strategies

**Non-Fatal Errors:**
Most errors are non-fatal. Connection remains open for retry.

```javascript
websocket.onmessage = (event) => {
  const message = JSON.parse(event.data);

  if (message.type === "error") {
    console.error("Sayna error:", message.message);

    // Retry configuration if validation failed
    if (message.message.includes("configuration")) {
      setTimeout(() => sendCorrectedConfig(), 1000);
    }

    // Retry speak if TTS failed
    if (message.message.includes("TTS synthesis failed")) {
      setTimeout(() => retrySpeakCommand(), 2000);
    }
  }
};
```

**Fatal Errors:**
Rare. Connection closes after error message.

**Monitoring:**
Log all error messages for debugging:

```javascript
const errorLog = [];

if (message.type === "error") {
  errorLog.push({
    timestamp: Date.now(),
    message: message.message,
    sessionId: currentSessionId
  });

  // Send to monitoring service
  sendToMonitoring(errorLog);
}
```

---

## Best Practices

### 1. Audio Format Consistency

**Always match `stt_config` exactly:**
```javascript
// Bad: sending 48kHz audio when config says 16kHz
const config = {
  stt_config: {
    sample_rate: 16000,
    channels: 1,
    encoding: "linear16"
  }
};

// Microphone captures at 48kHz...
// Results in garbled transcripts

// Good: resample audio to match config
const config = {
  stt_config: {
    sample_rate: 16000,
    channels: 1,
    encoding: "linear16"
  }
};

// Resample microphone from 48kHz to 16kHz before sending
const resampled = resampleAudio(micAudio, 48000, 16000);
websocket.send(resampled);
```

### 2. Chunk Size Optimization

**Send 100-200ms chunks for best latency:**
```javascript
// Bad: sending 5-second chunks (high latency)
setInterval(() => {
  websocket.send(fiveSecondBuffer);
}, 5000);

// Good: sending 100ms chunks (low latency)
setInterval(() => {
  websocket.send(hundredMsBuffer);
}, 100);
```

### 3. Handle Interim Transcripts

**Use interim results for live UI updates:**
```javascript
let liveTranscript = "";
let finalTranscripts = [];

if (message.type === "stt_result") {
  if (!message.is_final) {
    // Update live display
    liveTranscript = message.transcript;
    updateLiveDisplay(liveTranscript);
  } else {
    // Save final
    finalTranscripts.push(message.transcript);
    liveTranscript = "";

    // Respond when speech ends (detected by VAD + Smart Turn)
    if (message.is_speech_final) {
      generateResponse(finalTranscripts.join(" "));
    }
  }
}
```

### 4. Measure Latency

**Track TTS latency:**
```javascript
const speakTimestamps = new Map();

// When sending speak command
const speakId = generateId();
speakTimestamps.set(speakId, Date.now());
websocket.send(JSON.stringify({
  type: "speak",
  text: "Hello!",
  // Add custom ID in your app state
}));

// When receiving playback complete
if (message.type === "tts_playback_complete") {
  const latency = message.timestamp - speakTimestamps.get(currentSpeakId);
  console.log(`TTS latency: ${latency}ms`);

  // Monitor for performance issues
  if (latency > 2000) {
    console.warn("High TTS latency detected");
  }
}
```

### 5. Stable TTS Configuration

**Maximize cache hits:**
```javascript
// Bad: changing voice parameters frequently
{"voice_id": "voice-1", "speaking_rate": 1.0}  // Cache miss
{"voice_id": "voice-1", "speaking_rate": 1.1}  // Cache miss (different rate)
{"voice_id": "voice-2", "speaking_rate": 1.0}  // Cache miss (different voice)

// Good: consistent configuration
const ttsConfig = {
  voice_id: "voice-1",
  speaking_rate: 1.0,
  // Keep stable across session
};

// All speak commands with same config → cache hits
```

### 6. Graceful Reconnection

**Handle disconnects:**
```javascript
let reconnectAttempts = 0;
const MAX_RECONNECT_ATTEMPTS = 5;

websocket.onclose = (event) => {
  console.log("WebSocket closed:", event.code, event.reason);

  if (reconnectAttempts < MAX_RECONNECT_ATTEMPTS) {
    reconnectAttempts++;
    const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), 30000);

    console.log(`Reconnecting in ${delay}ms...`);
    setTimeout(() => {
      connectWebSocket();
    }, delay);
  } else {
    console.error("Max reconnection attempts reached");
    showErrorToUser("Connection lost. Please refresh.");
  }
};

websocket.onopen = () => {
  reconnectAttempts = 0;
  console.log("WebSocket connected");
};
```

### 7. LiveKit Participant Tracking

**Track participant state:**
```javascript
const participants = new Map();

if (message.type === "message") {
  // Track active participants
  if (!participants.has(message.message.identity)) {
    participants.set(message.message.identity, {
      identity: message.message.identity,
      lastSeen: message.message.timestamp,
      messages: []
    });
  }

  participants.get(message.message.identity).messages.push(message.message);
  participants.get(message.message.identity).lastSeen = message.message.timestamp;
}

if (message.type === "participant_disconnected") {
  participants.delete(message.participant.identity);
  console.log(`Participant left: ${message.participant.name}`);

  // Prepare for connection close
  setTimeout(() => {
    websocket.close();
    // Reconnect if still needed
  }, 150);
}
```

---

## Troubleshooting

### No Transcription Results

**Symptoms:** Sending audio, but no `stt_result` messages received.

**Checks:**
1. **Audio format mismatch:** Verify binary audio matches `stt_config` exactly.
   ```javascript
   console.log("Config:", config.stt_config.sample_rate);
   console.log("Actual audio:", actualSampleRate);
   // Must match exactly
   ```

2. **Audio not reaching server:** Check WebSocket send calls.
   ```javascript
   websocket.send(audioBuffer);
   console.log("Sent audio:", audioBuffer.byteLength, "bytes");
   ```

3. **Silent audio:** Ensure audio contains speech, not silence.
   ```javascript
   // Check audio amplitude
   const volume = calculateVolume(audioBuffer);
   if (volume < 0.01) {
     console.warn("Audio too quiet");
   }
   ```

4. **Audio enabled:** Check `audio=true` in config.

5. **Provider errors:** Look for error messages from server.

---

### Garbled Transcriptions

**Symptoms:** Receiving transcripts, but they're nonsense.

**Cause:** Audio format mismatch (most common).

**Solution:**
```javascript
// Verify exact match
const config = {
  stt_config: {
    sample_rate: 16000,
    channels: 1,
    encoding: "linear16"
  }
};

// Ensure audio is:
// - 16000 Hz sample rate
// - Mono (1 channel)
// - 16-bit signed PCM
// - Little-endian byte order

// Use audio processing library to ensure correctness
const correctAudio = convertToFormat(rawAudio, {
  sampleRate: 16000,
  channels: 1,
  encoding: "s16le"  // signed 16-bit little-endian
});
```

---

### No TTS Audio

**Symptoms:** Sending `speak` command, but no binary audio received.

**Checks:**
1. **Audio enabled:** Verify `audio=true` in config.
2. **TTS config present:** Ensure `tts_config` provided.
3. **Error messages:** Check for TTS synthesis errors.
4. **Binary message handling:** Ensure your client processes binary WebSocket messages.
   ```javascript
   websocket.onmessage = (event) => {
     if (event.data instanceof Blob || event.data instanceof ArrayBuffer) {
       // Binary TTS audio
       playAudio(event.data);
     } else {
       // JSON control message
       const message = JSON.parse(event.data);
     }
   };
   ```

---

### Connection Drops

**Symptoms:** WebSocket closes unexpectedly.

**Causes:**
1. **Inactivity timeout (10s):** Send data or ping periodically.
   ```javascript
   // Keep-alive
   setInterval(() => {
     if (websocket.readyState === WebSocket.OPEN) {
       // Send empty binary frame as keep-alive
       websocket.send(new ArrayBuffer(0));
     }
   }, 5000);
   ```

2. **Participant disconnect (LiveKit):** Expected behavior. Reconnect if needed.

3. **Server error:** Check server logs for details.

4. **Network issues:** Implement reconnection logic (see Best Practices).

---

### LiveKit Participants Can't Hear AI

**Symptoms:** TTS audio sent, but LiveKit participants don't hear it.

**Checks:**
1. **LiveKit configured:** Ensure `livekit` block in config.
2. **Participants joined:** Verify participants obtained tokens and joined room.
3. **Audio tracks subscribed:** Check LiveKit SDK subscription logic.
4. **Participant filtering:** Ensure AI not filtered out.
   ```json
   // If Sayna identity is "sayna-ai", don't filter it out
   {
     "livekit": {
       "listen_participants": ["user-1", "user-2"]
       // This is for INPUT filtering (who Sayna listens to)
       // OUTPUT (TTS) always goes to all participants
     }
   }
   ```

5. **Audio track published:** Check Sayna successfully published track (server logs).

---

### High Latency

**Symptoms:** Long delays between speak command and audio playback.

**Optimizations:**
1. **Use caching:** Keep TTS config stable for cache hits.
2. **Reduce chunk size:** Send smaller audio chunks (100ms).
3. **Check network:** Ensure low-latency connection to server.
4. **Provider selection:** Some TTS providers faster than others.
5. **Measure latency:** Use `tts_playback_complete.timestamp` to identify bottlenecks.

**Typical Latencies:**
- TTS cache hit: 50-150ms
- TTS cache miss: 500-2000ms (provider dependent)
- STT interim results: 100-300ms
- STT final results: 300-800ms

---

### Memory Leaks

**Symptoms:** Memory usage grows over time.

**Checks:**
1. **Audio buffer cleanup:** Release audio buffers after processing.
   ```javascript
   websocket.onmessage = (event) => {
     if (event.data instanceof Blob) {
       const reader = new FileReader();
       reader.onload = () => {
         playAudio(reader.result);
         // reader.result automatically garbage collected
       };
       reader.readAsArrayBuffer(event.data);
     }
   };
   ```

2. **Event listener cleanup:** Remove listeners on disconnect.
   ```javascript
   websocket.onclose = () => {
     // Clean up
     stopMicrophone();
     releaseAudioContext();
     clearInterval(keepAliveInterval);
   };
   ```

3. **Participant map cleanup:** Remove disconnected participants.
   ```javascript
   if (message.type === "participant_disconnected") {
     participants.delete(message.participant.identity);
   }
   ```

---

## Summary

The Sayna WebSocket API provides a powerful foundation for real-time voice applications. Key takeaways:

**Core Concepts:**
- WebSocket at `/ws` for bidirectional communication
- Configuration message must be first
- Audio streaming for STT, speak commands for TTS
- Optional LiveKit integration for multi-party voice

**Message Types:**
- **Incoming:** config, speak, binary audio, clear, send_message, sip_transfer
- **Outgoing:** ready, stt_result, tts_playback_complete, message, participant_disconnected, error, sip_transfer_error, binary audio

**Operating Modes:**
- WebSocket-only: Full STT/TTS control, your app handles audio I/O
- WebSocket + LiveKit: Multi-party voice with WebRTC distribution
- LiveKit control-only: Room management without audio processing

**Best Practices:**
- Match audio formats exactly
- Send audio in small chunks (100-200ms)
- Keep TTS config stable for cache hits
- Implement graceful reconnection
- Monitor latency and errors

**LiveKit Integration:**
- Dual transport: WebSocket for control, LiveKit for audio distribution
- Participant filtering for selective audio processing
- Data channel for custom messaging
- Automatic room cleanup on disconnect
- Cloud recording support

With this foundation, you can build sophisticated voice applications ranging from simple transcription services to complex multi-party voice assistants. Explore the examples, experiment with configurations, and build amazing voice experiences with Sayna!

---

**Additional Resources:**
- [Google STT Integration](google-stt.md)
- [Azure STT Integration](azure-stt.md)
- [Cartesia STT Integration](cartesia-stt.md)
- [Google TTS Integration](google-tts.md)
- [Azure TTS Integration](azure-tts.md)
- [Authentication Documentation](authentication.md)
- [LiveKit Webhook Documentation](livekit_webhook.md)
- [API Reference (REST Endpoints)](../CLAUDE.md#api-endpoints)
- [LiveKit SDK Documentation](https://docs.livekit.io/)

**Need Help?**
- GitHub Issues: [Sayna Issues](https://github.com/yourusername/sayna/issues)
- Community Support: [Discussions](https://github.com/yourusername/sayna/discussions)
