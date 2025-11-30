# Deployment Guide

Sayna ships as a single Axum binary that can be deployed anywhere you can run containers. This guide covers the recommended workflow for packaging, configuring, and running the service alongside LiveKit in Docker-based stacks (Docker Compose, Kubernetes, or any platform that can schedule OCI images).

## 1. Prerequisites

- **LiveKit cluster** reachable from the Sayna pod/container (LAN or VPC is ideal). Set `LIVEKIT_URL` to the internal address and `LIVEKIT_PUBLIC_URL` to the address clients should use.
- **Provider credentials** for the STT/TTS services you plan to use:
  - `DEEPGRAM_API_KEY` for Deepgram STT/TTS.
  - `ELEVENLABS_API_KEY` for ElevenLabs TTS.
- Optional: S3-compatible bucket for LiveKit recording egress (`RECORDING_S3_*` variables).
- Optional: Authentication settings documented in `docs/authentication.md`.
- Persistent volume (or host path) for `CACHE_PATH` when you want voice outputs and turn-detector assets to survive container restarts.

## 2. Building the Container Image

The provided multi-stage `Dockerfile` compiles Sayna with release optimizations and bundles required runtime assets. To build locally:

```bash
docker build -t sayna:latest .
```

Key details:

- `arg RUST_VERSION` lets you pin the toolchain. Override with `--build-arg RUST_VERSION=1.75.0` if needed.
- `sayna init` runs during the image build to pre-download turn-detection assets when `CACHE_PATH` is set. Re-run `sayna init` at runtime if you mount a fresh cache volume.
- Default exposed port is `3001`. Override with `-e PORT=XXXX` if necessary.

## 3. Runtime Environment Variables

Add the relevant variables before starting the container. The most common ones are listed below.

| Variable | Purpose | Example |
| --- | --- | --- |
| `HOST` | Bind address inside the container. Usually leave as `0.0.0.0`. | `0.0.0.0` |
| `PORT` | Axum listener port. | `3001` |
| `CACHE_PATH` | Directory that stores cached audio and turn-detect assets. Mount a volume for persistence. | `/data/cache` |
| `DEEPGRAM_API_KEY` | Enables Deepgram STT/TTS. | `dg-secret` |
| `ELEVENLABS_API_KEY` | Enables ElevenLabs TTS. | `el-secret` |
| `LIVEKIT_URL` | Server-to-server WebSocket URL (internal). | `ws://livekit:7880` |
| `LIVEKIT_PUBLIC_URL` | URL clients should dial (returned via APIs). | `https://rtc.yourdomain.com` |
| `LIVEKIT_API_KEY` / `LIVEKIT_API_SECRET` | Credentials used to mint LiveKit tokens in `/livekit/token` and during WebSocket LiveKit bring-up. | `lk_key` / `lk_secret` |
| `RECORDING_S3_*` | Bucket configuration for LiveKit recordings (bucket, region, endpoint, access key, secret key). | `recordings`, `us-east-1`, etc. |

Set `AUTH_REQUIRED`, `AUTH_SERVICE_URL`, `AUTH_SIGNING_KEY_PATH`, or `AUTH_API_SECRET` only if you follow the separate authentication guide.

## 4. Local Docker Run

```bash
docker run --rm \
  -p 3001:3001 \
  -e HOST=0.0.0.0 \
  -e PORT=3001 \
  -e CACHE_PATH=/data/cache \
  -e DEEPGRAM_API_KEY=dg-secret \
  -e ELEVENLABS_API_KEY=el-secret \
  -e LIVEKIT_URL=ws://livekit:7880 \
  -e LIVEKIT_PUBLIC_URL=https://rtc.localhost \
  -e LIVEKIT_API_KEY=lk_key \
  -e LIVEKIT_API_SECRET=lk_secret \
  -v sayna-cache:/data/cache \
  sayna:latest
```

Mounting `sayna-cache` ensures cached voices and turn detection assets persist.

## 5. Docker Compose Example

```yaml
# docker-compose.yml
version: "3.9"
services:
  livekit:
    image: livekit/livekit-server:latest
    environment:
      - LIVEKIT_KEYS=lk_key:lk_secret
    ports:
      - "7880:7880"     # WebRTC / signaling
      - "7881:7881"     # TURN (optional)

  sayna:
    image: sayna:latest
    depends_on:
      - livekit
    ports:
      - "3001:3001"
    environment:
      HOST: 0.0.0.0
      PORT: 3001
      CACHE_PATH: /data/cache
      DEEPGRAM_API_KEY: ${DEEPGRAM_API_KEY}
      ELEVENLABS_API_KEY: ${ELEVENLABS_API_KEY}
      LIVEKIT_URL: ws://livekit:7880
      LIVEKIT_PUBLIC_URL: https://rtc.example.com
      LIVEKIT_API_KEY: lk_key
      LIVEKIT_API_SECRET: lk_secret
      RECORDING_S3_BUCKET: sayna-egress
      RECORDING_S3_REGION: us-east-1
      RECORDING_S3_ENDPOINT: https://s3.amazonaws.com
      RECORDING_S3_ACCESS_KEY: ${S3_ACCESS_KEY}
      RECORDING_S3_SECRET_KEY: ${S3_SECRET_KEY}
    volumes:
      - sayna-cache:/data/cache

volumes:
  sayna-cache: {}
```

The Compose network ensures `LIVEKIT_URL=ws://livekit:7880` resolves internally while `LIVEKIT_PUBLIC_URL` remains the externally reachable URL.

## 6. Kubernetes Deployment Example

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: sayna
spec:
  replicas: 2
  selector:
    matchLabels:
      app: sayna
  template:
    metadata:
      labels:
        app: sayna
    spec:
      containers:
        - name: sayna
          image: ghcr.io/your-org/sayna:latest
          ports:
            - containerPort: 3001
          envFrom:
            - secretRef:
                name: sayna-secrets   # API keys, LiveKit credentials
            - configMapRef:
                name: sayna-config    # Non-sensitive values
          volumeMounts:
            - name: cache
              mountPath: /data/cache
      volumes:
        - name: cache
          persistentVolumeClaim:
            claimName: sayna-cache-pvc
---
apiVersion: v1
kind: Service
metadata:
  name: sayna
spec:
  selector:
    app: sayna
  ports:
    - name: http
      port: 3001
      targetPort: 3001
  type: ClusterIP
```

Recommended ConfigMap entries:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: sayna-config
data:
  HOST: "0.0.0.0"
  PORT: "3001"
  CACHE_PATH: "/data/cache"
  LIVEKIT_URL: "ws://livekit.livekit.svc.cluster.local:7880"
  LIVEKIT_PUBLIC_URL: "https://rtc.example.com"
```

Store `DEEPGRAM_API_KEY`, `ELEVENLABS_API_KEY`, `LIVEKIT_API_KEY`, `LIVEKIT_API_SECRET`, and S3 credentials in `sayna-secrets`.

## 7. LiveKit Configuration Walkthrough

1. **Networking**: Ensure the Sayna container can reach the LiveKit signaling endpoint specified in `LIVEKIT_URL`. In Compose, use the service name; in Kubernetes, use the service DNS name.
2. **Credentials**: Provide `LIVEKIT_API_KEY`/`LIVEKIT_API_SECRET`. Sayna uses these internally to mint:
   - Agent tokens during WebSocket configuration (`livekit` block of the `config` message).
   - User tokens via the `/livekit/token` REST endpoint.
3. **Client workflow**:
   - Call `/ws`, send a `config` payload that includes the `livekit` section:
     ```json
     {
       "type": "config",
       "stream_id": "support-room-42-2024-01-31",
       "audio": true,
       "stt_config": { "...": "..." },
       "tts_config": { "...": "..." },
       "livekit": {
         "room_name": "support-room-42",
         "enable_recording": true,
         "sayna_participant_identity": "sayna-ai",
         "sayna_participant_name": "Sayna AI",
         "listen_participants": ["agent-1", "customer-42"]
       }
     }
     ```
   - Receive `ready` with `livekit_room_name`, `livekit_url`, and the Sayna participant metadata.
   - For each human participant, request a token via `POST /livekit/token` and join the LiveKit room directly.

## 8. Configuration Variations

### A. Voice Pipeline Without LiveKit
- Leave all `LIVEKIT_*` variables unset and omit the `livekit` block in the WebSocket `config` message.
- Useful for pure WebSocket-only deployments or standalone REST TTS flows.

### B. LiveKit Mirroring With Recording
- Provide `LIVEKIT_URL`, `LIVEKIT_PUBLIC_URL`, `LIVEKIT_API_KEY`, `LIVEKIT_API_SECRET`.
- Configure `RECORDING_S3_*` so LiveKit recording egress can persist files.
- Set `"enable_recording": true` and include a session-level `stream_id` to control the `{server_prefix}/{stream_id}/audio.ogg` recording path. Omit `stream_id` to let the server generate one.

### C. Text/Data-Only Sessions
- Start Sayna with normal audio credentials but send a WebSocket `config` message where `"audio": false`.
- Lets you keep LiveKit messaging/data-plane behavior (including `/livekit/token`) while skipping STT/TTS provider initialization, ideal for environments without audio API keys.

### D. Custom Cache Strategies
- **Filesystem cache** (recommended for production): set `CACHE_PATH` to a mounted volume so `sayna init` persists turn-detect assets and TTS audio across restarts.
- **In-memory cache**: omit `CACHE_PATH` to keep assets in RAM; faster to bootstrap but cleared on restarts.

### E. Feature Flag Tuning
- Disable expensive DSP layers when you need minimal footprints:
  - `cargo run --no-default-features --features openapi` (no turn detection or noise filter).
  - Build flags propagate into Docker by overriding `cargo build` command if you maintain a custom Dockerfile.
- The published Dockerfile uses `--all-features` to keep the container ready for every scenario; create a variant if you want smaller images.

## 9. Verification Checklist

1. `curl http://<sayna-host>:3001/` returns `{ "status": "OK" }`.
2. `GET /voices` succeeds (requires provider keys).
3. WebSocket `config` message with LiveKit block yields a `ready` payload that includes `livekit_room_name`.
4. `POST /livekit/token` issues tokens that successfully join the LiveKit room.
5. If recordings are enabled, verify the output object lands in the configured S3 bucket.

Following this guide you can deploy Sayna in any container-friendly environment, pairing it with LiveKit for low-latency, bidirectional speech workflows.
