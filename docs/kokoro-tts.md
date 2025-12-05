# Kokoro TTS Integration

Kokoro TTS is a local ONNX-based text-to-speech provider integrated into Sayna. It uses the Kokoro 82M neural model to generate high-quality speech locally without requiring any cloud API.

## Overview

- **Provider ID**: `kokoro`
- **Type**: Local ONNX inference
- **Audio Output**: 24kHz mono PCM (linear16)
- **Languages**: American English, British English
- **Model Size**: 82 million parameters

## Features

- **Local Inference**: Runs entirely on your machine using ONNX Runtime
- **High Quality**: 82M parameter neural model for natural-sounding speech
- **10 English Voices**: American and British male/female voices
- **24kHz Audio**: CD-quality mono audio output
- **Word Timestamps**: Optional word-level timing for synchronization (with timestamped model)
- **Voice Mixing**: Blend multiple voices for unique combinations
- **No API Keys**: No cloud services or API keys required
- **Offline Capable**: Works without internet connection after initial setup

## Requirements

### System Dependencies

Kokoro TTS requires eSpeak-NG to be installed on your system for phoneme conversion:

**Ubuntu/Debian:**
```bash
sudo apt-get install espeak-ng libespeak-ng-dev
```

**macOS:**
```bash
brew install espeak-ng
```

**Fedora/RHEL:**
```bash
sudo dnf install espeak-ng espeak-ng-devel
```

### Build Requirements

Enable the `kokoro-tts` feature when building:

```bash
cargo build --features kokoro-tts
```

## Model Assets

Before using Kokoro TTS, you need to download the model and voice files. Use the `sayna init` command:

```bash
CACHE_PATH=/path/to/cache sayna init
```

This will download:
- `kokoro-v1.0.onnx` (~326 MB) - The ONNX model
- `voices-v1.0.bin` (~60 MB) - Voice style embeddings

Assets are stored in `$CACHE_PATH/kokoro/`.

### Manual Download

You can also download assets manually:

- **Model**: https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx
- **Voices**: https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin

For word-level timestamps, use the timestamped model variant from HuggingFace:
- **Timestamped Model**: https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped

## Configuration

### Basic Configuration

```rust
use sayna::core::tts::{create_tts_provider, TTSConfig};

let config = TTSConfig {
    provider: "kokoro".to_string(),
    voice_id: Some("af_bella".to_string()),  // American Female - Bella
    speaking_rate: Some(1.0),                 // 1.0 = normal speed
    model: "/path/to/kokoro-v1.0.onnx".to_string(),  // Model path
    ..Default::default()
};

let provider = create_tts_provider("kokoro", config)?;
```

### WebSocket Configuration

When using via WebSocket, configure the TTS provider in the config message:

```json
{
  "tts": {
    "provider": "kokoro",
    "voice_id": "af_bella",
    "speaking_rate": 1.0
  }
}
```

### Speaking Rate

The `speaking_rate` parameter controls the speed of speech:
- Range: 0.1 to 5.0
- Default: 1.0 (normal speed)
- Lower values = slower speech
- Higher values = faster speech

## Available Voices

### American Female (af_)
| Voice ID | Description |
|----------|-------------|
| `af_bella` | Bella - clear, friendly (default) |
| `af_nicole` | Nicole - warm, expressive |
| `af_sarah` | Sarah - conversational |
| `af_sky` | Sky - bright, energetic |

### American Male (am_)
| Voice ID | Description |
|----------|-------------|
| `am_adam` | Adam - deep, authoritative |
| `am_michael` | Michael - clear, professional |

### British Female (bf_)
| Voice ID | Description |
|----------|-------------|
| `bf_emma` | Emma - modern British |
| `bf_isabella` | Isabella - elegant |

### British Male (bm_)
| Voice ID | Description |
|----------|-------------|
| `bm_george` | George - traditional British |
| `bm_lewis` | Lewis - contemporary |

### Voice Naming Convention

Voice IDs follow the pattern: `{accent}{gender}_{name}`
- First letter: `a` = American, `b` = British
- Second letter: `f` = Female, `m` = Male
- After underscore: Voice name

## Voice Mixing

Kokoro supports blending multiple voices for unique combinations:

```rust
let config = TTSConfig {
    voice_id: Some("af_bella.5+am_adam.5".to_string()),  // 50% Bella, 50% Adam
    ..Default::default()
};
```

Format: `voice1.weight1+voice2.weight2`
- Weights are 0-10 (5 = 50%)
- Weights are normalized if they don't sum to 10

## Audio Output

Kokoro TTS produces:
- **Format**: Linear PCM 16-bit (little-endian)
- **Sample Rate**: 24000 Hz
- **Channels**: Mono
- **Bit Depth**: 16-bit

## Performance Considerations

### Memory Usage
- Model loading: ~400-500 MB RAM
- Inference: Additional ~100-200 MB

### Latency
- First inference: ~500-1000ms (model warmup)
- Subsequent: ~50-200ms per sentence (depending on length)

### Threading
- ONNX Runtime uses CPU threads by default
- Configure thread count via `KokoroConfig::num_threads`

## Troubleshooting

### "eSpeak-NG not found"

Ensure eSpeak-NG is installed:
```bash
espeak-ng --version
```

### "Model file not found"

Run `sayna init` to download models, or specify the model path directly:
```rust
let config = TTSConfig {
    model: "/absolute/path/to/kokoro-v1.0.onnx".to_string(),
    ..Default::default()
};
```

### "Voice not found"

Check available voices:
```rust
let provider = create_tts_provider("kokoro", config)?;
println!("{}", provider.get_provider_info());
```

### Build errors with espeak-rs

Ensure you have the development headers:
```bash
# Ubuntu/Debian
sudo apt-get install libespeak-ng-dev clang

# macOS
brew install espeak-ng llvm
```

## Example: Complete Usage

```rust
use sayna::core::tts::{
    create_tts_provider, AudioCallback, AudioData, BaseTTS, TTSConfig, TTSError,
};
use std::sync::Arc;
use std::pin::Pin;
use std::future::Future;

struct MyCallback;

impl AudioCallback for MyCallback {
    fn on_audio(&self, audio: AudioData) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            println!("Received {} bytes of audio at {}Hz",
                     audio.data.len(), audio.sample_rate);
            // Process audio.data (PCM16 bytes)
        })
    }

    fn on_error(&self, error: TTSError) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            eprintln!("TTS error: {}", error);
        })
    }

    fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            println!("Synthesis complete");
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = TTSConfig {
        provider: "kokoro".to_string(),
        voice_id: Some("af_bella".to_string()),
        speaking_rate: Some(1.0),
        ..Default::default()
    };

    let mut tts = create_tts_provider("kokoro", config)?;
    tts.connect().await?;
    tts.on_audio(Arc::new(MyCallback))?;

    tts.speak("Hello, world! This is Kokoro TTS.", true).await?;

    tts.disconnect().await?;
    Ok(())
}
```

## Comparison with Cloud Providers

| Feature | Kokoro (Local) | Cloud TTS (ElevenLabs, etc.) |
|---------|----------------|------------------------------|
| Latency | Variable (CPU-bound) | Network-bound |
| Cost | Free | Per-character pricing |
| Privacy | Local processing | Data sent to cloud |
| Offline | Yes | No |
| Voice variety | 10 voices | Hundreds |
| Languages | English only | Many languages |
| Setup complexity | Requires eSpeak-NG | API key only |
| Audio quality | High (82M params) | Very high |

**When to use Kokoro:**
- Privacy-sensitive applications
- Offline environments
- Cost-sensitive deployments
- Low-volume applications

**When to use Cloud TTS:**
- Multi-language support needed
- Maximum voice variety
- Highest audio quality requirements
- Minimal server setup preferred

## References

- [Kokoro ONNX GitHub](https://github.com/thewh1teagle/kokoro-onnx)
- [Kokoro HuggingFace](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped)
- [eSpeak-NG](https://github.com/espeak-ng/espeak-ng)
- [ONNX Runtime](https://onnxruntime.ai/)
