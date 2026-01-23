# Voice Manager

The Voice Manager module provides a unified interface for managing both Speech-to-Text (STT) and Text-to-Speech (TTS) providers in a single, coordinated system. It abstracts away the complexity of managing multiple providers and provides a clean API for real-time voice processing.

## Features

- **Unified Management**: Coordinate STT and TTS providers through a single interface
- **Real-time Processing**: Optimized for low-latency voice processing
- **Speech Final Detection**: Two complementary approaches based on feature flags:
  - With `stt-vad`: VAD detects silence → Smart-Turn ML model confirms turn completion
  - Without `stt-vad`: STT provider's native `is_speech_final` signal is used directly
  - Turn detection inference timeout is configurable (default: 800ms)
- **Error Handling**: Comprehensive error handling with proper error propagation
- **Callback System**: Event-driven architecture for handling results
- **Thread Safety**: Safe concurrent access using `Arc<RwLock<>>`
- **Provider Abstraction**: Support for multiple STT and TTS providers

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                        VoiceManager                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────────┐      ┌──────────────┐      ┌──────────────┐  │
│  │  STT Manager │      │  TTS Manager │      │  Callbacks   │  │
│  │              │      │              │      │              │  │
│  │  - Provider  │      │  - Provider  │      │  - on_stt    │  │
│  │  - State     │      │  - Queue     │      │  - on_tts    │  │
│  │  - Config    │      │  - State     │      │  - on_error  │  │
│  └──────┬───────┘      └──────┬───────┘      └──────────────┘  │
│         │                     │                                 │
│         ▼                     ▼                                 │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                   Audio Processing Pipeline               │  │
│  │                                                          │  │
│  │   Audio In ──► [Noise Filter] ──► [VAD] ──► STT ──► Text │  │
│  │                                                          │  │
│  │   Text ──► TTS ──► Audio Out                             │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

## Component Structure

### VoiceManagerConfig

Configuration container that holds:
- **STT Configuration**: Provider selection, API keys, language, sample rate, encoding
- **TTS Configuration**: Provider selection, voice ID, speaking rate, audio format
- **Speech Final Config**: Turn detection timeout, duplicate prevention window

### Core Components

| Component | Location | Responsibility |
|-----------|----------|----------------|
| `VoiceManager` | `manager/` | Main coordinator, lifecycle management |
| `VoiceManagerConfig` | `config.rs` | Configuration management |
| `SpeechFinalConfig` | `config.rs` | Turn detection timing settings |
| `STTResult` | `stt_result/` | STT result processing and emission |
| `VadProcessor` | `vad_processor/` | VAD integration (feature-gated) |
| `TurnDetectionTasks` | `turn_detection_tasks/` | Smart-turn ML inference |
| `Callbacks` | `callbacks.rs` | Event callback registration |

## Data Flow

### Audio Input Flow (STT)

```
1. Client sends audio data
         │
         ▼
2. receive_audio() called
         │
         ▼
3. [Optional] Noise filtering (DeepFilterNet)
         │
         ▼
4. [Optional] VAD processing (with stt-vad feature)
         │
         ▼
5. Audio forwarded to STT provider
         │
         ▼
6. STT provider returns transcription
         │
         ▼
7. Speech final detection:
   ├─ With stt-vad: VAD silence → Smart-Turn confirmation
   └─ Without stt-vad: Provider's is_speech_final signal
         │
         ▼
8. Callbacks invoked with STTResult
```

### Audio Output Flow (TTS)

```
1. speak() called with text
         │
         ▼
2. Text queued in TTS manager
         │
         ▼
3. TTS provider synthesizes audio
         │
         ▼
4. Audio chunks streamed back
         │
         ▼
5. on_tts_audio callbacks invoked
```

## Speech Final Detection

The Voice Manager supports two modes for detecting when a user has finished speaking:

### Mode 1: Provider-based (default)

When `stt-vad` feature is disabled, the system relies on the STT provider's native `is_speech_final` signal. This is simpler but depends on provider-specific behavior.

### Mode 2: VAD + Smart-Turn (with `stt-vad` feature)

A two-stage detection approach:

```
Audio Stream
     │
     ▼
┌─────────────┐
│  Silero VAD │  ← Detects speech/silence (~2ms per frame)
└──────┬──────┘
       │
       ▼ (silence detected)
┌─────────────┐
│ Smart-Turn  │  ← Confirms turn completion (~50ms)
│   Model     │
└──────┬──────┘
       │
       ├─ true  → Emit speech_final event
       │
       └─ false → Reset and continue listening
```

### SpeechFinalConfig Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `turn_detection_inference_timeout_ms` | 800 | Max time to wait for Smart-Turn inference |
| `duplicate_window_ms` | 1000 | Window to prevent duplicate final events |

## Lifecycle Management

### Startup Sequence

1. **Create** - `VoiceManager::new(config, callbacks)`
2. **Start** - `voice_manager.start().await`
3. **Wait for Ready** - Poll `is_ready()` or wait for both providers
4. **Process** - Send/receive audio

### Shutdown Sequence

1. **Stop** - `voice_manager.stop().await`
2. Providers disconnect gracefully
3. Pending callbacks complete
4. Resources released

## Callback System

The Voice Manager uses an event-driven callback system:

| Callback | Trigger | Data |
|----------|---------|------|
| `on_stt_result` | Transcription received | Transcript, confidence, is_final flag |
| `on_tts_audio` | Audio chunk synthesized | Audio bytes, format info |
| `on_tts_error` | TTS error occurred | Error details |

All callbacks are async and execute in the context of the Voice Manager's runtime.

## Thread Safety

The Voice Manager is designed for concurrent access:

- Main state protected by `Arc<RwLock<>>`
- Provider instances are independently thread-safe
- Callbacks execute without holding locks
- Audio processing is lock-free where possible

## Provider Status

Methods to check provider readiness:

| Method | Description |
|--------|-------------|
| `is_ready()` | Both STT and TTS are ready |
| `is_stt_ready()` | STT provider connected and ready |
| `is_tts_ready()` | TTS provider connected and ready |
| `get_stt_provider_info()` | Current STT provider name/details |
| `get_tts_provider_info()` | Current TTS provider name/details |

## Related Documentation

- [VAD + Turn Detection](vad.md) - Details on voice activity detection and turn detection
- [WebSocket API](websocket.md) - Real-time communication protocol
- [API Reference](api-reference.md) - Complete API documentation
