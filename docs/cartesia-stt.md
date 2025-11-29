# Cartesia Speech-to-Text Integration

Sayna supports Cartesia Speech-to-Text for real-time streaming speech recognition using WebSocket bidirectional streaming.

## Prerequisites

1. A Cartesia account
2. An API key from the Cartesia dashboard

## Authentication

Cartesia STT uses API key-based authentication via query parameters.

### Getting Credentials

1. Go to [Cartesia Dashboard](https://play.cartesia.ai/)
2. Sign up or log in
3. Navigate to API Keys section
4. Create a new API key
5. Copy the key

### Environment Variables

```bash
export CARTESIA_API_KEY=your-api-key
```

Or in `config.yaml`:
```yaml
providers:
  cartesia_api_key: "your-api-key"
```

## Configuration

### WebSocket Config Message

```json
{
  "type": "config",
  "audio": true,
  "stt_config": {
    "provider": "cartesia",
    "api_key": "",
    "language": "en",
    "sample_rate": 16000,
    "channels": 1,
    "punctuation": true,
    "encoding": "pcm_s16le",
    "model": "ink-whisper"
  },
  "tts_config": { ... }
}
```

**Note:** The `api_key` field can be left empty when credentials are configured via environment variables or YAML config. The server injects the API key automatically.

### STT Configuration Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `provider` | string | Yes | - | Must be `"cartesia"` |
| `api_key` | string | No | `""` | API key (injected from server config if empty) |
| `language` | string | Yes | `"en"` | ISO-639-1 language code |
| `sample_rate` | number | Yes | `16000` | Audio sample rate in Hz |
| `channels` | number | Yes | `1` | Number of audio channels (must be 1) |
| `encoding` | string | Yes | `"pcm_s16le"` | Audio encoding (must be pcm_s16le) |
| `model` | string | No | `"ink-whisper"` | STT model (currently only ink-whisper) |

## Supported Audio Formats

| Encoding | Description | Sample Rates |
|----------|-------------|--------------|
| `pcm_s16le` | PCM signed 16-bit little-endian | 8000, 16000, 22050, 24000, 44100, 48000 Hz |

**Important:** Cartesia only supports PCM S16LE format. Audio is sent as raw binary WebSocket frames, NOT base64 encoded. This differs from some other providers that expect base64-encoded audio.

## Supported Languages

Cartesia supports ISO-639-1 language codes:

| Code | Language |
|------|----------|
| `en` | English |
| `zh` | Chinese |
| `de` | German |
| `es` | Spanish |
| `fr` | French |
| `ja` | Japanese |
| `ko` | Korean |
| `pt` | Portuguese |
| `ru` | Russian |
| `tr` | Turkish |
| `pl` | Polish |
| `it` | Italian |
| `nl` | Dutch |
| `sv` | Swedish |
| `ar` | Arabic |
| `hi` | Hindi |

And many more ISO-639-1 codes.

## VAD Parameters

Optional Voice Activity Detection parameters can be configured:

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `min_volume` | float | - | Volume threshold (0.0-1.0) for speech detection. Audio below this level is treated as silence. |
| `max_silence_duration_secs` | float | - | Maximum silence duration in seconds before endpointing. When speech is followed by silence for this duration, the transcript is finalized. |

These parameters are passed as query parameters to the WebSocket URL.

## Response Format

Cartesia STT responses are converted to Sayna's unified `STTResult` format:

```json
{
  "type": "stt_result",
  "transcript": "Hello, how are you today?",
  "is_final": true,
  "is_speech_final": true,
  "confidence": 1.0
}
```

**Note:** Cartesia does not provide confidence scores. A default value of `1.0` is used for final results and `0.0` for interim results.

### Response Fields

| Field | Description |
|-------|-------------|
| `transcript` | The transcribed text |
| `is_final` | `true` when transcript won't change |
| `is_speech_final` | `true` when speaker has finished (same as is_final for Cartesia) |
| `confidence` | `1.0` for final results, `0.0` for interim (Cartesia doesn't provide confidence) |

### Cartesia Message Types

The underlying Cartesia WebSocket API uses the following message types:

| Type | Direction | Description |
|------|-----------|-------------|
| `transcript` | Server → Client | Transcription result (interim or final) |
| `flush_done` | Server → Client | Acknowledgment of finalize command |
| `done` | Server → Client | Session complete, connection closing |
| `error` | Server → Client | Error response |
| `"finalize"` | Client → Server | Signal end of audio stream |
| `"done"` | Client → Server | Signal end of session |

## Word Timestamps

Cartesia provides optional word-level timestamps in transcript responses:

```json
{
  "type": "transcript",
  "text": "Hello world",
  "is_final": true,
  "words": [
    {"word": "Hello", "start": 0.0, "end": 0.5},
    {"word": "world", "start": 0.6, "end": 1.2}
  ]
}
```

Word timestamp fields:

| Field | Type | Description |
|-------|------|-------------|
| `word` | string | The transcribed word |
| `start` | float | Start time in seconds from beginning of audio |
| `end` | float | End time in seconds from beginning of audio |

## Error Handling

### Common Errors

**Authentication Errors**
```
STTError::AuthenticationFailed: Cartesia API key is required
```
- Verify API key is correct
- Ensure key is set in environment or config

**Invalid Audio Format**
```
STTError::ConfigurationError: Unsupported sample rate
```
- Must use `pcm_s16le` encoding
- Sample rate must be one of: 8000, 16000, 22050, 24000, 44100, 48000 Hz

**Connection Errors**
```
STTError::ConnectionFailed: WebSocket connection failed
```
- Check network connectivity to `api.cartesia.ai`
- Verify firewall allows WebSocket connections

**Configuration Errors**
```
STTError::ConfigurationError: Language code is required
```
- Ensure language is specified in config

## Comparison with Other Providers

| Feature | Cartesia | Deepgram | Google | Azure |
|---------|----------|----------|--------|-------|
| Protocol | WebSocket | WebSocket | gRPC | WebSocket |
| Audio Format | Raw binary PCM | Raw binary PCM | PCM | PCM |
| Base64 Encoding | No | No | No | No |
| Model | ink-whisper | nova-3 | Various | Various |
| Confidence Score | No | Yes | Yes | Yes |
| Word Timestamps | Yes | Yes | Yes | Yes |
| Latency | ~200-400ms | ~200-400ms | ~300-500ms | ~300-500ms |

**Choose Cartesia when you need:**
- Low-latency streaming transcription
- Simple API with minimal configuration
- Integration with Cartesia TTS for voice AI applications
- ink-whisper model performance

## Example: Complete Configuration

```json
{
  "type": "config",
  "audio": true,
  "stt_config": {
    "provider": "cartesia",
    "api_key": "",
    "language": "en",
    "sample_rate": 16000,
    "channels": 1,
    "punctuation": true,
    "encoding": "pcm_s16le",
    "model": "ink-whisper"
  },
  "tts_config": {
    "provider": "cartesia",
    "model": "sonic-english",
    "voice_id": "your-voice-id",
    "speaking_rate": 1.0,
    "audio_format": "pcm_s16le",
    "sample_rate": 24000
  },
  "livekit": {
    "room_name": "voice-room-123"
  }
}
```

## Troubleshooting

### No Transcription Results

1. **Check audio format**: Audio must be PCM S16LE, sent as raw binary (not base64)
2. **Verify sample rate**: Must match one of the supported rates (8000, 16000, 22050, 24000, 44100, 48000)
3. **Check API key**: Ensure `CARTESIA_API_KEY` is set correctly
4. **Verify language code**: Must be a valid ISO-639-1 code

### Connection Issues

1. **Network access**: Verify connectivity to `wss://api.cartesia.ai`
2. **Firewall rules**: Ensure WebSocket connections are allowed on port 443
3. **API key validity**: Test key in Cartesia dashboard

### Garbled Transcription

1. **Sample rate mismatch**: Audio sample rate must exactly match configuration
2. **Encoding mismatch**: Must use PCM signed 16-bit little-endian
3. **Channel mismatch**: Must be mono (1 channel)

### High Latency

1. **Reduce chunk size**: Send smaller audio chunks (100-200ms)
2. **Check network**: Ensure stable connection to Cartesia servers
3. **Optimize audio pipeline**: Minimize buffering before sending

---

**Additional Resources:**
- [Cartesia Documentation](https://docs.cartesia.ai/)
- [Cartesia STT API Reference](https://docs.cartesia.ai/api-reference/stt/stt)
- [Cartesia Dashboard](https://play.cartesia.ai/)
- [WebSocket API Reference](websocket.md)
