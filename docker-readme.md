# Sayna

A high-performance real-time voice processing server built in Rust that provides unified Speech-to-Text (STT) and Text-to-Speech (TTS) services through WebSocket and REST APIs.

## Features

- **Unified Voice API**: Single interface for multiple STT/TTS providers
- **Real-time Processing**: WebSocket-based bidirectional audio streaming
- **LiveKit Integration**: WebRTC audio streaming with room-based communication
- **Advanced Noise Filtering**: DeepFilterNet integration for noise suppression
- **Provider Flexibility**: Pluggable architecture supporting multiple providers
  - Deepgram (STT/TTS)
  - ElevenLabs (STT/TTS)
  - Google Cloud (STT/TTS) - WaveNet, Neural2, and Studio voices
  - Microsoft Azure (STT/TTS) - 400+ neural voices across 140+ languages
  - Cartesia (STT/TTS)

## Quick Start

### Pull the Image

```bash
docker pull saynaai/sayna:latest
```

### Basic Run

```bash
docker run --rm \
  -p 3001:3001 \
  -e DEEPGRAM_API_KEY=your_deepgram_key \
  saynaai/sayna:latest
```

The server will start on `http://localhost:3001`.

### Run with All Provider Keys

```bash
docker run --rm \
  -p 3001:3001 \
  -e HOST=0.0.0.0 \
  -e PORT=3001 \
  -e CACHE_PATH=/data/cache \
  -e DEEPGRAM_API_KEY=your_deepgram_key \
  -e ELEVENLABS_API_KEY=your_elevenlabs_key \
  -e CARTESIA_API_KEY=your_cartesia_key \
  -e AZURE_SPEECH_SUBSCRIPTION_KEY=your_azure_key \
  -e AZURE_SPEECH_REGION=eastus \
  -v sayna-cache:/data/cache \
  saynaai/sayna:latest
```

### Run with LiveKit Integration

```bash
docker run --rm \
  -p 3001:3001 \
  -e HOST=0.0.0.0 \
  -e PORT=3001 \
  -e CACHE_PATH=/data/cache \
  -e DEEPGRAM_API_KEY=your_deepgram_key \
  -e ELEVENLABS_API_KEY=your_elevenlabs_key \
  -e LIVEKIT_URL=ws://livekit:7880 \
  -e LIVEKIT_PUBLIC_URL=https://rtc.yourdomain.com \
  -e LIVEKIT_API_KEY=your_livekit_key \
  -e LIVEKIT_API_SECRET=your_livekit_secret \
  -v sayna-cache:/data/cache \
  saynaai/sayna:latest
```

## Docker Compose Example

```yaml
version: "3.9"
services:
  livekit:
    image: livekit/livekit-server:latest
    environment:
      - LIVEKIT_KEYS=lk_key:lk_secret
    ports:
      - "7880:7880"
      - "7881:7881"

  sayna:
    image: saynaai/sayna:latest
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
    volumes:
      - sayna-cache:/data/cache

volumes:
  sayna-cache: {}
```

## Configuration

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `HOST` | Server bind address | `0.0.0.0` |
| `PORT` | Server port | `3001` |
| `CACHE_PATH` | Directory for cached audio and turn-detect assets | `/app/cache` |
| `DEEPGRAM_API_KEY` | Deepgram API key for STT/TTS | - |
| `ELEVENLABS_API_KEY` | ElevenLabs API key for STT/TTS | - |
| `CARTESIA_API_KEY` | Cartesia API key for STT/TTS | - |
| `GOOGLE_APPLICATION_CREDENTIALS` | Path to Google Cloud service account JSON | - |
| `AZURE_SPEECH_SUBSCRIPTION_KEY` | Azure Speech Services subscription key | - |
| `AZURE_SPEECH_REGION` | Azure region (e.g., eastus, westeurope) | `eastus` |
| `LIVEKIT_URL` | LiveKit server WebSocket URL (internal) | - |
| `LIVEKIT_PUBLIC_URL` | LiveKit URL for clients | - |
| `LIVEKIT_API_KEY` | LiveKit API key | - |
| `LIVEKIT_API_SECRET` | LiveKit API secret | - |
| `AUTH_REQUIRED` | Enable authentication | `false` |
| `AUTH_SERVICE_URL` | External auth service endpoint | - |
| `AUTH_SIGNING_KEY_PATH` | Path to JWT signing private key | - |

### Using a Config File

Mount a YAML config file:

```bash
docker run --rm \
  -p 3001:3001 \
  -v $(pwd)/config.yaml:/app/config.yaml \
  saynaai/sayna:latest -c /app/config.yaml
```

Example `config.yaml`:

```yaml
server:
  host: "0.0.0.0"
  port: 3001

providers:
  deepgram_api_key: "your-deepgram-key"
  elevenlabs_api_key: "your-elevenlabs-key"

cache:
  path: "/data/cache"
  ttl_seconds: 2592000

livekit:
  url: "ws://livekit:7880"
  public_url: "https://rtc.example.com"
  api_key: "your-livekit-key"
  api_secret: "your-livekit-secret"
```

## API Endpoints

### REST

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Health check |
| `/voices` | GET | List available TTS voices |
| `/speak` | POST | Generate speech from text |
| `/livekit/token` | POST | Generate LiveKit participant token |
| `/livekit/webhook` | POST | LiveKit event webhook receiver |

### WebSocket

Connect to `/ws` for real-time voice processing.

**Configuration Message:**

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

**With LiveKit Integration:**

```json
{
  "type": "config",
  "stream_id": "session-123",
  "audio": true,
  "stt_config": { ... },
  "tts_config": { ... },
  "livekit": {
    "room_name": "support-room-42",
    "enable_recording": true,
    "sayna_participant_identity": "sayna-ai",
    "sayna_participant_name": "Sayna AI",
    "listen_participants": ["agent-1", "customer-42"]
  }
}
```

**Speak Command:**

```json
{
  "type": "speak",
  "text": "Hello! How can I help you today?",
  "flush": true,
  "allow_interruption": true
}
```

## Supported Providers

### STT Providers

| Provider | Model Examples |
|----------|---------------|
| Deepgram | `nova-2`, `nova-2-general` |
| Google | `latest_long`, `latest_short`, `chirp_2`, `telephony` |
| ElevenLabs | Default streaming model |
| Azure | Neural speech recognition |
| Cartesia | `ink-whisper` |

### TTS Providers

| Provider | Voice Examples |
|----------|---------------|
| Deepgram | `aura-asteria-en`, `aura-luna-en`, `aura-stella-en` |
| ElevenLabs | Custom voice IDs from your ElevenLabs account |
| Google | `en-US-Wavenet-D`, `en-US-Neural2-C`, `en-US-Studio-M` |
| Azure | `en-US-JennyNeural`, `en-US-GuyNeural`, `de-DE-ConradNeural` |
| Cartesia | `sonic-3`, `sonic-3-2025-10-27` |

## Volume Mounts

| Path | Purpose |
|------|---------|
| `/data/cache` | Persist cached TTS audio and turn-detection assets |
| `/app/config.yaml` | Optional configuration file |

## Health Check

```bash
curl http://localhost:3001/
# Returns: {"status":"ok"}
```

## Architecture Support

This image supports:
- `linux/amd64` (Intel/AMD)
- `linux/arm64` (Apple Silicon, ARM servers)

## Source Code

GitHub: [https://github.com/SaynaAI/sayna](https://github.com/SaynaAI/sayna)

## License

See the [GitHub repository](https://github.com/SaynaAI/sayna) for license information.
