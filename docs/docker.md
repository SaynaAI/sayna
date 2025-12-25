# Sayna

A high-performance real-time voice processing server providing unified Speech-to-Text (STT) and Text-to-Speech (TTS) services through WebSocket and REST APIs.

## Quick Start

```bash
docker run -d \
  -p 3001:3001 \
  -e DEEPGRAM_API_KEY=your-key \
  saynaai/sayna
```

The server will be available at `http://localhost:3001`.

## Supported Providers

- **Deepgram** - STT and TTS
- **ElevenLabs** - STT and TTS
- **Google Cloud** - STT and TTS (WaveNet, Neural2, Studio voices)
- **Microsoft Azure** - STT and TTS (400+ neural voices)
- **Cartesia** - STT and TTS

## Environment Variables

### Server Configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `HOST` | Bind address | `0.0.0.0` |
| `PORT` | Server port | `3001` |
| `CACHE_PATH` | Cache directory for models and audio | `/app/cache` |
| `RUST_LOG` | Log level (error, warn, info, debug, trace) | `info` |

### Provider API Keys

| Variable | Description |
|----------|-------------|
| `DEEPGRAM_API_KEY` | Deepgram API key |
| `ELEVENLABS_API_KEY` | ElevenLabs API key |
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to Google Cloud service account JSON |
| `AZURE_SPEECH_SUBSCRIPTION_KEY` | Azure Speech subscription key |
| `AZURE_SPEECH_REGION` | Azure region (default: `eastus`) |
| `CARTESIA_API_KEY` | Cartesia API key |

### LiveKit Integration

| Variable | Description | Default |
|----------|-------------|---------|
| `LIVEKIT_URL` | LiveKit server WebSocket URL | `ws://localhost:7880` |
| `LIVEKIT_PUBLIC_URL` | Public LiveKit URL for clients | - |
| `LIVEKIT_API_KEY` | LiveKit API key | - |
| `LIVEKIT_API_SECRET` | LiveKit API secret | - |

### Authentication (Optional)

| Variable | Description | Default |
|----------|-------------|---------|
| `AUTH_REQUIRED` | Enable authentication | `false` |
| `AUTH_API_SECRETS_JSON` | API secrets JSON array (`[{id, secret}]`) | - |
| `AUTH_API_SECRET` | Legacy single API secret | - |
| `AUTH_API_SECRET_ID` | Legacy API secret id for `AUTH_API_SECRET` | `default` |
| `AUTH_SERVICE_URL` | External auth service URL | - |
| `AUTH_SIGNING_KEY_PATH` | Path to JWT signing key | - |
| `AUTH_TIMEOUT_SECONDS` | Auth request timeout | `5` |

Minimal multi-secret example:

```bash
AUTH_API_SECRETS_JSON='[{"id":"default","secret":"sk_test_default_123"}]'
```

### S3 Recording (Optional)

| Variable | Description |
|----------|-------------|
| `RECORDING_S3_BUCKET` | S3 bucket name |
| `RECORDING_S3_REGION` | S3 region |
| `RECORDING_S3_ENDPOINT` | S3 endpoint URL |
| `RECORDING_S3_ACCESS_KEY` | S3 access key |
| `RECORDING_S3_SECRET_KEY` | S3 secret key |

## Docker Compose

```yaml
version: "3.9"
services:
  sayna:
    image: saynaai/sayna
    ports:
      - "3001:3001"
    environment:
      DEEPGRAM_API_KEY: ${DEEPGRAM_API_KEY}
      ELEVENLABS_API_KEY: ${ELEVENLABS_API_KEY}
      CACHE_PATH: /data/cache
    volumes:
      - sayna-cache:/data/cache

volumes:
  sayna-cache: {}
```

### With LiveKit

```yaml
version: "3.9"
services:
  livekit:
    image: livekit/livekit-server:latest
    environment:
      LIVEKIT_KEYS: "devkey:secret"
    ports:
      - "7880:7880"

  sayna:
    image: saynaai/sayna
    depends_on:
      - livekit
    ports:
      - "3001:3001"
    environment:
      DEEPGRAM_API_KEY: ${DEEPGRAM_API_KEY}
      ELEVENLABS_API_KEY: ${ELEVENLABS_API_KEY}
      LIVEKIT_URL: ws://livekit:7880
      LIVEKIT_PUBLIC_URL: ws://localhost:7880
      LIVEKIT_API_KEY: devkey
      LIVEKIT_API_SECRET: secret
      CACHE_PATH: /data/cache
    volumes:
      - sayna-cache:/data/cache

volumes:
  sayna-cache: {}
```

## Persistent Cache

Mount a volume to `/data/cache` (or your configured `CACHE_PATH`) to persist:
- Turn detection model assets
- Cached TTS audio outputs

```bash
docker run -d \
  -p 3001:3001 \
  -e DEEPGRAM_API_KEY=your-key \
  -e CACHE_PATH=/data/cache \
  -v sayna-cache:/data/cache \
  saynaai/sayna
```

## Health Check

```bash
curl http://localhost:3001/
# Returns: {"status":"OK"}
```

## API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Health check |
| `/ws` | WebSocket | Real-time voice processing |
| `/voices` | GET | List available TTS voices |
| `/speak` | POST | Generate speech from text |
| `/livekit/token` | POST | Generate LiveKit participant token |
| `/livekit/webhook` | POST | LiveKit event webhook |

## Audio-Disabled Mode

Run without provider API keys for development and testing:

```bash
docker run -d -p 3001:3001 saynaai/sayna
```

Then send a WebSocket config with `audio_disabled: true`:

```json
{
  "type": "config",
  "config": {
    "audio_disabled": true
  }
}
```

## Image Variants

The default image includes:
- **Turn Detection** - ONNX-based speech turn detection
- **Noise Filter** - DeepFilterNet noise suppression

Pre-downloaded model assets are included in the image.

## Architecture

- **amd64** (x86_64)
- **arm64** (aarch64)

## Documentation

For complete documentation, visit [https://docs.sayna.ai](https://docs.sayna.ai)

## Source Code

[https://github.com/saynaai/sayna](https://github.com/saynaai/sayna)
