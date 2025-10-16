//! # WebSocket Voice Handler Module
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
//! - `{"type": "tts_playback_complete", "timestamp": 1234567890}` - TTS audio generation completed
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
//!       case 'tts_playback_complete':
//!         // Handle TTS playback completion
//!         const completionTime = new Date(message.timestamp);
//!         console.log('TTS playback completed at:', completionTime);
//!         // Calculate server-side latency
//!         const latency = Date.now() - message.timestamp;
//!         console.log('Completion latency:', latency, 'ms');
//!         // Update UI (hide loading spinner, enable next action, etc.)
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
//!                     Some("tts_playback_complete") => {
//!                         let timestamp = parsed["timestamp"].as_u64().unwrap_or(0);
//!                         println!("TTS playback completed at timestamp: {}", timestamp);
//!                         // Calculate latency if needed
//!                         let now = std::time::SystemTime::now()
//!                             .duration_since(std::time::UNIX_EPOCH)
//!                             .unwrap()
//!                             .as_millis() as u64;
//!                         let latency = now.saturating_sub(timestamp);
//!                         println!("Completion latency: {} ms", latency);
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

pub mod audio_handler;
pub mod command_handler;
pub mod config;
pub mod config_handler;
pub mod error;
pub mod handler;
pub mod messages;
pub mod processor;
pub mod state;

#[cfg(test)]
mod tests;

// Re-export commonly used items
pub use config::{LiveKitWebSocketConfig, STTWebSocketConfig, TTSWebSocketConfig};
pub use handler::ws_voice_handler;
pub use messages::{IncomingMessage, OutgoingMessage, ParticipantDisconnectedInfo, UnifiedMessage};
pub use state::ConnectionState;
