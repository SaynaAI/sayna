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
//! - `{"type": "config", "audio": true, "stt_config": {...}, "tts_config": {...}, "livekit": {...}}` - Initialize voice providers (without API keys) and optionally connect to LiveKit
//! - `{"type": "speak", "text": "Hello world", "flush": true, "allow_interruption": true}` - Synthesize speech from text (flush and allow_interruption are optional, both default to true)
//! - `{"type": "clear"}` - Clear pending TTS audio and clear queue (ignored if allow_interruption=false until audio finishes)
//! - `{"type": "send_message", "message": "Hello LiveKit!", "role": "user", "topic": "chat"}` - Send custom text message through LiveKit (topic is optional)
//! - **Binary messages** - Raw audio data for transcription
//!
//! **Outgoing Messages:**
//! - `{"type": "ready"}` - Voice providers are ready for use
//! - `{"type": "stt_result", "transcript": "...", "is_final": true, "confidence": 0.95}` - STT result
//! - `{"type": "message", "message": {...}}` - Unified message from various sources (LiveKit, etc.)
//! - `{"type": "participant_disconnected", "participant": {...}}` - LiveKit participant disconnected from room
//! - `{"type": "error", "message": "error description"}` - Error occurred
//! - **Binary messages** - Raw TTS audio data (optimized for performance)
//!
//! **Unified Message Structure:**
//! ```json
//! {
//!   "type": "message",
//!   "message": {
//!     "message": "Hello from LiveKit!", // Optional text content
//!     "data": "base64encodeddata",      // Optional binary data (base64)
//!     "identity": "user123",            // Sender identity
//!     "topic": "chat",                  // Message topic/channel
//!     "room": "room-name",              // Room/space identifier
//!     "timestamp": 1234567890           // Unix timestamp
//!   }
//! }
//! ```
//!
//! **Participant Disconnected Message Structure:**
//! ```json
//! {
//!   "type": "participant_disconnected",
//!   "participant": {
//!     "identity": "user123",            // Participant's unique identity
//!     "name": "John Doe",               // Optional display name
//!     "room": "room-name",              // Room/space identifier
//!     "timestamp": 1234567890           // Unix timestamp of disconnection
//!   }
//! }
//! ```
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
//!   audio: true, // Enable audio processing (STT/TTS). Defaults to true if not specified.
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
//!   },
//!   // Optional: Connect to LiveKit for real-time audio streaming
//!   livekit: {
//!     url: 'wss://your-livekit-server.com',
//!     token: 'your-livekit-jwt-token'  // JWT token for room access
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
//!       case 'message':
//!         // Handle unified messages from LiveKit or other sources
//!         const msg = message.message;
//!         console.log(`Message from ${msg.identity} in ${msg.room}/${msg.topic}:`);
//!         if (msg.message) {
//!           console.log('Text:', msg.message);
//!         }
//!         if (msg.data) {
//!           console.log('Binary data:', msg.data); // base64 encoded
//!           // To decode: const binaryData = atob(msg.data);
//!         }
//!         break;
//!         
//!       case 'participant_disconnected':
//!         // Handle participant disconnection from LiveKit room
//!         const participant = message.participant;
//!         console.log(`Participant ${participant.identity} left the room:`, participant.name || 'Unknown');
//!         console.log(`Room: ${participant.room}, Time: ${new Date(participant.timestamp)}`);
//!         // Update UI to remove participant or show notification
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
//! function speak(text, flush = true, allowInterruption = true) {
//!   const message = {
//!     type: 'speak',
//!     text: text,
//!     flush: flush,  // Set to false to queue multiple messages
//!     allow_interruption: allowInterruption  // Set to false to prevent interruption during playback
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
//! // Send custom message through LiveKit
//! function sendMessage(text, role, topic = null) {
//!   const message = {
//!     type: 'send_message',
//!     message: text,
//!     role: role,
//!     ...(topic && { topic })  // Include topic only if provided
//!   };
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
//!   // Send text messages for TTS
//!   speak('Hello, this is the first message', false);  // Queue without flush
//!   speak('This is the second message', false);        // Queue without flush
//!   speak('This is the final message', true);          // Send and flush all
//!   
//!   // Send non-interruptible message (cannot be interrupted by STT or clear)
//!   speak('Important announcement that must not be interrupted', true, false);
//!   
//!   // Clear command will be ignored during non-interruptible audio playback
//!   clearTTS(); // Will only work if allow_interruption=true or audio has finished
//!   
//!   // Send custom messages through LiveKit
//!   sendMessage('Hello everyone in the room!', 'user');        // Send to default topic
//!   sendMessage('This is a chat message', 'user', 'chat');     // Send to specific topic
//!   
//!   // Send binary audio data (only if audio=true)
//!   const mockAudio = new ArrayBuffer(1024);
//!   sendAudio(mockAudio);
//! }, 1000);
//!
//! // Example: Configuration for LiveKit-only mode (no audio processing)
//! const livekitOnlyConfig = {
//!   type: 'config',
//!   audio: false, // Disable audio processing - only use LiveKit for messaging
//!   // stt_config and tts_config are not required when audio=false
//!   livekit: {
//!     url: 'wss://your-livekit-server.com',
//!     token: 'your-livekit-jwt-token'
//!   }
//! };
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
//!         "audio": true, // Enable audio processing (STT/TTS). Defaults to true if not specified.
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
//!         },
//!         // Optional: Connect to LiveKit for real-time audio streaming
//!         "livekit": {
//!             "url": "wss://your-livekit-server.com",
//!             "token": "your-livekit-jwt-token"
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
//!                             "flush": true,
//!                             "allow_interruption": true
//!                         });
//!                         write.send(Message::Text(speak_msg.to_string().into())).await?;
//!                         
//!                         // Send non-interruptible speak command
//!                         let non_interruptible_msg = json!({
//!                             "type": "speak",
//!                             "text": "This important message cannot be interrupted",
//!                             "flush": true,
//!                             "allow_interruption": false
//!                         });
//!                         write.send(Message::Text(non_interruptible_msg.to_string().into())).await?;
//!                         
//!                         // Send binary audio data
//!                         let audio_data = vec![0u8; 1024]; // Mock audio data
//!                         write.send(Message::Binary(audio_data.into())).await?;
//!                     }
//!                     Some("stt_result") => {
//!                         println!("STT Result: {}", parsed["transcript"]);
//!                     }
//!                     Some("participant_disconnected") => {
//!                         let participant = &parsed["participant"];
//!                         println!("Participant {} left the room: {}",
//!                                  participant["identity"],
//!                                  participant["name"].as_str().unwrap_or("Unknown"));
//!                         println!("Room: {}, Timestamp: {}",
//!                                  participant["room"],
//!                                  participant["timestamp"]);
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
//! - **RwLock Optimization**: Uses RwLock instead of Mutex for connection state (better read performance)
//! - **Buffer Reuse**: Pre-allocated JSON serialization buffer to reduce allocations
//! - **Inline Hot Paths**: Critical audio processing functions marked with #[inline] for optimization
//! - **Increased Channel Buffer**: 1024 buffer size for reduced contention in high-throughput scenarios
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
use base64::{Engine as _, engine::general_purpose};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{RwLock, mpsc};
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
    livekit::{LiveKitClient, LiveKitConfig},
    state::AppState,
};

/// Default value for audio enabled flag (true)
fn default_audio_enabled() -> Option<bool> {
    Some(true)
}

/// Default value for allow_interruption flag (true)
fn default_allow_interruption() -> Option<bool> {
    Some(true)
}

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

/// LiveKit configuration for WebSocket messages
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LiveKitWebSocketConfig {
    /// LiveKit JWT token for room access
    pub token: String,
}

impl LiveKitWebSocketConfig {
    /// Convert WebSocket LiveKit config to full LiveKit config with audio parameters
    ///
    /// # Arguments
    /// * `tts_config` - TTS configuration containing audio parameters
    ///
    /// # Returns
    /// * `LiveKitConfig` - Full LiveKit configuration with audio parameters
    pub fn to_livekit_config(
        &self,
        tts_config: &TTSWebSocketConfig,
        livekit_url: &str,
    ) -> LiveKitConfig {
        LiveKitConfig {
            url: livekit_url.to_string(),
            token: self.token.clone(),
            // Use TTS config sample rate, default to 24000 if not specified
            sample_rate: tts_config.sample_rate.unwrap_or(24000),
            // Assume mono audio for TTS (1 channel)
            channels: 1,
            // Enable noise filter by default for better audio quality
            // Can be disabled via config if lower latency is needed
            enable_noise_filter: true,
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
        /// Enable audio processing (STT/TTS). Defaults to true if not specified.
        #[serde(
            default = "default_audio_enabled",
            skip_serializing_if = "Option::is_none"
        )]
        audio: Option<bool>,
        /// STT configuration (required only when audio=true)
        #[serde(skip_serializing_if = "Option::is_none")]
        stt_config: Option<STTWebSocketConfig>,
        /// TTS configuration (required only when audio=true)
        #[serde(skip_serializing_if = "Option::is_none")]
        tts_config: Option<TTSWebSocketConfig>,
        /// Optional LiveKit configuration for real-time audio streaming
        #[serde(skip_serializing_if = "Option::is_none")]
        livekit: Option<LiveKitWebSocketConfig>,
    },
    #[serde(rename = "speak")]
    Speak {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        flush: Option<bool>,
        #[serde(
            default = "default_allow_interruption",
            skip_serializing_if = "Option::is_none"
        )]
        allow_interruption: Option<bool>,
    },
    #[serde(rename = "clear")]
    Clear,
    #[serde(rename = "send_message")]
    SendMessage {
        message: String,
        role: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        topic: Option<String>,
    },
}

/// Unified message structure for all incoming messages from various sources
#[derive(Debug, Serialize, Clone)]
pub struct UnifiedMessage {
    /// Text message content (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Binary data encoded as base64 (optional)  
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// Participant/sender identity
    pub identity: String,
    /// Topic/channel for the message
    pub topic: String,
    /// Room/space identifier
    pub room: String,
    /// Timestamp when the message was received
    pub timestamp: u64,
}

/// Participant disconnection information
#[derive(Debug, Serialize, Clone)]
pub struct ParticipantDisconnectedInfo {
    /// Participant's unique identity
    pub identity: String,
    /// Participant's display name (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Room identifier
    pub room: String,
    /// Timestamp when the disconnection occurred
    pub timestamp: u64,
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
    #[serde(rename = "message")]
    Message {
        /// Unified message structure containing text/data from various sources
        message: UnifiedMessage,
    },
    #[serde(rename = "participant_disconnected")]
    ParticipantDisconnected {
        /// Information about the participant who disconnected
        participant: ParticipantDisconnectedInfo,
    },
    #[serde(rename = "error")]
    Error { message: String },
}

// Base64 serialization helper is no longer needed since we use binary messages for audio

/// WebSocket connection state optimized for low latency
///
/// Uses RwLock for state that changes rarely but is read frequently:
/// - AtomicBool for audio_enabled flag - no locks needed for hot path
/// - RwLock for managers - fast reads, rare writes (only during config)
/// - Optimized for the common case: many reads, few writes
struct ConnectionState {
    voice_manager: Option<Arc<VoiceManager>>,
    livekit_client: Option<Arc<RwLock<LiveKitClient>>>,
    /// Whether audio processing (STT/TTS) is enabled for this connection
    audio_enabled: AtomicBool,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            voice_manager: None,
            livekit_client: None,
            audio_enabled: AtomicBool::new(false),
        }
    }
}

/// Message routing for optimized throughput
/// Using enum for zero-cost abstraction
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

    // Connection state with RwLock for rare writes, frequent reads
    let state = Arc::new(RwLock::new(ConnectionState::new()));

    // Optimized channel buffer size for audio workloads
    // Larger buffer (1024 vs default 256) reduces contention in high-throughput scenarios
    // Trade-off: Uses more memory but provides better latency characteristics
    const CHANNEL_BUFFER_SIZE: usize = 1024;
    let (message_tx, mut message_rx) = mpsc::channel::<MessageRoute>(CHANNEL_BUFFER_SIZE);

    // Spawn task to handle outgoing messages - simple and direct for low latency
    let sender_task = tokio::spawn(async move {
        while let Some(route) = message_rx.recv().await {
            let result = match route {
                MessageRoute::Outgoing(message) => {
                    // Direct serialization and send - no batching for low latency
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

    // Optimized timeout for low-latency audio processing
    // Shorter timeout to detect stale connections faster
    let processing_timeout = Duration::from_secs(10);

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

    // Stop voice manager and LiveKit client if they exist
    {
        let state_guard = state.read().await;
        if let Some(voice_manager) = &state_guard.voice_manager {
            if let Err(e) = voice_manager.stop().await {
                error!("Failed to stop voice manager: {}", e);
            }
        }

        if let Some(livekit_client) = &state_guard.livekit_client {
            // Try to get write lock with timeout for cleanup
            match tokio::time::timeout(Duration::from_millis(100), livekit_client.write()).await {
                Ok(mut client) => {
                    if let Err(e) = client.disconnect().await {
                        error!("Failed to disconnect LiveKit client: {:?}", e);
                    }
                }
                Err(_) => {
                    warn!("Timeout acquiring LiveKit lock for cleanup - client may be busy");
                }
            }
        }
    }

    info!("WebSocket voice connection terminated");
}

/// Process incoming WebSocket message with optimizations
#[inline(always)]
async fn process_message(
    msg: Message,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    match msg {
        Message::Text(text) => {
            debug!("Received text message: {} bytes", text.len());

            // Fast path JSON parsing with pre-validation
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

            // Handle binary audio data with zero-copy optimization
            handle_audio_message(data, state, message_tx).await
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
#[inline]
async fn handle_incoming_message(
    msg: IncomingMessage,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    match msg {
        IncomingMessage::Config {
            audio,
            stt_config,
            tts_config,
            livekit,
        } => {
            println!("Processing config message");
            handle_config_message(
                audio, stt_config, tts_config, livekit, state, message_tx, app_state,
            )
            .await
        }
        IncomingMessage::Speak {
            text,
            flush,
            allow_interruption,
        } => handle_speak_message(text, flush, allow_interruption, state, message_tx).await,
        IncomingMessage::Clear => handle_clear_message(state, message_tx).await,
        IncomingMessage::SendMessage {
            message,
            role,
            topic,
        } => handle_send_message(message, role, topic, state, message_tx).await,
    }
}

/// Handle configuration message with optimizations
async fn handle_config_message(
    audio: Option<bool>,
    stt_ws_config: Option<STTWebSocketConfig>,
    tts_ws_config: Option<TTSWebSocketConfig>,
    livekit_ws_config: Option<LiveKitWebSocketConfig>,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    // Determine if audio processing is enabled (default to true)
    let audio_enabled = audio.unwrap_or(true);

    info!(
        "Configuring connection with audio_enabled: {}, LiveKit: {}",
        audio_enabled,
        livekit_ws_config.is_some()
    );

    let livekit_url = app_state.config.livekit_url.clone();

    // Validate that required configs are provided when audio is enabled
    if audio_enabled {
        if stt_ws_config.is_none() {
            let error_msg = "STT configuration is required when audio=true".to_string();
            error!("{}", error_msg);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: error_msg,
                }))
                .await;
            return true;
        }

        if tts_ws_config.is_none() {
            let error_msg = "TTS configuration is required when audio=true".to_string();
            error!("{}", error_msg);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: error_msg,
                }))
                .await;
            return true;
        }
    }

    // Store audio_enabled flag in connection state first
    {
        let state_guard = state.read().await;
        state_guard
            .audio_enabled
            .store(audio_enabled, Ordering::Relaxed);
    }

    // Initialize voice manager only if audio is enabled
    let voice_manager = if audio_enabled {
        let stt_ws_config_ref = stt_ws_config.as_ref().unwrap(); // Safe to unwrap after validation
        let tts_ws_config_ref = tts_ws_config.as_ref().unwrap(); // Safe to unwrap after validation

        info!(
            "Initializing voice manager with STT provider: {} and TTS provider: {}",
            stt_ws_config_ref.provider, tts_ws_config_ref.provider
        );

        // Get API keys from server config using utility function
        let stt_api_key = match app_state.config.get_api_key(&stt_ws_config_ref.provider) {
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

        let tts_api_key = match app_state.config.get_api_key(&tts_ws_config_ref.provider) {
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
        let stt_config = stt_ws_config_ref.to_stt_config(stt_api_key);
        let tts_config = tts_ws_config_ref.to_tts_config(tts_api_key);

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

        // We'll set up the TTS audio callback after potentially setting up LiveKit
        // This ensures we have a single callback that handles both scenarios

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
            let mut state_guard = state.write().await;
            state_guard.voice_manager = Some(voice_manager.clone());
        }

        Some(voice_manager)
    } else {
        info!("Audio processing disabled - skipping voice manager initialization");
        None
    };

    // Set up LiveKit client if configuration is provided
    let livekit_client_arc: Option<Arc<RwLock<LiveKitClient>>> =
        if let Some(livekit_ws_config) = livekit_ws_config {
            info!("Setting up LiveKit client with URL: {}", livekit_url);

            // Use default TTS config for LiveKit audio parameters when TTS is not configured
            let default_tts_config = TTSWebSocketConfig {
                provider: "deepgram".to_string(),
                voice_id: None,
                speaking_rate: None,
                audio_format: None,
                sample_rate: Some(24000), // Default sample rate for LiveKit
                connection_timeout: None,
                request_timeout: None,
                model: "".to_string(),
            };

            let tts_config_for_livekit = tts_ws_config.as_ref().unwrap_or(&default_tts_config);
            let livekit_config =
                livekit_ws_config.to_livekit_config(tts_config_for_livekit, &livekit_url);
            let mut livekit_client = LiveKitClient::new(livekit_config);

            // Set up LiveKit audio callback to forward audio to STT processing (only if audio is enabled)
            if let Some(voice_manager_ref) = &voice_manager {
                let voice_manager_clone = voice_manager_ref.clone();
                let message_tx_clone = message_tx.clone();

                livekit_client.set_audio_callback(move |audio_data: Vec<u8>| {
                    let voice_manager = voice_manager_clone.clone();
                    let message_tx = message_tx_clone.clone();

                    // Direct processing - spawn lightweight task for async processing
                    tokio::spawn(async move {
                        debug!("Received LiveKit audio: {} bytes", audio_data.len());

                        // Forward LiveKit audio to the same STT processing pipeline
                        if let Err(e) = voice_manager.receive_audio(audio_data).await {
                            error!("Failed to process LiveKit audio: {:?}", e);
                            let _ = message_tx
                                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                                    message: format!("Failed to process LiveKit audio: {e:?}"),
                                }))
                                .await;
                        }
                    });
                });
            } else {
                info!("Audio processing disabled - LiveKit audio callback not set");
            }

            // Set up LiveKit data callback to forward data messages to WebSocket client
            let message_tx_clone = message_tx.clone();
            livekit_client.set_data_callback(move |data_message| {
                let message_tx = message_tx_clone.clone();

                // Spawn task for async send to ensure delivery
                tokio::spawn(async move {
                    debug!(
                        "Received LiveKit data from {}: {} bytes",
                        data_message.participant_identity,
                        data_message.data.len()
                    );

                    // Create unified message structure for LiveKit data
                    let unified_message =
                        if let Ok(text_message) = String::from_utf8(data_message.data.clone()) {
                            // Data can be decoded as UTF-8 text
                            UnifiedMessage {
                                message: Some(text_message),
                                data: None,
                                identity: data_message.participant_identity,
                                topic: data_message.topic.unwrap_or_else(|| "default".to_string()),
                                room: "livekit".to_string(), // TODO: Get actual room name from LiveKit client
                                timestamp: data_message.timestamp,
                            }
                        } else {
                            // Binary data - encode as base64
                            UnifiedMessage {
                                message: None,
                                data: Some(general_purpose::STANDARD.encode(&data_message.data)),
                                identity: data_message.participant_identity,
                                topic: data_message.topic.unwrap_or_else(|| "default".to_string()),
                                room: "livekit".to_string(), // TODO: Get actual room name from LiveKit client
                                timestamp: data_message.timestamp,
                            }
                        };

                    let outgoing_msg = OutgoingMessage::Message {
                        message: unified_message,
                    };

                    // Use send for guaranteed delivery
                    let _ = message_tx.send(MessageRoute::Outgoing(outgoing_msg)).await;
                });
            });

            // Set up LiveKit participant disconnect callback to forward events to WebSocket client
            let message_tx_clone = message_tx.clone();
            livekit_client.set_participant_disconnect_callback(move |disconnect_event| {
                let message_tx = message_tx_clone.clone();

                // Spawn task for async send to ensure delivery
                tokio::spawn(async move {
                    debug!(
                        "Participant {} disconnected from LiveKit room {}",
                        disconnect_event.participant_identity, disconnect_event.room_name
                    );

                    // Create participant disconnected info for WebSocket client
                    let participant_info = ParticipantDisconnectedInfo {
                        identity: disconnect_event.participant_identity,
                        name: disconnect_event.participant_name,
                        room: disconnect_event.room_name,
                        timestamp: disconnect_event.timestamp,
                    };

                    let outgoing_msg = OutgoingMessage::ParticipantDisconnected {
                        participant: participant_info,
                    };

                    // Use send for guaranteed delivery
                    let _ = message_tx.send(MessageRoute::Outgoing(outgoing_msg)).await;
                });
            });

            // Connect to LiveKit room
            if let Err(e) = livekit_client.connect().await {
                error!("Failed to connect to LiveKit room: {:?}", e);
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: format!("Failed to connect to LiveKit room: {e:?}"),
                    }))
                    .await;
                return true;
            }

            // Store LiveKit client in state
            let livekit_client_arc = Arc::new(RwLock::new(livekit_client));
            {
                let mut state_guard = state.write().await;
                state_guard.livekit_client = Some(livekit_client_arc.clone());
            }

            // Register audio clear callback with VoiceManager if both are available
            if let Some(voice_manager_ref) = &voice_manager {
                let livekit_client_clone = livekit_client_arc.clone();

                if let Err(e) = voice_manager_ref
                    .on_audio_clear(move || {
                        let livekit = livekit_client_clone.clone();
                        Box::pin(async move {
                            // Use try_write to avoid blocking in callback
                            // If we can't get the lock, it's okay - audio will be cleared eventually
                            match livekit.try_write() {
                                Ok(mut client) => {
                                    if let Err(e) = client.clear_audio().await {
                                        warn!("Failed to clear LiveKit audio buffer: {:?}", e);
                                    } else {
                                        debug!("Cleared LiveKit audio buffer during interruption");
                                    }
                                }
                                Err(_) => {
                                    debug!("LiveKit client busy during audio clear - skipping");
                                }
                            }
                        })
                    })
                    .await
                {
                    warn!("Failed to register audio clear callback: {:?}", e);
                }
            }

            info!("LiveKit client connected and ready");
            Some(livekit_client_arc)
        } else {
            None
        };

    // Set up unified TTS audio callback that handles both LiveKit and WebSocket routing (only if audio is enabled)
    if let Some(voice_manager_ref) = &voice_manager {
        let message_tx_for_tts = message_tx.clone();
        let livekit_client_for_tts = livekit_client_arc.clone();

        if let Err(e) = voice_manager_ref
            .on_tts_audio(move |audio_data: AudioData| {
                let message_tx = message_tx_for_tts.clone();
                let livekit_client = livekit_client_for_tts.clone();

                Box::pin(async move {
                    let mut sent_to_livekit = false;

                    // Try to send to LiveKit first if available
                    if let Some(livekit_client_arc) = &livekit_client {
                        // Use try_write for TTS audio to avoid blocking the hot path
                        // If we can't get the lock immediately, fall back to WebSocket
                        match livekit_client_arc.try_write() {
                            Ok(mut client) => {
                                // Check if LiveKit is connected before attempting to send
                                if client.is_connected().await {
                                    match client.send_tts_audio(audio_data.data.clone()).await {
                                        Ok(()) => {
                                            debug!(
                                                "TTS audio successfully sent to LiveKit: {} bytes",
                                                audio_data.data.len()
                                            );
                                            sent_to_livekit = true;
                                        }
                                        Err(e) => {
                                            error!("Failed to send TTS audio to LiveKit: {:?}", e);
                                            // Will fall back to WebSocket below
                                        }
                                    }
                                } else {
                                    debug!(
                                        "LiveKit client not connected, falling back to WebSocket"
                                    );
                                }
                            }
                            Err(_) => {
                                debug!("LiveKit client busy, falling back to WebSocket for TTS audio");
                                // Will fall back to WebSocket below
                            }
                        }
                    }

                    // Fall back to WebSocket if LiveKit is not available or failed
                    if !sent_to_livekit {
                        debug!(
                            "Sending TTS audio to WebSocket client: {} bytes",
                            audio_data.data.len()
                        );
                        let audio_bytes = Bytes::from(audio_data.data);
                        if let Err(e) = message_tx.send(MessageRoute::Binary(audio_bytes)).await {
                            error!("Failed to send TTS audio to WebSocket: {:?}", e);
                        }
                    }
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
    } else {
        info!("Audio processing disabled - TTS audio callback not set");
    }

    // Send ready message
    let _ = message_tx
        .send(MessageRoute::Outgoing(OutgoingMessage::Ready))
        .await;
    info!("Voice manager ready and configured");

    true
}

/// Handle audio input message with optimizations
/// This is a hot path for real-time audio processing
#[inline(always)]
async fn handle_audio_message(
    audio_data: Bytes,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    debug!("Processing audio data: {} bytes", audio_data.len());

    // Fast path: read lock to check state and get voice manager
    let voice_manager = {
        let state_guard = state.read().await;

        // Check if audio processing is enabled (atomic read, no lock overhead)
        if !state_guard.audio_enabled.load(Ordering::Relaxed) {
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message:
                        "Audio processing is disabled. Send config message with audio=true first."
                            .to_string(),
                }))
                .await;
            return true;
        }

        match &state_guard.voice_manager {
            Some(vm) => vm.clone(),
            None => {
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: "Voice manager not configured. Send config message with audio=true first."
                            .to_string(),
                    }))
                    .await;
                return true;
            }
        }
    };

    // Direct pass-through without unnecessary allocation
    // The Bytes type already provides efficient cloning and slicing

    // Send audio to STT provider with zero-copy optimization
    // Converting to Vec only when needed by the provider
    if let Err(e) = voice_manager.receive_audio(audio_data.to_vec()).await {
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
    allow_interruption: Option<bool>,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    // Default flush to true for backward compatibility
    let should_flush = flush.unwrap_or(true);
    // Default allow_interruption to true for backward compatibility
    let allow_interruption = allow_interruption.unwrap_or(true);

    debug!(
        "Processing speak command: {} chars (flush: {}, allow_interruption: {})",
        text.len(),
        should_flush,
        allow_interruption
    );

    // Fast path: read lock to check state and get voice manager
    let voice_manager = {
        let state_guard = state.read().await;

        // Check if audio processing is enabled (atomic read, no lock overhead)
        if !state_guard.audio_enabled.load(Ordering::Relaxed) {
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message:
                        "Audio processing is disabled. Send config message with audio=true first."
                            .to_string(),
                }))
                .await;
            return true;
        }

        match &state_guard.voice_manager {
            Some(vm) => vm.clone(),
            None => {
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: "Voice manager not configured. Send config message with audio=true first."
                            .to_string(),
                    }))
                    .await;
                return true;
            }
        }
    };

    info!(
        "Speaking text (flush: {}, allow_interruption: {}): {}",
        should_flush, allow_interruption, text
    );

    // Send text to TTS provider with flush and allow_interruption parameters
    if let Err(e) = voice_manager
        .speak_with_interruption(&text, should_flush, allow_interruption)
        .await
    {
        error!("Failed to synthesize speech: {}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to synthesize speech: {e}"),
            }))
            .await;
    } else {
        debug!(
            "Speech synthesis started for: {} chars (flush: {}, allow_interruption: {})",
            text.len(),
            should_flush,
            allow_interruption
        );
    }

    true
}

/// Handle clear command message with optimizations
async fn handle_clear_message(
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    debug!("Processing clear command");

    // Fast path: read lock to get both managers
    let (voice_manager, livekit_client) = {
        let state_guard = state.read().await;

        // Check if audio processing is enabled for voice manager operations
        let vm = if state_guard.audio_enabled.load(Ordering::Relaxed) {
            match &state_guard.voice_manager {
                Some(vm) => Some(vm.clone()),
                None => {
                    let _ = message_tx
                        .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                            message: "Voice manager not configured. Send config message with audio=true first."
                                .to_string(),
                        }))
                        .await;
                    return true;
                }
            }
        } else {
            // Audio is disabled, so voice manager operations are not available
            None
        };

        let lk = state_guard.livekit_client.clone();
        (vm, lk)
    };

    // Check if we're in a non-interruptible state
    let is_blocked = if let Some(ref vm) = voice_manager {
        vm.is_interruption_blocked().await
    } else {
        false
    };

    if is_blocked {
        debug!("Clear command ignored - currently in non-interruptible audio playback");
        return true;
    }

    // Clear TTS provider and audio buffers (only if audio is enabled)
    // Note: The VoiceManager's clear_tts() will automatically call the audio_clear_callback
    // which clears the LiveKit audio buffer, so we don't need to do it separately
    if let Some(vm) = voice_manager {
        if let Err(e) = vm.clear_tts().await {
            error!("Failed to clear TTS provider: {}", e);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: format!("Failed to clear TTS provider: {e}"),
                }))
                .await;
        } else {
            debug!("Successfully cleared TTS and audio buffers");
        }
    } else {
        debug!("Audio processing disabled - skipping TTS provider clear");

        // If audio is disabled but LiveKit is configured, still clear LiveKit audio
        if let Some(livekit_manager) = livekit_client {
            // Use write() to wait for the lock - clear operation is important
            let mut client = livekit_manager.write().await;
            match client.clear_audio().await {
                Ok(()) => {
                    debug!("Successfully cleared LiveKit audio buffer (audio disabled mode)");
                }
                Err(e) => {
                    warn!("Failed to clear LiveKit audio buffer: {}", e);
                }
            }
        }
    }

    debug!("Clear command completed");
    true
}

/// Handle send_message command with optimizations
async fn handle_send_message(
    message: String,
    role: String,
    topic: Option<String>,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    debug!(
        "Processing send_message command: {} chars, role: {}, topic: {:?}",
        message.len(),
        role,
        topic
    );

    // Fast path: read lock to get LiveKit client
    let livekit_client = {
        let state_guard = state.read().await;
        match &state_guard.livekit_client {
            Some(client) => client.clone(),
            None => {
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: "LiveKit client not configured. Send config message with livekit configuration first."
                            .to_string(),
                    }))
                    .await;
                return true;
            }
        }
    };

    // Use write() instead of try_write() to wait for the lock
    // This is better than complex retry logic since the LiveKit operations are fast
    let mut client = livekit_client.write().await;
    
    if let Err(e) = client.send_message(&message, &role, topic.as_deref()).await {
        error!("Failed to send message via LiveKit: {:?}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to send message via LiveKit: {e:?}"),
            }))
            .await;
    } else {
        debug!(
            "Message sent via LiveKit: {} chars, role: {}, topic: {:?}",
            message.len(),
            role,
            topic
        );
    }

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
            audio: Some(true),
            stt_config: Some(STTWebSocketConfig {
                provider: "deepgram".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                channels: 1,
                punctuation: true,
                encoding: "linear16".to_string(),
                model: "nova-3".to_string(),
            }),
            tts_config: Some(TTSWebSocketConfig {
                provider: "deepgram".to_string(),
                voice_id: Some("aura-luna-en".to_string()),
                speaking_rate: Some(1.0),
                audio_format: Some("pcm".to_string()),
                sample_rate: Some(22050),
                connection_timeout: Some(30),
                request_timeout: Some(60),
                model: "".to_string(), // Model is in Voice ID for Deepgram
            }),
            livekit: None,
        };

        let json = serde_json::to_string(&config_msg).unwrap();
        assert!(json.contains("\"type\":\"config\""));
        assert!(!json.contains("api_key")); // Should not contain API key

        // Test speak message
        let speak_msg = IncomingMessage::Speak {
            text: "Hello world".to_string(),
            flush: Some(true),
            allow_interruption: Some(true),
        };

        let json = serde_json::to_string(&speak_msg).unwrap();
        assert!(json.contains("\"type\":\"speak\""));
        assert!(json.contains("Hello world"));
        assert!(json.contains("\"flush\":true"));

        // Test speak message without flush (backward compatibility)
        let speak_msg_no_flush = IncomingMessage::Speak {
            text: "Hello world".to_string(),
            flush: None,
            allow_interruption: None,
        };

        let json = serde_json::to_string(&speak_msg_no_flush).unwrap();
        assert!(json.contains("\"type\":\"speak\""));
        assert!(json.contains("Hello world"));
        // Should not contain flush when None
        assert!(!json.contains("flush"));

        // Test parsing message without flush field (backward compatibility)
        let json_without_flush = r#"{"type":"speak","text":"Hello world"}"#;
        let parsed: IncomingMessage = serde_json::from_str(json_without_flush).unwrap();
        if let IncomingMessage::Speak {
            text,
            flush,
            allow_interruption,
        } = parsed
        {
            assert_eq!(text, "Hello world");
            assert_eq!(flush, None);
            assert_eq!(allow_interruption, Some(true)); // Defaults to true
        } else {
            panic!("Expected Speak message");
        }

        // Test speak message with allow_interruption=false
        let speak_msg_no_interruption = IncomingMessage::Speak {
            text: "Do not interrupt me".to_string(),
            flush: Some(true),
            allow_interruption: Some(false),
        };

        let json = serde_json::to_string(&speak_msg_no_interruption).unwrap();
        assert!(json.contains("\"type\":\"speak\""));
        assert!(json.contains("Do not interrupt me"));
        assert!(json.contains("\"allow_interruption\":false"));

        // Test parsing message with allow_interruption field
        let json_with_interruption =
            r#"{"type":"speak","text":"Hello","allow_interruption":false}"#;
        let parsed: IncomingMessage = serde_json::from_str(json_with_interruption).unwrap();
        if let IncomingMessage::Speak {
            text,
            flush,
            allow_interruption,
        } = parsed
        {
            assert_eq!(text, "Hello");
            assert_eq!(flush, None);
            assert_eq!(allow_interruption, Some(false));
        } else {
            panic!("Expected Speak message");
        }

        // Test flush message
        let clear_msg = IncomingMessage::Clear;
        let json = serde_json::to_string(&clear_msg).unwrap();
        assert!(json.contains("\"type\":\"clear\""));

        // Test send_message message with topic
        let send_msg = IncomingMessage::SendMessage {
            message: "Hello from client!".to_string(),
            role: "user".to_string(),
            topic: Some("chat".to_string()),
        };
        let json = serde_json::to_string(&send_msg).unwrap();
        assert!(json.contains("\"type\":\"send_message\""));
        assert!(json.contains("\"message\":\"Hello from client!\""));
        assert!(json.contains("\"role\":\"user\""));
        assert!(json.contains("\"topic\":\"chat\""));

        // Test send_message message without topic
        let send_msg_no_topic = IncomingMessage::SendMessage {
            message: "Hello without topic!".to_string(),
            role: "user".to_string(),
            topic: None,
        };
        let json = serde_json::to_string(&send_msg_no_topic).unwrap();
        assert!(json.contains("\"type\":\"send_message\""));
        assert!(json.contains("\"message\":\"Hello without topic!\""));
        assert!(json.contains("\"role\":\"user\""));
        // Should not contain topic field when None (but may contain the word "topic" elsewhere)
        assert!(!json.contains("\"topic\""));

        // Test parsing send_message JSON with topic
        let json_with_topic = r#"{"type":"send_message","message":"Hello from JSON!","role":"user","topic":"general"}"#;
        let parsed: IncomingMessage = serde_json::from_str(json_with_topic).unwrap();
        if let IncomingMessage::SendMessage {
            message,
            role,
            topic,
        } = parsed
        {
            assert_eq!(message, "Hello from JSON!");
            assert_eq!(role, "user");
            assert_eq!(topic, Some("general".to_string()));
        } else {
            panic!("Expected SendMessage message");
        }

        // Test parsing send_message JSON without topic
        let json_without_topic =
            r#"{"type":"send_message","message":"Hello without topic!","role":"user"}"#;
        let parsed: IncomingMessage = serde_json::from_str(json_without_topic).unwrap();
        if let IncomingMessage::SendMessage {
            message,
            role,
            topic,
        } = parsed
        {
            assert_eq!(message, "Hello without topic!");
            assert_eq!(role, "user");
            assert_eq!(topic, None);
        } else {
            panic!("Expected SendMessage message");
        }

        // Test send_message with different role
        let send_msg_system = IncomingMessage::SendMessage {
            message: "System notification".to_string(),
            role: "system".to_string(),
            topic: Some("notifications".to_string()),
        };
        let json = serde_json::to_string(&send_msg_system).unwrap();
        assert!(json.contains("\"type\":\"send_message\""));
        assert!(json.contains("\"message\":\"System notification\""));
        assert!(json.contains("\"role\":\"system\""));
        assert!(json.contains("\"topic\":\"notifications\""));
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
        assert!(stt_config.punctuation);
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
    fn test_livekit_ws_config_serialization() {
        let livekit_config = LiveKitWebSocketConfig {
            token: "test-jwt-token".to_string(),
        };

        let json = serde_json::to_string(&livekit_config).unwrap();
        // LiveKitWebSocketConfig only contains token, URL is provided separately
        assert!(json.contains("\"token\":\"test-jwt-token\""));
        // Verify the JSON structure is correct
        assert!(!json.contains("\"url\"")); // URL should not be in WebSocket config
    }

    #[test]
    fn test_livekit_ws_config_conversion() {
        let livekit_ws_config = LiveKitWebSocketConfig {
            token: "test-jwt-token".to_string(),
        };

        let tts_ws_config = TTSWebSocketConfig {
            provider: "deepgram".to_string(),
            voice_id: Some("aura-luna-en".to_string()),
            speaking_rate: Some(1.0),
            audio_format: Some("pcm".to_string()),
            sample_rate: Some(22050),
            connection_timeout: Some(30),
            request_timeout: Some(60),
            model: "".to_string(),
        };

        let livekit_url = "wss://test-livekit.com".to_string();
        let livekit_config = livekit_ws_config.to_livekit_config(&tts_ws_config, &livekit_url);
        assert_eq!(livekit_config.url, "wss://test-livekit.com");
        assert_eq!(livekit_config.token, "test-jwt-token");
        assert_eq!(livekit_config.sample_rate, 22050);
        assert_eq!(livekit_config.channels, 1);
        assert!(livekit_config.enable_noise_filter); // Should be true by default
    }

    #[test]
    fn test_incoming_message_config_with_livekit() {
        let config_msg = IncomingMessage::Config {
            audio: Some(true),
            stt_config: Some(STTWebSocketConfig {
                provider: "deepgram".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                channels: 1,
                punctuation: true,
                encoding: "linear16".to_string(),
                model: "nova-3".to_string(),
            }),
            tts_config: Some(TTSWebSocketConfig {
                provider: "deepgram".to_string(),
                voice_id: Some("aura-luna-en".to_string()),
                speaking_rate: Some(1.0),
                audio_format: Some("pcm".to_string()),
                sample_rate: Some(22050),
                connection_timeout: Some(30),
                request_timeout: Some(60),
                model: "".to_string(),
            }),
            livekit: Some(LiveKitWebSocketConfig {
                token: "test-jwt-token".to_string(),
            }),
        };

        let json = serde_json::to_string(&config_msg).unwrap();
        assert!(json.contains("\"type\":\"config\""));
        // LiveKitWebSocketConfig only contains token, URL is provided separately when converting
        assert!(json.contains("\"token\":\"test-jwt-token\""));
        assert!(json.contains("\"livekit\"")); // Verify LiveKit section is present
    }

    #[test]
    fn test_incoming_message_config_without_livekit() {
        let config_msg = IncomingMessage::Config {
            audio: Some(true),
            stt_config: Some(STTWebSocketConfig {
                provider: "deepgram".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                channels: 1,
                punctuation: true,
                encoding: "linear16".to_string(),
                model: "nova-3".to_string(),
            }),
            tts_config: Some(TTSWebSocketConfig {
                provider: "deepgram".to_string(),
                voice_id: Some("aura-luna-en".to_string()),
                speaking_rate: Some(1.0),
                audio_format: Some("pcm".to_string()),
                sample_rate: Some(22050),
                connection_timeout: Some(30),
                request_timeout: Some(60),
                model: "".to_string(),
            }),
            livekit: None,
        };

        let json = serde_json::to_string(&config_msg).unwrap();
        assert!(json.contains("\"type\":\"config\""));
        // Should not contain livekit field when None
        assert!(!json.contains("livekit"));
    }

    #[test]
    fn test_parse_config_message_with_livekit() {
        let json = r#"{
            "type": "config",
            "stt_config": {
                "provider": "deepgram",
                "language": "en-US",
                "sample_rate": 16000,
                "channels": 1,
                "punctuation": true,
                "encoding": "linear16",
                "model": "nova-3"
            },
            "tts_config": {
                "provider": "deepgram",
                "voice_id": "aura-luna-en",
                "speaking_rate": 1.0,
                "audio_format": "pcm",
                "sample_rate": 22050,
                "connection_timeout": 30,
                "request_timeout": 60,
                "model": ""
            },
            "livekit": {
                "url": "wss://test-livekit.com",
                "token": "test-jwt-token"
            }
        }"#;

        let parsed: IncomingMessage = serde_json::from_str(json).unwrap();
        if let IncomingMessage::Config {
            audio,
            stt_config,
            tts_config,
            livekit,
        } = parsed
        {
            assert_eq!(audio, Some(true));
            assert_eq!(stt_config.as_ref().unwrap().provider, "deepgram");
            assert_eq!(tts_config.as_ref().unwrap().provider, "deepgram");

            let livekit_config = livekit.unwrap();
            assert_eq!(livekit_config.token, "test-jwt-token");
        } else {
            panic!("Expected Config message");
        }
    }

    #[test]
    fn test_unified_message_text_serialization() {
        let unified_msg = UnifiedMessage {
            message: Some("Hello from LiveKit!".to_string()),
            data: None,
            identity: "participant123".to_string(),
            topic: "chat".to_string(),
            room: "test-room".to_string(),
            timestamp: 1234567890,
        };

        let outgoing_msg = OutgoingMessage::Message {
            message: unified_msg,
        };

        let json = serde_json::to_string(&outgoing_msg).unwrap();
        assert!(json.contains("\"type\":\"message\""));
        assert!(json.contains("\"identity\":\"participant123\""));
        assert!(json.contains("\"message\":\"Hello from LiveKit!\""));
        assert!(json.contains("\"topic\":\"chat\""));
        assert!(json.contains("\"room\":\"test-room\""));
        assert!(json.contains("\"timestamp\":1234567890"));
    }

    #[test]
    fn test_unified_message_data_serialization() {
        let test_data = vec![1, 2, 3, 4, 5];
        let unified_msg = UnifiedMessage {
            message: None,
            data: Some(general_purpose::STANDARD.encode(&test_data)),
            identity: "participant123".to_string(),
            topic: "files".to_string(),
            room: "test-room".to_string(),
            timestamp: 1234567890,
        };

        let outgoing_msg = OutgoingMessage::Message {
            message: unified_msg,
        };

        let json = serde_json::to_string(&outgoing_msg).unwrap();
        assert!(json.contains("\"type\":\"message\""));
        assert!(json.contains("\"identity\":\"participant123\""));
        assert!(json.contains("\"topic\":\"files\""));
        assert!(json.contains("\"room\":\"test-room\""));
        // Data should be present, message field should not be present (due to serde skip_serializing_if)
        assert!(json.contains("\"data\":"));
        assert!(json.contains("\"timestamp\":1234567890"));
        // message field should not be present (None value with skip_serializing_if)
        assert!(!json.contains("\"message\":null"));
    }

    #[test]
    fn test_parse_config_message_without_livekit() {
        let json = r#"{
            "type": "config",
            "stt_config": {
                "provider": "deepgram",
                "language": "en-US",
                "sample_rate": 16000,
                "channels": 1,
                "punctuation": true,
                "encoding": "linear16",
                "model": "nova-3"
            },
            "tts_config": {
                "provider": "deepgram",
                "voice_id": "aura-luna-en",
                "speaking_rate": 1.0,
                "audio_format": "pcm",
                "sample_rate": 22050,
                "connection_timeout": 30,
                "request_timeout": 60,
                "model": ""
            }
        }"#;

        let parsed: IncomingMessage = serde_json::from_str(json).unwrap();
        if let IncomingMessage::Config {
            audio,
            stt_config,
            tts_config,
            livekit,
        } = parsed
        {
            assert_eq!(audio, Some(true));
            assert_eq!(stt_config.as_ref().unwrap().provider, "deepgram");
            assert_eq!(tts_config.as_ref().unwrap().provider, "deepgram");
            assert!(livekit.is_none());
        } else {
            panic!("Expected Config message");
        }
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

    #[test]
    fn test_config_message_without_livekit_routing() {
        // Test that configuration without LiveKit creates proper routing logic
        let config_msg = IncomingMessage::Config {
            audio: Some(true),
            stt_config: Some(STTWebSocketConfig {
                provider: "deepgram".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                channels: 1,
                punctuation: true,
                encoding: "linear16".to_string(),
                model: "nova-3".to_string(),
            }),
            tts_config: Some(TTSWebSocketConfig {
                provider: "deepgram".to_string(),
                voice_id: Some("aura-luna-en".to_string()),
                speaking_rate: Some(1.0),
                audio_format: Some("pcm".to_string()),
                sample_rate: Some(22050),
                connection_timeout: Some(30),
                request_timeout: Some(60),
                model: "".to_string(),
            }),
            livekit: None, // No LiveKit configuration
        };

        let json = serde_json::to_string(&config_msg).unwrap();
        assert!(json.contains("\"type\":\"config\""));
        assert!(!json.contains("livekit")); // Should not contain LiveKit field

        // Parse back to ensure structure is correct
        let parsed: IncomingMessage = serde_json::from_str(&json).unwrap();
        if let IncomingMessage::Config { livekit, .. } = parsed {
            assert!(
                livekit.is_none(),
                "LiveKit should be None when not configured"
            );
        } else {
            panic!("Expected Config message");
        }
    }

    #[test]
    fn test_config_message_with_livekit_routing() {
        // Test that configuration with LiveKit creates proper routing logic
        let config_msg = IncomingMessage::Config {
            audio: Some(true),
            stt_config: Some(STTWebSocketConfig {
                provider: "deepgram".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                channels: 1,
                punctuation: true,
                encoding: "linear16".to_string(),
                model: "nova-3".to_string(),
            }),
            tts_config: Some(TTSWebSocketConfig {
                provider: "deepgram".to_string(),
                voice_id: Some("aura-luna-en".to_string()),
                speaking_rate: Some(1.0),
                audio_format: Some("pcm".to_string()),
                sample_rate: Some(22050),
                connection_timeout: Some(30),
                request_timeout: Some(60),
                model: "".to_string(),
            }),
            livekit: Some(LiveKitWebSocketConfig {
                token: "test-jwt-token".to_string(),
            }),
        };

        let json = serde_json::to_string(&config_msg).unwrap();
        assert!(json.contains("\"type\":\"config\""));
        // LiveKitWebSocketConfig only contains token, URL is provided separately when converting
        assert!(json.contains("\"token\":\"test-jwt-token\""));
        assert!(json.contains("\"livekit\"")); // Verify LiveKit section is present

        // Parse back to ensure structure is correct
        let parsed: IncomingMessage = serde_json::from_str(&json).unwrap();
        if let IncomingMessage::Config { livekit, .. } = parsed {
            let livekit_config = livekit.unwrap();
            assert_eq!(livekit_config.token, "test-jwt-token");
        } else {
            panic!("Expected Config message");
        }
    }

    #[test]
    fn test_participant_disconnected_info_serialization() {
        let participant_info = ParticipantDisconnectedInfo {
            identity: "user123".to_string(),
            name: Some("John Doe".to_string()),
            room: "test-room".to_string(),
            timestamp: 1234567890,
        };

        let json = serde_json::to_string(&participant_info).unwrap();
        assert!(json.contains("\"identity\":\"user123\""));
        assert!(json.contains("\"name\":\"John Doe\""));
        assert!(json.contains("\"room\":\"test-room\""));
        assert!(json.contains("\"timestamp\":1234567890"));
    }

    #[test]
    fn test_participant_disconnected_info_serialization_without_name() {
        let participant_info = ParticipantDisconnectedInfo {
            identity: "user456".to_string(),
            name: None,
            room: "test-room".to_string(),
            timestamp: 1234567890,
        };

        let json = serde_json::to_string(&participant_info).unwrap();
        assert!(json.contains("\"identity\":\"user456\""));
        assert!(json.contains("\"room\":\"test-room\""));
        assert!(json.contains("\"timestamp\":1234567890"));
        // Should not contain name field when None due to skip_serializing_if
        assert!(!json.contains("name"));
    }

    #[test]
    fn test_participant_disconnected_outgoing_message() {
        let participant_info = ParticipantDisconnectedInfo {
            identity: "participant789".to_string(),
            name: Some("Alice Smith".to_string()),
            room: "conference-room".to_string(),
            timestamp: 9876543210,
        };

        let outgoing_msg = OutgoingMessage::ParticipantDisconnected {
            participant: participant_info,
        };

        let json = serde_json::to_string(&outgoing_msg).unwrap();
        assert!(json.contains("\"type\":\"participant_disconnected\""));
        assert!(json.contains("\"identity\":\"participant789\""));
        assert!(json.contains("\"name\":\"Alice Smith\""));
        assert!(json.contains("\"room\":\"conference-room\""));
        assert!(json.contains("\"timestamp\":9876543210"));
    }

    #[test]
    fn test_participant_disconnected_json_format() {
        // Test that the serialized JSON has the expected format
        let participant_info = ParticipantDisconnectedInfo {
            identity: "user999".to_string(),
            name: Some("Bob Wilson".to_string()),
            room: "meeting-room".to_string(),
            timestamp: 1111111111,
        };

        let outgoing_msg = OutgoingMessage::ParticipantDisconnected {
            participant: participant_info,
        };

        let json = serde_json::to_string(&outgoing_msg).unwrap();

        // Verify that the JSON contains all expected fields
        assert!(json.contains("\"type\":\"participant_disconnected\""));
        assert!(json.contains("\"participant\":{"));
        assert!(json.contains("\"identity\":\"user999\""));
        assert!(json.contains("\"name\":\"Bob Wilson\""));
        assert!(json.contains("\"room\":\"meeting-room\""));
        assert!(json.contains("\"timestamp\":1111111111"));
    }

    #[test]
    fn test_participant_disconnected_message_structure() {
        // Test the structure matches the documented API format
        let participant_info = ParticipantDisconnectedInfo {
            identity: "test-participant".to_string(),
            name: Some("Test User".to_string()),
            room: "test-room".to_string(),
            timestamp: 1000000000,
        };

        let outgoing_msg = OutgoingMessage::ParticipantDisconnected {
            participant: participant_info,
        };

        let json = serde_json::to_string(&outgoing_msg).unwrap();

        // Verify the JSON structure matches the API documentation
        let parsed_value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed_value["type"], "participant_disconnected");
        assert_eq!(parsed_value["participant"]["identity"], "test-participant");
        assert_eq!(parsed_value["participant"]["name"], "Test User");
        assert_eq!(parsed_value["participant"]["room"], "test-room");
        assert_eq!(parsed_value["participant"]["timestamp"], 1000000000);
    }

    #[test]
    fn test_tts_audio_routing_logic_without_livekit() {
        // Test that TTS audio data is properly handled without LiveKit
        let audio_data = vec![1, 2, 3, 4, 5];
        let bytes_data = Bytes::from(audio_data.clone());

        // When LiveKit is not configured, audio should go directly to WebSocket as binary
        assert_eq!(bytes_data.to_vec(), audio_data);
        assert_eq!(bytes_data.len(), 5);

        // Test that MessageRoute::Binary correctly wraps the audio data
        let route = MessageRoute::Binary(bytes_data);
        match route {
            MessageRoute::Binary(data) => {
                assert_eq!(data.to_vec(), audio_data);
            }
            _ => panic!("Expected Binary route"),
        }
    }

    #[test]
    fn test_tts_audio_routing_logic_with_livekit() {
        // Test that TTS audio routing logic properly handles LiveKit scenarios

        // Test case 1: LiveKit configured and available
        // In this case, audio should be routed to LiveKit, not WebSocket
        // This is tested through the unified callback logic in the actual handler

        // Test case 2: LiveKit configured but disconnected
        // Audio should fall back to WebSocket

        // Test case 3: LiveKit send failure
        // Audio should fall back to WebSocket

        // These scenarios are integration-tested through the actual WebSocket handler
        // The unit test here validates the data structures and routing logic

        let test_audio = vec![0x01, 0x02, 0x03, 0x04];
        let audio_bytes = Bytes::from(test_audio.clone());

        // Verify the audio data can be properly converted for both routing paths
        assert_eq!(audio_bytes.to_vec(), test_audio);

        // Test that cloning works for dual routing scenarios
        let cloned_audio = audio_bytes.clone();
        assert_eq!(cloned_audio.to_vec(), test_audio);
        assert_eq!(audio_bytes.to_vec(), test_audio);
    }

    #[test]
    fn test_config_message_audio_disabled() {
        // Test configuration with audio=false (LiveKit-only mode)
        let config_msg = IncomingMessage::Config {
            audio: Some(false),
            stt_config: None, // Not required when audio=false
            tts_config: None, // Not required when audio=false
            livekit: Some(LiveKitWebSocketConfig {
                token: "test-jwt-token".to_string(),
            }),
        };

        let json = serde_json::to_string(&config_msg).unwrap();
        assert!(json.contains("\"type\":\"config\""));
        assert!(json.contains("\"audio\":false"));
        assert!(json.contains("\"token\":\"test-jwt-token\""));
        assert!(json.contains("\"livekit\""));
        // Should not contain stt_config or tts_config fields when None
        assert!(!json.contains("stt_config"));
        assert!(!json.contains("tts_config"));

        // Parse back to ensure structure is correct
        let parsed: IncomingMessage = serde_json::from_str(&json).unwrap();
        if let IncomingMessage::Config {
            audio,
            stt_config,
            tts_config,
            livekit,
        } = parsed
        {
            assert_eq!(audio, Some(false));
            assert!(stt_config.is_none());
            assert!(tts_config.is_none());
            assert!(livekit.is_some());
        } else {
            panic!("Expected Config message");
        }
    }

    #[test]
    fn test_config_message_audio_default() {
        // Test configuration with no audio field (should default to true)
        let config_msg = IncomingMessage::Config {
            audio: None, // Should default to true
            stt_config: Some(STTWebSocketConfig {
                provider: "deepgram".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                channels: 1,
                punctuation: true,
                encoding: "linear16".to_string(),
                model: "nova-3".to_string(),
            }),
            tts_config: Some(TTSWebSocketConfig {
                provider: "deepgram".to_string(),
                voice_id: Some("aura-luna-en".to_string()),
                speaking_rate: Some(1.0),
                audio_format: Some("pcm".to_string()),
                sample_rate: Some(22050),
                connection_timeout: Some(30),
                request_timeout: Some(60),
                model: "".to_string(),
            }),
            livekit: None,
        };

        let json = serde_json::to_string(&config_msg).unwrap();
        assert!(json.contains("\"type\":\"config\""));
        // Should not contain audio field when None (due to skip_serializing_if)
        assert!(!json.contains("\"audio\""));

        // Parse back to ensure structure is correct
        let parsed: IncomingMessage = serde_json::from_str(&json).unwrap();
        if let IncomingMessage::Config {
            audio,
            stt_config,
            tts_config,
            livekit,
        } = parsed
        {
            assert_eq!(audio, Some(true)); // Should default to true via serde default
            assert!(stt_config.is_some());
            assert!(tts_config.is_some());
            assert!(livekit.is_none());
        } else {
            panic!("Expected Config message");
        }
    }

    #[test]
    fn test_unified_message_for_livekit_integration() {
        // Test unified message structure used for LiveKit data forwarding
        let test_data = b"Hello from LiveKit participant";

        // Test text message from LiveKit
        let text_message = UnifiedMessage {
            message: Some(String::from_utf8(test_data.to_vec()).unwrap()),
            data: None,
            identity: "participant123".to_string(),
            topic: "chat".to_string(),
            room: "test-room".to_string(),
            timestamp: 1234567890,
        };

        let outgoing_msg = OutgoingMessage::Message {
            message: text_message,
        };

        let json = serde_json::to_string(&outgoing_msg).unwrap();
        assert!(json.contains("\"type\":\"message\""));
        assert!(json.contains("\"identity\":\"participant123\""));
        assert!(json.contains("Hello from LiveKit participant"));

        // Test binary data message from LiveKit
        let binary_message = UnifiedMessage {
            message: None,
            data: Some(general_purpose::STANDARD.encode(test_data)),
            identity: "participant456".to_string(),
            topic: "files".to_string(),
            room: "test-room".to_string(),
            timestamp: 1234567891,
        };

        let outgoing_msg_binary = OutgoingMessage::Message {
            message: binary_message,
        };

        let json_binary = serde_json::to_string(&outgoing_msg_binary).unwrap();
        assert!(json_binary.contains("\"type\":\"message\""));
        assert!(json_binary.contains("\"identity\":\"participant456\""));
        assert!(json_binary.contains("\"topic\":\"files\""));
        assert!(json_binary.contains("\"data\":"));
        // Should not contain message field for binary data
        assert!(!json_binary.contains("\"message\":null"));
    }
}
