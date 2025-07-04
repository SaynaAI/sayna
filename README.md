# Sayna Server

A simple Axum-based web server with health check endpoint and environment-based configuration, following Axum best practices for code organization.

## Features

- Health check endpoint at `/` that returns `{"status": "OK"}`
- Environment-based configuration with sensible defaults
- Support for `.env` files
- Structured logging with tracing
- Modular architecture following Axum best practices
- Comprehensive error handling
- Integration testing setup

## Project Structure

```
sayna/
├── src/
│   ├── main.rs             # Application entry point
│   ├── lib.rs              # Shared library for core logic
│   ├── config.rs           # Configuration management
│   ├── routes/             # Route definitions
│   │   ├── mod.rs
│   │   └── api.rs          # REST endpoints
│   ├── handlers/           # Request handlers
│   │   ├── mod.rs
│   │   └── api_handlers.rs # API request handlers
│   ├── state/              # Shared application state
│   │   └── mod.rs
│   ├── errors/             # Error types and conversion
│   │   ├── mod.rs
│   │   └── app_error.rs    # Custom error types
│   └── utils/              # Utility functions
│       └── mod.rs
├── tests/                  # Integration tests
│   └── api_tests.rs        # API endpoint tests
├── Cargo.toml
└── README.md
```

## Configuration

The server can be configured using environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `HOST` | Server host address | `0.0.0.0` |
| `PORT` | Server port number | `3001` |

## Usage

### Running the server

```bash
cargo run
```

### Using environment variables

You can set environment variables directly:

```bash
HOST=127.0.0.1 PORT=8080 cargo run
```

Or create a `.env` file in the project root:

```
HOST=127.0.0.1
PORT=8080
```

### Testing the health check

Once the server is running, you can test the health check endpoint:

```bash
curl http://localhost:3001/
```

Expected response:
```json
{"status":"OK"}
```

## Development

### Building

```bash
cargo build
```

### Running tests

```bash
cargo test
```

### Running in release mode

```bash
cargo run --release
```

## Architecture

This project follows Axum best practices for code organization:

- **Modular Structure**: Clear separation of concerns with dedicated modules
- **State Management**: Centralized application state using `Arc<AppState>`
- **Error Handling**: Custom error types implementing `IntoResponse`
- **Route Organization**: Logical grouping of routes in dedicated modules
- **Handler Separation**: Request handlers separated from route definitions
- **Testing**: Comprehensive integration tests using Axum's testing utilities 

# Sayna Project

This Rust Axum project is a unified STT, TTS server that handles a real time websocket APIs for STT and TTS providers like DeepGram, ElevenLabs, Google TTS, Google STT or other open source projects into one unified Websocket API, that can abstract away from providers.

The main business logic is inside [mod.rs](mdc:src/core/mod.rs) ("core" folder), that uses the abstraction concept to have a unified higher level implementations of TTS and STT separated as a dedicated folders.

## STT Base Abstraction

`BaseSTT` abstraction should have following functions
* "connect" - The main function to initiate a connection to STT providers like Deepgram or ElevenLabs
* "disconnect" - The function to disconnect from provider
* "is_ready" - The function to indicate whenever the connection is ready with a provider to be used
* "send_audio" - The function to send the audio bytes chunk to deliver to the provider for STT
* "on_result" - The function to register a callback that gets triggered when there is a transcription result from STT provider

The `STTResult` Type should contain following fields
```
transcript: str
is_final: bool
is_speech_final: bool
confidence: float
```

## TTS Base Abstraction

`BaseTTS` abstraction should have following functions
* "connect" - The main function to initiate a connection to TTS providers like Deepgram or ElevenLabs
* "disconnect" - The function to disconnect from provider
* "is_ready" - The function to indicate whenever the connection is ready with a provider to be used
* "speak" - The function to send a text to the target TTS provider
* "clear" - Send a clear command to the target provider for the active connection
* "flush" - Flush command to the target TTS provider, that forces the TTS provider to start processing the queued text
* "on_audio" - The function to register a callback that will be triggered when the audio provider responds with an audio data

## Deepgram STT Implementation

The project now includes a comprehensive Deepgram STT WebSocket implementation that follows the base STT abstraction. Here's how to use it:

### Factory Functions (Recommended)

The easiest way to create STT providers is using the factory functions:

```rust
use sayna::core::{create_stt_provider, STTConfig, STTResult};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure the STT provider
    let config = STTConfig {
        api_key: "your-deepgram-api-key".to_string(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
    };
    
    // Create a Deepgram STT provider using the factory function
    let mut stt = create_stt_provider("deepgram", config).await?;
    
    // Register a callback to handle transcription results
    let callback = Arc::new(|result: STTResult| {
        Box::pin(async move {
            println!("Transcript: {}", result.transcript);
            println!("Is final: {}", result.is_final);
            println!("Confidence: {:.2}", result.confidence);
        })
    });
    
    stt.on_result(callback).await?;
    
    // Send audio data
    if stt.is_ready() {
        let audio_data = vec![0u8; 1024]; // Replace with actual audio bytes
        stt.send_audio(audio_data).await?;
    }
    
    // Disconnect when done
    stt.disconnect().await?;
    
    Ok(())
}
```

### Available Factory Functions

The STT module provides several factory functions:

1. **`create_stt_provider(provider: &str, config: STTConfig)`** - Creates and connects a provider
2. **`create_stt_provider_unconnected(provider: &str)`** - Creates provider without connecting
3. **`create_stt_provider_from_enum(provider: STTProvider, config: STTConfig)`** - Uses enum instead of string
4. **`get_supported_stt_providers()`** - Returns list of supported providers

```rust
use sayna::core::{create_stt_provider_unconnected, get_supported_stt_providers, STTConfig};

// Get list of supported providers
let providers = get_supported_stt_providers();
println!("Supported providers: {:?}", providers); // ["deepgram"]

// Create unconnected provider
let mut stt = create_stt_provider_unconnected("deepgram")?;

// Connect later
let config = STTConfig { /* ... */ };
stt.connect(config).await?;
```

### Basic Usage

```rust
use sayna::core::{BaseSTT, STTConfig, STTResult, DeepgramSTT};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a new Deepgram STT instance
    let mut stt = DeepgramSTT::new();
    
    // Configure the STT provider
    let config = STTConfig {
        api_key: "your-deepgram-api-key".to_string(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
    };
    
    // Register a callback to handle transcription results
    let callback = Arc::new(|result: STTResult| {
        Box::pin(async move {
            println!("Transcript: {}", result.transcript);
            println!("Is final: {}", result.is_final);
            println!("Speech final: {}", result.is_speech_final);
            println!("Confidence: {:.2}", result.confidence);
        })
    });
    
    // Set up the result callback
    stt.on_result(callback).await?;
    
    // Connect to Deepgram
    stt.connect(config).await?;
    
    // Check if the connection is ready
    if stt.is_ready() {
        // Send audio data (example with dummy data)
        let audio_data = vec![0u8; 1024]; // Replace with actual audio bytes
        stt.send_audio(audio_data).await?;
    }
    
    // Disconnect when done
    stt.disconnect().await?;
    
    Ok(())
}
```

### Advanced Configuration

```rust
use sayna::core::{DeepgramSTT, DeepgramSTTConfig, STTConfig};

// Create a custom Deepgram configuration with advanced features
let deepgram_config = DeepgramSTTConfig {
    base: STTConfig {
        api_key: "your-deepgram-api-key".to_string(),
        language: "en-US".to_string(),
        sample_rate: 16000,
        channels: 1,
        punctuation: true,
    },
    model: "nova-2".to_string(),
    punctuation: true,
    diarize: true,  // Enable speaker diarization
    interim_results: true,  // Get interim results
    filler_words: false,
    profanity_filter: true,
    smart_format: true,
    keywords: vec!["important".to_string(), "urgent".to_string()],
    redact: vec!["ssn".to_string(), "credit_card".to_string()],
    vad_events: true,  // Voice activity detection
    endpointing: Some(300),  // 300ms endpointing
    tag: Some("my-app-v1".to_string()),
};
```

### Features

The Deepgram STT implementation supports:

- **Real-time WebSocket connection** to Deepgram's STT API
- **Interim and final results** for immediate transcription feedback
- **Speaker diarization** to identify different speakers
- **Smart formatting** with punctuation and capitalization
- **Keyword boosting** for domain-specific terminology
- **Content redaction** for sensitive information
- **Voice activity detection** events
- **Configurable endpointing** for speech detection
- **Error handling** with detailed error messages
- **Connection management** with automatic reconnection capabilities
- **Statistics tracking** for monitoring performance

### Error Handling

The implementation provides comprehensive error handling:

```rust
match stt.connect(config).await {
    Ok(_) => println!("Connected successfully"),
    Err(e) => match e {
        STTError::AuthenticationFailed(msg) => eprintln!("Auth error: {}", msg),
        STTError::ConnectionFailed(msg) => eprintln!("Connection error: {}", msg),
        STTError::NetworkError(msg) => eprintln!("Network error: {}", msg),
        _ => eprintln!("Other error: {}", e),
    }
}
```

## Running Tests

To run the Deepgram STT tests:

```bash
cargo test core::stt::deepgram::tests
```

To run all tests:

```bash
cargo test
```

## Dependencies

The Deepgram STT implementation uses the following key dependencies:

- `tokio-tungstenite` - WebSocket client implementation
- `serde` and `serde_json` - JSON serialization/deserialization
- `futures` - Async stream processing
- `tracing` - Structured logging
- `url` - URL parsing and construction 