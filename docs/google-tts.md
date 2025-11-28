# Google Cloud Text-to-Speech Integration

Sayna supports Google Cloud Text-to-Speech API for high-quality speech synthesis using WaveNet, Neural2, and Studio voice technologies.

## Prerequisites

1. A Google Cloud project with billing enabled
2. Text-to-Speech API enabled in your project
3. Service account credentials or Application Default Credentials (ADC) configured

**Note:** Google TTS uses the same credentials as Google STT. If you've already configured Google STT, no additional credential setup is required.

## Authentication

Google TTS supports three authentication methods, determined by the `google_credentials` configuration or `GOOGLE_APPLICATION_CREDENTIALS` environment variable:

### Option 1: Service Account JSON File (Recommended for Production)

1. Create a service account in Google Cloud Console
2. Grant the "Cloud Text-to-Speech Client" role
3. Download the JSON key file
4. Set the environment variable or provide the path in config:

```bash
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json
```

Or in `config.yaml`:
```yaml
providers:
  google_credentials: "/path/to/service-account.json"
```

**Note:** The `project_id` is automatically extracted from the service account JSON file. You don't need to set `GOOGLE_CLOUD_PROJECT` separately.

### Option 2: Application Default Credentials (ADC)

If running on Google Cloud infrastructure (GKE, Cloud Run, Compute Engine), ADC is configured automatically using the attached service account.

For local development:
```bash
gcloud auth application-default login
```

To use ADC, leave `google_credentials` empty:
```yaml
providers:
  google_credentials: ""
```

### Option 3: Inline JSON Credentials

You can pass the JSON content directly (useful for secrets management systems like Vault or AWS Secrets Manager):

```yaml
providers:
  google_credentials: '{"type": "service_account", "project_id": "...", ...}'
```

**Security Note:** Avoid logging or exposing the JSON content. The server validates JSON structure but never logs credential contents.

## Configuration

### WebSocket Config Message

```json
{
  "type": "config",
  "audio": true,
  "stt_config": { ... },
  "tts_config": {
    "provider": "google",
    "voice_id": "en-US-Wavenet-D",
    "audio_format": "linear16",
    "sample_rate": 24000,
    "speaking_rate": 1.0
  }
}
```

### TTS Configuration Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `provider` | string | Yes | - | Must be `"google"` |
| `voice_id` | string | Recommended | - | Voice name (see Voice Selection) |
| `audio_format` | string | No | `"linear16"` | Output audio format |
| `sample_rate` | number | No | `24000` | Sample rate in Hz |
| `speaking_rate` | number | No | `1.0` | Speech speed (0.25 to 4.0) |
| `model` | string | No | - | Not used for Google TTS |

## Voice Selection

### Voice Naming Convention

Voice names follow the pattern: `{language}-{region}-{type}-{variant}`

**Examples:**
- `en-US-Wavenet-D` - English US, WaveNet technology, voice variant D
- `en-GB-Neural2-A` - British English, Neural2 technology, voice variant A
- `es-ES-Standard-B` - Spanish Spain, Standard technology, voice variant B
- `de-DE-Studio-B` - German, Studio technology, voice variant B

### Voice Types

Voice types are ordered by quality and price:

| Type | Description | Best For |
|------|-------------|----------|
| **Standard** | Basic quality, fast synthesis | High-volume, cost-sensitive |
| **WaveNet** | High quality, natural prosody | General use, good quality |
| **Neural2** | Improved WaveNet, more natural | Premium quality |
| **Studio** | Highest quality, limited languages | Premium applications |

### Popular Voice Examples

| Voice ID | Language | Gender | Type |
|----------|----------|--------|------|
| `en-US-Wavenet-D` | English (US) | Male | WaveNet |
| `en-US-Wavenet-F` | English (US) | Female | WaveNet |
| `en-US-Neural2-A` | English (US) | Male | Neural2 |
| `en-US-Neural2-C` | English (US) | Female | Neural2 |
| `en-GB-Wavenet-A` | English (UK) | Male | WaveNet |
| `en-GB-Neural2-A` | English (UK) | Female | Neural2 |
| `es-ES-Wavenet-B` | Spanish (Spain) | Male | WaveNet |
| `es-US-Wavenet-A` | Spanish (US) | Female | WaveNet |
| `fr-FR-Wavenet-A` | French | Female | WaveNet |
| `de-DE-Wavenet-B` | German | Male | WaveNet |
| `ja-JP-Wavenet-A` | Japanese | Female | WaveNet |
| `zh-CN-Wavenet-A` | Chinese (Mandarin) | Female | WaveNet |

For a complete list of available voices, see [Google's Voice List](https://cloud.google.com/text-to-speech/docs/voices).

## Audio Formats

| Format | Config Value | Description | Use Case |
|--------|--------------|-------------|----------|
| LINEAR16 | `"linear16"` | 16-bit PCM, uncompressed | Real-time audio, highest quality |
| MP3 | `"mp3"` | Compressed audio | Storage, bandwidth-limited |
| OGG_OPUS | `"ogg_opus"` | Efficient compression | Web playback, streaming |
| MULAW | `"mulaw"` | 8-bit mu-law | Phone systems (US) |
| ALAW | `"alaw"` | 8-bit A-law | Phone systems (Europe) |

### Sample Rate Recommendations

| Format | Recommended Sample Rates |
|--------|-------------------------|
| LINEAR16 | 8000, 16000, 22050, 24000, 32000, 44100, 48000 |
| MP3 | 8000, 16000, 22050, 24000, 32000, 44100, 48000 |
| OGG_OPUS | 8000, 16000, 24000, 48000 |
| MULAW | 8000 |
| ALAW | 8000 |

## Usage Examples

### WebSocket Configuration

```json
{
  "type": "config",
  "audio": true,
  "stt_config": {
    "provider": "google",
    "api_key": "",
    "language": "en-US",
    "sample_rate": 16000,
    "channels": 1,
    "punctuation": true,
    "encoding": "linear16",
    "model": "latest_long"
  },
  "tts_config": {
    "provider": "google",
    "voice_id": "en-US-Wavenet-D",
    "audio_format": "linear16",
    "sample_rate": 24000,
    "speaking_rate": 1.0
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
    "provider": "google",
    "voice_id": "en-US-Wavenet-D",
    "audio_format": "linear16",
    "sample_rate": 24000
  }'
```

### Speaking Rate Examples

```json
// Slower speech (good for clarity)
{
  "provider": "google",
  "voice_id": "en-US-Neural2-A",
  "speaking_rate": 0.8
}

// Normal speech
{
  "provider": "google",
  "voice_id": "en-US-Neural2-A",
  "speaking_rate": 1.0
}

// Faster speech
{
  "provider": "google",
  "voice_id": "en-US-Neural2-A",
  "speaking_rate": 1.2
}
```

### Telephony Configuration

For phone system integration:

```json
{
  "tts_config": {
    "provider": "google",
    "voice_id": "en-US-Wavenet-D",
    "audio_format": "mulaw",
    "sample_rate": 8000,
    "speaking_rate": 1.0
  }
}
```

## Error Handling

### Common Errors

| Error | Cause | Solution |
|-------|-------|----------|
| `Google credentials are required` | No credentials configured | Set `GOOGLE_APPLICATION_CREDENTIALS` or `google_credentials` config |
| `Could not extract project_id` | Invalid credentials format | Verify JSON file structure contains `project_id` |
| `Authentication failed` | Expired or invalid credentials | Regenerate service account key |
| `Invalid voice name` | Voice doesn't exist | Check [voice list](https://cloud.google.com/text-to-speech/docs/voices) |
| `Quota exceeded` | API quota limit reached | Check billing and quotas in Cloud Console |
| `Permission denied` | Missing Text-to-Speech API access | Grant "Cloud Text-to-Speech Client" role |

### Error Recovery

The Google TTS provider implements automatic retry for transient errors:
- Network timeouts trigger reconnection
- Rate limiting triggers exponential backoff
- Authentication failures require manual intervention

## Performance Considerations

### Latency Optimization

1. **Voice Type Trade-off**: Standard voices are fastest, Studio voices are slowest
2. **Audio Format**: LINEAR16 has minimal encoding overhead
3. **Sample Rate**: Lower sample rates are slightly faster to generate
4. **Caching**: Sayna caches TTS audio based on configuration hash

### Caching Behavior

Sayna caches TTS audio using a hash of:
- Provider (`google`)
- Voice ID
- Audio format
- Sample rate
- Speaking rate
- Text content

**Maximize cache hits by:**
- Keeping TTS configuration stable across sessions
- Using consistent speaking rates
- Reusing common phrases

### Typical Latencies

| Scenario | Latency |
|----------|---------|
| Cache hit | 50-150ms |
| Standard voice | 300-600ms |
| WaveNet voice | 400-800ms |
| Neural2 voice | 500-1000ms |
| Studio voice | 600-1200ms |

## Pricing

Google Cloud Text-to-Speech pricing is based on characters synthesized:

| Voice Type | Price per 1M characters |
|------------|------------------------|
| Standard | $4.00 |
| WaveNet | $16.00 |
| Neural2 | $16.00 |
| Studio | Contact Google |

**Notes:**
- Prices as of 2024; check [Google's pricing page](https://cloud.google.com/text-to-speech/pricing) for current rates
- Free tier: 1 million characters/month for Standard, 1 million for WaveNet/Neural2
- SSML markup characters are counted

## Comparison with Other Providers

| Feature | Google TTS | Deepgram | ElevenLabs |
|---------|-----------|----------|------------|
| Voice Count | 400+ | 10+ | 100+ |
| Voice Quality | High | Good | Excellent |
| Languages | 60+ | Limited | 30+ |
| Custom Voices | No | No | Yes (cloning) |
| SSML Support | Full | Limited | No |
| Latency | Medium | Low | Medium-High |
| Pricing Model | Per character | Per character | Per character |

**Choose Google TTS when you need:**
- Wide language coverage
- Integration with Google Cloud ecosystem
- SSML support for fine-grained control
- Consistent enterprise-grade quality

## Advanced Features

### SSML Support

Google TTS supports Speech Synthesis Markup Language (SSML) for fine-grained control:

```json
{
  "type": "speak",
  "text": "<speak>Hello <break time='500ms'/> world!</speak>"
}
```

**Common SSML tags:**
- `<break>` - Add pauses
- `<emphasis>` - Emphasize words
- `<prosody>` - Control pitch, rate, volume
- `<say-as>` - Interpret as date, time, currency, etc.

See [Google's SSML Reference](https://cloud.google.com/text-to-speech/docs/ssml) for full documentation.

---

**Additional Resources:**
- [Google Cloud Text-to-Speech Documentation](https://cloud.google.com/text-to-speech/docs)
- [Voice List](https://cloud.google.com/text-to-speech/docs/voices)
- [SSML Reference](https://cloud.google.com/text-to-speech/docs/ssml)
- [Pricing](https://cloud.google.com/text-to-speech/pricing)
- [Google STT Integration](google-stt.md)
- [WebSocket API Reference](websocket.md)
