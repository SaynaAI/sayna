# Whisper Speech-to-Text Integration (Local ONNX)

Sayna supports Whisper STT for local speech-to-text transcription using ONNX Runtime. Unlike cloud-based providers, Whisper runs entirely on your machine without requiring API keys or network connectivity.

## Overview

Whisper is OpenAI's general-purpose speech recognition model, capable of multilingual speech recognition, speech translation, and language identification. Sayna integrates the ONNX-optimized version for efficient local inference.

### Local vs Cloud Providers

| Feature | Whisper (Local) | Cloud Providers |
|---------|-----------------|-----------------|
| API Key Required | No | Yes |
| Network Required | No | Yes |
| Latency | Model-dependent | Network-dependent |
| Cost | Free (compute only) | Per-minute billing |
| Privacy | Full data privacy | Data sent to cloud |
| Streaming | Yes (VAD-based) | Yes (real-time) |
| Languages | 99 languages | Varies by provider |

### Use Cases

- **Privacy-sensitive applications**: Medical, legal, or confidential transcription
- **Offline deployments**: Air-gapped or edge environments
- **Cost optimization**: High-volume transcription without per-minute fees
- **Self-hosted solutions**: Full control over infrastructure

## Prerequisites

### System Requirements

1. **Feature Flag**: Enable `whisper-stt` in Cargo.toml
   ```bash
   cargo build --features whisper-stt
   ```

2. **ONNX Runtime**: Bundled with the crate (no separate installation)

3. **Model Files**: Download via `sayna init` command

4. **Memory Requirements**:
   - whisper-base: ~300MB RAM
   - whisper-large-v3: ~6GB RAM

### Model Download

```bash
# Download whisper-base model (default)
cargo run --features whisper-stt -- init

# Specify cache path
CACHE_PATH=/app/cache cargo run --features whisper-stt -- init

# Download specific model
cargo run --features whisper-stt -- init --model whisper-large-v3
```

Models are downloaded from HuggingFace and cached in `$CACHE_PATH/whisper/`.

**Note**: The `CACHE_PATH` environment variable must be set. This matches the server configuration (`cache.path` in config.yaml).

## Configuration

### WebSocket Config Message

```json
{
  "type": "config",
  "audio": true,
  "stt_config": {
    "provider": "whisper",
    "api_key": "",
    "language": "en",
    "sample_rate": 16000,
    "channels": 1,
    "punctuation": true,
    "encoding": "linear16",
    "model": "whisper-base"
  }
}
```

### STT Configuration Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `provider` | string | Yes | - | Must be `"whisper"` |
| `api_key` | string | No | `""` | Not used (local model) |
| `language` | string | No | `"en"` | ISO-639-1 language code or empty for auto-detect |
| `sample_rate` | number | Yes | `16000` | Must be 16000 Hz |
| `channels` | number | Yes | `1` | Must be 1 (mono) |
| `encoding` | string | Yes | `"linear16"` | Must be `"linear16"` |
| `model` | string | No | `"whisper-base"` | Model name (see Available Models) |

### Rust Configuration

```rust
use sayna::core::stt::{STTConfig, BaseSTT, WhisperSTT};

let config = STTConfig {
    provider: "whisper".to_string(),
    api_key: String::new(),  // Not required
    language: "en".to_string(),
    sample_rate: 16000,
    channels: 1,
    punctuation: true,
    encoding: "linear16".to_string(),
    model: "whisper-base".to_string(),
};

let mut stt = WhisperSTT::new(config)?;
stt.connect().await?;
```

### Streaming Configuration (Rust)

```rust
use sayna::core::stt::whisper::{WhisperSTTConfig, StreamingConfig, VADConfig};

// Default streaming configuration
let config = WhisperSTTConfig::default();

// Low latency configuration (faster response, may sacrifice accuracy)
let config = WhisperSTTConfig::default().low_latency();

// High accuracy configuration (slower but more accurate)
let config = WhisperSTTConfig::default().high_accuracy();

// Batch mode (disable streaming, original behavior)
let config = WhisperSTTConfig::default().batch_mode();

// Custom streaming configuration
let streaming_config = StreamingConfig {
    min_audio_ms: 2000,        // Minimum audio before first transcription
    max_audio_ms: 10000,       // Maximum buffer before forced transcription
    interim_interval_ms: 500,   // Emit interim results every 500ms
    overlap_ms: 200,            // Overlap between transcriptions
    vad: VADConfig::default(),  // Voice activity detection settings
    enabled: true,              // Enable streaming mode
};
let config = WhisperSTTConfig::default()
    .with_streaming(streaming_config);
```

### Streaming Configuration Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `min_audio_ms` | 3000 | Minimum audio (ms) before first transcription |
| `max_audio_ms` | 15000 | Maximum buffer (ms) before forced transcription |
| `interim_interval_ms` | 500 | How often to emit interim results (0 = disable) |
| `overlap_ms` | 200 | Audio overlap between transcriptions |
| `enabled` | true | Enable/disable streaming mode |

### VAD Configuration

```rust
use sayna::core::stt::whisper::VADConfig;

// Default VAD settings
let vad = VADConfig::default();

// Responsive VAD (faster detection, more false positives)
let vad = VADConfig::responsive();

// Accurate VAD (slower detection, fewer false positives)
let vad = VADConfig::accurate();

// Custom VAD settings
let vad = VADConfig {
    energy_threshold: 0.01,      // Speech detection threshold (0.0-1.0)
    silence_duration_ms: 500,    // Silence required to mark speech end
    min_speech_duration_ms: 250, // Minimum speech to be valid
    frame_size: 1600,            // Frame size for energy calculation
};
```

## Available Models

| Model | Size | Memory | Speed | Accuracy | Use Case |
|-------|------|--------|-------|----------|----------|
| `whisper-base` | ~150MB | ~300MB | Fast | Good | General transcription, English-focused |
| `whisper-large-v3` | ~3GB | ~6GB | Slow | Best | Multilingual, high accuracy requirements |

### Model Selection Guidelines

- **whisper-base** (default): Best for English transcription, real-time applications, and resource-constrained environments
- **whisper-large-v3**: Best for multilingual content, accented speech, and when accuracy is critical

## Supported Audio Format

Whisper requires specific audio format:

| Parameter | Value | Notes |
|-----------|-------|-------|
| Encoding | PCM 16-bit signed | `linear16` / `pcm_s16le` |
| Sample Rate | 16000 Hz | Exactly 16kHz required |
| Channels | 1 (mono) | Stereo must be downmixed |
| Bit Depth | 16 bits | 2 bytes per sample |

Audio not matching these requirements will be rejected with a `ConfigurationError`.

## Supported Languages

Whisper supports 99 languages. Language codes follow ISO-639-1:

| Code | Language | Code | Language | Code | Language |
|------|----------|------|----------|------|----------|
| `en` | English | `zh` | Chinese | `de` | German |
| `es` | Spanish | `ru` | Russian | `ko` | Korean |
| `fr` | French | `ja` | Japanese | `pt` | Portuguese |
| `tr` | Turkish | `pl` | Polish | `ca` | Catalan |
| `nl` | Dutch | `ar` | Arabic | `sv` | Swedish |
| `it` | Italian | `id` | Indonesian | `hi` | Hindi |
| `fi` | Finnish | `vi` | Vietnamese | `he` | Hebrew |
| `uk` | Ukrainian | `el` | Greek | `ms` | Malay |
| `cs` | Czech | `ro` | Romanian | `da` | Danish |
| `hu` | Hungarian | `ta` | Tamil | `no` | Norwegian |
| `th` | Thai | `ur` | Urdu | `hr` | Croatian |
| `bg` | Bulgarian | `lt` | Lithuanian | `la` | Latin |
| `mi` | Maori | `ml` | Malayalam | `cy` | Welsh |
| `sk` | Slovak | `te` | Telugu | `fa` | Persian |
| `lv` | Latvian | `bn` | Bengali | `sr` | Serbian |
| `az` | Azerbaijani | `sl` | Slovenian | `kn` | Kannada |
| `et` | Estonian | `mk` | Macedonian | `br` | Breton |
| `eu` | Basque | `is` | Icelandic | `hy` | Armenian |
| `ne` | Nepali | `mn` | Mongolian | `bs` | Bosnian |
| `kk` | Kazakh | `sq` | Albanian | `sw` | Swahili |
| `gl` | Galician | `mr` | Marathi | `pa` | Punjabi |
| `si` | Sinhala | `km` | Khmer | `sn` | Shona |
| `yo` | Yoruba | `so` | Somali | `af` | Afrikaans |
| `oc` | Occitan | `ka` | Georgian | `be` | Belarusian |
| `tg` | Tajik | `sd` | Sindhi | `gu` | Gujarati |
| `am` | Amharic | `yi` | Yiddish | `lo` | Lao |
| `uz` | Uzbek | `fo` | Faroese | `ht` | Haitian Creole |
| `ps` | Pashto | `tk` | Turkmen | `nn` | Norwegian Nynorsk |
| `mt` | Maltese | `sa` | Sanskrit | `lb` | Luxembourgish |
| `my` | Myanmar | `bo` | Tibetan | `tl` | Tagalog |
| `mg` | Malagasy | `as` | Assamese | `tt` | Tatar |
| `haw` | Hawaiian | `ln` | Lingala | `ha` | Hausa |
| `ba` | Bashkir | `jw` | Javanese | `su` | Sundanese |

### Language Auto-Detection

Set `language` to an empty string for automatic language detection:

```json
{
  "stt_config": {
    "provider": "whisper",
    "language": "",
    "model": "whisper-large-v3"
  }
}
```

**Note**: `whisper-large-v3` provides better language detection accuracy than `whisper-base`.

## Response Format

Whisper supports both streaming and batch modes:

### Streaming Mode (Default)

In streaming mode, Whisper uses Voice Activity Detection (VAD) to detect speech boundaries and emits results at appropriate times:

**Interim Result** (during speech):
```json
{
  "type": "stt_result",
  "transcript": "Hello, how are",
  "is_final": false,
  "is_speech_final": false,
  "confidence": 0.7
}
```

**Final Result** (when speech ends):
```json
{
  "type": "stt_result",
  "transcript": "Hello, how are you today?",
  "is_final": true,
  "is_speech_final": true,
  "confidence": 0.95
}
```

### Batch Mode

In batch mode (streaming disabled), Whisper behaves like a traditional batch transcription:
```json
{
  "type": "stt_result",
  "transcript": "Hello, how are you today?",
  "is_final": true,
  "is_speech_final": true,
  "confidence": 0.95
}
```

### Response Fields

| Field | Description |
|-------|-------------|
| `transcript` | The transcribed text |
| `is_final` | `true` for final results, `false` for interim results |
| `is_speech_final` | `true` when speech segment ends (triggers TTS response) |
| `confidence` | `0.95` for final results, `0.7` for interim results |

### Streaming Behavior

In streaming mode (default), Whisper:
- Uses VAD to detect when the user starts/stops speaking
- Emits interim results periodically during speech (configurable interval)
- Sets `is_speech_final=true` when silence is detected after speech
- Maintains a sliding window buffer with overlap for better word boundaries
- Supports configurable min/max audio buffer sizes

### Batch Processing Behavior

In batch mode, Whisper:
- Buffers audio until max buffer size (30 seconds)
- Processes entire buffer as single transcription
- Returns one final result per buffer
- Call `flush()` to process remaining audio before disconnecting

## Usage Examples

### Basic Usage

```rust
use sayna::core::stt::{BaseSTT, STTConfig, STTResult, WhisperSTT};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = STTConfig {
        provider: "whisper".to_string(),
        language: "en".to_string(),
        sample_rate: 16000,
        ..Default::default()
    };

    let mut stt = WhisperSTT::new(config)?;

    // Register callback
    let callback = Arc::new(|result: STTResult| {
        Box::pin(async move {
            println!("Transcript: {}", result.transcript);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    });
    stt.on_result(callback).await?;

    // Connect (loads model)
    stt.connect().await?;

    // Send audio (16kHz mono PCM)
    let audio_data: Vec<u8> = load_audio_file("speech.pcm")?;
    stt.send_audio(audio_data).await?;

    // Flush remaining audio
    if let Some(result) = stt.flush().await? {
        println!("Final: {}", result.transcript);
    }

    stt.disconnect().await?;
    Ok(())
}
```

### WebSocket Integration

```rust
use sayna::core::stt::{create_stt_provider, STTConfig};

let config = STTConfig {
    provider: "whisper".to_string(),
    language: "en".to_string(),
    sample_rate: 16000,
    model: "whisper-base".to_string(),
    ..Default::default()
};

// Create via factory function
let stt = create_stt_provider("whisper", config)?;
```

### Multilingual Transcription

```rust
let config = STTConfig {
    provider: "whisper".to_string(),
    language: "".to_string(),  // Auto-detect
    model: "whisper-large-v3".to_string(),  // Best for multilingual
    sample_rate: 16000,
    ..Default::default()
};
```

## Error Handling

### Common Errors

**Model Not Found**
```
STTError::ConfigurationError: Whisper whisper-base assets not found. Run 'sayna init' to download model files.
```
- Run `sayna init` to download model files
- Verify cache path is accessible

**Invalid Sample Rate**
```
STTError::ConfigurationError: Whisper requires 16000Hz audio, got 44100Hz. Please resample your audio.
```
- Resample audio to 16kHz before sending

**Invalid Encoding**
```
STTError::InvalidAudioFormat: Whisper requires linear16 encoding, got mulaw
```
- Convert audio to PCM 16-bit signed

**Invalid Model Name**
```
STTError::ConfigurationError: Unknown Whisper model 'whisper-tiny'. Supported models: whisper-base, whisper-large-v3
```
- Use `whisper-base` or `whisper-large-v3`

**Not Connected**
```
STTError::ConnectionFailed: Not connected to Whisper STT
```
- Call `connect()` before `send_audio()`

### Error Callback Registration

```rust
let error_callback = Arc::new(|error: STTError| {
    Box::pin(async move {
        eprintln!("STT Error: {}", error);
    }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
});
stt.on_error(error_callback).await?;
```

## Performance Tuning

### Memory Optimization

- Use `whisper-base` for lower memory footprint
- Model is loaded on `connect()` and unloaded on `disconnect()`
- Call `disconnect()` when not in use to free memory

### CPU Optimization

- ONNX Runtime uses all available CPU cores by default
- Model inference is CPU-bound (no GPU support in ONNX build)
- Transcription time scales with audio length

### Latency Considerations

Since Whisper is batch-based:
- Results arrive after buffer fills (30 seconds) or `flush()` is called
- For shorter audio, call `flush()` to get immediate results
- Audio shorter than 0.5 seconds is skipped during flush

## Comparison with Cloud Providers

| Feature | Whisper | Deepgram | Google | ElevenLabs | Azure |
|---------|---------|----------|--------|------------|-------|
| API Key | No | Yes | Yes | Yes | Yes |
| Streaming | Yes (VAD-based) | Yes | Yes | Yes | Yes |
| Latency | Configurable | Real-time | Real-time | Real-time | Real-time |
| Languages | 99 | 30+ | 125+ | 30+ | 100+ |
| Cost | Free | Per-minute | Per-minute | Per-minute | Per-minute |
| Privacy | Full | Cloud | Cloud | Cloud | Cloud |
| Accuracy | High | High | High | High | High |

**Choose Whisper when you need:**
- Complete data privacy
- Offline/air-gapped operation
- Zero operational costs
- Multilingual transcription
- Full control over streaming behavior

**Choose cloud providers when you need:**
- Lower first-result latency
- Smaller resource footprint
- Managed infrastructure
- Real-time word-level timestamps

## Troubleshooting

### Model Download Issues

1. **Slow download**: Models are large, especially whisper-large-v3 (~3GB)
   - Ensure stable network connection
   - Use `CACHE_PATH` to download to faster storage

2. **Disk space**: Check available space
   - whisper-base needs ~150MB
   - whisper-large-v3 needs ~3GB

3. **Permission errors**: Ensure write access to cache directory
   ```bash
   mkdir -p ~/.cache/whisper
   chmod 755 ~/.cache/whisper
   ```

### Transcription Quality Issues

1. **Poor accuracy**:
   - Try `whisper-large-v3` for better results
   - Specify correct language instead of auto-detection
   - Ensure audio is clear with minimal background noise

2. **Missing words**:
   - Ensure audio is 16kHz mono PCM
   - Check audio isn't clipping or too quiet
   - Longer audio segments generally transcribe better

3. **Wrong language detected**:
   - Explicitly set language code
   - Use `whisper-large-v3` for better detection

### Memory Issues

1. **Out of memory**:
   - Use `whisper-base` instead of `whisper-large-v3`
   - Increase system swap space
   - Process audio in smaller sessions

2. **Memory not freed**:
   - Call `disconnect()` to unload model
   - Model remains loaded between `send_audio()` calls

---

**Additional Resources:**
- [OpenAI Whisper Paper](https://cdn.openai.com/papers/whisper.pdf)
- [HuggingFace Whisper Models](https://huggingface.co/onnx-community/whisper-base)
- [ONNX Runtime Documentation](https://onnxruntime.ai/)
- [WebSocket API Reference](websocket.md)
