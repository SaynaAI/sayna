# Microsoft Azure Speech-to-Text Integration

Sayna supports Microsoft Azure Cognitive Services Speech-to-Text for real-time streaming speech recognition using WebSocket bidirectional streaming.

## Prerequisites

1. An Azure subscription with billing enabled
2. A Speech resource created in Azure Portal
3. Subscription key and region from the Speech resource

## Authentication

Azure STT uses subscription key-based authentication. The subscription key is tied to a specific Azure region.

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
  "tts_config": { ... }
}
```

**Note:** The `api_key` field can be left empty when credentials are configured via environment variables or YAML config. The server injects the subscription key automatically.

### STT Configuration Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `provider` | string | Yes | - | Must be `"microsoft-azure"` or `"azure"` |
| `api_key` | string | No | `""` | Subscription key (injected from server config if empty) |
| `language` | string | Yes | - | BCP-47 language code (e.g., `"en-US"`, `"de-DE"`) |
| `sample_rate` | number | Yes | - | Audio sample rate in Hz (8000, 16000, 24000, 48000) |
| `channels` | number | Yes | - | Number of audio channels (1 = mono) |
| `punctuation` | boolean | Yes | - | Enable automatic punctuation |
| `encoding` | string | Yes | - | Audio encoding format |
| `model` | string | No | `""` | Custom Speech model endpoint ID (optional) |

## Supported Regions

Azure Speech-to-Text is available in numerous regions worldwide. Choose the region closest to your users for optimal latency:

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

## Supported Audio Formats

| Encoding | Description | Sample Rates |
|----------|-------------|--------------|
| `linear16` | 16-bit signed PCM (recommended) | 8000, 16000, 24000, 48000 Hz |
| `mulaw` | G.711 mu-law | 8000 Hz |
| `alaw` | G.711 A-law | 8000 Hz |

**Note:** Azure Speech Service expects audio in 16kHz mono PCM format for best results.

## Supported Languages

Azure Speech-to-Text supports 100+ languages and locales. Common examples:

| Code | Language |
|------|----------|
| `en-US` | English (United States) |
| `en-GB` | English (United Kingdom) |
| `en-AU` | English (Australia) |
| `es-ES` | Spanish (Spain) |
| `es-MX` | Spanish (Mexico) |
| `fr-FR` | French (France) |
| `de-DE` | German (Germany) |
| `it-IT` | Italian (Italy) |
| `pt-BR` | Portuguese (Brazil) |
| `ja-JP` | Japanese (Japan) |
| `ko-KR` | Korean (Korea) |
| `zh-CN` | Chinese (Mandarin, Simplified) |
| `zh-TW` | Chinese (Mandarin, Traditional) |
| `ar-SA` | Arabic (Saudi Arabia) |
| `hi-IN` | Hindi (India) |
| `ru-RU` | Russian (Russia) |

See [Azure supported languages](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/language-support?tabs=stt) for the complete list.

## Advanced Configuration Options

### Output Format

Azure supports two output formats:

| Format | Description | Use Case |
|--------|-------------|----------|
| `simple` | Basic results with display text only | Simple transcription |
| `detailed` | Rich results with NBest alternatives and confidence | Analytics, quality assessment |

### Profanity Handling

Control how profane words appear in transcriptions:

| Option | Behavior |
|--------|----------|
| `masked` | Replace with asterisks (e.g., "What the ****") |
| `removed` | Completely remove profane words |
| `raw` | Return unchanged (exact transcription) |

### Word-Level Timing

When using detailed output format, you can request word-level timing information with start and end times for each recognized word.

### Custom Speech Models

For domain-specific recognition (medical, legal, technical), Azure supports Custom Speech models:

1. Train a custom model in Azure Speech Studio
2. Deploy the model and get the endpoint ID
3. Specify the endpoint ID in the `model` field

```json
{
  "stt_config": {
    "provider": "microsoft-azure",
    "model": "your-custom-endpoint-id"
  }
}
```

### Automatic Language Detection

Azure can automatically detect which language is being spoken from a list of candidates:

```json
{
  "stt_config": {
    "provider": "microsoft-azure",
    "language": "en-US",
    "auto_detect_languages": ["en-US", "es-ES", "fr-FR"]
  }
}
```

## Response Format

Azure STT responses are converted to Sayna's unified `STTResult` format:

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

The Azure provider maps Speech Service events to Sayna's unified format:

- **Interim Results** (`speech.hypothesis`): `is_final=false`, transcript may change
- **Final Results** (`speech.phrase`): `is_final=true`, transcript is stable
- **End of Utterance** (`speech.endDetected`): `is_speech_final=true`

## Limits and Quotas

| Limit | Value |
|-------|-------|
| Maximum audio length | 10 minutes per connection |
| Request timeout | 5 minutes of inactivity |
| NBest alternatives | Up to 5 (detailed format) |
| Concurrent connections | Based on pricing tier |

For rate limits and quotas, see [Azure quotas and limits](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/speech-services-quotas-and-limits).

## Error Handling

### Common Errors

**Authentication Errors (401 Unauthorized)**
```
STTError::AuthenticationFailed: Invalid subscription key or region mismatch
```
- Verify subscription key is correct
- Ensure region matches where the Speech resource was created
- Check if the key has expired or been regenerated

**Connection Errors**
```
STTError::ConnectionFailed: Connection timeout
```
- Check network connectivity to `<region>.stt.speech.microsoft.com`
- Verify TLS certificates are valid
- Check firewall rules allow outbound WebSocket (port 443)

**Configuration Errors**
```
STTError::ConfigurationError: Missing subscription key
```
- Ensure `AZURE_SPEECH_SUBSCRIPTION_KEY` is set
- Or provide the key in the `api_key` field

**Quota Errors**
```
STTError::ProviderError: Too many requests
```
- Check your subscription tier limits
- Implement rate limiting on your side
- Consider upgrading to a higher tier

### Error Recovery

The Azure STT provider implements automatic reconnection for transient errors:
- Network timeouts trigger reconnection attempts
- Authentication failures require manual intervention
- Quota exhaustion requires waiting or tier upgrade

## Comparison with Other Providers

| Feature | Azure STT | Deepgram | Google STT |
|---------|-----------|----------|------------|
| Protocol | WebSocket | WebSocket | gRPC |
| Streaming | Bidirectional | Bidirectional | Bidirectional |
| Languages | 100+ | 30+ | 100+ |
| Custom Models | Yes (Custom Speech) | Yes (Keywords) | Yes (Recognizers) |
| VAD | Built-in | Built-in | Built-in |
| Word Timing | Yes (detailed mode) | Yes | Yes |
| Latency | ~300-500ms | ~200-400ms | ~300-500ms |
| Pricing | Per audio minute | Per audio minute | Per audio minute |

**Choose Azure STT when you need:**
- Integration with Azure ecosystem
- Custom Speech model support
- Specific language support not available elsewhere
- Enterprise compliance requirements
- Multi-language detection

## Example: Complete Configuration

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
2. **Verify credentials**: Test with Azure Speech Studio to confirm key works
3. **Check region**: Region must match where your Speech resource was created
4. **Enable Speech Service**: Ensure Speech-to-Text is enabled in your Azure subscription

### High Latency

1. **Use regional endpoint**: Choose the region closest to your users
2. **Optimize audio format**: Use 16kHz mono PCM for best performance
3. **Reduce chunk size**: Send smaller audio chunks (100-200ms)

### Garbled Transcription

1. **Sample rate mismatch**: Audio sample rate must exactly match configuration
2. **Encoding mismatch**: Ensure encoding matches actual audio format
3. **Channel mismatch**: Mono audio should have `channels: 1`

### 401 Unauthorized Errors

1. **Wrong region**: The most common cause - ensure region matches the Speech resource location
2. **Expired key**: Regenerate keys in Azure Portal if needed
3. **Incorrect key**: Verify you're using the correct subscription key

---

**Additional Resources:**
- [Azure Speech-to-Text Documentation](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/speech-to-text)
- [Speech Service Pricing](https://azure.microsoft.com/en-us/pricing/details/cognitive-services/speech-services/)
- [Supported Languages](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/language-support?tabs=stt)
- [Custom Speech](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/custom-speech-overview)
- [WebSocket API Reference](websocket.md)
