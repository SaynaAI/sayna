# Voice Activity Detection (VAD) API Reference

Sayna supports Silero-VAD for audio-level voice activity detection to determine when a user has stopped speaking. This feature is gated behind the `stt-vad` Cargo feature.

## Overview

VAD provides an additional signal for turn detection by monitoring audio for silence. When continuous silence exceeds a configurable threshold, it triggers the text-based turn detection model to confirm if the speaker's turn is complete.

### How It Works

1. Audio frames are processed through the Silero-VAD ONNX model (32ms frames at 16kHz)
2. Each frame returns a speech probability (0.0 to 1.0)
3. The SilenceTracker monitors for continuous silence after speech
4. When silence exceeds the threshold (default: 300ms), turn detection is triggered
5. If the turn detection model confirms the turn is complete, a `speech_final` event is emitted

## Enabling VAD

### Build with Feature

```bash
# Build with VAD support
cargo build --features stt-vad

# Build with both turn detection and VAD
cargo build --features turn-detect,stt-vad

# Run with VAD
cargo run --features stt-vad
```

### Server Configuration (YAML)

```yaml
vad:
  enabled: true
  threshold: 0.5              # Speech probability threshold (0.0-1.0)
  silence_duration_ms: 300    # Silence duration to trigger turn end
  min_speech_duration_ms: 100 # Minimum speech before checking silence
  # model_path: /path/to/silero_vad.onnx  # Optional: custom model path
```

### WebSocket Configuration

Configure VAD via the WebSocket config message:

```json
{
  "type": "config",
  "vad": {
    "enabled": true,
    "threshold": 0.5,
    "silence_duration_ms": 300,
    "min_speech_duration_ms": 100
  }
}
```

## Configuration Options

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | `false` | Enable VAD-based silence detection |
| `threshold` | float | `0.5` | Speech probability threshold (0.0-1.0). Audio with probability above this is considered speech. |
| `silence_duration_ms` | int | `300` | Silence duration in milliseconds to trigger turn end. This is the primary control for turn-taking responsiveness. |
| `min_speech_duration_ms` | int | `100` | Minimum speech duration (ms) before checking for silence. Prevents false triggers on brief pauses. |
| `model_path` | string | `null` | Optional path to custom Silero-VAD ONNX model. |
| `model_url` | string | HuggingFace URL | URL to download the model if not available locally. |

## VAD Events

The VAD system emits internal events that are used by the turn detection system:

| Event | Description |
|-------|-------------|
| `SpeechStart` | Speech detected after silence - beginning of a potential utterance |
| `SilenceDetected` | Speech transitioned to silence (not yet exceeding threshold) |
| `SpeechResumed` | Speech resumed before silence threshold was exceeded |
| `TurnEnd` | Silence exceeded threshold after sufficient speech |

These events are used internally to trigger turn detection. The `TurnEnd` event specifically triggers the text-based turn detection model to confirm if the user's turn is complete.

## Integration with Turn Detection

VAD works alongside the text-based turn detection system:

```
Audio Stream → Silero-VAD → SilenceTracker → Turn Detection Model → speech_final
                   ↓              ↓
             Speech Prob    VAD Events
```

1. **VAD monitors audio**: Silero-VAD processes each audio frame
2. **SilenceTracker tracks state**: Monitors speech/silence transitions
3. **Silence triggers evaluation**: When `TurnEnd` event fires, turn detection model is invoked
4. **Model confirms turn**: Text-based model analyzes accumulated transcript
5. **speech_final emitted**: If turn is complete, artificial `speech_final` event is sent

### Why Both VAD and Text-Based Detection?

- **VAD alone** can trigger on natural pauses mid-sentence
- **Text-based detection alone** may wait too long if the STT provider doesn't send `speech_final`
- **Combined approach** uses audio silence as a trigger, but text content as confirmation

## Performance Considerations

- **Latency**: VAD adds minimal latency (~1-2ms per 32ms frame)
- **CPU Usage**: Silero-VAD is lightweight and uses a single thread
- **Memory**: Model is loaded once and shared across sessions
- **Accuracy**: Default threshold of 0.5 provides good balance of sensitivity and false positive rejection

## Tuning Guidelines

### For Fast-Paced Conversations

```json
{
  "vad": {
    "enabled": true,
    "silence_duration_ms": 200,
    "threshold": 0.4
  }
}
```

### For Thoughtful/Slow Speakers

```json
{
  "vad": {
    "enabled": true,
    "silence_duration_ms": 500,
    "threshold": 0.5
  }
}
```

### For Noisy Environments

```json
{
  "vad": {
    "enabled": true,
    "threshold": 0.6,
    "min_speech_duration_ms": 150
  }
}
```

## Troubleshooting

### VAD Not Detecting Speech

- Check that `enabled: true` is set
- Lower the `threshold` value (e.g., 0.3)
- Ensure audio is 16kHz mono PCM format

### Too Many False Turn Ends

- Increase `silence_duration_ms` (e.g., 400-500ms)
- Increase `min_speech_duration_ms` to require more speech
- Increase `threshold` to require clearer speech signal

### Turn Detection Too Slow

- Decrease `silence_duration_ms` (minimum recommended: 150ms)
- This trades off against more mid-sentence triggers

## Related Documentation

- [WebSocket API](websocket.md) - Real-time audio streaming
- [Turn Detection](../CLAUDE.md#voice-activity-detection-vad) - Text-based turn detection
