//! # WebSocket Voice Handler
//!
//! This module provides a WebSocket interface for real-time voice processing using the VoiceManager.
//! It supports STT (Speech-to-Text) and TTS (Text-to-Speech) operations through simple WebSocket messages.
//!
//! ## WebSocket API
//!
//! ### Connection Flow
//! 1. Client connects to `/ws` endpoint
//! 2. Client sends configuration message to initialize STT and TTS providers
//! 3. Server responds with "ready" message when both providers are connected
//! 4. Client can then send audio data, speak commands, or flush commands
//! 5. Server sends back STT results and TTS audio data
//!
//! ### Message Types
//!
//! **Incoming Messages:**
//! - `{"type": "config", "stt_config": {...}, "tts_config": {...}}` - Initialize voice providers (without API keys)
//! - `{"type": "speak", "text": "Hello world", "flush": true}` - Synthesize speech from text (flush is optional, defaults to true)
//! - `{"type": "clear"}` - Clear pending TTS audio and clear queue
//! - **Binary messages** - Raw audio data for transcription
//!
//! **Outgoing Messages:**
//! - `{"type": "ready"}` - Voice providers are ready for use
//! - `{"type": "stt_result", "transcript": "...", "is_final": true, "confidence": 0.95}` - STT result
//! - `{"type": "error", "message": "error description"}` - Error occurred
//! - **Binary messages** - Raw TTS audio data (optimized for performance)
//!
//! ## JavaScript Client Example
//!
//! ```javascript
//! // Connect to the WebSocket
//! const ws = new WebSocket('ws://localhost:3000/ws');
//!
//! // Configuration for STT and TTS providers (no API keys needed)
//! const config = {
//!   type: 'config',
//!   stt_config: {
//!     provider: 'deepgram',
//!     language: 'en-US',
//!     sample_rate: 16000,
//!     channels: 1,
//!     punctuation: true
//!   },
//!   tts_config: {
//!     provider: 'deepgram',
//!     voice_id: 'aura-asteria-en',
//!     speaking_rate: 1.0,
//!     audio_format: 'linear16',
//!     sample_rate: 24000,
//!     connection_timeout: 30,
//!     request_timeout: 60
//!   }
//! };
//!
//! // Send configuration when connection opens
//! ws.onopen = () => {
//!   console.log('Connected to voice WebSocket');
//!   ws.send(JSON.stringify(config));
//! };
//!
//! // Handle incoming messages
//! ws.onmessage = (event) => {
//!   // Try to parse as JSON first, if it fails, treat as binary audio
//!   try {
//!     const message = JSON.parse(event.data);
//!     // If parsing succeeds, it's a JSON control message
//!     switch (message.type) {
//!       case 'ready':
//!         console.log('Voice providers are ready');
//!         // Start sending audio or text
//!         break;
//!         
//!       case 'stt_result':
//!         console.log('STT Result:', message.transcript);
//!         if (message.is_final) {
//!           console.log('Final transcription:', message.transcript);
//!         }
//!         break;
//!         
//!       case 'error':
//!         console.error('Error:', message.message);
//!         break;
//!     }
//!   } catch (error) {
//!     // If JSON parsing fails, treat as binary audio data
//!     console.log('Received TTS audio:', event.data.byteLength || event.data.size, 'bytes');
//!     playAudioData(event.data);
//!   }
//! };
//!
//! // Send text for synthesis
//! function speak(text, flush = true) {
//!   const message = {
//!     type: 'speak',
//!     text: text,
//!     flush: flush  // Set to false to queue multiple messages
//!   };
//!   ws.send(JSON.stringify(message));
//! }
//!
//! // Send audio data for transcription (binary message)
//! function sendAudio(audioBuffer) {
//!   // Send as binary message for optimal performance
//!   ws.send(audioBuffer);
//! }
//!
//! // Clear TTS queue
//! function clearTTS() {
//!   const message = { type: 'clear' };
//!   ws.send(JSON.stringify(message));
//! }
//!
//! // Play audio data from binary message
//! function playAudioData(audioData) {
//!   // Convert to ArrayBuffer if needed
//!   const buffer = audioData instanceof ArrayBuffer ? audioData : await audioData.arrayBuffer();
//!   
//!   // Use Web Audio API to play the audio
//!   const audioContext = new (window.AudioContext || window.webkitAudioContext)();
//!   audioContext.decodeAudioData(buffer.slice(), (audioBuffer) => {
//!     const source = audioContext.createBufferSource();
//!     source.buffer = audioBuffer;
//!     source.connect(audioContext.destination);
//!     source.start();
//!   });
//! }
//!
//! // Example usage
//! setTimeout(() => {
//!   // Send text messages
//!   speak('Hello, this is the first message', false);  // Queue without flush
//!   speak('This is the second message', false);        // Queue without flush
//!   speak('This is the final message', true);          // Send and flush all
//!   
//!   // Send binary audio data
//!   const mockAudio = new ArrayBuffer(1024);
//!   sendAudio(mockAudio);
//! }, 1000);
//! ```
//!
//! ## Rust Client Example
//!
//! ```rust,no_run
//! use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
//! use serde_json::json;
//! use futures_util::{SinkExt, StreamExt};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Connect to WebSocket
//!     let (ws_stream, _) = connect_async("ws://localhost:3000/ws/voice").await?;
//!     let (mut write, mut read) = ws_stream.split();
//!
//!     // Send configuration (no API keys needed)
//!     let config = json!({
//!         "type": "config",
//!         "stt_config": {
//!             "provider": "deepgram",
//!             "language": "en-US",
//!             "sample_rate": 16000,
//!             "channels": 1,
//!             "punctuation": true
//!         },
//!         "tts_config": {
//!             "provider": "deepgram",
//!             "voice_id": "aura-asteria-en",
//!             "speaking_rate": 1.0,
//!             "audio_format": "linear16",
//!             "sample_rate": 24000,
//!             "connection_timeout": 30,
//!             "request_timeout": 60
//!         }
//!     });
//!
//!     write.send(Message::Text(config.to_string().into())).await?;
//!
//!     // Handle incoming messages
//!     while let Some(message) = read.next().await {
//!         match message? {
//!             Message::Text(text) => {
//!                 // JSON control messages
//!                 let parsed: serde_json::Value = serde_json::from_str(&text)?;
//!                 match parsed["type"].as_str() {
//!                     Some("ready") => {
//!                         println!("Voice providers are ready");
//!                         
//!                         // Send a speak command
//!                         let speak_msg = json!({
//!                             "type": "speak",
//!                             "text": "Hello from Rust!",
//!                             "flush": true
//!                         });
//!                         write.send(Message::Text(speak_msg.to_string().into())).await?;
//!                         
//!                         // Send binary audio data
//!                         let audio_data = vec![0u8; 1024]; // Mock audio data
//!                         write.send(Message::Binary(audio_data.into())).await?;
//!                     }
//!                     Some("stt_result") => {
//!                         println!("STT Result: {}", parsed["transcript"]);
//!                     }
//!                     Some("error") => {
//!                         eprintln!("Error: {}", parsed["message"]);
//!                     }
//!                     _ => {}
//!                 }
//!             }
//!             Message::Binary(data) => {
//!                 // Binary audio messages
//!                 println!("Received binary TTS audio data: {} bytes", data.len());
//!                 // Process audio data directly
//!             }
//!             _ => {}
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Performance Considerations
//!
//! - **Binary Messages**: Audio data is sent as binary WebSocket messages for optimal performance
//! - **Zero-Copy Processing**: Audio data is processed without base64 encoding/decoding
//! - **Smart Detection**: JSON parsing attempt distinguishes control messages from binary audio
//! - **Batch Processing**: Send multiple audio chunks together when possible
//! - **Connection Pooling**: Reuse WebSocket connections for multiple operations
//! - **Error Handling**: Implement proper error handling and reconnection logic
//! - **Backpressure**: Handle cases where audio generation is faster than network transmission
//!
//! ## Error Handling
//!
//! The WebSocket handler provides comprehensive error handling:
//!
//! - **Configuration Errors**: Invalid STT/TTS configurations
//! - **Connection Errors**: Failed to connect to providers
//! - **Processing Errors**: Audio processing or synthesis failures
//! - **Network Errors**: WebSocket connection issues
//!
//! All errors are sent back to the client as JSON messages with `type: "error"`.

use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::{
    select,
    time::{Duration, timeout},
};
use tracing::{debug, error, info, warn};

use crate::{
    core::{
        stt::{STTConfig, STTResult},
        tts::{AudioData, TTSConfig},
        voice_manager::{VoiceManager, VoiceManagerConfig},
    },
    state::AppState,
};

/// STT configuration for WebSocket messages (without API key)
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct STTWebSocketConfig {
    /// Provider name (e.g., "deepgram")
    pub provider: String,
    /// Language code for transcription (e.g., "en-US", "es-ES")
    pub language: String,
    /// Sample rate of the audio in Hz
    pub sample_rate: u32,
    /// Number of audio channels (1 for mono, 2 for stereo)
    pub channels: u16,
    /// Enable punctuation in results
    pub punctuation: bool,
    /// Encoding of the audio
    pub encoding: String,
    /// Model to use for transcription
    pub model: String,
}

impl STTWebSocketConfig {
    /// Convert WebSocket STT config to full STT config with API key
    ///
    /// # Arguments
    /// * `api_key` - The API key to use for this provider
    ///
    /// # Returns
    /// * `STTConfig` - Full STT configuration
    pub fn to_stt_config(&self, api_key: String) -> STTConfig {
        STTConfig {
            provider: self.provider.clone(),
            api_key,
            language: self.language.clone(),
            sample_rate: self.sample_rate,
            channels: self.channels,
            punctuation: self.punctuation,
            encoding: self.encoding.clone(),
            model: self.model.clone(),
        }
    }
}

/// TTS configuration for WebSocket messages (without API key)
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TTSWebSocketConfig {
    /// Provider name (e.g., "deepgram")
    pub provider: String,
    /// Voice ID or name to use for synthesis
    pub voice_id: Option<String>,
    /// Speaking rate (0.25 to 4.0, 1.0 is normal)
    pub speaking_rate: Option<f32>,
    /// Audio format preference
    pub audio_format: Option<String>,
    /// Sample rate preference
    pub sample_rate: Option<u32>,
    /// Connection timeout in seconds
    pub connection_timeout: Option<u64>,
    /// Request timeout in seconds
    pub request_timeout: Option<u64>,
    /// Model to use for TTS
    pub model: String,
}

impl TTSWebSocketConfig {
    /// Convert WebSocket TTS config to full TTS config with API key and proper defaults
    ///
    /// # Arguments
    /// * `api_key` - The API key to use for this provider
    ///
    /// # Returns
    /// * `TTSConfig` - Full TTS configuration with defaults applied
    pub fn to_tts_config(&self, api_key: String) -> TTSConfig {
        // Start with defaults
        let defaults = TTSConfig::default();

        TTSConfig {
            provider: self.provider.clone(),
            api_key,
            model: self.model.clone(),
            // Use provided values or fall back to defaults
            voice_id: self.voice_id.clone().or(defaults.voice_id),
            speaking_rate: self.speaking_rate.or(defaults.speaking_rate),
            audio_format: self.audio_format.clone().or(defaults.audio_format),
            sample_rate: self.sample_rate.or(defaults.sample_rate),
            connection_timeout: self.connection_timeout.or(defaults.connection_timeout),
            request_timeout: self.request_timeout.or(defaults.request_timeout),
        }
    }
}

/// WebSocket message types for incoming messages
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum IncomingMessage {
    #[serde(rename = "config")]
    Config {
        stt_config: STTWebSocketConfig,
        tts_config: TTSWebSocketConfig,
    },
    #[serde(rename = "speak")]
    Speak {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        flush: Option<bool>,
    },
    #[serde(rename = "clear")]
    Clear,
}

/// WebSocket message types for outgoing messages
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum OutgoingMessage {
    #[serde(rename = "ready")]
    Ready,
    #[serde(rename = "stt_result")]
    STTResult {
        transcript: String,
        is_final: bool,
        is_speech_final: bool,
        confidence: f32,
    },
    #[serde(rename = "error")]
    Error { message: String },
}

// Base64 serialization helper is no longer needed since we use binary messages for audio

/// WebSocket connection state optimized for low latency
struct ConnectionState {
    voice_manager: Option<Arc<VoiceManager>>,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            voice_manager: None,
        }
    }
}

/// Message routing for optimized throughput
enum MessageRoute {
    Outgoing(OutgoingMessage),
    Binary(Bytes),
}

/// WebSocket voice processing handler
/// Upgrades the HTTP connection to WebSocket for real-time voice processing
pub async fn ws_voice_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    info!("WebSocket voice connection upgrade requested");
    ws.on_upgrade(move |socket| handle_voice_socket(socket, state))
}

/// Handle WebSocket voice connection with optimized performance
/// This function manages the entire WebSocket session for voice processing
async fn handle_voice_socket(socket: WebSocket, app_state: Arc<AppState>) {
    info!("WebSocket voice connection established");

    // Split the socket into sender and receiver
    let (mut sender, mut receiver) = socket.split();

    // Connection state with async-friendly mutex
    let state = Arc::new(Mutex::new(ConnectionState::new()));

    // Use bounded channel for backpressure handling
    const CHANNEL_BUFFER_SIZE: usize = 256;
    let (message_tx, mut message_rx) = mpsc::channel::<MessageRoute>(CHANNEL_BUFFER_SIZE);

    // Spawn optimized task to handle outgoing messages
    let sender_task = tokio::spawn(async move {
        while let Some(route) = message_rx.recv().await {
            let result = match route {
                MessageRoute::Outgoing(message) => {
                    // Optimize JSON serialization by reusing string buffer
                    match serde_json::to_string(&message) {
                        Ok(json_str) => sender.send(Message::Text(json_str.into())).await,
                        Err(e) => {
                            error!("Failed to serialize outgoing message: {}", e);
                            continue;
                        }
                    }
                }
                MessageRoute::Binary(data) => sender.send(Message::Binary(data)).await,
            };

            if let Err(e) = result {
                error!("Failed to send WebSocket message: {}", e);
                break;
            }
        }
    });

    // Process incoming messages with timeout handling
    let processing_timeout = Duration::from_secs(30);

    loop {
        select! {
            msg_result = receiver.next() => {
                match msg_result {
                    Some(Ok(msg)) => {
                        let continue_processing = process_message(
                            msg,
                            &state,
                            &message_tx,
                            &app_state
                        ).await;

                        if !continue_processing {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error: {}", e);
                        let _ = message_tx.send(MessageRoute::Outgoing(OutgoingMessage::Error {
                            message: format!("WebSocket error: {e}"),
                        })).await;
                        break;
                    }
                    None => {
                        info!("WebSocket connection closed by client");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(processing_timeout) => {
                // Handle connection timeout
                debug!("WebSocket connection timeout check");
                continue;
            }
        }
    }

    // Clean up resources
    sender_task.abort();

    // Stop voice manager if it exists
    {
        let connection_state = state.lock().await;
        if let Some(voice_manager) = &connection_state.voice_manager {
            if let Err(e) = voice_manager.stop().await {
                error!("Failed to stop voice manager: {}", e);
            }
        }
    }

    info!("WebSocket voice connection terminated");
}

/// Process incoming WebSocket message with optimizations
async fn process_message(
    msg: Message,
    state: &Arc<Mutex<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    match msg {
        Message::Text(text) => {
            debug!("Received text message: {} bytes", text.len());

            // Optimized JSON parsing
            let incoming_msg: IncomingMessage = match serde_json::from_str(&text) {
                Ok(msg) => msg,
                Err(e) => {
                    error!("Failed to parse incoming message: {}", e);
                    let _ = message_tx
                        .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                            message: format!("Invalid message format: {e}"),
                        }))
                        .await;
                    return true;
                }
            };

            handle_incoming_message(incoming_msg, state, message_tx, app_state).await
        }
        Message::Binary(data) => {
            debug!("Received binary message: {} bytes", data.len());

            // Handle binary audio data directly for optimal performance
            let audio_data = data;
            handle_audio_message(audio_data, state, message_tx).await
        }
        Message::Ping(_data) => {
            debug!("Received ping message");
            // Ping/Pong is handled automatically by axum
            true
        }
        Message::Pong(_) => {
            debug!("Received pong message");
            true
        }
        Message::Close(_) => {
            info!("WebSocket connection closed by client");
            false
        }
    }
}

/// Handle parsed incoming message with optimizations
async fn handle_incoming_message(
    msg: IncomingMessage,
    state: &Arc<Mutex<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    match msg {
        IncomingMessage::Config {
            stt_config,
            tts_config,
        } => {
            println!("Processing config message");
            handle_config_message(stt_config, tts_config, state, message_tx, app_state).await
        }
        IncomingMessage::Speak { text, flush } => {
            handle_speak_message(text, flush, state, message_tx).await
        }
        IncomingMessage::Clear => handle_clear_message(state, message_tx).await,
    }
}

/// Handle configuration message with optimizations
async fn handle_config_message(
    stt_ws_config: STTWebSocketConfig,
    tts_ws_config: TTSWebSocketConfig,
    state: &Arc<Mutex<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    info!(
        "Configuring voice manager with STT provider: {} and TTS provider: {}",
        stt_ws_config.provider, tts_ws_config.provider
    );

    // Get API keys from server config using utility function
    let stt_api_key = match app_state.config.get_api_key(&stt_ws_config.provider) {
        Ok(key) => key,
        Err(error_msg) => {
            error!("{}", error_msg);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: error_msg,
                }))
                .await;
            return true;
        }
    };

    let tts_api_key = match app_state.config.get_api_key(&tts_ws_config.provider) {
        Ok(key) => key,
        Err(error_msg) => {
            error!("{}", error_msg);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: error_msg,
                }))
                .await;
            return true;
        }
    };

    // Create full configs with API keys
    let stt_config = stt_ws_config.to_stt_config(stt_api_key);
    let tts_config = tts_ws_config.to_tts_config(tts_api_key);

    // Create voice manager configuration
    let voice_config = VoiceManagerConfig {
        stt_config,
        tts_config,
    };

    // Create voice manager
    let voice_manager = match VoiceManager::new(voice_config) {
        Ok(vm) => Arc::new(vm),
        Err(e) => {
            error!("Failed to create voice manager: {}", e);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: format!("Failed to create voice manager: {e}"),
                }))
                .await;
            return true;
        }
    };

    // Start voice manager
    if let Err(e) = voice_manager.start().await {
        error!("Failed to start voice manager: {}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to start voice manager: {e}"),
            }))
            .await;
        return true;
    }

    // Set up STT callback with optimized message routing
    let message_tx_clone = message_tx.clone();
    if let Err(e) = voice_manager
        .on_stt_result(move |result: STTResult| {
            let message_tx = message_tx_clone.clone();
            Box::pin(async move {
                let msg = OutgoingMessage::STTResult {
                    transcript: result.transcript,
                    is_final: result.is_final,
                    is_speech_final: result.is_speech_final,
                    confidence: result.confidence,
                };
                let _ = message_tx.send(MessageRoute::Outgoing(msg)).await;
            })
        })
        .await
    {
        error!("Failed to set up STT callback: {}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to set up STT callback: {e}"),
            }))
            .await;
        return true;
    }

    // Set up TTS audio callback with binary optimization
    let message_tx_clone = message_tx.clone();
    if let Err(e) = voice_manager
        .on_tts_audio(move |audio_data: AudioData| {
            let message_tx = message_tx_clone.clone();
            Box::pin(async move {
                // Send immediately as binary for optimal performance
                let audio_bytes = Bytes::from(audio_data.data);
                let _ = message_tx.send(MessageRoute::Binary(audio_bytes)).await;
            })
        })
        .await
    {
        error!("Failed to set up TTS audio callback: {}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to set up TTS audio callback: {e}"),
            }))
            .await;
        return true;
    }

    // Set up TTS error callback
    let message_tx_clone = message_tx.clone();
    if let Err(e) = voice_manager
        .on_tts_error(move |error| {
            let message_tx = message_tx_clone.clone();
            Box::pin(async move {
                let msg = OutgoingMessage::Error {
                    message: format!("TTS error: {error}"),
                };
                let _ = message_tx.send(MessageRoute::Outgoing(msg)).await;
            })
        })
        .await
    {
        error!("Failed to set up TTS error callback: {}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to set up TTS error callback: {e}"),
            }))
            .await;
        return true;
    }

    // Wait for providers to be ready with timeout
    let ready_timeout = Duration::from_secs(30);
    let ready_check = async {
        while !voice_manager.is_ready().await {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };

    if timeout(ready_timeout, ready_check).await.is_err() {
        error!("Timeout waiting for voice providers to be ready");
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: "Timeout waiting for voice providers to be ready".to_string(),
            }))
            .await;
        return true;
    }

    // Store voice manager in state
    {
        let mut connection_state = state.lock().await;
        connection_state.voice_manager = Some(voice_manager);
    }

    // Send ready message
    let _ = message_tx
        .send(MessageRoute::Outgoing(OutgoingMessage::Ready))
        .await;
    info!("Voice manager ready and configured");

    true
}

/// Handle audio input message with optimizations
async fn handle_audio_message(
    audio_data: Bytes,
    state: &Arc<Mutex<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    debug!("Processing audio data: {} bytes", audio_data.len());

    let voice_manager = {
        let connection_state = state.lock().await;
        match &connection_state.voice_manager {
            Some(vm) => vm.clone(),
            None => {
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: "Voice manager not configured. Send config message first."
                            .to_string(),
                    }))
                    .await;
                return true;
            }
        }
    };

    // Use zero-copy conversion to Vec<u8>
    let audio_vec = audio_data.to_vec();

    // Send audio to STT provider
    if let Err(e) = voice_manager.receive_audio(audio_vec).await {
        error!("Failed to process audio: {}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to process audio: {e}"),
            }))
            .await;
    }

    true
}

/// Handle speak command message with optimizations
async fn handle_speak_message(
    text: String,
    flush: Option<bool>,
    state: &Arc<Mutex<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    // Default flush to true for backward compatibility
    let should_flush = flush.unwrap_or(true);
    debug!(
        "Processing speak command: {} chars (flush: {})",
        text.len(),
        should_flush
    );

    let voice_manager = {
        let connection_state = state.lock().await;
        match &connection_state.voice_manager {
            Some(vm) => vm.clone(),
            None => {
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: "Voice manager not configured. Send config message first."
                            .to_string(),
                    }))
                    .await;
                return true;
            }
        }
    };

    // Send text to TTS provider with flush parameter
    if let Err(e) = voice_manager.speak(&text, should_flush).await {
        error!("Failed to synthesize speech: {}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to synthesize speech: {e}"),
            }))
            .await;
    } else {
        debug!(
            "Speech synthesis started for: {} chars (flush: {})",
            text.len(),
            should_flush
        );
    }

    true
}

/// Handle clear command message with optimizations
async fn handle_clear_message(
    state: &Arc<Mutex<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    debug!("Processing clear command");

    let voice_manager = {
        let connection_state = state.lock().await;

        match &connection_state.voice_manager {
            Some(vm) => vm.clone(),
            None => {
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: "Voice manager not configured. Send config message first."
                            .to_string(),
                    }))
                    .await;
                return true;
            }
        }
    };

    // Clear TTS provider
    if let Err(e) = voice_manager.clear_tts().await {
        error!("Failed to clear TTS provider: {}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to clear TTS provider: {e}"),
            }))
            .await;
    }

    debug!("Clear command completed");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    // Test imports - STT and TTS config types are now defined in this file

    #[test]
    fn test_ws_config_serialization() {
        // Test STT WebSocket config
        let stt_ws_config = STTWebSocketConfig {
            provider: "deepgram".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "nova-3".to_string(),
        };

        let json = serde_json::to_string(&stt_ws_config).unwrap();
        assert!(json.contains("\"provider\":\"deepgram\""));
        assert!(json.contains("\"language\":\"en-US\""));
        assert!(!json.contains("api_key")); // Should not contain API key

        // Test TTS WebSocket config
        let tts_ws_config = TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: Some("aura-luna-en".to_string()),
            speaking_rate: Some(1.0),
            audio_format: Some("pcm".to_string()),
            sample_rate: Some(22050),
            connection_timeout: Some(30),
            request_timeout: Some(60),
            model: "".to_string(), // Model is in Voice ID for Deepgram
        };

        let json = serde_json::to_string(&tts_ws_config).unwrap();
        assert!(json.contains("\"provider\":\"deepgram\""));
        assert!(json.contains("\"voice_id\":\"aura-luna-en\""));
        assert!(!json.contains("api_key")); // Should not contain API key
    }

    #[test]
    fn test_incoming_message_serialization() {
        // Test config message
        let config_msg = IncomingMessage::Config {
            stt_config: STTWebSocketConfig {
                provider: "deepgram".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                channels: 1,
                punctuation: true,
                encoding: "linear16".to_string(),
                model: "nova-3".to_string(),
            },
            tts_config: TTSWebSocketConfig {
                provider: "deepgram".to_string(),
                voice_id: Some("aura-luna-en".to_string()),
                speaking_rate: Some(1.0),
                audio_format: Some("pcm".to_string()),
                sample_rate: Some(22050),
                connection_timeout: Some(30),
                request_timeout: Some(60),
                model: "".to_string(), // Model is in Voice ID for Deepgram
            },
        };

        let json = serde_json::to_string(&config_msg).unwrap();
        assert!(json.contains("\"type\":\"config\""));
        assert!(!json.contains("api_key")); // Should not contain API key

        // Test speak message
        let speak_msg = IncomingMessage::Speak {
            text: "Hello world".to_string(),
            flush: Some(true),
        };

        let json = serde_json::to_string(&speak_msg).unwrap();
        assert!(json.contains("\"type\":\"speak\""));
        assert!(json.contains("Hello world"));
        assert!(json.contains("\"flush\":true"));

        // Test speak message without flush (backward compatibility)
        let speak_msg_no_flush = IncomingMessage::Speak {
            text: "Hello world".to_string(),
            flush: None,
        };

        let json = serde_json::to_string(&speak_msg_no_flush).unwrap();
        assert!(json.contains("\"type\":\"speak\""));
        assert!(json.contains("Hello world"));
        // Should not contain flush when None
        assert!(!json.contains("flush"));

        // Test parsing message without flush field (backward compatibility)
        let json_without_flush = r#"{"type":"speak","text":"Hello world"}"#;
        let parsed: IncomingMessage = serde_json::from_str(json_without_flush).unwrap();
        if let IncomingMessage::Speak { text, flush } = parsed {
            assert_eq!(text, "Hello world");
            assert_eq!(flush, None);
        } else {
            panic!("Expected Speak message");
        }

        // Test flush message
        let clear_msg = IncomingMessage::Clear;
        let json = serde_json::to_string(&clear_msg).unwrap();
        assert!(json.contains("\"type\":\"clear\""));
    }

    #[test]
    fn test_outgoing_message_serialization() {
        // Test ready message
        let ready_msg = OutgoingMessage::Ready;
        let json = serde_json::to_string(&ready_msg).unwrap();
        assert!(json.contains("\"type\":\"ready\""));

        // Test STT result message
        let stt_msg = OutgoingMessage::STTResult {
            transcript: "Hello world".to_string(),
            is_final: true,
            is_speech_final: true,
            confidence: 0.95,
        };

        let json = serde_json::to_string(&stt_msg).unwrap();
        assert!(json.contains("\"type\":\"stt_result\""));
        assert!(json.contains("Hello world"));
        assert!(json.contains("0.95"));

        // Test error message
        let error_msg = OutgoingMessage::Error {
            message: "Test error".to_string(),
        };

        let json = serde_json::to_string(&error_msg).unwrap();
        assert!(json.contains("\"type\":\"error\""));
        assert!(json.contains("Test error"));
    }

    #[test]
    fn test_binary_audio_handling() {
        // Test that binary audio data is handled directly as bytes
        // without JSON serialization/deserialization
        let audio_data = vec![1, 2, 3, 4, 5];
        let bytes_data = Bytes::from(audio_data.clone());

        // Binary audio messages are now handled directly
        // No JSON serialization involved for better performance
        assert_eq!(bytes_data.to_vec(), audio_data);
        assert_eq!(bytes_data.len(), 5);
    }

    #[test]
    fn test_stt_ws_config_conversion() {
        let stt_ws_config = STTWebSocketConfig {
            provider: "deepgram".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "nova-3".to_string(),
        };

        let api_key = "test_api_key".to_string();
        let stt_config = stt_ws_config.to_stt_config(api_key.clone());

        assert_eq!(stt_config.provider, "deepgram");
        assert_eq!(stt_config.api_key, api_key);
        assert_eq!(stt_config.language, "en-US");
        assert_eq!(stt_config.sample_rate, 16000);
        assert_eq!(stt_config.channels, 1);
        assert_eq!(stt_config.punctuation, true);
    }

    #[test]
    fn test_tts_ws_config_conversion_with_all_values() {
        let tts_ws_config = TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: Some("custom-voice".to_string()),
            speaking_rate: Some(1.5),
            audio_format: Some("wav".to_string()),
            sample_rate: Some(22050),
            connection_timeout: Some(60),
            request_timeout: Some(120),
            model: "".to_string(), // Model is in Voice ID for Deepgram
        };

        let api_key = "test_api_key".to_string();
        let tts_config = tts_ws_config.to_tts_config(api_key.clone());

        assert_eq!(tts_config.provider, "deepgram");
        assert_eq!(tts_config.api_key, api_key);
        assert_eq!(tts_config.voice_id, Some("custom-voice".to_string()));
        assert_eq!(tts_config.speaking_rate, Some(1.5));
        assert_eq!(tts_config.audio_format, Some("wav".to_string()));
        assert_eq!(tts_config.sample_rate, Some(22050));
        assert_eq!(tts_config.connection_timeout, Some(60));
        assert_eq!(tts_config.request_timeout, Some(120));
    }

    #[test]
    fn test_tts_ws_config_conversion_with_defaults() {
        let tts_ws_config = TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: None,
            speaking_rate: None,
            audio_format: None,
            sample_rate: None,
            connection_timeout: None,
            request_timeout: None,
            model: "".to_string(), // Model is in Voice ID for Deepgram
        };

        let api_key = "test_api_key".to_string();
        let tts_config = tts_ws_config.to_tts_config(api_key.clone());

        assert_eq!(tts_config.provider, "deepgram");
        assert_eq!(tts_config.api_key, api_key);

        // Should use default values
        assert_eq!(tts_config.voice_id, Some("aura-asteria-en".to_string()));
        assert_eq!(tts_config.speaking_rate, Some(1.0));
        assert_eq!(tts_config.audio_format, Some("linear16".to_string()));
        assert_eq!(tts_config.sample_rate, Some(24000));
        assert_eq!(tts_config.connection_timeout, Some(30));
        assert_eq!(tts_config.request_timeout, Some(60));
    }

    #[test]
    fn test_tts_ws_config_conversion_mixed_values() {
        let tts_ws_config = TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: Some("custom-voice".to_string()),
            speaking_rate: None, // Should use default
            audio_format: Some("pcm".to_string()),
            sample_rate: None, // Should use default
            connection_timeout: Some(45),
            request_timeout: None, // Should use default
            model: "".to_string(), // Model is in Voice ID for Deepgram
        };

        let api_key = "test_api_key".to_string();
        let tts_config = tts_ws_config.to_tts_config(api_key.clone());

        assert_eq!(tts_config.provider, "deepgram");
        assert_eq!(tts_config.api_key, api_key);
        assert_eq!(tts_config.voice_id, Some("custom-voice".to_string()));
        assert_eq!(tts_config.speaking_rate, Some(1.0)); // Default
        assert_eq!(tts_config.audio_format, Some("pcm".to_string()));
        assert_eq!(tts_config.sample_rate, Some(24000)); // Default
        assert_eq!(tts_config.connection_timeout, Some(45));
        assert_eq!(tts_config.request_timeout, Some(60)); // Default
    }
}
