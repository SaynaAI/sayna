# API Reference

Sayna exposes a concise REST surface for voice and LiveKit utilities together with a high-level WebSocket interface for real-time speech workflows. This document enumerates every public entry point, the payload shapes they expect, and the messages they emit.

## REST API

### GET /
- **Purpose**: Health probe to confirm the service is running.
- **Success**: `200 OK` with JSON object containing `status` (string equal to `OK`).

### GET /voices
- **Purpose**: Retrieve the catalogue of text-to-speech voices grouped by provider.
- **Success**: `200 OK` with a JSON object where each key is a provider identifier and the value is an array of voice descriptors.
- **Voice descriptor fields**:
  | Field | Type | Description |
  | --- | --- | --- |
  | `id` | string | Provider-specific identifier for the voice profile. |
  | `sample` | string | URL to a preview audio sample (may be empty). |
  | `name` | string | Human-readable name supplied by the provider. |
  | `accent` | string | Detected accent associated with the voice (falls back to `Unknown`). |
  | `gender` | string | Inferred gender label from provider metadata (falls back to `Unknown`). |
  | `language` | string | Primary language for synthesis (falls back to `Unknown`). |
- **Failure**:
  - `500 Internal Server Error` with JSON object containing `error` when provider credentials are missing or upstream calls fail.

### POST /speak
- **Purpose**: Synthesize a text snippet into audio using a configured provider.
- **Request body (application/json)**:
  | Field | Type | Required | Description |
  | --- | --- | --- | --- |
  | `text` | string | Yes | Text to convert to speech. Must be non-empty after trimming. |
  | `tts_config` | object | Yes | Provider configuration without API credentials. Uses the schema described under **TTS configuration object**. |
- **TTS configuration object**:
  | Field | Type | Required | Default | Description |
  | --- | --- | --- | --- | --- |
  | `provider` | string | Yes | – | Identifier of the TTS service (e.g., `deepgram`, `elevenlabs`). |
  | `voice_id` | string | No | Provider default | Voice or model variant to use. |
  | `speaking_rate` | number | No | `1.0` | Multiplier applied to the synthesized speech rate. |
  | `audio_format` | string | No | `linear16` | Preferred output codec (`wav`, `mp3`, `ogg`, etc.). |
  | `sample_rate` | integer | No | `24000` | Target sample rate in Hz. |
  | `connection_timeout` | integer | No | `30` | Maximum seconds to establish the provider connection. |
  | `request_timeout` | integer | No | `60` | Maximum seconds to wait for a synthesis request to complete. |
  | `model` | string | Yes | – | Provider model identifier. |
  | `pronunciations` | array | No | `[]` | Sequence of replacements applied before synthesis. Each entry contains `word` (string) and `pronunciation` (string). |
- **Success**: `200 OK` returning binary audio. Response headers include:
  - `Content-Type`: Chosen from `audio/wav`, `audio/mpeg`, `audio/ogg`, `audio/pcm`, `audio/basic`, or `application/octet-stream` based on the provider format.
  - `Content-Length`: Size in bytes.
  - `x-audio-format`: Raw format string reported by the provider.
  - `x-sample-rate`: Numeric sample rate selected for the output.
- **Failure**:
  - `400 Bad Request` with JSON `{ "error": "Text cannot be empty" }` when `text` is blank.
  - `500 Internal Server Error` with JSON `{ "error": "<description>" }` for missing credentials, connection problems, or synthesis errors.

### POST /livekit/token
- **Purpose**: Issue a LiveKit access token tailored for a participant.
- **Request body (application/json)**:
  | Field | Type | Required | Description |
  | --- | --- | --- | --- |
  | `room_name` | string | Yes | LiveKit room to join or create. |
  | `participant_name` | string | Yes | Display name assigned to the participant. |
  | `participant_identity` | string | Yes | Unique identifier for the participant. |
- **Success**: `200 OK` with JSON object containing:
  | Field | Type | Description |
  | --- | --- | --- |
  | `token` | string | JWT granting LiveKit permissions for the specified participant. |
  | `room_name` | string | Echo of the requested room. |
  | `participant_identity` | string | Echo of the requested identity. |
  | `livekit_url` | string | WebSocket endpoint for the LiveKit server. |
- **Failure**:
  - `400 Bad Request` with JSON `{ "error": "<description>" }` when any input field is blank.
  - `500 Internal Server Error` with JSON `{ "error": "<description>" }` when LiveKit services are unavailable or token generation fails.

## WebSocket API

- **Endpoint**: Connect to `GET /ws` using a WebSocket client.
- **Lifecycle overview**:
  1. Establish the WebSocket connection.
  2. Send a `config` message to declare audio providers and optional LiveKit options.
  3. Wait for a `ready` message before streaming audio or issuing commands.
  4. Stream binary audio frames (matching the declared STT encoding) and/or send JSON commands.
  5. Process server messages such as `stt_result`, binary audio, `message`, and `tts_playback_complete`.
  6. Close the connection or react to server-driven termination (for example after `participant_disconnected`).

### Client → Server Messages

#### `config`
Initializes the session. When `audio` is true (default), both STT and TTS configurations must be supplied so the server can provision the voice pipeline. LiveKit options are optional.

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `type` | string | Yes | `config` | Message discriminator. |
| `audio` | boolean | No | `true` | Enables STT/TTS handling. Set to `false` to create a data-only session while still allowing LiveKit messaging. |
| `stt_config` | object | Conditional | – | Required when `audio` is true. See **STT configuration object**. |
| `tts_config` | object | Conditional | – | Required when `audio` is true. Schema matches the **TTS configuration object** used by the REST `/speak` endpoint. |
| `livekit` | object | No | – | Optional LiveKit session parameters. See **LiveKit configuration object**. |

**STT configuration object**:

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `provider` | string | Yes | Identifier of the STT service (`deepgram`). |
| `language` | string | Yes | Language code for recognition (for example `en-US`). |
| `sample_rate` | integer | Yes | Audio sample rate in Hz expected from the client stream. |
| `channels` | integer | Yes | Number of channels (typically `1`). |
| `punctuation` | boolean | Yes | Enables punctuation in transcripts. |
| `encoding` | string | Yes | Audio encoding label (for example `linear16`). |
| `model` | string | Yes | Provider model name. |

**LiveKit configuration object**:

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `room_name` | string | Yes | – | LiveKit room to join or create. |
| `enable_recording` | boolean | No | `false` | Start a LiveKit room composite recording. |
| `recording_file_key` | string | Conditional | – | Required when `enable_recording` is true; used as the recording key. |
| `sayna_participant_identity` | string | No | `sayna-ai` | Identity assigned to the agent participant. |
| `sayna_participant_name` | string | No | `Sayna AI` | Display name for the agent participant. |
| `listen_participants` | array | No | `[]` | Optional list of participant identities to monitor; empty list means “all participants”. |

Once processed, the server fetches provider credentials from its own configuration, initializes the voice pipeline, optionally connects to LiveKit, and responds with a `ready` message.

#### `speak`
Queues text for TTS synthesis.

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `type` | string | Yes | `speak` | Message discriminator. |
| `text` | string | Yes | – | Text to synthesize. |
| `flush` | boolean | No | `true` | When true, clears any pending TTS audio before synthesizing. |
| `allow_interruption` | boolean | No | `true` | When false, subsequent `speak` or `clear` commands wait until playback finishes. |

#### `clear`
Clears queued TTS audio and resets any LiveKit audio buffers. Ignored while a non-interruptible synthesis (`allow_interruption = false`) is playing.

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `type` | string | Yes | `clear` |

#### `send_message`
Publishes a data message to the LiveKit room. Requires the connection to have been configured with LiveKit options.

| Field | Type | Required | Default | Description |
| --- | --- | --- | --- | --- |
| `type` | string | Yes | `send_message` | Message discriminator. |
| `message` | string | Yes | – | Textual payload delivered through LiveKit. |
| `role` | string | Yes | – | Label identifying the sender role. |
| `topic` | string | No | `messages` | LiveKit topic/channel to publish to. |
| `debug` | object | No | – | Auxiliary metadata included alongside the message for downstream consumers. |

#### Binary audio frames
Send raw audio bytes as binary WebSocket messages. Frames must match the `sample_rate`, `channels`, and `encoding` provided in the `stt_config`. These frames are routed directly into the speech-to-text pipeline and, when LiveKit is configured, mirrored through the same processing path.

### Server → Client Messages

#### `ready`
Indicates that the requested providers (and LiveKit, when applicable) are connected.

| Field | Type | Optional | Description |
| --- | --- | --- | --- |
| `type` | string | – | Always `ready`. |
| `livekit_room_name` | string | Yes | LiveKit room name acknowledged by the server (present only when LiveKit is enabled). |
| `livekit_url` | string | No | LiveKit WebSocket URL configured on the server. |
| `sayna_participant_identity` | string | Yes | Identity assigned to the agent participant when LiveKit is enabled. |
| `sayna_participant_name` | string | Yes | Display name assigned to the agent participant when LiveKit is enabled. |

#### `stt_result`
Real-time transcription output from the speech-to-text provider.

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | Always `stt_result`. |
| `transcript` | string | Recognized text. |
| `is_final` | boolean | Indicates whether the transcript is final (no further updates expected for this utterance). |
| `is_speech_final` | boolean | Signals the end of a speech segment; useful for turn-taking. |
| `confidence` | number | Provider confidence score between 0.0 and 1.0. |

#### `message`
Wraps LiveKit data channel payloads into a unified structure.

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | Always `message`. |
| `message` | object | Unified message payload detailed below. |

**Unified message payload**:

| Field | Type | Optional | Description |
| --- | --- | --- | --- |
| `message` | string | Yes | Text content if the payload was UTF-8. |
| `data` | string | Yes | Base64-encoded data when the payload was binary. |
| `identity` | string | No | Sender identity reported by LiveKit. |
| `topic` | string | No | Topic/channel associated with the message. |
| `room` | string | No | Room identifier (currently `livekit`). |
| `timestamp` | integer | No | Unix timestamp in milliseconds when the payload was received. |

#### `participant_disconnected`
Notifies the client when a participant leaves the LiveKit room. After this message the server initiates a clean shutdown of the WebSocket session.

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | Always `participant_disconnected`. |
| `participant` | object | Participant details. |

**Participant details**:

| Field | Type | Optional | Description |
| --- | --- | --- | --- |
| `identity` | string | No | Unique participant identity. |
| `name` | string | Yes | Display name if provided by LiveKit. |
| `room` | string | No | Room identifier associated with the event. |
| `timestamp` | integer | No | Unix timestamp in milliseconds when the disconnection occurred. |

#### `tts_playback_complete`
Emitted after all audio chunks for a `speak` command have been delivered.

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | Always `tts_playback_complete`. |
| `timestamp` | integer | Milliseconds since Unix epoch representing when completion was recorded. |

#### `error`
Represents a recoverable problem encountered while processing the session.

| Field | Type | Description |
| --- | --- | --- |
| `type` | string | Always `error`. |
| `message` | string | Human-readable explanation. |

#### Binary audio frames
The server streams synthesized TTS audio as binary messages. Payloads use the format declared in the headers of the initial `ready` response and mirror the selection returned by the TTS provider. When LiveKit is enabled, the same audio is also published to the LiveKit room.

### LiveKit Integration Notes
- The server generates and manages LiveKit agent tokens internally; clients only supply the user token via REST before connecting.
- When recording is requested, the server attempts to start a room composite egress and remembers the `recording_file_key`. Recording stoppage and room deletion are handled during WebSocket teardown.
- The `clear` command also flushes LiveKit audio buffers, ensuring immediate interruption across both WebSocket and LiveKit listeners.
- If the server loses connection to LiveKit or encounters queueing issues, an `error` message is emitted describing the failure.


