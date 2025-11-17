# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Sayna is a real-time voice processing server built in Rust that provides unified Speech-to-Text (STT) and Text-to-Speech (TTS) services through WebSocket and REST APIs. It integrates with LiveKit for real-time audio streaming and includes advanced noise filtering using DeepFilterNet.

## Development Commands

```bash
# Run the development server
cargo run

# Run all tests
cargo test

# Run a specific test
cargo test test_name

# Build for release
cargo build --release

# Check code without building
cargo check

# Format code
cargo fmt

# Run linter
cargo clippy

# Build Docker image
docker build -t sayna .

# Run with environment variables
cargo run
```

### Feature Flags
- `turn-detect` (default): ONNX-based turn detection. Required for `sayna init`.
- `noise-filter` (default): DeepFilterNet noise suppression. Disable to reduce dependencies.
- `openapi`: OpenAPI 3.1 specification generation and documentation endpoints using utoipa crate.

Use Cargo features to enable or disable these at compile time, e.g. `cargo check --no-default-features` or `cargo build --features turn-detect,openapi`.

## High-Level Architecture

### Project-Specific Rules and Guidelines

The codebase includes detailed development rules in `.cursor/rules/`:
- **`rust.mdc`**: Comprehensive Rust best practices including code organization, design patterns, performance optimization, security, and testing strategies
- **`core.mdc`**: Business logic specifications for STT/TTS provider abstractions and unified API design
- **`axum.mdc`**: Axum framework best practices for WebSocket and REST API development
- **`livekit.mdc`**: LiveKit integration patterns and WebSocket API implementation details
- **`openapi.mdc`**: OpenAPI 3.1 documentation guidelines using utoipa crate, including schema annotations, path documentation, and spec generation

Always consult these rule files when implementing new features or modifying existing code to ensure consistency with established patterns.


### Core Components

1. **VoiceManager** (`src/core/voice_manager.rs`): Central coordinator for STT/TTS operations
   - Manages provider lifecycle and switching
   - Implements speech final timing control with fallback mechanisms
   - Thread-safe with Arc<RwLock<>> for concurrent access
   - Handles callbacks for STT results and audio output

2. **Provider System** (`src/core/stt/` and `src/core/tts/`):
   - Trait-based abstraction for pluggable providers
   - Factory pattern for provider instantiation
   - Current providers: Deepgram (STT/TTS), ElevenLabs (TTS)
   - Providers implement `STTProvider` or `TTSProvider` traits

3. **WebSocket Handler** (`src/handlers/ws.rs`):
   - Real-time bidirectional communication endpoint
   - Processes audio streams, configuration updates, and control messages
   - Integrates with LiveKit for room-based audio
   - Unified message handling for different data sources

4. **LiveKit Integration** (`src/livekit/`):
   - WebRTC audio streaming with room/participant management
   - Audio track subscription and processing
   - Data message forwarding between participants
   - Handles connection lifecycle and error recovery

5. **DeepFilterNet** (`src/utils/noise_filter.rs`):
   - Advanced noise reduction with adaptive processing
   - Thread pool for CPU-intensive operations
   - Conservative blending to preserve speech quality
   - Lazy static initialization for model loading

6. **Authentication System** (`src/auth/` and `src/middleware/auth.rs`):
   - Optional JWT-based authentication with external validation service
   - AuthClient for communicating with auth service
   - JWT signing for request integrity and tamper prevention
   - Middleware for protecting API endpoints
   - Configurable via environment variables

### Request Flow

1. **WebSocket Connection**: Client connects to `/ws` endpoint
2. **Configuration**: Client sends config with provider selection and parameters
3. **LiveKit Token Generation** (if using LiveKit): After receiving Ready message with room info, call `POST /livekit/token` to get participant token
4. **Audio Processing**:
   - Incoming audio → DeepFilterNet (optional) → STT Provider → Text results
   - Text input → TTS Provider → Audio output → Client
5. **LiveKit Mode**: Audio streams from LiveKit rooms processed in real-time

### Key Design Patterns

- **Factory Pattern**: Provider creation through factory functions
- **Observer Pattern**: Callback registration for STT/TTS events  
- **Singleton Pattern**: Lazy static for DeepFilterNet model
- **Actor Pattern**: Message passing for WebSocket communication
- **Repository Pattern**: State management with AppState

## Configuration

Sayna supports two configuration methods:

### YAML Configuration File (Recommended)

Use a YAML configuration file for cleaner, more maintainable configuration:

```bash
# Start server with YAML config
sayna -c config.yaml
```

See [config.example.yaml](config.example.yaml) for all available options. The file uses logical prefixes:
- `server`: Server settings (host, port)
- `livekit`: LiveKit integration
- `providers`: Provider API keys (Deepgram, ElevenLabs)
- `recording`: S3 recording configuration
- `cache`: Cache settings
- `auth`: Authentication configuration

### Environment Variables

All configuration options can also be set via environment variables. When using a YAML file, environment variables **override** YAML values, allowing flexible deployment configurations.

Required for production:
- `DEEPGRAM_API_KEY`: Deepgram API authentication
- `ELEVENLABS_API_KEY`: ElevenLabs API authentication
- `LIVEKIT_URL`: LiveKit server WebSocket URL (default: ws://localhost:7880)
- `HOST`: Server bind address (default: 0.0.0.0)
- `PORT`: Server port (default: 3001)

Optional authentication:
- `AUTH_REQUIRED`: Enable authentication (default: false, accepts: true/false/1/0/yes/no)
- `AUTH_SERVICE_URL`: External auth service endpoint (required if auth enabled)
- `AUTH_SIGNING_KEY_PATH`: Path to JWT signing private key (required if auth enabled)
- `AUTH_API_SECRET`: API secret for simple token-based auth (alternative to JWT)
- `AUTH_TIMEOUT_SECONDS`: Auth request timeout in seconds (default: 5)

**Configuration Priority**: Environment Variables > YAML File > Defaults

## Testing Strategy

- **Unit Tests**: Embedded in modules, run with `cargo test`
- **Integration Tests**: In `/tests/` directory, test API endpoints and WebSocket
- **Provider Tests**: Mock external APIs to test provider implementations
- **Performance Tests**: Audio processing benchmarks in noise filter module

When adding new features:
1. Add unit tests in the same file using `#[cfg(test)]` module
2. For API changes, update integration tests in `/tests/`
3. Test error cases and edge conditions explicitly
4. Use `#[tokio::test]` for async test functions

## Critical Files and Their Purposes

- `src/core/voice_manager.rs`: Central orchestration of voice processing
- `src/handlers/ws.rs`: WebSocket message handling and routing
- `src/handlers/livekit.rs`: LiveKit token generation REST endpoint
- `src/livekit/livekit_manager.rs`: LiveKit room and participant management
- `src/livekit/room_handler.rs`: LiveKit room creation and JWT token generation
- `src/utils/noise_filter.rs`: DeepFilterNet integration and audio processing
- `src/config/`: Modular configuration system
  - `mod.rs`: ServerConfig struct and public API
  - `yaml.rs`: YAML file loading
  - `env.rs`: Environment variable loading
  - `merge.rs`: Configuration merging logic
  - `validation.rs`: Configuration validation
- `src/errors/mod.rs`: Centralized error types using thiserror
- `src/docs/openapi.rs`: OpenAPI 3.1 specification and documentation (feature-gated)

## Adding New Providers

1. Implement the `STTProvider` or `TTSProvider` trait in `src/core/stt/` or `src/core/tts/`
2. Add factory function following existing pattern (e.g., `create_deepgram_stt`)
3. Update `VoiceManager` to support the new provider in configuration
4. Add provider-specific configuration to `WebSocketMessage::Config`
5. Update tests to cover new provider functionality

## API Endpoints

### REST API

- `GET /` - Health check endpoint (public, no auth required)
- `GET /voices` - List available TTS voices (requires auth if AUTH_REQUIRED=true)
- `POST /speak` - Generate speech from text (requires auth if AUTH_REQUIRED=true)
- `POST /livekit/token` - Generate LiveKit participant token (requires auth if AUTH_REQUIRED=true)
  - Request: `{"room_name": "room-123", "participant_name": "User", "participant_identity": "user-id"}`
  - Response: `{"token": "JWT...", "room_name": "room-123", "participant_identity": "user-id", "livekit_url": "ws://..."}`

**Authentication**: When `AUTH_REQUIRED=true`, protected endpoints require a valid `Authorization: Bearer {token}` header. See [docs/authentication.md](docs/authentication.md) for setup details.

### WebSocket API

- `/ws` - Main WebSocket endpoint for real-time voice processing
  - Receives: Config, audio data, speak commands, control messages
  - Sends: Ready (with LiveKit room info), STT results, TTS audio, unified messages

**Breaking Change (2025-10-17)**: The WebSocket Ready message no longer includes `livekit_token`. Instead, it provides `livekit_room_name`, `sayna_participant_identity`, and `sayna_participant_name`. Clients must call the `/livekit/token` endpoint to obtain participant tokens.

**Configuration Note**: The `sayna_participant_identity` and `sayna_participant_name` fields can be customized in the LiveKit config during WebSocket initialization. Defaults are "sayna-ai" and "Sayna AI" respectively.

#### LiveKit Configuration Options

The LiveKit configuration in the WebSocket config message supports the following fields:

```json
{
  "livekit": {
    "room_name": "my-room",
    "enable_recording": false,
    "recording_file_key": "optional-file-key",
    "sayna_participant_identity": "sayna-ai",
    "sayna_participant_name": "Sayna AI",
    "listen_participants": ["user-123", "user-456"]
  }
}
```

**Participant Filtering** (`listen_participants`):
- **Empty array (default)**: Processes audio tracks and data messages from **all participants** in the room
- **Populated array**: Only processes audio/data from participants whose identities are in the list
- Useful for selective audio processing in multi-participant rooms where you only want to handle specific users
- Filtering applies to both audio tracks (STT) and data messages
- Server-side messages (participant = None) are always processed regardless of the filter

## Performance Considerations

- DeepFilterNet processing is CPU-intensive; uses thread pool to avoid blocking
- Audio buffers are processed in chunks to maintain low latency
- Provider connections are reused when possible
- WebSocket messages are processed asynchronously to prevent blocking
- Memory is managed carefully in audio processing loops

## OpenAPI Documentation

When the `openapi` feature is enabled, you can generate the OpenAPI 3.1 specification:

```bash
# Generate YAML spec to file
cargo run --features openapi -- openapi -o docs/openapi.yaml

# Generate JSON spec
cargo run --features openapi -- openapi --format json -o docs/openapi.json
```

### Adding OpenAPI Annotations

When creating new REST endpoints or types, follow the guidelines in `.cursor/rules/openapi.mdc`:

1. Add `#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]` to all request/response types
2. Add `#[cfg_attr(feature = "openapi", utoipa::path(...))]` to handler functions
3. Provide examples for all fields using `#[cfg_attr(feature = "openapi", schema(example = "..."))]`
4. Register all types in `src/docs/openapi.rs` under `components(schemas(...))`
5. Register all handlers in `src/docs/openapi.rs` under `paths(...)`
6. Regenerate spec: `cargo run --features openapi -- openapi -o docs/openapi.yaml`

See `.cursor/rules/openapi.mdc` for comprehensive guidelines and examples.