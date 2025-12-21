# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Sayna is a real-time voice processing server built in Rust that provides unified Speech-to-Text (STT) and Text-to-Speech (TTS) services through WebSocket and REST APIs. It integrates with LiveKit for real-time audio streaming and includes advanced noise filtering using DeepFilterNet.

## Development Commands

```bash
cargo run                    # Run development server
cargo run -- -c config.yaml  # Run with YAML config file
cargo test                   # Run all tests
cargo test test_name         # Run specific test
cargo build --release        # Build for release
cargo check                  # Check without building
cargo fmt                    # Format code
cargo clippy                 # Run linter
docker build -t saynaai/sayna .      # Build Docker image
```

### Feature Flags
By default, no optional features are enabled.

- `turn-detect` (disabled by default): ONNX-based turn detection. Required for `sayna init`.
- `noise-filter` (disabled by default): DeepFilterNet noise suppression. Disable to reduce dependencies.
- `openapi` (disabled by default): OpenAPI 3.1 specification generation using utoipa crate.

```bash
cargo check                                   # Default build, no optional features
cargo check --no-default-features             # Explicitly disable optional features
cargo build --features turn-detect,openapi     # Enable specific features
cargo run --features openapi -- openapi -o docs/openapi.yaml  # Generate OpenAPI spec
```

## High-Level Architecture

### Development Rules

The codebase includes detailed development rules in `.cursor/rules/`:
- **`rust.mdc`**: Rust best practices, design patterns, performance, security, testing
- **`core.mdc`**: STT/TTS provider abstractions (`BaseSTT`, `TTSProvider` traits)
- **`axum.mdc`**: Axum framework patterns for WebSocket and REST APIs
- **`livekit.mdc`**: LiveKit integration patterns and WebSocket API details
- **`openapi.mdc`**: OpenAPI 3.1 documentation guidelines using utoipa

Always consult these rule files when implementing new features.

### Core Components

1. **VoiceManager** (`src/core/voice_manager/`): Central coordinator for STT/TTS
   - Manages provider lifecycle and switching
   - Thread-safe with `Arc<RwLock<>>` for concurrent access
   - Handles callbacks for STT results and audio output

2. **Provider System** (`src/core/stt/` and `src/core/tts/`):
   - Trait-based abstraction for pluggable providers
   - **STT**: Deepgram, Google (gRPC), ElevenLabs, Microsoft Azure, Cartesia
   - **TTS**: Deepgram, ElevenLabs, Google, Microsoft Azure, Cartesia

3. **WebSocket Handler** (`src/handlers/ws/`):
   - Real-time bidirectional communication at `/ws`
   - Processes audio streams, config updates, control messages

4. **LiveKit Integration** (`src/livekit/`):
   - WebRTC audio streaming with room/participant management
   - Audio track subscription and data message forwarding

5. **DeepFilterNet** (`src/utils/noise_filter.rs`):
   - Advanced noise reduction with thread pool for CPU-intensive operations

6. **Authentication** (`src/auth/` and `src/middleware/auth.rs`):
   - Optional JWT-based auth with external validation service

### Request Flow

1. Client connects to `/ws` WebSocket endpoint
2. Client sends config with provider selection
3. Server sends `Ready` message with `stream_id` and LiveKit room info
4. Audio processing: Incoming audio → DeepFilterNet (optional) → STT → Text
5. TTS: Text → TTS Provider → Audio → Client
6. For LiveKit: Call `POST /livekit/token` to get participant token

### Key Design Patterns

- **Factory Pattern**: Provider creation through factory functions
- **Observer Pattern**: Callback registration for STT/TTS events
- **Singleton Pattern**: Lazy static for DeepFilterNet model
- **Actor Pattern**: Message passing for WebSocket communication

## Configuration

See [config.example.yaml](config.example.yaml) for all available options.

**Priority**: YAML File > Environment Variables > .env File > Defaults

Key environment variables:
- `DEEPGRAM_API_KEY`, `ELEVENLABS_API_KEY`, `CARTESIA_API_KEY`
- `GOOGLE_APPLICATION_CREDENTIALS` (path to service account JSON)
- `AZURE_SPEECH_SUBSCRIPTION_KEY`, `AZURE_SPEECH_REGION`
- `LIVEKIT_URL`, `LIVEKIT_API_KEY`, `LIVEKIT_API_SECRET`
- `AUTH_REQUIRED`, `AUTH_SERVICE_URL`, `AUTH_SIGNING_KEY_PATH`

## Adding New Providers

### STT Providers

1. Implement `BaseSTT` trait in `src/core/stt/`
2. Create provider-specific config struct
3. Add factory function following existing pattern
4. Register in `src/core/stt/mod.rs` (enum + factory)
5. Add tests

### TTS Providers

1. Implement `TTSProvider` trait in `src/core/tts/`
2. Add factory function
3. Update `VoiceManager` configuration support
4. Add tests

See existing implementations for patterns:
- WebSocket-based: Deepgram, ElevenLabs, Cartesia, Azure
- gRPC-based: Google

## API Endpoints

### REST
- `GET /` - Health check (public)
- `GET /voices` - List TTS voices
- `POST /speak` - Generate speech from text
- `POST /livekit/token` - Generate LiveKit participant token
- `GET /recording/{stream_id}` - Download recording from S3
- `GET/POST /sip/hooks` - Manage SIP webhook hooks

### WebSocket
- `/ws` - Real-time voice processing
  - Receives: Config, audio data, speak commands
  - Sends: Ready, STT results, TTS audio

### Webhooks
- `POST /livekit/webhook` - LiveKit event receiver (signature verified)

See [docs/](docs/) for detailed API documentation.

## Critical Files

- `src/core/voice_manager/manager.rs`: Central voice processing orchestration
- `src/handlers/ws/handler.rs`: WebSocket message handling
- `src/livekit/manager.rs`: LiveKit room management
- `src/config/mod.rs`: Configuration loading and merging
- `src/errors/mod.rs`: Centralized error types (thiserror)
- `src/docs/openapi.rs`: OpenAPI spec generation (feature-gated)

## Testing

```bash
cargo test                          # All tests
cargo test test_name                # Specific test
cargo test -- --nocapture           # With output
```

- Unit tests: Embedded in modules using `#[cfg(test)]`
- Integration tests: In `/tests/` directory
- Use `#[tokio::test]` for async tests

## OpenAPI Documentation

```bash
# Generate YAML spec
cargo run --features openapi -- openapi -o docs/openapi.yaml

# Generate JSON spec
cargo run --features openapi -- openapi --format json -o docs/openapi.json
```

See `.cursor/rules/openapi.mdc` for annotation guidelines.
