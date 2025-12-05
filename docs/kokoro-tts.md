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

This clones the HuggingFace repository containing:
- `onnx/model_quantized.onnx` (~92 MB) - The quantized ONNX model
- `voices/*.bin` (~0.5 MB each) - 60 voice style embeddings

Assets are stored in `$CACHE_PATH/kokoro/`.

### Requirements

The `sayna init` command requires:
- **Git**: For cloning the repository
- **Git LFS**: For downloading large binary files (model and voices)

Install Git LFS if not already installed:
```bash
# Ubuntu/Debian
sudo apt-get install git-lfs

# macOS
brew install git-lfs

# Then initialize
git lfs install
```

### Manual Clone

You can also clone the repository manually:

```bash
git clone --depth=1 https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped /path/to/cache/kokoro
```

### Source Repository

All assets come from: https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped

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

Kokoro includes 60 voices across multiple languages and accents.

### American English (21 voices)

**Female (af_)**: `af`, `af_alloy`, `af_aoede`, `af_bella` (default), `af_heart`, `af_jessica`, `af_kore`, `af_nicole`, `af_nova`, `af_river`, `af_sarah`, `af_sky`

**Male (am_)**: `am_adam`, `am_echo`, `am_eric`, `am_fenrir`, `am_liam`, `am_michael`, `am_onyx`, `am_puck`, `am_santa`

### British English (8 voices)

**Female (bf_)**: `bf_alice`, `bf_emma`, `bf_isabella`, `bf_lily`

**Male (bm_)**: `bm_daniel`, `bm_fable`, `bm_george`, `bm_lewis`

### Other Languages (31 voices)

| Prefix | Language | Female | Male |
|--------|----------|--------|------|
| `ef_`/`em_` | Spanish/Other | `ef_dora` | `em_alex`, `em_santa` |
| `ff_` | French | `ff_siwis` | - |
| `hf_`/`hm_` | Hindi | `hf_alpha`, `hf_beta` | `hm_omega`, `hm_psi` |
| `if_`/`im_` | Italian | `if_sara` | `im_nicola` |
| `jf_`/`jm_` | Japanese | `jf_alpha`, `jf_gongitsune`, `jf_nezumi`, `jf_tebukuro` | `jm_kumo` |
| `pf_`/`pm_` | Portuguese | `pf_dora` | `pm_alex`, `pm_santa` |
| `zf_`/`zm_` | Chinese | `zf_xiaobei`, `zf_xiaoni`, `zf_xiaoxiao`, `zf_xiaoyi` | `zm_yunjian`, `zm_yunxi`, `zm_yunxia`, `zm_yunyang` |

### Voice Naming Convention

Voice IDs follow the pattern: `{language}{gender}_{name}`
- First letter: `a` = American, `b` = British, `e` = Spanish, `f` = French, `h` = Hindi, `i` = Italian, `j` = Japanese, `p` = Portuguese, `z` = Chinese
- Second letter: `f` = Female, `m` = Male
- After underscore: Voice name

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
| Voice variety | 60 voices | Hundreds |
| Languages | 9 languages | Many languages |
| Setup complexity | Requires eSpeak-NG + Git LFS | API key only |
| Audio quality | High (82M params) | Very high |

**When to use Kokoro:**
- Privacy-sensitive applications
- Offline environments
- Cost-sensitive deployments
- Low-volume applications

**When to use Cloud TTS:**
- Maximum voice variety needed
- Highest audio quality requirements
- Minimal server setup preferred

## References

- [Kokoro ONNX GitHub](https://github.com/thewh1teagle/kokoro-onnx)
- [Kokoro HuggingFace](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX-timestamped)
- [eSpeak-NG](https://github.com/espeak-ng/espeak-ng)
- [ONNX Runtime](https://onnxruntime.ai/)
