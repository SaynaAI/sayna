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

- `stt-vad` (disabled by default): Silero-VAD voice activity detection with integrated ONNX-based turn detection. When enabled, VAD monitors audio for silence and triggers the turn detection model to confirm if the speaker's turn is complete.
- `noise-filter` (disabled by default): DeepFilterNet noise suppression. Disable to reduce dependencies.
- `openapi` (disabled by default): OpenAPI 3.1 specification generation using utoipa crate.

```bash
cargo check                                        # Default build, no optional features
cargo check --no-default-features                  # Explicitly disable optional features
cargo build --features stt-vad                     # Enable VAD with turn detection
cargo build --features stt-vad,openapi             # Enable specific features
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

7. **VAD + Turn Detection** (`src/core/vad/`, `src/core/turn_detect/`) - Feature-gated: `stt-vad`:
   - **SileroVAD**: ONNX-based voice activity detection model
   - **SilenceTracker**: Tracks continuous silence duration from VAD
   - **Turn Detection**: ONNX-based model that confirms turn completion when silence is detected
   - Integrates with VoiceManager for audio processing
   - VAD and turn detection are always bundled together under `stt-vad` feature

### Voice Activity Detection (VAD) with Turn Detection

When `stt-vad` feature is enabled, Sayna provides both Silero-VAD for audio-level silence detection and smart-turn v3 model for semantic turn detection. Both features are always bundled together under the `stt-vad` feature flag.

**Why Use Both VAD and Smart-Turn?**

VAD (Silero) provides:
- Fast, efficient silence detection (~2ms per frame)
- Low latency feedback on speech activity
- Immediate signal when user stops speaking

Smart-Turn provides:
- Semantic understanding of turn completion
- Higher accuracy (>90% for most languages)
- Context-aware decision making

**Combined Pattern**: VAD detects silence first (fast, cheap), then smart-turn confirms if the turn is semantically complete (more accurate, slightly slower). This two-stage approach provides both speed and accuracy.

**How it works:**
```
Audio In -> VAD -> SilenceTracker -> TurnEnd Event -> Smart-Turn -> speech_final
                         ^                               |
                         |                               |
                         +-- If false, reset and wait ---+
```

1. Audio frames are processed through Silero-VAD ONNX model
2. When configurable silence threshold (default: 200ms per PipeCat recommendation) is detected, the smart-turn model is triggered
3. The smart-turn model analyzes the accumulated audio to confirm if the turn is complete
4. If confirmed, an artificial `speech_final` event is emitted
5. If smart-turn returns false (turn incomplete), the system resets and waits for the next silence detection

**Smart-Turn Model:**
- Model: [pipecat-ai/smart-turn-v3](https://huggingface.co/pipecat-ai/smart-turn-v3)
- Version: `smart-turn-v3.2-cpu.onnx` (quantized int8)
- Input: Up to 8 seconds of 16kHz mono PCM audio (minimum 0.5 seconds required)
- Input Format: Mel spectrogram (1, 80, 800) via Whisper-style preprocessing
- Output: Probability that the speaker has finished their turn (0.0-1.0)
- Architecture: Whisper Tiny encoder + linear classifier (~8M parameters)
- Performance: Feature extraction ~10-30ms + model inference ~12-20ms = ~50ms total (release build)

**Filler Sound Handling:**
- Smart-Turn v2/v3 is explicitly trained on filler words ("um", "mmm", "ehhh")
- The model recognizes these as "user still thinking" patterns
- Audio buffer is preserved across speech pauses for full context analysis
- Minimum 0.5 seconds of audio required before turn detection runs

**Mel Spectrogram Parameters (Whisper Standard):**
- Sample Rate: 16000 Hz
- FFT Size: 400
- Hop Length: 160
- Mel Bins: 80
- Max Frames: 800 (8 seconds)

**Configuration:**
```yaml
vad:
  threshold: 0.5              # Speech probability threshold for VAD (0.0-1.0)
  silence_duration_ms: 200    # Silence duration to trigger turn detection (PipeCat: stop_secs=0.2)
  min_speech_duration_ms: 250 # Minimum speech before checking silence (filters filler sounds)

turn_detect:
  threshold: 0.6              # Turn completion probability threshold (0.0-1.0), increased for robustness
  num_threads: 4              # ONNX inference threads (default: 4)
  # Advanced options (rarely need changing):
  # model_path: /path/to/model.onnx  # Override model location
  # model_url: https://...           # Override download URL
  # use_quantized: true              # Use int8 quantized model (default: true)
```

> **Note:** When `stt-vad` is compiled, VAD and turn detection are always active. There is no runtime `enabled` toggle.

**WebSocket API:**
```json
{
  "type": "config",
  "vad": {
    "silence_duration_ms": 200
  }
}
```

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
  - Receives: Config, audio data, speak commands, **clear command**, send_message, sip_transfer
  - Sends: Ready, STT results, TTS audio, messages, participant events
  - **Clear command**: Immediately stops TTS and clears audio buffers (fire-and-forget, respects `allow_interruption` setting)

### Webhooks
- `POST /livekit/webhook` - LiveKit event receiver (signature verified)

See [docs/](docs/) for detailed API documentation.

## Critical Files

- `src/core/voice_manager/manager.rs`: Central voice processing orchestration
- `src/core/voice_manager/stt_result.rs`: STT result processing with VAD integration
- `src/core/vad/detector.rs`: Silero-VAD detector (feature-gated: `stt-vad`)
- `src/core/vad/silence_tracker.rs`: Silence duration tracking for turn detection
- `src/core/turn_detect/assets.rs`: Model download and hash verification for smart-turn (feature-gated: `stt-vad`)
- `src/core/turn_detect/config.rs`: Configuration and constants for smart-turn detection
- `src/core/turn_detect/detector.rs`: Smart-turn audio-based turn detection (feature-gated: `stt-vad`)
- `src/core/turn_detect/feature_extractor.rs`: Mel spectrogram extraction for smart-turn (feature-gated: `stt-vad`)
- `src/core/turn_detect/model_manager.rs`: ONNX model management for smart-turn (feature-gated: `stt-vad`)
- `src/handlers/ws/handler.rs`: WebSocket message handling
- `src/livekit/manager.rs`: LiveKit room management
- `src/config/mod.rs`: Configuration loading and merging
- `src/errors/mod.rs`: Centralized error types (thiserror)
- `src/docs/openapi.rs`: OpenAPI spec generation (feature-gated: `openapi`)

## Testing

```bash
cargo test                          # All tests
cargo test test_name                # Specific test
cargo test -- --nocapture           # With output
cargo test --features stt-vad       # Tests including VAD feature tests
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
