# Sayna

A high-performance real-time voice processing server built in Rust that provides unified Speech-to-Text (STT) and Text-to-Speech (TTS) services through WebSocket and REST APIs.

## Features

- **Unified Voice API**: Single interface for multiple STT/TTS providers
- **Real-time Processing**: WebSocket-based bidirectional audio streaming
- **LiveKit Integration**: WebRTC audio streaming with room-based communication
- **Advanced Noise Filtering**: Optional DeepFilterNet integration (`noise-filter` feature)
- **Provider Flexibility**: Pluggable architecture supporting multiple providers
  - Deepgram (STT/TTS)
  - ElevenLabs (TTS)
- **Audio-Disabled Mode**: Development mode without API keys

## Quick Start

### Prerequisites

- Rust 1.70+ and Cargo
- Optional: Deepgram API key (for STT/TTS)
- Optional: ElevenLabs API key (for TTS)
- Optional: LiveKit server (for WebRTC features)

### Installation

1. **Clone the repository**
```bash
git clone https://github.com/yourusername/sayna.git
cd sayna
```

2. **Set up environment variables**
```bash
cp .env.example .env
```

3. **Configure your `.env` file**
```env
# Optional - can run without these in audio-disabled mode
DEEPGRAM_API_KEY=your_deepgram_api_key_here
ELEVENLABS_API_KEY=your_elevenlabs_api_key_here

# Server configuration
PORT=3001
HOST=0.0.0.0

# Optional - for LiveKit integration
LIVEKIT_URL=ws://localhost:7880
```

4. **Run the server**
```bash
cargo run
```

The server will start on `http://localhost:3001`

## Feature Toggles

Sayna exposes several Cargo features that gate heavyweight subsystems or documentation features.

- `turn-detect` (default): ONNX-based speech turn detection and asset preparation
- `noise-filter` (default): DeepFilterNet noise suppression pipeline
- `openapi`: OpenAPI 3.1 specification generation and endpoints

Example commands:

```bash
# Run with default features
cargo run

# Run with turn detection
cargo run --features turn-detect

# Run with noise filter
cargo run --features noise-filter

# Run with OpenAPI documentation
cargo run --features openapi

# Run with multiple features
cargo run --features turn-detect,noise-filter,openapi
```

Features are compile-time only. Disable them when you need smaller builds or want to avoid optional dependencies.

`sayna init` downloads turn-detection assets when the `turn-detect` feature is enabled. Without that feature the command exits early with an explanatory error, and runtime logs mention the timer-based fallback.

## OpenAPI Documentation

Sayna provides machine-generated OpenAPI 3.1 specification for all REST endpoints and WebSocket message types. This feature is gated behind the `openapi` Cargo feature to keep production builds lean.

### Generating the OpenAPI Spec

The OpenAPI specification is automatically generated from Rust code annotations via CLI commands:

```bash
# Generate YAML to stdout (default)
cargo run --features openapi -- openapi

# Generate and save YAML to file
cargo run --features openapi -- openapi --output docs/openapi.yaml
# or use short form
cargo run --features openapi -- openapi -o docs/openapi.yaml

# Generate JSON format
cargo run --features openapi -- openapi --format json --output docs/openapi.json
# or use short form
cargo run --features openapi -- openapi -f json -o docs/openapi.json
```

**CLI Options:**
- `-f, --format <FORMAT>`: Output format (`yaml` or `json`). Default: `yaml`
- `-o, --output <FILE>`: Write to file instead of stdout

The generated spec can be used with:
- **Postman**: For API testing and collection generation
- **Swagger UI**: For interactive documentation
- **Redoc**: For modern API docs
- **OpenAPI Generators**: Generate client SDKs in various languages
- **API Documentation Tools**: Import into your documentation platform

### Running Without API Keys (Audio-Disabled Mode)

You can run Sayna without Deepgram or ElevenLabs API keys by using the audio-disabled mode. Simply start the server without configuring the API keys, then send a WebSocket configuration message with `audio_disabled: true`:

```json
{
  "type": "config",
  "config": {
    "audio_disabled": true,
    "stt_provider": "deepgram",
    "tts_provider": "elevenlabs"
  }
}
```

This mode is useful for:
- Local development and testing
- UI/UX development without audio processing
- Testing WebSocket message flows
- Debugging non-audio features

## Authentication (Optional)

Sayna supports customer-based authentication that delegates token validation to an external authentication service. When enabled, protected API endpoints require a valid bearer token.

### Enabling Authentication

Add to your `.env` file:
```env
AUTH_REQUIRED=true
AUTH_SERVICE_URL=https://your-auth-service.com/auth
AUTH_SIGNING_KEY_PATH=/path/to/auth_private_key.pem
AUTH_TIMEOUT_SECONDS=5
```

Generate signing keys:
```bash
# Generate RSA private key
openssl genrsa -out auth_private_key.pem 2048

# Extract public key (share with auth service)
openssl rsa -in auth_private_key.pem -pubout -out auth_public_key.pem
```

### Making Authenticated Requests

```bash
curl -X POST http://localhost:3001/speak \
  -H "Authorization: Bearer your-token-here" \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello world"}'
```

For complete authentication setup and architecture details, see [docs/authentication.md](docs/authentication.md).

## API Endpoints

### WebSocket

- **Endpoint**: `/ws`
- **Protocol**: WebSocket
- **Purpose**: Real-time bidirectional audio streaming and control

#### Message Types

**Configuration Message**:
```json
{
  "type": "config",
  "config": {
    "stt_provider": "deepgram",
    "tts_provider": "elevenlabs",
    "audio_disabled": false,
    "deepgram_model": "nova-2",
    "elevenlabs_voice_id": "voice_id_here"
  }
}
```

**Audio Input**: Binary audio data (16kHz, 16-bit PCM)

**Text Input**:
```json
{
  "type": "text",
  "text": "Convert this text to speech"
}
```

### REST API

- **Health Check**: `GET /` - Server health check endpoint
- **Voices**: `GET /voices` - List available TTS voices (requires auth if `AUTH_REQUIRED=true`)
- **Speak**: `POST /speak` - Generate speech from text (requires auth if `AUTH_REQUIRED=true`)
- **LiveKit Token**: `POST /livekit/token` - Generate LiveKit participant token (requires auth if `AUTH_REQUIRED=true`)
- **LiveKit Webhook**: `POST /livekit/webhook` - Webhook endpoint for LiveKit events (unauthenticated, uses LiveKit signature verification)
  - Called by LiveKit to deliver room and participant events
  - Validates requests using LiveKit's JWT signature mechanism
  - Logs SIP-related attributes for phone call troubleshooting
  - See [docs/livekit_webhook.md](docs/livekit_webhook.md) for details

## Architecture Overview

### Core Components

- **VoiceManager**: Central coordinator for all voice processing operations
- **Provider System**: Trait-based abstraction for pluggable STT/TTS providers
- **WebSocket Handler**: Real-time communication and message routing
- **LiveKit Integration**: WebRTC audio streaming and room management
- **DeepFilterNet**: Advanced noise reduction with adaptive processing

### Request Flow

1. Client establishes WebSocket connection to `/ws`
2. Client sends configuration with provider selection
3. Audio processing pipeline:
   - **STT**: Audio � Noise Filter (optional) � STT Provider � Text
   - **TTS**: Text � TTS Provider � Audio � Client
4. LiveKit mode enables room-based audio streaming

## Development

### Building

```bash
# Development build
cargo build

# Release build (optimized)
cargo build --release

# Check code without building
cargo check
```

### Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture
```

### Code Quality

```bash
# Format code
cargo fmt

# Run linter
cargo clippy

# Check for security vulnerabilities
cargo audit
```

### Docker

```bash
# Build Docker image
docker build -t sayna .

# Run container
docker run -p 3001:3001 --env-file .env sayna
```

## Configuration

### Environment Variables

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `DEEPGRAM_API_KEY` | Deepgram API authentication | - | No* |
| `ELEVENLABS_API_KEY` | ElevenLabs API authentication | - | No* |
| `LIVEKIT_URL` | LiveKit server WebSocket URL | `ws://localhost:7880` | No |
| `LIVEKIT_API_KEY` | LiveKit API key (for webhooks and token generation) | - | No*** |
| `LIVEKIT_API_SECRET` | LiveKit API secret (for webhooks and token generation) | - | No*** |
| `HOST` | Server bind address | `0.0.0.0` | No |
| `PORT` | Server port | `3001` | No |
| `AUTH_REQUIRED` | Enable authentication | `false` | No |
| `AUTH_SERVICE_URL` | External auth service endpoint | - | Yes** |
| `AUTH_SIGNING_KEY_PATH` | Path to JWT signing private key | - | Yes** |
| `AUTH_TIMEOUT_SECONDS` | Auth request timeout | `5` | No |

*Not required when using audio-disabled mode
**Required when `AUTH_REQUIRED=true`
***Required for LiveKit webhook validation and token generation features

### SIP Configuration (Optional)

Sayna supports first-class SIP configuration for managing SIP-specific settings. See [docs/sip_config.md](docs/sip_config.md) for detailed documentation.

**YAML Configuration:**
```yaml
sip:
  room_prefix: "sip-"
  allowed_addresses:
    - "192.168.1.0/24"
    - "10.0.0.1"
  hooks:
    - host: "example.com"
      url: "https://webhook.example.com/events"
```

**Environment Variables:**
- `SIP_ROOM_PREFIX`: Room name prefix for SIP calls (required if SIP enabled)
- `SIP_ALLOWED_ADDRESSES`: Comma-separated IP addresses/CIDRs
- `SIP_HOOKS_JSON`: JSON array of webhook configurations

## Performance Considerations

- **DeepFilterNet**: CPU-intensive processing uses thread pooling
- **Audio Buffering**: Optimized chunk processing for low latency
- **Connection Reuse**: Provider connections are maintained for efficiency
- **Async Processing**: Non-blocking WebSocket message handling
- **Memory Management**: Careful buffer management in audio loops

## Contributing

1. Review the development rules in `.cursor/rules/`:
   - `rust.mdc`: Rust best practices
   - `core.mdc`: Business logic specifications
   - `axum.mdc`: Framework patterns
   - `livekit.mdc`: LiveKit integration details

2. Follow the existing code patterns and conventions
3. Add tests for new features
4. Ensure `cargo fmt` and `cargo clippy` pass

## Support

For issues, questions, or contributions, please visit the [GitHub repository](https://github.com/SaynaAI/sayna).