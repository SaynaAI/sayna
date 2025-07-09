//! # WebSocket Voice Handler
//!
//! This module provides a WebSocket interface for real-time voice processing using the VoiceManager.
//! It supports STT (Speech-to-Text) and TTS (Text-to-Speech) operations through simple WebSocket messages.
//!
//! ## WebSocket API
//!
//! ### Connection Flow
//! 1. Client connects to `/ws/voice` endpoint
//! 2. Client sends configuration message to initialize STT and TTS providers
//! 3. Server responds with "ready" message when both providers are connected
//! 4. Client can then send audio data, speak commands, or flush commands
//! 5. Server sends back STT results and TTS audio data
//!
//! ### Message Types
//!
//! **Incoming Messages:**
//! - `{"type": "config", "stt_config": {...}, "tts_config": {...}}` - Initialize voice providers
//! - `{"type": "audio", "data": "base64_audio_data"}` - Send audio for transcription
//! - `{"type": "speak", "text": "Hello world", "flush": true}` - Synthesize speech from text (flush is optional, defaults to true)
//! - `{"type": "clear"}` - Clear pending TTS audio and clear queue
//! - Binary messages are treated as raw audio data
//!
//! **Outgoing Messages:**
//! - `{"type": "ready"}` - Voice providers are ready for use
//! - `{"type": "stt_result", "transcript": "...", "is_final": true, "confidence": 0.95}` - STT result
//! - `{"type": "tts_audio", "data": "base64_audio_data", "format": "pcm", "sample_rate": 22050}` - TTS audio
//! - `{"type": "error", "message": "error description"}` - Error occurred
//! - Binary messages contain raw TTS audio data
//!
//! ## JavaScript Client Example
//!
//! ```javascript
//! // Connect to the WebSocket
//! const ws = new WebSocket('ws://localhost:3000/ws/voice');
//!
//! // Configuration for STT and TTS providers
//! const config = {
//!   type: 'config',
//!   stt_config: {
//!     provider: 'deepgram',
//!     api_key: 'your-deepgram-stt-key',
//!     language: 'en-US',
//!     sample_rate: 16000,
//!     channels: 1,
//!     punctuation: true
//!   },
//!   tts_config: {
//!     provider: 'deepgram',
//!     api_key: 'your-deepgram-tts-key',
//!     voice_id: 'aura-luna-en',
//!     speaking_rate: 1.0,
//!     audio_format: 'pcm',
//!     sample_rate: 22050,
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
//!   const message = JSON.parse(event.data);
//!   
//!   switch (message.type) {
//!     case 'ready':
//!       console.log('Voice providers are ready');
//!       // Start sending audio or text
//!       break;
//!       
//!     case 'stt_result':
//!       console.log('STT Result:', message.transcript);
//!       if (message.is_final) {
//!         console.log('Final transcription:', message.transcript);
//!       }
//!       break;
//!       
//!     case 'tts_audio':
//!       console.log('Received TTS audio:', message.data.length, 'bytes');
//!       // Decode base64 and play audio
//!       const audioData = atob(message.data);
//!       // Play audio through Web Audio API
//!       break;
//!       
//!     case 'error':
//!       console.error('Error:', message.message);
//!       break;
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
//! // Queue multiple messages and flush at once
//! function queueSpeak(text) {
//!   const message = {
//!     type: 'speak',
//!     text: text,
//!     flush: false  // Queue without immediate processing
//!   };
//!   ws.send(JSON.stringify(message));
//! }
//!
//! function flushQueue() {
//!   // Flush any queued messages by calling the flush method on voice manager
//!   // This will be handled by a separate flush command if needed
//! }
//!
//! // Send audio data for transcription
//! function sendAudio(audioBuffer) {
//!   // Convert audio to base64 for JSON message
//!   const base64Audio = btoa(String.fromCharCode(...new Uint8Array(audioBuffer)));
//!   const message = {
//!     type: 'audio',
//!     data: base64Audio
//!   };
//!   ws.send(JSON.stringify(message));
//!   
//!   // Or send as binary message (more efficient)
//!   // ws.send(audioBuffer);
//! }
//!
//! // Clear TTS queue
//! function clearTTS() {
//!   const message = { type: 'clear' };
//!   ws.send(JSON.stringify(message));
//! }
//!
//! // Example usage
//! setTimeout(() => {
//!   // Send multiple queued messages
//!   speak('Hello, this is the first message', false);  // Queue without flush
//!   speak('This is the second message', false);        // Queue without flush
//!   speak('This is the final message', true);          // Send and flush all
//!   
//!   // Send some mock audio data
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
//!     // Send configuration
//!     let config = json!({
//!         "type": "config",
//!         "stt_config": {
//!             "provider": "deepgram",
//!             "api_key": "your-deepgram-stt-key",
//!             "language": "en-US",
//!             "sample_rate": 16000,
//!             "channels": 1,
//!             "punctuation": true
//!         },
//!         "tts_config": {
//!             "provider": "deepgram",
//!             "api_key": "your-deepgram-tts-key",
//!             "voice_id": "aura-luna-en",
//!             "speaking_rate": 1.0,
//!             "audio_format": "pcm",
//!             "sample_rate": 22050,
//!             "connection_timeout": 30,
//!             "request_timeout": 60
//!         }
//!     });
//!
//!     write.send(Message::Text(config.to_string())).await?;
//!
//!     // Handle incoming messages
//!     while let Some(message) = read.next().await {
//!         match message? {
//!             Message::Text(text) => {
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
//!                         write.send(Message::Text(speak_msg.to_string())).await?;
//!                     }
//!                     Some("stt_result") => {
//!                         println!("STT Result: {}", parsed["transcript"]);
//!                     }
//!                     Some("tts_audio") => {
//!                         println!("Received TTS audio: {} bytes",
//!                                  parsed["data"].as_str().unwrap_or("").len());
//!                     }
//!                     Some("error") => {
//!                         eprintln!("Error: {}", parsed["message"]);
//!                     }
//!                     _ => {}
//!                 }
//!             }
//!             Message::Binary(data) => {
//!                 println!("Received binary audio data: {} bytes", data.len());
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
//! - **Binary Messages**: Use binary WebSocket messages for audio data for better performance
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
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info, warn};

use crate::{
    core::{
        stt::{STTConfig, STTResult},
        tts::{AudioData, TTSConfig},
        voice_manager::{VoiceManager, VoiceManagerConfig},
    },
    state::AppState,
};

/// WebSocket message types for incoming messages
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum IncomingMessage {
    #[serde(rename = "config")]
    Config {
        stt_config: STTConfig,
        tts_config: TTSConfig,
    },
    #[serde(rename = "audio")]
    Audio {
        #[serde(with = "base64_serde")]
        data: Vec<u8>,
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
    #[serde(rename = "tts_audio")]
    TTSAudio {
        #[serde(with = "base64_serde")]
        data: Vec<u8>,
        format: String,
        sample_rate: u32,
        duration_ms: Option<u32>,
    },
    #[serde(rename = "error")]
    Error { message: String },
}

/// Base64 serialization helper
mod base64_serde {
    use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(data: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        BASE64.encode(data).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        BASE64
            .decode(encoded)
            .map_err(|e| serde::de::Error::custom(format!("Invalid base64: {}", e)))
    }
}

/// WebSocket connection state
struct ConnectionState {
    voice_manager: Option<Arc<VoiceManager>>,
    tts_audio_queue: VecDeque<AudioData>,
    is_flushing: bool,
}

impl ConnectionState {
    fn new() -> Self {
        Self {
            voice_manager: None,
            tts_audio_queue: VecDeque::new(),
            is_flushing: false,
        }
    }
}

/// WebSocket voice processing handler
/// Upgrades the HTTP connection to WebSocket for real-time voice processing
pub async fn ws_voice_handler(
    ws: WebSocketUpgrade,
    State(_state): State<Arc<AppState>>,
) -> Response {
    info!("WebSocket voice connection upgrade requested");
    ws.on_upgrade(handle_voice_socket)
}

/// Handle WebSocket voice connection
/// This function manages the entire WebSocket session for voice processing
async fn handle_voice_socket(socket: WebSocket) {
    info!("WebSocket voice connection established");

    // Split the socket into sender and receiver
    let (mut sender, mut receiver) = socket.split();

    // Connection state
    let state = Arc::new(RwLock::new(ConnectionState::new()));

    // Channel for outgoing messages
    let (outgoing_tx, mut outgoing_rx) = mpsc::unbounded_channel::<OutgoingMessage>();

    // Spawn task to handle outgoing messages
    let sender_task = {
        let _outgoing_tx = outgoing_tx.clone();
        tokio::spawn(async move {
            while let Some(message) = outgoing_rx.recv().await {
                let json_message = match serde_json::to_string(&message) {
                    Ok(json) => json,
                    Err(e) => {
                        error!("Failed to serialize outgoing message: {}", e);
                        continue;
                    }
                };

                if let Err(e) = sender.send(Message::Text(json_message.into())).await {
                    error!("Failed to send WebSocket message: {}", e);
                    break;
                }
            }
        })
    };

    // Process incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(msg) => {
                let should_continue = process_message(msg, &state, &outgoing_tx).await;
                if !should_continue {
                    break;
                }
            }
            Err(e) => {
                warn!("WebSocket error: {}", e);
                let _ = outgoing_tx.send(OutgoingMessage::Error {
                    message: format!("WebSocket error: {}", e),
                });
                break;
            }
        }
    }

    // Clean up
    sender_task.abort();

    // Stop voice manager if it exists
    if let Some(voice_manager) = &state.read().await.voice_manager {
        if let Err(e) = voice_manager.stop().await {
            error!("Failed to stop voice manager: {}", e);
        }
    }

    info!("WebSocket voice connection terminated");
}

/// Process incoming WebSocket message
async fn process_message(
    msg: Message,
    state: &Arc<RwLock<ConnectionState>>,
    outgoing_tx: &mpsc::UnboundedSender<OutgoingMessage>,
) -> bool {
    match msg {
        Message::Text(text) => {
            debug!("Received text message: {}", text);

            let incoming_msg: IncomingMessage = match serde_json::from_str(&text) {
                Ok(msg) => msg,
                Err(e) => {
                    error!("Failed to parse incoming message: {}", e);
                    let _ = outgoing_tx.send(OutgoingMessage::Error {
                        message: format!("Invalid message format: {}", e),
                    });
                    return true;
                }
            };

            handle_incoming_message(incoming_msg, state, outgoing_tx).await
        }
        Message::Binary(data) => {
            debug!("Received binary message: {} bytes", data.len());

            // Treat binary messages as raw audio data
            let audio_msg = IncomingMessage::Audio {
                data: data.to_vec(),
            };
            handle_incoming_message(audio_msg, state, outgoing_tx).await
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

/// Handle parsed incoming message
async fn handle_incoming_message(
    msg: IncomingMessage,
    state: &Arc<RwLock<ConnectionState>>,
    outgoing_tx: &mpsc::UnboundedSender<OutgoingMessage>,
) -> bool {
    match msg {
        IncomingMessage::Config {
            stt_config,
            tts_config,
        } => handle_config_message(stt_config, tts_config, state, outgoing_tx).await,
        IncomingMessage::Audio { data } => handle_audio_message(data, state, outgoing_tx).await,
        IncomingMessage::Speak { text, flush } => {
            handle_speak_message(text, flush, state, outgoing_tx).await
        }
        IncomingMessage::Clear => handle_clear_message(state, outgoing_tx).await,
    }
}

/// Handle configuration message
async fn handle_config_message(
    stt_config: STTConfig,
    mut tts_config: TTSConfig,
    state: &Arc<RwLock<ConnectionState>>,
    outgoing_tx: &mpsc::UnboundedSender<OutgoingMessage>,
) -> bool {
    info!(
        "Configuring voice manager with STT provider: {} and TTS provider: {}",
        stt_config.provider, tts_config.provider
    );

    // WORKAROUND: Force correct TTS config values
    if tts_config.provider == "deepgram" {
        if tts_config.voice_id.is_none() {
            tts_config.voice_id = Some("aura-asteria-en".to_string());
        }
        if tts_config.audio_format.is_none() || tts_config.audio_format == Some("pcm".to_string()) {
            tts_config.audio_format = Some("linear16".to_string());
        }
        if tts_config.sample_rate.is_none() || tts_config.sample_rate == Some(22050) {
            tts_config.sample_rate = Some(24000);
        }
        info!("Applied TTS config workaround: {:?}", tts_config);
    }

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
            let _ = outgoing_tx.send(OutgoingMessage::Error {
                message: format!("Failed to create voice manager: {}", e),
            });
            return true;
        }
    };

    // Start voice manager
    if let Err(e) = voice_manager.start().await {
        error!("Failed to start voice manager: {}", e);
        let _ = outgoing_tx.send(OutgoingMessage::Error {
            message: format!("Failed to start voice manager: {}", e),
        });
        return true;
    }

    // Set up STT callback
    let outgoing_tx_clone = outgoing_tx.clone();
    if let Err(e) = voice_manager
        .on_stt_result(move |result: STTResult| {
            let outgoing_tx = outgoing_tx_clone.clone();
            Box::pin(async move {
                let msg = OutgoingMessage::STTResult {
                    transcript: result.transcript,
                    is_final: result.is_final,
                    is_speech_final: result.is_speech_final,
                    confidence: result.confidence,
                };
                let _ = outgoing_tx.send(msg);
            })
        })
        .await
    {
        error!("Failed to set up STT callback: {}", e);
        let _ = outgoing_tx.send(OutgoingMessage::Error {
            message: format!("Failed to set up STT callback: {}", e),
        });
        return true;
    }

    // Set up TTS audio callback
    let state_clone = state.clone();
    let outgoing_tx_clone = outgoing_tx.clone();
    if let Err(e) = voice_manager
        .on_tts_audio(move |audio_data: AudioData| {
            let state = state_clone.clone();
            let outgoing_tx = outgoing_tx_clone.clone();
            Box::pin(async move {
                let mut connection_state = state.write().await;

                // If flushing, add to queue instead of sending immediately
                if connection_state.is_flushing {
                    connection_state.tts_audio_queue.push_back(audio_data);
                } else {
                    // Send immediately
                    let msg = OutgoingMessage::TTSAudio {
                        data: audio_data.data,
                        format: audio_data.format,
                        sample_rate: audio_data.sample_rate,
                        duration_ms: audio_data.duration_ms,
                    };
                    let _ = outgoing_tx.send(msg);
                }
            })
        })
        .await
    {
        error!("Failed to set up TTS audio callback: {}", e);
        let _ = outgoing_tx.send(OutgoingMessage::Error {
            message: format!("Failed to set up TTS audio callback: {}", e),
        });
        return true;
    }

    // Set up TTS error callback
    let outgoing_tx_clone = outgoing_tx.clone();
    if let Err(e) = voice_manager
        .on_tts_error(move |error| {
            let outgoing_tx = outgoing_tx_clone.clone();
            Box::pin(async move {
                let msg = OutgoingMessage::Error {
                    message: format!("TTS error: {}", error),
                };
                let _ = outgoing_tx.send(msg);
            })
        })
        .await
    {
        error!("Failed to set up TTS error callback: {}", e);
        let _ = outgoing_tx.send(OutgoingMessage::Error {
            message: format!("Failed to set up TTS error callback: {}", e),
        });
        return true;
    }

    // Wait for providers to be ready
    let timeout = tokio::time::Duration::from_secs(30);
    let start_time = tokio::time::Instant::now();

    while !voice_manager.is_ready().await {
        if start_time.elapsed() > timeout {
            error!("Timeout waiting for voice providers to be ready");
            let _ = outgoing_tx.send(OutgoingMessage::Error {
                message: "Timeout waiting for voice providers to be ready".to_string(),
            });
            return true;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Store voice manager in state
    {
        let mut connection_state = state.write().await;
        connection_state.voice_manager = Some(voice_manager);
    }

    // Send ready message
    let _ = outgoing_tx.send(OutgoingMessage::Ready);
    info!("Voice manager ready and configured");

    true
}

/// Handle audio input message
async fn handle_audio_message(
    audio_data: Vec<u8>,
    state: &Arc<RwLock<ConnectionState>>,
    outgoing_tx: &mpsc::UnboundedSender<OutgoingMessage>,
) -> bool {
    debug!("Processing audio data: {} bytes", audio_data.len());

    let voice_manager = {
        let connection_state = state.read().await;
        match &connection_state.voice_manager {
            Some(vm) => vm.clone(),
            None => {
                let _ = outgoing_tx.send(OutgoingMessage::Error {
                    message: "Voice manager not configured. Send config message first.".to_string(),
                });
                return true;
            }
        }
    };

    // Send audio to STT provider
    if let Err(e) = voice_manager.receive_audio(audio_data).await {
        error!("Failed to process audio: {}", e);
        let _ = outgoing_tx.send(OutgoingMessage::Error {
            message: format!("Failed to process audio: {}", e),
        });
    }

    true
}

/// Handle speak command message
async fn handle_speak_message(
    text: String,
    flush: Option<bool>,
    state: &Arc<RwLock<ConnectionState>>,
    outgoing_tx: &mpsc::UnboundedSender<OutgoingMessage>,
) -> bool {
    // Default flush to true for backward compatibility
    let should_flush = flush.unwrap_or(true);
    debug!(
        "Processing speak command: {} (flush: {})",
        text, should_flush
    );

    let voice_manager = {
        let connection_state = state.read().await;
        match &connection_state.voice_manager {
            Some(vm) => vm.clone(),
            None => {
                let _ = outgoing_tx.send(OutgoingMessage::Error {
                    message: "Voice manager not configured. Send config message first.".to_string(),
                });
                return true;
            }
        }
    };

    // Send text to TTS provider with flush parameter
    if let Err(e) = voice_manager.speak(&text, should_flush).await {
        error!("Failed to synthesize speech: {}", e);
        let _ = outgoing_tx.send(OutgoingMessage::Error {
            message: format!("Failed to synthesize speech: {}", e),
        });
    } else {
        debug!(
            "Speech synthesis started for: {} (flush: {})",
            text, should_flush
        );
    }

    true
}

/// Handle flush command message
async fn handle_clear_message(
    state: &Arc<RwLock<ConnectionState>>,
    outgoing_tx: &mpsc::UnboundedSender<OutgoingMessage>,
) -> bool {
    debug!("Processing flush command");

    let voice_manager = {
        let mut connection_state = state.write().await;
        connection_state.is_flushing = true;

        match &connection_state.voice_manager {
            Some(vm) => vm.clone(),
            None => {
                let _ = outgoing_tx.send(OutgoingMessage::Error {
                    message: "Voice manager not configured. Send config message first.".to_string(),
                });
                return true;
            }
        }
    };

    // Clear TTS provider
    if let Err(e) = voice_manager.clear_tts().await {
        error!("Failed to clear TTS provider: {}", e);
        let _ = outgoing_tx.send(OutgoingMessage::Error {
            message: format!("Failed to clear TTS provider: {}", e),
        });
    }

    // Clear the audio queue and stop flushing
    {
        let mut connection_state = state.write().await;
        connection_state.tts_audio_queue.clear();
        connection_state.is_flushing = false;
    }

    debug!("Flush command completed");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{stt::STTConfig, tts::TTSConfig};

    #[test]
    fn test_incoming_message_serialization() {
        // Test config message
        let config_msg = IncomingMessage::Config {
            stt_config: STTConfig {
                provider: "deepgram".to_string(),
                api_key: "test_key".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                channels: 1,
                punctuation: true,
            },
            tts_config: TTSConfig {
                provider: "deepgram".to_string(),
                api_key: "test_key".to_string(),
                voice_id: Some("aura-luna-en".to_string()),
                speaking_rate: Some(1.0),
                audio_format: Some("pcm".to_string()),
                sample_rate: Some(22050),
                connection_timeout: Some(30),
                request_timeout: Some(60),
            },
        };

        let json = serde_json::to_string(&config_msg).unwrap();
        assert!(json.contains("\"type\":\"config\""));

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
    fn test_base64_audio_encoding() {
        let audio_data = vec![1, 2, 3, 4, 5];
        let msg = IncomingMessage::Audio {
            data: audio_data.clone(),
        };

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"audio\""));

        // Deserialize back
        let deserialized: IncomingMessage = serde_json::from_str(&json).unwrap();
        if let IncomingMessage::Audio { data } = deserialized {
            assert_eq!(data, audio_data);
        } else {
            panic!("Expected Audio message");
        }
    }
}
