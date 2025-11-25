# Google Cloud Speech-to-Text Integration

Sayna supports Google Cloud Speech-to-Text v2 API for real-time streaming speech recognition using gRPC bidirectional streaming.

## Prerequisites

1. A Google Cloud project with billing enabled
2. Speech-to-Text API enabled in your project
3. Service account credentials or Application Default Credentials (ADC) configured

## Authentication

Google STT supports three authentication methods, determined by the `api_key` field in the STT configuration:

### Option 1: Service Account JSON File (Recommended for Production)

1. Create a service account in Google Cloud Console
2. Grant the "Cloud Speech Client" role (`roles/speech.client`)
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

If running on Google Cloud infrastructure (GKE, Cloud Run, Compute Engine), ADC is configured automatically using the attached service account. The `project_id` is automatically extracted from the credentials.

For local development:
```bash
gcloud auth application-default login
```

**Note:** When using `gcloud auth application-default login`, the credentials file typically contains the `project_id` (see `~/.config/gcloud/application_default_credentials.json`). Alternatively, you can specify the project ID in the model field as `project_id:model_name`.

To use ADC, leave the `api_key` field empty in your WebSocket config:
```json
{
  "stt_config": {
    "provider": "google",
    "api_key": ""
  }
}
```

### Option 3: Inline JSON Credentials

You can pass the JSON content directly in the `api_key` field (useful for secrets management systems like Vault or AWS Secrets Manager):

```json
{
  "stt_config": {
    "provider": "google",
    "api_key": "{\"type\": \"service_account\", \"project_id\": \"...\", ...}"
  }
}
```

**Security Note:** Avoid logging or exposing the JSON content. The server validates JSON structure but never logs credential contents.

## Configuration

### WebSocket Config Message

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
  "tts_config": { ... }
}
```

### STT Configuration Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `provider` | string | Yes | - | Must be `"google"` |
| `api_key` | string | No | `""` | Credential source (see Authentication section) |
| `language` | string | Yes | - | BCP-47 language code (e.g., `"en-US"`, `"es-ES"`) |
| `sample_rate` | number | Yes | - | Audio sample rate in Hz |
| `channels` | number | Yes | - | Number of audio channels (1 = mono) |
| `punctuation` | boolean | Yes | - | Enable automatic punctuation |
| `encoding` | string | Yes | - | Audio encoding format |
| `model` | string | Yes | - | Google STT model to use |

### Supported Models

| Model | Description | Best For |
|-------|-------------|----------|
| `latest_long` | Optimized for long-form audio | Conversations, meetings, podcasts |
| `latest_short` | Optimized for short utterances | Voice commands, short queries |
| `chirp_2` | Multilingual Chirp model | Multiple languages, code-switching |
| `telephony` | Phone audio optimized | Call centers, telephony applications |
| `medical_dictation` | Medical terminology | Healthcare transcription |
| `medical_conversation` | Medical conversations | Doctor-patient interactions |

**Note:** Model availability varies by language and location. Check [Google's documentation](https://cloud.google.com/speech-to-text/v2/docs/speech-to-text-supported-languages) for current availability.

### Supported Audio Encodings

| Encoding | Description | Sample Rates |
|----------|-------------|--------------|
| `linear16` | 16-bit signed PCM (recommended) | Any |
| `flac` | FLAC lossless compression | Any |
| `mulaw` | G.711 mu-law | 8000 Hz |
| `alaw` | G.711 A-law | 8000 Hz |
| `ogg_opus` | Opus in Ogg container | 8000-48000 Hz |
| `webm_opus` | Opus in WebM container | 8000-48000 Hz |
| `amr` | Adaptive Multi-Rate | 8000 Hz |
| `amr_wb` | AMR Wideband | 16000 Hz |

### Supported Languages

Google STT v2 supports 100+ languages. Common examples:

| Code | Language |
|------|----------|
| `en-US` | English (US) |
| `en-GB` | English (UK) |
| `es-ES` | Spanish (Spain) |
| `es-MX` | Spanish (Mexico) |
| `fr-FR` | French |
| `de-DE` | German |
| `ja-JP` | Japanese |
| `zh-CN` | Chinese (Mandarin) |
| `pt-BR` | Portuguese (Brazil) |
| `ko-KR` | Korean |

See [full language list](https://cloud.google.com/speech-to-text/v2/docs/speech-to-text-supported-languages) for all supported codes.

## Response Format

Google STT responses are converted to Sayna's unified `STTResult` format:

```json
{
  "type": "stt_result",
  "transcript": "Hello, how are you today?",
  "is_final": true,
  "is_speech_final": true,
  "confidence": 0.95
}
```

### Understanding Result Fields

| Field | Description |
|-------|-------------|
| `transcript` | The transcribed text |
| `is_final` | `true` when transcript won't change for this segment |
| `is_speech_final` | `true` when speaker has finished (utterance end detected) |
| `confidence` | Recognition confidence score (0.0 to 1.0) |

### Speech Events

The Google provider maps Speech-to-Text v2 events to Sayna's unified format:

- **Interim Results**: `is_final=false`, transcript may change
- **Final Results**: `is_final=true`, transcript is stable
- **End of Utterance**: `is_speech_final=true`, maps to `END_OF_SINGLE_UTTERANCE` event

## Recognizers

Google Speech-to-Text v2 uses "Recognizers" to store reusable configuration. Sayna supports both implicit and explicit recognizers:

### Implicit Recognizer (Default)

When no `recognizer_id` is specified, Sayna uses the implicit recognizer (`_`). This is suitable for most use cases and requires no pre-configuration.

```json
{
  "stt_config": {
    "provider": "google",
    "model": "latest_long"
  }
}
```

### Explicit Recognizer

For advanced use cases (custom vocabularies, specific adaptation), you can create a recognizer in advance and reference it:

1. Create a recognizer via Google Cloud Console or API
2. Reference it in your configuration (server-side only)

Pre-created recognizers can:
- Improve cold-start latency
- Store custom vocabularies and phrases
- Enable model adaptation

## Advanced Configuration

### Voice Activity Detection (VAD)

Google STT includes built-in voice activity detection. The following events are supported:

- **Speech Start**: Detected when audio begins
- **Speech End**: Detected when speaker stops (silence threshold)

### Project and Location

The recognizer path is constructed as:
```
projects/{project_id}/locations/{location}/recognizers/{recognizer_id}
```

Default location is `global`. For data residency requirements, use regional locations:
- `us` - United States
- `eu` - European Union
- `asia-southeast1` - Singapore
- etc.

## Limits and Quotas

| Limit | Value |
|-------|-------|
| Maximum audio length | 480 minutes (streaming) |
| Streaming timeout | 5 minutes of inactivity |
| Maximum alternatives | 30 |
| Request size | 10 MB |

For rate limits and quotas, see [Google's quotas page](https://cloud.google.com/speech-to-text/quotas).

## Error Handling

### Common Errors

**Authentication Errors**
```
STTError::AuthenticationFailed: Invalid credentials
```
- Verify service account has correct permissions (`roles/speech.client`)
- Check JSON file path is correct and file exists
- Ensure project ID matches credentials

**Connection Errors**
```
STTError::ConnectionFailed: Connection timeout
```
- Check network connectivity to `speech.googleapis.com`
- Verify TLS certificates are valid
- Check firewall rules allow outbound gRPC (port 443)

**Configuration Errors**
```
STTError::ConfigurationError: Missing project_id
```
- Ensure the service account JSON file contains a `project_id` field
- Or specify the project ID in the model field: `"model": "your-project-id:latest_long"`

**Quota Errors**
```
STTError::ProviderError: RESOURCE_EXHAUSTED
```
- Check your project's quota usage in Google Cloud Console
- Request quota increase if needed

### Error Recovery

The Google STT provider implements automatic reconnection for transient errors:
- Network timeouts trigger reconnection
- Authentication failures require manual intervention
- Quota exhaustion requires waiting or quota increase

## Comparison with Deepgram

| Feature | Google STT | Deepgram |
|---------|-----------|----------|
| Protocol | gRPC | WebSocket |
| Streaming | Bidirectional | Bidirectional |
| Languages | 100+ | 30+ |
| Models | General + specialized | General + Nova |
| VAD | Built-in | Built-in |
| Custom vocabulary | Via recognizers | Via keywords |
| Latency | ~300-500ms | ~200-400ms |
| Pricing | Per audio minute | Per audio minute |

Choose Google STT when you need:
- Specific language support not available in Deepgram
- Integration with Google Cloud ecosystem
- Medical or telephony-specific models
- Data residency in specific regions

Choose Deepgram when you need:
- Lower latency
- Simpler authentication
- WebSocket-native protocol

## Example: Complete Configuration

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
    "provider": "deepgram",
    "model": "aura-asteria-en",
    "voice_id": "aura-asteria-en",
    "speaking_rate": 1.0,
    "audio_format": "linear16",
    "sample_rate": 24000
  },
  "livekit": {
    "room_name": "voice-room-123"
  }
}
```

## Troubleshooting

### No Transcription Results

1. **Check audio format**: Ensure binary audio matches `stt_config` (sample rate, channels, encoding)
2. **Verify credentials**: Test with `gcloud auth application-default print-access-token`
3. **Check project ID**: Must match the project where Speech API is enabled
4. **Enable API**: Ensure Speech-to-Text API is enabled in Google Cloud Console

### High Latency

1. **Use regional endpoint**: If in EU/Asia, use regional location instead of `global`
2. **Optimize model**: Use `latest_short` for command-like utterances
3. **Reduce chunk size**: Send smaller audio chunks (100-200ms)

### Garbled Transcription

1. **Sample rate mismatch**: Audio sample rate must exactly match configuration
2. **Encoding mismatch**: Ensure encoding matches actual audio format
3. **Channel mismatch**: Mono audio should have `channels: 1`

---

**Additional Resources:**
- [Google Cloud Speech-to-Text Documentation](https://cloud.google.com/speech-to-text/v2/docs)
- [Speech-to-Text Pricing](https://cloud.google.com/speech-to-text/pricing)
- [Supported Languages](https://cloud.google.com/speech-to-text/v2/docs/speech-to-text-supported-languages)
- [WebSocket API Reference](websocket.md)
