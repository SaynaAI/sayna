# Microsoft Azure Text-to-Speech Integration

Sayna supports Microsoft Azure Cognitive Services Text-to-Speech for high-quality neural speech synthesis using the REST API.

## Prerequisites

1. An Azure subscription with billing enabled
2. A Speech resource created in Azure Portal
3. Subscription key and region from the Speech resource

**Note:** Azure TTS uses the same credentials as Azure STT. If you've already configured Azure STT, no additional credential setup is required.

## Authentication

Azure TTS uses subscription key-based authentication. The subscription key is tied to a specific Azure region.

### Getting Credentials

1. Go to [Azure Portal](https://portal.azure.com)
2. Create or navigate to a Speech resource
3. Go to **Keys and Endpoint**
4. Copy **Key 1** or **Key 2** (either works)
5. Note the **Region** (e.g., `eastus`, `westeurope`)

**Important:** The subscription key is region-specific. Using a key with the wrong region will result in 401 Unauthorized errors.

### Environment Variables

Set the following environment variables:

```bash
export AZURE_SPEECH_SUBSCRIPTION_KEY=your-subscription-key
export AZURE_SPEECH_REGION=eastus
```

Or in `config.yaml`:
```yaml
providers:
  azure_speech_subscription_key: "your-subscription-key"
  azure_speech_region: "eastus"
```

## Configuration

### WebSocket Config Message

```json
{
  "type": "config",
  "audio": true,
  "stt_config": { ... },
  "tts_config": {
    "provider": "azure",
    "voice_id": "en-US-JennyNeural",
    "audio_format": "linear16",
    "sample_rate": 24000,
    "speaking_rate": 1.0
  }
}
```

**Note:** The `api_key` field can be left empty when credentials are configured via environment variables or YAML config. The server injects the subscription key automatically.

### TTS Configuration Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `provider` | string | Yes | - | Must be `"azure"` or `"microsoft-azure"` |
| `voice_id` | string | Recommended | `"en-US-JennyNeural"` | Azure voice name |
| `audio_format` | string | No | `"linear16"` | Output audio format |
| `sample_rate` | number | No | `24000` | Sample rate in Hz |
| `speaking_rate` | number | No | `1.0` | Speech speed (0.5 to 2.0) |
| `model` | string | No | - | Alternative to voice_id (falls back if voice_id empty) |

## Voice Selection

### Voice Naming Convention

Azure voice names follow the pattern: `{language}-{region}-{name}Neural`

**Examples:**
- `en-US-JennyNeural` - English US, Jenny voice (female)
- `en-US-GuyNeural` - English US, Guy voice (male)
- `en-GB-SoniaNeural` - British English, Sonia voice (female)
- `de-DE-ConradNeural` - German, Conrad voice (male)
- `fr-FR-DeniseNeural` - French, Denise voice (female)
- `es-ES-ElviraNeural` - Spanish (Spain), Elvira voice (female)
- `ja-JP-NanamiNeural` - Japanese, Nanami voice (female)
- `zh-CN-XiaoxiaoNeural` - Chinese (Mandarin), Xiaoxiao voice (female)

### Popular Voice Examples

| Voice ID | Language | Gender | Style |
|----------|----------|--------|-------|
| `en-US-JennyNeural` | English (US) | Female | Conversational |
| `en-US-GuyNeural` | English (US) | Male | Conversational |
| `en-US-AriaNeural` | English (US) | Female | Professional |
| `en-US-DavisNeural` | English (US) | Male | Professional |
| `en-GB-SoniaNeural` | English (UK) | Female | Conversational |
| `en-GB-RyanNeural` | English (UK) | Male | Conversational |
| `de-DE-KatjaNeural` | German | Female | Conversational |
| `de-DE-ConradNeural` | German | Male | Conversational |
| `fr-FR-DeniseNeural` | French | Female | Conversational |
| `fr-FR-HenriNeural` | French | Male | Conversational |
| `es-ES-ElviraNeural` | Spanish (Spain) | Female | Conversational |
| `es-MX-DaliaNeural` | Spanish (Mexico) | Female | Conversational |
| `ja-JP-NanamiNeural` | Japanese | Female | Conversational |
| `zh-CN-XiaoxiaoNeural` | Chinese (Mandarin) | Female | Conversational |

For a complete list of available voices, see [Azure Voice Gallery](https://speech.microsoft.com/portal/voicegallery).

## Supported Regions

Azure Text-to-Speech is available in numerous regions worldwide. Choose the region closest to your users for optimal latency:

### Americas
| Region | Identifier |
|--------|------------|
| East US (Virginia) | `eastus` |
| East US 2 (Virginia) | `eastus2` |
| West US (California) | `westus` |
| West US 2 (Washington) | `westus2` |
| West US 3 (Arizona) | `westus3` |
| Central US (Iowa) | `centralus` |
| North Central US (Illinois) | `northcentralus` |
| South Central US (Texas) | `southcentralus` |
| Canada Central (Toronto) | `canadacentral` |
| Brazil South (Sao Paulo) | `brazilsouth` |

### Europe
| Region | Identifier |
|--------|------------|
| West Europe (Netherlands) | `westeurope` |
| North Europe (Ireland) | `northeurope` |
| UK South (London) | `uksouth` |
| France Central (Paris) | `francecentral` |
| Germany West Central (Frankfurt) | `germanywestcentral` |
| Switzerland North (Zurich) | `switzerlandnorth` |

### Asia Pacific
| Region | Identifier |
|--------|------------|
| East Asia (Hong Kong) | `eastasia` |
| Southeast Asia (Singapore) | `southeastasia` |
| Japan East (Tokyo) | `japaneast` |
| Japan West (Osaka) | `japanwest` |
| Korea Central (Seoul) | `koreacentral` |
| Australia East (Sydney) | `australiaeast` |
| India Central (Pune) | `centralindia` |

See [Azure regions documentation](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/regions) for the complete list.

## Audio Formats

Azure TTS supports various audio output formats:

### Raw PCM Formats (Streaming)

| Format Config | Azure Format | Sample Rate | Use Case |
|---------------|--------------|-------------|----------|
| `linear16` / `pcm` | `raw-24khz-16bit-mono-pcm` | 8-48kHz | Real-time streaming, highest quality |

**Supported PCM Sample Rates:** 8000, 16000, 22050, 24000, 44100, 48000 Hz

### Compressed Formats

| Format Config | Azure Format | Sample Rate | Use Case |
|---------------|--------------|-------------|----------|
| `mp3` | `audio-24khz-96kbitrate-mono-mp3` | 16-48kHz | Storage, bandwidth-limited |
| `opus` | `audio-24khz-16bit-48kbps-mono-opus` | 16-24kHz | Low-latency streaming |

### Telephony Formats

| Format Config | Azure Format | Sample Rate | Use Case |
|---------------|--------------|-------------|----------|
| `mulaw` / `ulaw` | `raw-8khz-8bit-mono-mulaw` | 8000 | Phone systems (US) |
| `alaw` | `raw-8khz-8bit-mono-alaw` | 8000 | Phone systems (Europe) |

### Sample Rate Recommendations

| Format | Recommended Sample Rates |
|--------|-------------------------|
| PCM | 24000 (default), 16000, 48000 |
| MP3 | 24000, 48000 |
| Opus | 24000 |
| μ-law/A-law | 8000 |

## Usage Examples

### WebSocket Configuration

```json
{
  "type": "config",
  "audio": true,
  "stt_config": {
    "provider": "microsoft-azure",
    "api_key": "",
    "language": "en-US",
    "sample_rate": 16000,
    "channels": 1,
    "punctuation": true,
    "encoding": "linear16",
    "model": ""
  },
  "tts_config": {
    "provider": "azure",
    "voice_id": "en-US-JennyNeural",
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
    "provider": "azure",
    "voice_id": "en-US-JennyNeural",
    "audio_format": "linear16",
    "sample_rate": 24000
  }'
```

### Speaking Rate Examples

```json
// Slower speech (good for clarity)
{
  "provider": "azure",
  "voice_id": "en-US-JennyNeural",
  "speaking_rate": 0.8
}

// Normal speech
{
  "provider": "azure",
  "voice_id": "en-US-JennyNeural",
  "speaking_rate": 1.0
}

// Faster speech
{
  "provider": "azure",
  "voice_id": "en-US-JennyNeural",
  "speaking_rate": 1.2
}
```

### Telephony Configuration

For phone system integration:

```json
{
  "tts_config": {
    "provider": "azure",
    "voice_id": "en-US-JennyNeural",
    "audio_format": "mulaw",
    "sample_rate": 8000,
    "speaking_rate": 1.0
  }
}
```

## SSML Support

Azure TTS supports Speech Synthesis Markup Language (SSML) for fine-grained control over speech synthesis. Sayna automatically wraps plain text in SSML with the configured voice and speaking rate.

**SSML Generated by Sayna:**
```xml
<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>
    <voice name='en-US-JennyNeural'>
        <prosody rate="120%">Your text here</prosody>
    </voice>
</speak>
```

**Features:**
- Automatic XML character escaping for special characters (`&`, `<`, `>`, `"`, `'`)
- Speaking rate conversion to percentage format
- Language code extraction from voice name
- Prosody element only added when speaking rate differs from 1.0

## Error Handling

### Common Errors

**Authentication Errors (401 Unauthorized)**
```
TTSError::AuthenticationFailed: Invalid subscription key or region mismatch
```
- Verify subscription key is correct
- Ensure region matches where the Speech resource was created
- Check if the key has expired or been regenerated

**Connection Errors**
```
TTSError::ConnectionFailed: Connection timeout
```
- Check network connectivity to `{region}.tts.speech.microsoft.com`
- Verify TLS certificates are valid
- Check firewall rules allow outbound HTTPS (port 443)

**Configuration Errors**
```
TTSError::ConfigurationError: Missing subscription key
```
- Ensure `AZURE_SPEECH_SUBSCRIPTION_KEY` is set
- Or provide the key in the server configuration

**Voice Not Found**
```
TTSError::ProviderError: Voice not found
```
- Verify the voice name is correct and available in your region
- Check [Azure Voice Gallery](https://speech.microsoft.com/portal/voicegallery)

### Error Recovery

The Azure TTS provider implements automatic retry for transient errors:
- Network timeouts trigger reconnection attempts
- Authentication failures require manual intervention
- Rate limiting triggers exponential backoff

## Performance Considerations

### Latency Optimization

1. **Choose regional endpoint**: Select the region closest to your users
2. **Use PCM format**: Raw PCM has minimal encoding overhead
3. **Optimal sample rate**: 24kHz provides good quality with low latency
4. **Enable caching**: Sayna caches TTS audio based on configuration hash

### Caching Behavior

Sayna caches TTS audio using a hash of:
- Provider (`azure`)
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
| PCM format (new) | 400-800ms |
| MP3 format (new) | 500-1000ms |

## Limits and Quotas

| Limit | Value |
|-------|-------|
| Maximum text length | 10,000 characters per request |
| Concurrent requests | Based on pricing tier |
| Audio output | Up to 10 minutes per request |

For rate limits and quotas, see [Azure quotas and limits](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/speech-services-quotas-and-limits).

## Pricing

Azure Text-to-Speech pricing is based on characters synthesized:

| Voice Type | Price per 1M characters |
|------------|------------------------|
| Neural voices | $15.00 |

**Notes:**
- Prices as of 2024; check [Azure's pricing page](https://azure.microsoft.com/en-us/pricing/details/cognitive-services/speech-services/) for current rates
- Free tier: 500,000 characters/month for neural voices
- SSML markup characters are not counted

## Comparison with Other Providers

| Feature | Azure TTS | Google TTS | Deepgram | ElevenLabs |
|---------|-----------|-----------|----------|------------|
| Voice Count | 400+ | 400+ | 10+ | 100+ |
| Voice Quality | High | High | Good | Excellent |
| Languages | 140+ | 60+ | Limited | 30+ |
| Custom Voices | Yes (Custom Neural) | No | No | Yes (cloning) |
| SSML Support | Full | Full | Limited | No |
| Latency | Medium | Medium | Low | Medium-High |
| Regional Endpoints | Yes | No | No | Yes |
| Pricing Model | Per character | Per character | Per character | Per character |

**Choose Azure TTS when you need:**
- Integration with Azure ecosystem
- Wide language coverage (140+ languages)
- Regional endpoint selection for compliance or latency
- Custom Neural Voice support
- SSML for fine-grained speech control
- Enterprise-grade reliability and SLAs

---

**Additional Resources:**
- [Azure Text-to-Speech Documentation](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/text-to-speech)
- [Voice Gallery](https://speech.microsoft.com/portal/voicegallery)
- [SSML Reference](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/speech-synthesis-markup)
- [Pricing](https://azure.microsoft.com/en-us/pricing/details/cognitive-services/speech-services/)
- [Azure STT Integration](azure-stt.md)
- [WebSocket API Reference](websocket.md)
