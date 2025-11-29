# Cartesia Text-to-Speech Integration

Sayna supports Cartesia Text-to-Speech for high-quality speech synthesis using the Sonic voice models via REST API.

## Prerequisites

1. A Cartesia account with API access
2. API key from Cartesia dashboard

**Note:** Cartesia TTS uses the same API key as Cartesia STT. If you've already configured Cartesia STT, no additional credential setup is required.

## Authentication

Cartesia TTS uses API key-based authentication with Bearer token.

### Getting Credentials

1. Go to [Cartesia Dashboard](https://play.cartesia.ai/)
2. Sign up or log in
3. Navigate to API Keys section
4. Create a new API key
5. Copy the key

### Environment Variables

Set the following environment variable:

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
  "stt_config": { ... },
  "tts_config": {
    "provider": "cartesia",
    "voice_id": "a0e99841-438c-4a64-b679-ae501e7d6091",
    "model": "sonic-3",
    "audio_format": "linear16",
    "sample_rate": 24000,
    "speaking_rate": 1.0
  }
}
```

**Note:** The `api_key` field can be left empty when credentials are configured via environment variables or YAML config. The server injects the API key automatically.

### TTS Configuration Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `provider` | string | Yes | - | Must be `"cartesia"` |
| `voice_id` | string | Yes | - | Cartesia voice UUID |
| `model` | string | No | `"sonic-3"` | Sonic model version |
| `audio_format` | string | No | `"linear16"` | Output audio format |
| `sample_rate` | number | No | `24000` | Sample rate in Hz |
| `speaking_rate` | number | No | `1.0` | Speech speed (reserved for future use) |

## Voice Selection

### Voice IDs

Cartesia uses UUID-based voice identifiers. You can browse available voices at [Cartesia Voice Library](https://play.cartesia.ai/voices).

**Example Voice IDs:**
- `a0e99841-438c-4a64-b679-ae501e7d6091` - Barbershop Man
- `79a125e8-cd45-4c13-8a67-188112f4dd22` - British Lady
- `694f9389-aac1-45b6-b726-9d9369183238` - Confident British Man
- `87748186-23bb-4f21-9cb4-3f2f7e4e5c88` - Friendly Reading Man
- `f9836c6e-a0bd-460e-9d3c-f7299fa60f94` - Midwestern Woman
- `421b3369-f63f-4b03-8980-37a44df1d4e8` - Professional Woman

### Recommended Voices for Voice Agents

Cartesia recommends stable, realistic voices for voice agents:
- Look for voices tagged as "Stable" in the voice library
- For expressive applications, use voices tagged as "Emotive"
- For professional applications, use voices tagged as "Professional"

### Voice Cloning

Cartesia supports custom voice cloning. Create custom voices through the Cartesia dashboard and use their UUIDs in the `voice_id` field.

## Supported Models

| Model ID | Description | Latency | Quality |
|----------|-------------|---------|---------|
| `sonic-3` | Latest stable model (recommended) | Low | High |
| `sonic-3-2025-10-27` | Pinned version for consistency | Low | High |

### Model Selection

Use `sonic-3` for the latest improvements, or pin to a specific date version for production consistency:

```json
{
  "model": "sonic-3"
}
```

For consistent behavior across deployments, use a pinned version:

```json
{
  "model": "sonic-3-2025-10-27"
}
```

## Audio Formats

Cartesia TTS supports multiple audio output formats:

### Raw PCM Formats (Streaming)

| Format Config | Container | Encoding | Bytes/Sample | Use Case |
|---------------|-----------|----------|--------------|----------|
| `linear16` / `pcm` | raw | pcm_s16le | 2 | Real-time streaming (default) |
| `pcm_f32le` | raw | pcm_f32le | 4 | High-precision processing |

**Supported PCM Sample Rates:** 8000, 16000, 22050, 24000, 44100, 48000 Hz

### WAV Format

| Format Config | Container | Encoding | Use Case |
|---------------|-----------|----------|----------|
| `wav` | wav | pcm_s16le | File storage |

### Compressed Formats

| Format Config | Container | Use Case |
|---------------|-----------|----------|
| `mp3` | mp3 | Bandwidth optimization, storage |

**MP3 Default Bitrate:** 128 kbps

### Telephony Formats

| Format Config | Encoding | Use Case |
|---------------|----------|----------|
| `mulaw` / `ulaw` | pcm_mulaw | Phone systems (US/Japan) |
| `alaw` | pcm_alaw | Phone systems (Europe) |

### Sample Rate Recommendations

| Use Case | Recommended Sample Rate |
|----------|------------------------|
| Voice agents | 24000 Hz |
| Telephony | 8000 Hz |
| High quality | 44100 or 48000 Hz |
| Low latency | 16000 Hz |

## Usage Examples

### WebSocket Configuration

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
    "voice_id": "a0e99841-438c-4a64-b679-ae501e7d6091",
    "model": "sonic-3",
    "audio_format": "linear16",
    "sample_rate": 24000
  },
  "livekit": {
    "room_name": "voice-room-123"
  }
}
```

### REST API Example

```bash
curl -X POST http://localhost:3001/speak \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-token" \
  -d '{
    "text": "Hello! How can I help you today?",
    "provider": "cartesia",
    "voice_id": "a0e99841-438c-4a64-b679-ae501e7d6091",
    "audio_format": "linear16",
    "sample_rate": 24000
  }'
```

### Telephony Configuration

For phone system integration:

```json
{
  "tts_config": {
    "provider": "cartesia",
    "voice_id": "your-voice-uuid",
    "model": "sonic-3",
    "audio_format": "mulaw",
    "sample_rate": 8000
  }
}
```

### High-Quality Configuration

For maximum audio quality:

```json
{
  "tts_config": {
    "provider": "cartesia",
    "voice_id": "your-voice-uuid",
    "model": "sonic-3",
    "audio_format": "wav",
    "sample_rate": 48000
  }
}
```

## Language Support

Cartesia Sonic-3 supports 30+ languages using ISO-639-1 codes:

| Language | Code | Language | Code |
|----------|------|----------|------|
| English | `en` | French | `fr` |
| German | `de` | Spanish | `es` |
| Portuguese | `pt` | Chinese | `zh` |
| Japanese | `ja` | Hindi | `hi` |
| Italian | `it` | Korean | `ko` |
| Dutch | `nl` | Polish | `pl` |
| Russian | `ru` | Swedish | `sv` |
| Turkish | `tr` | Arabic | `ar` |
| Czech | `cs` | Danish | `da` |
| Greek | `el` | Finnish | `fi` |
| Hebrew | `he` | Hungarian | `hu` |
| Indonesian | `id` | Malay | `ms` |
| Norwegian | `no` | Romanian | `ro` |
| Slovak | `sk` | Thai | `th` |
| Ukrainian | `uk` | Vietnamese | `vi` |

**Note:** Language is typically inferred from the voice or defaults to English. The language code is included in the API request to help the model with pronunciation.

## Pronunciation Customization

Cartesia TTS supports pronunciation customization through Sayna's pronunciation replacement feature:

```json
{
  "tts_config": {
    "provider": "cartesia",
    "voice_id": "your-voice-uuid",
    "pronunciations": [
      { "word": "API", "pronunciation": "A P I" },
      { "word": "SDK", "pronunciation": "S D K" },
      { "word": "Sayna", "pronunciation": "Say-na" }
    ]
  }
}
```

Pronunciation patterns are compiled once when the provider is created and applied to all text before sending to the API.

## Error Handling

### Common Errors

**Authentication Errors (401 Unauthorized)**
```
TTSError::AuthenticationFailed: Invalid API key
```
- Verify API key is correct
- Check key hasn't been revoked in Cartesia dashboard

**Configuration Errors**
```
TTSError::ConfigurationError: Missing API key
```
- Ensure `CARTESIA_API_KEY` is set in environment or config

**Invalid Sample Rate**
```
TTSError::InvalidConfiguration: Unsupported sample rate
```
- Use one of the supported sample rates: 8000, 16000, 22050, 24000, 44100, 48000 Hz

**Voice Not Found**
```
TTSError::ProviderError: Voice not found
```
- Verify voice UUID is correct and exists in your Cartesia account
- Check [Cartesia Voice Library](https://play.cartesia.ai/voices)

**Connection Errors**
```
TTSError::ConnectionFailed: Connection timeout
```
- Check network connectivity to `api.cartesia.ai`
- Verify firewall allows HTTPS (port 443)

### Error Recovery

The Cartesia TTS provider implements automatic retry for transient errors:
- Network timeouts trigger reconnection attempts
- Rate limiting triggers exponential backoff
- The provider auto-reconnects when `speak()` is called after disconnection

## Performance Considerations

### Latency Optimization

1. **Use raw PCM format**: Raw PCM (`linear16`) has minimal encoding overhead
2. **Optimal sample rate**: 24kHz provides good quality with low latency
3. **Connection pooling**: Sayna reuses HTTP connections for multiple requests
4. **Enable caching**: Sayna caches TTS audio based on configuration hash

### Caching Behavior

Sayna caches TTS audio using a hash of:
- Provider (`cartesia`)
- Voice ID
- Model
- Output format (container, encoding, sample_rate)
- Speaking rate
- Text content

**Maximize cache hits by:**
- Keeping TTS configuration stable across sessions
- Using consistent speaking rates
- Reusing common phrases

### Connection Pooling

Cartesia TTS uses HTTP connection pooling for efficient request handling:

```json
{
  "tts_config": {
    "provider": "cartesia",
    "connection_timeout": 30,
    "request_timeout": 60,
    "request_pool_size": 4
  }
}
```

| Setting | Default | Description |
|---------|---------|-------------|
| `connection_timeout` | 30s | Timeout for establishing connection |
| `request_timeout` | 60s | Timeout for complete request |
| `request_pool_size` | 4 | Number of pooled connections |

### Typical Latencies

| Scenario | Latency |
|----------|---------|
| Cache hit | 50-150ms |
| PCM format (new) | 200-500ms |
| WAV format (new) | 250-550ms |
| MP3 format (new) | 300-600ms |

## API Details

### Request Format

Cartesia TTS uses a REST API at `POST https://api.cartesia.ai/tts/bytes`:

**Headers:**
| Header | Value | Purpose |
|--------|-------|---------|
| `Authorization` | `Bearer {api_key}` | Authentication |
| `Cartesia-Version` | `2025-04-16` | API version |
| `Content-Type` | `application/json` | Request body format |
| `Accept` | `application/octet-stream` | Response format |

**Body:**
```json
{
  "model_id": "sonic-3",
  "transcript": "Hello, world!",
  "voice": {
    "mode": "id",
    "id": "voice-uuid"
  },
  "output_format": {
    "container": "raw",
    "encoding": "pcm_s16le",
    "sample_rate": 24000
  },
  "language": "en"
}
```

### Response Format

The API returns raw audio bytes in the requested format. For streaming:
- Raw PCM: Direct audio bytes
- WAV: Audio with WAV header
- MP3: Compressed audio stream

## Comparison with Other Providers

| Feature | Cartesia | Deepgram | ElevenLabs | Azure | Google |
|---------|----------|----------|------------|-------|--------|
| Voice Count | 50+ | 10+ | 100+ | 400+ | 400+ |
| Voice Quality | High | Good | Excellent | High | High |
| Languages | 30+ | Limited | 30+ | 140+ | 60+ |
| Custom Voices | Yes | No | Yes (cloning) | Yes | No |
| Voice Cloning | Yes | No | Yes | Yes | No |
| Latency | Low | Low | Medium-High | Medium | Medium |
| SSML Support | No | Limited | No | Full | Full |
| Pricing Model | Per character | Per character | Per character | Per character | Per character |

**Choose Cartesia TTS when you need:**
- Low-latency synthesis for voice agents
- High-quality Sonic voice models
- Voice cloning capability
- Simple REST API integration
- Unified STT/TTS provider (with Cartesia STT)

## Pricing

Cartesia TTS pricing is based on characters synthesized. Visit [Cartesia Pricing](https://cartesia.ai/pricing) for current rates.

**Notes:**
- Free tier available for development
- Enterprise pricing available for high volume

## Troubleshooting

### No Audio Output

1. **Check API key**: Ensure `CARTESIA_API_KEY` is set correctly
2. **Verify voice ID**: Voice UUID must be valid and accessible
3. **Check sample rate**: Must be one of: 8000, 16000, 22050, 24000, 44100, 48000
4. **Verify format**: Audio format must be: linear16, pcm, wav, mp3, mulaw, or alaw

### Garbled Audio

1. **Sample rate mismatch**: Ensure playback sample rate matches configured rate
2. **Format mismatch**: Verify client expects the configured audio format
3. **Encoding mismatch**: For PCM, ensure client expects signed 16-bit little-endian

### High Latency

1. **Reduce text length**: Split long texts into smaller chunks
2. **Use PCM format**: Raw PCM has lowest encoding overhead
3. **Enable caching**: Reuse common phrases to leverage cache
4. **Check network**: Ensure stable connection to Cartesia servers

### Connection Issues

1. **Network access**: Verify connectivity to `https://api.cartesia.ai`
2. **Firewall rules**: Ensure HTTPS connections are allowed on port 443
3. **API key validity**: Test key in Cartesia dashboard
4. **Rate limits**: Check if you're exceeding API limits

---

**Additional Resources:**
- [Cartesia TTS API Documentation](https://docs.cartesia.ai/api-reference/tts/bytes)
- [Cartesia Voice Library](https://play.cartesia.ai/voices)
- [Cartesia Dashboard](https://play.cartesia.ai/)
- [Cartesia Pricing](https://cartesia.ai/pricing)
- [Cartesia STT Integration](cartesia-stt.md)
- [WebSocket API Reference](websocket.md)
