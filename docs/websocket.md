# Realtime WebSocket Sessions

Sayna's `/ws` endpoint is the heart of the platform. Every low-latency STT/TTS experience, every LiveKit bridge, and every conversational agent loop begins with a WebSocket session. This document explains the session theory of operation, message grammar, LiveKit integration modes, and the internal plumbing (VoiceManager, LiveKitClient, queues, caches) that keep the audio graph stable under heavy load.

---

## Transport & Authentication Model
- **Endpoint:** `GET /ws`. The Axum router upgrades the connection and wraps it with `TraceLayer` so every connection is traced like an HTTP request.
- **Authentication:** WebSockets are intentionally unauthenticated. Gate access at the network or proxy layer or embed your own token in the first `config` message. REST endpoints (`/voices`, `/speak`, `/livekit/token`) remain protected when `AUTH_REQUIRED=true`.
- **Liveness:** Axum handles ping/pong. The server checks for stalled receivers every 10s to close abandoned sockets.
- **Backpressure:** A dedicated Tokio channel (`CHANNEL_BUFFER_SIZE = 1024`) buffers outgoing frames so bursts of STT results or TTS audio do not fight each other.

---

## Session Lifecycle (High Level)
1. **Connect:** Client opens a WebSocket to `/ws`.
2. **Configure:** First message must be a JSON `config` block describing STT, TTS, optional LiveKit settings, and whether audio features are enabled. Missing configs yield an `error` frame but the socket stays open so the client can retry.
3. **Initialization:** `VoiceManager` boots the requested providers, wires STT/TTS callbacks, primes the audio cache, and (when requested) spins up a LiveKit client plus operation worker.
4. **Ready:** Once providers are live (and LiveKit is joined), the server emits `{"type":"ready", ...}` describing the LiveKit room/URL plus the Sayna agent identity.
5. **Streaming:** Clients stream binary audio frames for STT, fire `speak` commands for TTS, send `clear`/`send_message` controls, and listen for transcripts, TTS chunks, LiveKit messages, and telemetry.
6. **Cleanup:** When the socket closes, the handler stops the VoiceManager, disconnects LiveKit, stops any recording egress, and deletes the LiveKit room to avoid orphaned meetings.

---

## Connection State Internals

Each connection owns a `ConnectionState` guarded by an `Arc<RwLock<_>>`. Important fields:
- `voice_manager: Arc<VoiceManager>` – handles STT/TTS providers, callbacks, caches, and interruption state.
- `audio_enabled: AtomicBool` – avoids locking on the hot path; `audio=false` sessions can still leverage LiveKit data channels.
- `livekit_client: Arc<RwLock<LiveKitClient>>` plus `OperationQueue` – serializes LiveKit audio/data operations so WebSocket handling never blocks on RTC calls.
- `livekit_room_name` & `recording_egress_id` – tracked for cleanup when the socket exits.

---

## Configuration Message

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
    "sample_rate": 24000,
    "connection_timeout": 30,
    "request_timeout": 60,
    "pronunciations": []
  },
  "livekit": {
    "room_name": "demo-room",
    "enable_recording": true,
    "recording_file_key": "sessions/demo-room",
    "sayna_participant_identity": "sayna-ai",
    "sayna_participant_name": "Sayna AI",
    "listen_participants": ["host", "moderator"]
  }
}
```

### Audio Flag
- Defaults to `true`. When `false`, the WebSocket behaves as a control/data bridge (LiveKit messages, LiveKit recording automation) without spinning up STT/TTS providers.
- Attempting to send binary audio or `speak` commands before enabling audio yields an `error` frame but does not sever the connection.

### STT Block (`stt_config`)
- Contains the provider metadata minus API keys. The server injects secrets from `ServerConfig`.
- `sample_rate`, `channels`, and `encoding` must match the binary frames you will stream. The hot path does no conversion, so mismatches manifest as garbage transcripts.
- `model` drives provider-specific behaviour. Whatever you request is passed directly to the provider factory.

### TTS Block (`tts_config`)
- Same keyless contract: just provider/model/voice parameters.
- VoiceManager hashes the config (via `compute_tts_config_hash`) and seeds the shared cache so repeated prompts can be replayed instantly.
- `speaking_rate`, `audio_format`, and `sample_rate` control provider output. The binary frames pushed to the WebSocket (and optionally to LiveKit) mirror the provider’s native format—no transcoding layer hides errors.
- `pronunciations` can be used to patch provider lexicons before synthesis.

### LiveKit Block (`livekit`)
- Optional. When supplied, the handler creates the room through `LiveKitRoomHandler`, generates a Sayna agent token (`sayna_participant_*` overrides the defaults), and optionally starts a recording egress if `enable_recording=true`.
- `listen_participants`: filter list of participant identities. Leave empty to consume every track/data channel.
- Sessions using LiveKit should instruct human participants to call `POST /livekit/token` with the `room_name` to obtain their own tokens.

---

## Incoming Messages

| Type | Payload | Purpose |
| --- | --- | --- |
| `config` | See above | Bootstraps providers and (optionally) LiveKit. Must arrive first. |
| `speak` | `{ "text": "...", "flush": true, "allow_interruption": false }` | Queues or flushes TTS work. `flush` defaults to `true`. `allow_interruption=false` pins the interruption guard so `clear` or STT events cannot cancel playback until chunks finish streaming. |
| Binary | raw PCM/Opus/etc. | Direct audio frames sent as `Bytes`. The handler forwards them to `VoiceManager::receive_audio` with zero-copy semantics. |
| `clear` | `{ "type": "clear" }` | Clears queued TTS plus LiveKit audio buffers (unless playback is currently marked non-interruptible). |
| `send_message` | `{ "message": "...", "role": "user", "topic": "chat", "debug": {...} }` | Publishes a LiveKit data-channel message. Requires a LiveKit session because it is executed through `LiveKitOperation::SendMessage`. |

---

## Outgoing Messages

| Type | Fields | Notes |
| --- | --- | --- |
| `ready` | `livekit_room_name`, `livekit_url`, `sayna_participant_identity`, `sayna_participant_name` | Emitted once STT, TTS, cache, and LiveKit (if requested) are ready. Use `livekit_url` to dial the corresponding room via REST token. |
| `stt_result` | `transcript`, `is_final`, `is_speech_final`, `confidence` | Produced from the VoiceManager STT callback. `is_speech_final` benefits from the optional ONNX turn detector so you can know when Sayna believes a speaker is done. |
| `message` | `message: UnifiedMessage` | Unified envelope for LiveKit data received by the agent. Contains optional `message` (UTF‑8 text) or base64 `data`, plus `identity`, `topic`, `room`, `timestamp`. |
| `participant_disconnected` | `participant` block | Fired when LiveKit signals a participant disconnect. The handler gives clients 100 ms to flush UI updates before it schedules a WebSocket close. |
| `tts_playback_complete` | `timestamp` | Fired after the TTS dispatcher emits the final chunk for a `speak` command. Compare with the time you sent the prompt to measure latency. |
| `error` | `message` | Non-fatal errors (missing config, LiveKit queue failures, provider initialization issues). Fatal errors are followed by a socket close. |
| Binary | audio bytes | Direct TTS frames. When LiveKit is enabled, the same bytes are piped into the RTC track via the operation queue so browser participants hear the AI voice exactly as the WebSocket client receives it. |

---

## Audio Flow Without LiveKit

1. **Binary frames** -> `handle_audio_message` -> `VoiceManager::receive_audio`.
2. `VoiceManager` feeds STT, which calls back into the handler with transcripts.
3. `speak` (flush true) -> `VoiceManager::speak_with_interruption` -> provider -> TTS callback -> `MessageRoute::Binary`.
4. Optional caching replays identical prompts immediately (the early callback fires as soon as cached bytes are available).
5. `clear` uses the `audio_clear_callback` to purge pending audio, after which `VoiceManager` resets the interruption guard so new prompts can start.

Because there is no LiveKit leg, the WebSocket is both the ingest and egress transport for all media.

---

## LiveKit-Enabled Sessions

When `livekit` is supplied in the config:

### Room Creation & Tokens
- `LiveKitRoomHandler` ensures the room exists, generates an agent token, and (if `enable_recording`) starts an S3 egress using `recording_file_key`.
- The handler also records `livekit_room_name` so it can delete the room and stop recording once the socket drops.
- Clients joining the same room request their own token via `POST /livekit/token`. The response echoes the room name, participant identity, and `livekit_public_url` so browsers know which URL to dial.

### Audio Routing
- Incoming LiveKit audio is converted to raw bytes and passed through the same STT pipeline as WebSocket audio, which means DeepFilterNet, turn detection, and confidence scoring apply uniformly.
- TTS output is duplicated: first streamed to the WebSocket, then sent through `LiveKitOperation::SendAudio`. The queue gives audio operations the highest priority so RTC playback never starves.
- `listen_participants` filters whose tracks/data are forwarded (handy in multiparty rooms).

### Data Channel & Control Plane
- `send_message` commands are executed by queuing `LiveKitOperation::SendMessage`. The operation worker handles retries and enforces backpressure against the RTC data channel.
- Received LiveKit data messages are wrapped into the `message` envelope described above, preserving `topic` and sender identity.

### Recording & Cleanup
- If recording was enabled, the `recording_egress_id` stored in `ConnectionState` is used to call `stop_room_recording` before room deletion.
- Participant disconnect events are forwarded to the client and deliberately followed by a WebSocket close to ensure the server tears down the LiveKit session promptly.

### LiveKit Control Mode (`audio: false`)
- You can omit STT/TTS and still leverage Sayna as a managed LiveKit agent. Set `audio: false`, provide a `livekit` block, and use `send_message` or LiveKit data messages to exchange metadata while TTS/STT happen elsewhere.

---

## Interruption & Playback Semantics

VoiceManager enforces predictable playback:
- `allow_interruption=false` sets a non-interruptible window. New `speak` commands queue but do not pre-empt, `clear` returns silently, and STT results are ignored until playback completes.
- When audio is interruptible, `clear` immediately flushes provider queues and calls the LiveKit audio clear callback so both transports stay synchronized.
- `tts_playback_complete` is emitted after the queue drains, giving clients a hook to resume microphone capture or UI prompts.

---

## Error Handling & Observability

- Most misconfigurations yield `{"type":"error","message":"..."}` and keep the socket open so you can resubmit a `config`.
- Provider readiness is enforced with a 30 s timeout. Missing API keys, invalid models, or queue failures surface as errors before `ready` is sent.
- LiveKit operations use oneshot channels; failures (e.g., queue overflow, RTC disconnect) are bubbled up via `error` messages.
- Trace logging is enabled for every request, and key transitions (`ready`, LiveKit connect/disconnect, recording start/stop) are logged at `info` level for auditability.

---

## Implementation Patterns

### 1. Pure WebSocket STT/TTS
```text
Client --> /ws
  config (audio=true, no livekit)
  ready
  [binary audio frames] --> stt_result … stt_result(is_final)
  speak("answer", flush=true)
  [binary TTS audio] + tts_playback_complete
```
- Use when you want tight control over audio capture/playback inside your own application.

### 2. Hybrid WebSocket + LiveKit Room
```text
Client --> /ws
  config(audio=true, livekit.room_name="demo")
  ready(livekit_room_name="demo", livekit_url="wss://lk.example.com")

Browser --> POST /livekit/token { room_name: "demo", ... }
Browser --> LiveKit using token

LiveKit participants speak -> Sayna streams transcripts via WebSocket and mirrors synthesized audio back into the room.
```
- Perfect for “copilot in a meeting” scenarios. WebSocket remains your control plane while LiveKit handles distribution to browsers.

### 3. LiveKit Message Bus Only
```jsonc
// config
{
  "type": "config",
  "audio": false,
  "livekit": { "room_name": "support-handoff" }
}
```
- Use `send_message` to push JSON to other participants and listen to `message` events for remote signals. Add LiveKit recording toggles or SIP dispatch rules as needed.

---

## Practical Tips
- **Align audio formats:** Always match your binary audio frames to the `stt_config.sample_rate`, `channels`, and `encoding`. The server never resamples.
- **Batch speech intelligently:** Turn detection (`turn-detect` feature flag) can mark `is_speech_final=true` sooner if you stream continuous audio rather than clipping per word.
- **Measure latency:** Compare the `tts_playback_complete.timestamp` with the timestamp when you sent `speak`. Same for STT—the `is_final` flag denotes when providers flush.
- **Handle LiveKit exits:** When you receive `participant_disconnected`, prepare for the socket to close and re-establish the session if you still need it.
- **Reuse configs:** Identical `tts_config` values benefit from cache hits. Keep a stable config per conversational agent to maximize reuse.

---

By understanding the full WebSocket session architecture—configuration, control messages, LiveKit hooks, and the VoiceManager/TTS/STT pipeline—you can compose any realtime voice experience on top of Sayna, from simple streaming transcription to rich multiparty meetings where an AI assistant hears and speaks inside LiveKit while your application orchestrates behaviour through the same WebSocket. Let the `/ws` channel be your single source of truth for the session’s state machine, and lean on the documented message types above to keep clients and the server perfectly synchronized.
