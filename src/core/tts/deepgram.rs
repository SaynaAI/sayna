//! # Deepgram TTS Implementation
//!
//! This module provides a Deepgram Text-to-Speech implementation using WebSocket
//! streaming API. It implements the BaseTTS trait for unified TTS provider interface.
//!
//! ## Features
//!
//! - Real-time streaming TTS via WebSocket
//! - Multiple audio formats (mp3, wav, pcm, etc.)
//! - Configurable voice models and settings
//! - Async/await support with proper error handling
//! - Audio callback system for real-time audio processing
//! - Optimized for low-latency operation
//!
//! ## Example Usage
//!
//! ```rust,no_run
//! use sayna::core::tts::{BaseTTS, TTSConfig, AudioCallback};
//! use sayna::core::tts::deepgram::DeepgramTTS;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = TTSConfig {
//!         voice_id: Some("aura-asteria-en".to_string()),
//!         audio_format: Some("mp3".to_string()),
//!         sample_rate: Some(24000),
//!         ..Default::default()
//!     };
//!     
//!     let mut tts = DeepgramTTS::new(config)?;
//!     tts.connect().await?;
//!     tts.speak("Hello from Deepgram!", true).await?;
//!     tts.disconnect().await?;
//!     
//!     Ok(())
//! }
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        handshake::client::generate_key,
        http::Request,
        http::header::{AUTHORIZATION, USER_AGENT},
        protocol::Message,
    },
};
use tracing::{debug, error, info, warn};
use url::Url;

use super::base::{
    AudioCallback, AudioData, BaseTTS, ConnectionState, TTSConfig, TTSError, TTSResult,
};

/// Pre-allocated string constants for JSON messages (avoids runtime string formatting)
const SPEAK_PREFIX: &str = r#"{"type":"Speak","text":""#;
const SPEAK_SUFFIX: &str = r#""}"#;
const FLUSH_MESSAGE: &str = r#"{"type":"Flush"}"#;
const CLEAR_MESSAGE: &str = r#"{"type":"Clear"}"#;
const CLOSE_MESSAGE: &str = r#"{"type":"Close"}"#;

/// Optimized message types using compact representations
#[derive(Debug)]
pub enum TTSCommand {
    /// Send text to synthesize - uses String directly to avoid enum size overhead
    Speak(String, bool), // text, flush_immediately
    /// Clear the synthesis queue
    Clear,
    /// Flush the synthesis queue
    Flush,
    /// Close the connection
    Close,
}

/// Response message types optimized for parsing
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DeepgramTTSResponse {
    /// Audio data response
    Audio { data: String },
    /// Control message response
    Control { message: String },
    /// Error response
    Error { error: String },
    /// Close frame response
    Close { reason: String },
}

/// WebSocket connection wrapper
type WebSocketConnection = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Reusable message buffer to avoid allocations
#[derive(Default)]
struct MessageBuffer {
    json_buffer: String,
}

impl MessageBuffer {
    fn format_speak_message(&mut self, text: &str) -> &str {
        self.json_buffer.clear();
        self.json_buffer
            .reserve(SPEAK_PREFIX.len() + text.len() + SPEAK_SUFFIX.len());
        self.json_buffer.push_str(SPEAK_PREFIX);

        // Escape JSON string manually for better performance
        for c in text.chars() {
            match c {
                '"' => self.json_buffer.push_str("\\\""),
                '\\' => self.json_buffer.push_str("\\\\"),
                '\n' => self.json_buffer.push_str("\\n"),
                '\r' => self.json_buffer.push_str("\\r"),
                '\t' => self.json_buffer.push_str("\\t"),
                _ => self.json_buffer.push(c),
            }
        }

        self.json_buffer.push_str(SPEAK_SUFFIX);
        &self.json_buffer
    }
}

/// Optimized connection state using atomic for lockless access
#[derive(Default)]
struct ConnectionStateAtomic {
    connected: AtomicBool,
    error: RwLock<Option<String>>,
}

impl ConnectionStateAtomic {
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    fn set_connected(&self, connected: bool) {
        self.connected.store(connected, Ordering::Relaxed);
    }

    async fn set_error(&self, error: String) {
        self.set_connected(false);
        *self.error.write().await = Some(error);
    }

    #[cfg(test)]
    async fn get_connection_state(&self) -> ConnectionState {
        if self.is_connected() {
            ConnectionState::Connected
        } else if let Some(error) = self.error.read().await.as_ref() {
            ConnectionState::Error(error.clone())
        } else {
            ConnectionState::Disconnected
        }
    }
}

/// Deepgram TTS provider implementation optimized for low latency
pub struct DeepgramTTS {
    /// Optimized connection state with atomic operations
    state: Arc<ConnectionStateAtomic>,
    /// WebSocket connection
    connection: Arc<RwLock<Option<WebSocketConnection>>>,
    /// Audio callback for handling audio data
    audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
    /// Configuration
    config: TTSConfig,
    /// Message sender for WebSocket - bounded channel for backpressure
    message_sender: Arc<RwLock<Option<mpsc::Sender<TTSCommand>>>>,
    /// Task handles for cleanup
    task_handles: Arc<RwLock<Vec<tokio::task::JoinHandle<()>>>>,
    /// Reusable message buffer
    message_buffer: Arc<RwLock<MessageBuffer>>,
}

impl DeepgramTTS {
    /// Create a new Deepgram TTS instance
    pub fn new(config: TTSConfig) -> TTSResult<Self> {
        Ok(Self {
            state: Arc::new(ConnectionStateAtomic::default()),
            connection: Arc::new(RwLock::new(None)),
            audio_callback: Arc::new(RwLock::new(None)),
            config,
            message_sender: Arc::new(RwLock::new(None)),
            task_handles: Arc::new(RwLock::new(Vec::new())),
            message_buffer: Arc::new(RwLock::new(MessageBuffer::default())),
        })
    }

    /// Build the WebSocket URL with query parameters
    fn build_websocket_url(config: &TTSConfig) -> TTSResult<Url> {
        let mut url = Url::parse("wss://api.deepgram.com/v1/speak")
            .map_err(|e| TTSError::InvalidConfiguration(format!("Invalid base URL: {e}")))?;

        // Pre-allocate query parameters to avoid reallocations
        {
            let mut pairs = url.query_pairs_mut();

            if let Some(encoding) = config.audio_format.as_ref() {
                pairs.append_pair("encoding", encoding);
            }

            if let Some(sample_rate) = config.sample_rate {
                pairs.append_pair("sample_rate", &sample_rate.to_string());
            }

            if let Some(model) = config.voice_id.as_ref() {
                pairs.append_pair("model", model);
            }
        }

        Ok(url)
    }

    /// Get authorization header value
    fn get_auth_header(config: &TTSConfig) -> TTSResult<String> {
        if !config.api_key.is_empty() {
            return Ok(format!("token {}", config.api_key));
        }

        Err(TTSError::InvalidConfiguration(
            "Missing API key in provider configuration".to_string(),
        ))
    }

    /// Handle incoming WebSocket messages - optimized for minimal allocations
    async fn handle_websocket_message(
        message: Message,
        audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
        config: &TTSConfig,
    ) {
        match message {
            Message::Binary(data) => {
                // Deepgram TTS sends raw binary audio data
                debug!("Received binary audio data: {} bytes", data.len());

                if let Some(callback) = audio_callback.read().await.as_ref() {
                    let audio_format = config.audio_format.as_deref().unwrap_or("linear16");

                    let sample_rate = config.sample_rate.unwrap_or(24000);

                    let audio_data = AudioData {
                        data: data.to_vec(), // Convert Bytes to Vec<u8>
                        sample_rate,
                        format: audio_format.to_string(),
                        duration_ms: None,
                    };

                    callback.on_audio(audio_data).await;
                }
            }
            Message::Text(text) => {
                // Handle text control messages with minimal parsing
                debug!("Received text message: {}", text);

                // Fast path for common control messages
                if text.starts_with("{\"type\":\"Error\"") {
                    if let Some(callback) = audio_callback.read().await.as_ref() {
                        // Extract error message without full JSON parsing if possible
                        let error_msg = if let Some(start) = text.find("\"error\":\"") {
                            let start = start + 9;
                            if let Some(end) = text[start..].find("\"") {
                                &text[start..start + end]
                            } else {
                                "Unknown error"
                            }
                        } else {
                            "Unknown error"
                        };

                        let tts_error = TTSError::ProviderError(error_msg.to_string());
                        callback.on_error(tts_error).await;
                    }
                } else if text.starts_with("{\"type\":\"Close\"") {
                    info!("WebSocket closed by server");
                    if let Some(callback) = audio_callback.read().await.as_ref() {
                        callback.on_complete().await;
                    }
                } else {
                    // Full JSON parsing only when necessary
                    if let Ok(response) = serde_json::from_str::<DeepgramTTSResponse>(&text) {
                        match response {
                            DeepgramTTSResponse::Control { message } => {
                                debug!("Control message: {}", message);
                            }
                            DeepgramTTSResponse::Audio { data: _ } => {
                                // This shouldn't happen with the new API
                                warn!("Received unexpected JSON audio data");
                            }
                            _ => {} // Already handled above
                        }
                    }
                }
            }
            Message::Close(_) => {
                info!("WebSocket connection closed");
                if let Some(callback) = audio_callback.read().await.as_ref() {
                    callback.on_complete().await;
                }
            }
            Message::Ping(_) => {
                debug!("Received ping");
            }
            Message::Pong(_) => {
                debug!("Received pong");
            }
            Message::Frame(_) => {
                debug!("Received raw frame");
            }
        }
    }

    /// Start the WebSocket message handling task - optimized for low latency
    async fn start_message_handler(
        mut ws_stream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
        audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
        config: TTSConfig,
        mut message_receiver: mpsc::Receiver<TTSCommand>,
        state: Arc<ConnectionStateAtomic>,
        message_buffer: Arc<RwLock<MessageBuffer>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            debug!("TTS WebSocket message handler started");

            state.set_connected(true);

            // Create channel for delayed messages
            let (delayed_tx, mut delayed_rx) = mpsc::unbounded_channel::<String>();
            let delay_ms = 500;

            loop {
                tokio::select! {
                    // Handle incoming messages from WebSocket
                    message = ws_stream.next() => {
                        match message {
                            Some(Ok(msg)) => {
                                Self::handle_websocket_message(
                                    msg,
                                    audio_callback.clone(),
                                    &config,
                                ).await;
                            }
                            Some(Err(e)) => {
                                error!("WebSocket error: {}", e);
                                state.set_error(e.to_string()).await;
                                break;
                            }
                            None => {
                                info!("WebSocket stream ended");
                                break;
                            }
                        }
                    }
                    // Handle outgoing messages to WebSocket
                    command = message_receiver.recv() => {
                        match command {
                            Some(cmd) => {
                                debug!("Processing TTS command: {:?}", cmd);

                                let message_text = match cmd {
                                    TTSCommand::Speak(text, flush_immediately) => {
                                        let mut buffer = message_buffer.write().await;
                                        let json_msg = buffer.format_speak_message(&text);
                                        let msg = json_msg.to_string();
                                        drop(buffer); // Release lock early

                                        debug!("Sending message TEXT: {}", msg);

                                        if let Err(e) = ws_stream.send(Message::Text(msg.into())).await {
                                            error!("Failed to send speak message: {}", e);
                                            state.set_error(e.to_string()).await;
                                            break;
                                        }

                                        // Send flush with delay if requested
                                        if flush_immediately {
                                            debug!("Scheduling flush message with 500ms delay");

                                            // Spawn task to send flush message with delay
                                            let delayed_sender = delayed_tx.clone();
                                            let flush_message = FLUSH_MESSAGE.to_string();
                                            tokio::spawn(async move {
                                                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                                                if let Err(_) = delayed_sender.send(flush_message) {
                                                    debug!("Failed to send delayed flush message - receiver closed");
                                                }
                                            });
                                        }
                                        continue;
                                    }
                                    TTSCommand::Clear => CLEAR_MESSAGE,
                                    TTSCommand::Flush => FLUSH_MESSAGE,
                                    TTSCommand::Close => CLOSE_MESSAGE,
                                };

                                debug!("Scheduling message control command with 500ms delay: {}", message_text);

                                let delayed_sender = delayed_tx.clone();
                                let message_text_clone = message_text.to_string();
                                tokio::spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                                    if let Err(_) = delayed_sender.send(message_text_clone) {
                                        debug!("Failed to send delayed message - receiver closed");
                                    }
                                });
                            }
                            None => {
                                info!("Message receiver closed");
                                break;
                            }
                        }
                    }
                    // Handle delayed messages
                    delayed_message = delayed_rx.recv() => {
                        if let Some(message_text) = delayed_message {
                            debug!("Sending delayed message control command: {}", message_text);
                            if let Err(e) = ws_stream.send(Message::Text(message_text.into())).await {
                                error!("Failed to send delayed message: {}", e);
                                state.set_error(e.to_string()).await;
                                break;
                            }
                        }
                    }
                }
            }

            // Clean up connection state
            state.set_connected(false);
            debug!("TTS WebSocket message handler ended");
        })
    }
}

impl Default for DeepgramTTS {
    fn default() -> Self {
        Self::new(TTSConfig::default()).unwrap()
    }
}

#[async_trait]
impl BaseTTS for DeepgramTTS {
    fn new(config: TTSConfig) -> TTSResult<Self> {
        Ok(Self {
            state: Arc::new(ConnectionStateAtomic::default()),
            connection: Arc::new(RwLock::new(None)),
            audio_callback: Arc::new(RwLock::new(None)),
            config,
            message_sender: Arc::new(RwLock::new(None)),
            task_handles: Arc::new(RwLock::new(Vec::new())),
            message_buffer: Arc::new(RwLock::new(MessageBuffer::default())),
        })
    }

    async fn connect(&mut self) -> TTSResult<()> {
        // Build WebSocket URL
        let url = Self::build_websocket_url(&self.config)?;
        debug!("Connecting to Deepgram TTS WebSocket: {}", url);

        // Get authorization header
        let auth_header = Self::get_auth_header(&self.config)?;
        debug!("Using auth header: {}", auth_header);

        // Create request with WebSocket handshake headers
        let host = url.host_str().unwrap_or("api.deepgram.com");

        let request = Request::builder()
            .method("GET")
            .uri(url.as_str())
            .header("Host", host)
            .header("Upgrade", "websocket")
            .header("Connection", "upgrade")
            .header("Sec-WebSocket-Key", generate_key())
            .header("Sec-WebSocket-Version", "13")
            .header(AUTHORIZATION, auth_header)
            .header(USER_AGENT, "sayna-tts/1.0")
            .body(())
            .map_err(|e| TTSError::ConnectionFailed(format!("Failed to create request: {e}")))?;

        debug!("WebSocket request created successfully");

        // Connect to WebSocket
        let (ws_stream, response) = connect_async(request)
            .await
            .map_err(|e| TTSError::ConnectionFailed(format!("WebSocket connection failed: {e}")))?;

        info!(
            "WebSocket connection established, response status: {:?}",
            response.status()
        );

        // Use bounded channel for backpressure control (prevents memory buildup)
        let (message_sender, message_receiver) = mpsc::channel(32);

        // Start message handler task
        let task_handle = Self::start_message_handler(
            ws_stream,
            self.audio_callback.clone(),
            self.config.clone(),
            message_receiver,
            self.state.clone(),
            self.message_buffer.clone(),
        )
        .await;

        // Store task handle for cleanup
        self.task_handles.write().await.push(task_handle);

        // Store message sender
        *self.message_sender.write().await = Some(message_sender);

        info!("Deepgram TTS WebSocket connection ready");

        Ok(())
    }

    async fn disconnect(&mut self) -> TTSResult<()> {
        // Send close message if connected
        if let Some(sender) = self.message_sender.read().await.as_ref() {
            let _ = sender.send(TTSCommand::Close).await;
        }

        // Clean up task handles
        let mut handles = self.task_handles.write().await;
        for handle in handles.drain(..) {
            handle.abort();
        }

        // Clear state
        self.state.set_connected(false);
        *self.connection.write().await = None;
        *self.message_sender.write().await = None;
        *self.audio_callback.write().await = None;

        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.state.is_connected()
    }

    fn get_connection_state(&self) -> ConnectionState {
        // Use lockless atomic check for the common case
        if self.state.is_connected() {
            ConnectionState::Connected
        } else {
            // For error state, we need to check but can't use async in this sync method
            // Use try_read to avoid blocking in tests and sync contexts
            if let Ok(error) = self.state.error.try_read() {
                if let Some(err) = error.as_ref() {
                    ConnectionState::Error(err.clone())
                } else {
                    ConnectionState::Disconnected
                }
            } else {
                // If we can't read the error state, assume disconnected
                ConnectionState::Disconnected
            }
        }
    }

    async fn speak(&self, text: &str, flush: bool) -> TTSResult<()> {
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "Deepgram TTS provider not connected".to_string(),
            ));
        }

        let mut command = TTSCommand::Speak(text.to_string(), flush);

        if text.is_empty() || text == " " {
            command = TTSCommand::Flush;
        }

        let sender = self.message_sender.read().await;
        if let Some(sender) = sender.as_ref() {
            sender.send(command).await.map_err(|e| {
                TTSError::InternalError(format!("Failed to send speak command: {e}"))
            })?;

            debug!("Sent speak command for text: {}", text);
        } else {
            return Err(TTSError::ProviderNotReady(
                "Message sender not available".to_string(),
            ));
        }

        Ok(())
    }

    async fn clear(&self) -> TTSResult<()> {
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "Deepgram TTS provider not connected".to_string(),
            ));
        }

        let sender = self.message_sender.read().await;
        if let Some(sender) = sender.as_ref() {
            sender.send(TTSCommand::Clear).await.map_err(|e| {
                TTSError::InternalError(format!("Failed to send clear command: {e}"))
            })?;
        }

        Ok(())
    }

    async fn flush(&self) -> TTSResult<()> {
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "Deepgram TTS provider not connected".to_string(),
            ));
        }

        let sender = self.message_sender.read().await;
        if let Some(sender) = sender.as_ref() {
            sender.send(TTSCommand::Flush).await.map_err(|e| {
                TTSError::InternalError(format!("Failed to send flush command: {e}"))
            })?;
        }

        Ok(())
    }

    fn on_audio(&mut self, callback: Arc<dyn AudioCallback>) -> TTSResult<()> {
        // Use try_write to avoid blocking in tests
        if let Ok(mut audio_callback) = self.audio_callback.try_write() {
            *audio_callback = Some(callback);
            Ok(())
        } else {
            Err(TTSError::InternalError(
                "Failed to register audio callback".to_string(),
            ))
        }
    }

    fn remove_audio_callback(&mut self) -> TTSResult<()> {
        // Use try_write to avoid blocking in tests
        if let Ok(mut audio_callback) = self.audio_callback.try_write() {
            *audio_callback = None;
            Ok(())
        } else {
            Err(TTSError::InternalError(
                "Failed to remove audio callback".to_string(),
            ))
        }
    }

    fn get_provider_info(&self) -> serde_json::Value {
        serde_json::json!({
            "provider": "deepgram",
            "version": "1.0.0",
            "supported_formats": ["mp3", "wav", "pcm", "aac", "flac", "opus"],
            "supported_sample_rates": [8000, 16000, 22050, 24000, 44100, 48000],
            "supported_models": [
                "aura-asteria-en",
                "aura-luna-en",
                "aura-stella-en",
                "aura-athena-en",
                "aura-hera-en",
                "aura-orion-en",
                "aura-arcas-en",
                "aura-perseus-en",
                "aura-angus-en",
                "aura-orpheus-en",
                "aura-helios-en",
                "aura-zeus-en"
            ],
            "websocket_endpoint": "wss://api.deepgram.com/v1/speak",
            "documentation": "https://developers.deepgram.com/reference/text-to-speech-api/speak-streaming",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tts::base::ChannelAudioCallback;
    use std::future::Future;
    use std::pin::Pin;
    // use tokio::time::{Duration, timeout}; // Unused for now

    #[tokio::test]
    async fn test_deepgram_tts_creation() {
        let config = TTSConfig::default();
        let tts = DeepgramTTS::new(config).unwrap();
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_websocket_url_building() {
        let config = TTSConfig {
            audio_format: Some("mp3".to_string()),
            sample_rate: Some(24000),
            voice_id: Some("aura-asteria-en".to_string()),
            ..Default::default()
        };

        let url = DeepgramTTS::build_websocket_url(&config).unwrap();
        assert_eq!(url.scheme(), "wss");
        assert_eq!(url.host_str(), Some("api.deepgram.com"));
        assert_eq!(url.path(), "/v1/speak");

        let query_params: std::collections::HashMap<String, String> =
            url.query_pairs().into_owned().collect();
        assert_eq!(query_params.get("encoding"), Some(&"mp3".to_string()));
        assert_eq!(query_params.get("sample_rate"), Some(&"24000".to_string()));
        assert_eq!(
            query_params.get("model"),
            Some(&"aura-asteria-en".to_string())
        );
    }

    #[tokio::test]
    async fn test_auth_header_generation() {
        let config = TTSConfig {
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let auth_header = DeepgramTTS::get_auth_header(&config).unwrap();
        assert_eq!(auth_header, "token test_key");
    }

    #[tokio::test]
    async fn test_provider_info() {
        let config = TTSConfig::default();
        let tts = DeepgramTTS::new(config).unwrap();
        let info = tts.get_provider_info();

        assert_eq!(info["provider"], "deepgram");
        assert!(info["supported_formats"].is_array());
        assert!(info["supported_models"].is_array());
    }

    #[tokio::test]
    async fn test_callback_registration() {
        let config = TTSConfig::default();
        let mut tts = DeepgramTTS::new(config).unwrap();
        let (callback, _audio_receiver, _error_receiver, _complete_receiver) =
            ChannelAudioCallback::new();

        let result = tts.on_audio(Arc::new(callback));
        assert!(result.is_ok());

        let remove_result = tts.remove_audio_callback();
        assert!(remove_result.is_ok());
    }

    #[tokio::test]
    async fn test_speak_when_not_ready() {
        let config = TTSConfig {
            provider: "deepgram".to_string(),
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let tts = <DeepgramTTS as BaseTTS>::new(config).unwrap();

        // Try to speak when not connected
        let result = tts.speak("Hello, world!", false).await;
        assert!(result.is_err());

        // Check that the error is the expected type
        match result.unwrap_err() {
            TTSError::ProviderNotReady(_) => {}
            _ => panic!("Expected ProviderNotReady error"),
        }
    }

    #[tokio::test]
    async fn test_message_buffer_format() {
        let mut buffer = MessageBuffer::default();

        // Test basic string
        let result = buffer.format_speak_message("Hello world");
        assert_eq!(result, r#"{"type":"Speak","text":"Hello world"}"#);

        // Test string with quotes
        let result = buffer.format_speak_message("Say \"hello\"");
        assert_eq!(result, r#"{"type":"Speak","text":"Say \"hello\""}"#);

        // Test string with newlines
        let result = buffer.format_speak_message("Line 1\nLine 2");
        assert_eq!(result, r#"{"type":"Speak","text":"Line 1\nLine 2"}"#);
    }

    #[tokio::test]
    async fn test_connection_state_atomic() {
        let state = ConnectionStateAtomic::default();

        // Initial state
        assert!(!state.is_connected());
        assert_eq!(
            state.get_connection_state().await,
            ConnectionState::Disconnected
        );

        // Set connected
        state.set_connected(true);
        assert!(state.is_connected());
        assert_eq!(
            state.get_connection_state().await,
            ConnectionState::Connected
        );

        // Set error
        state.set_error("Test error".to_string()).await;
        assert!(!state.is_connected());
        assert_eq!(
            state.get_connection_state().await,
            ConnectionState::Error("Test error".to_string())
        );
    }

    #[tokio::test]
    async fn test_tts_callback_integration() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let config = TTSConfig {
            provider: "deepgram".to_string(),
            api_key: "test_key".to_string(),
            ..Default::default()
        };

        let mut tts = <DeepgramTTS as BaseTTS>::new(config).unwrap();

        // Create a callback that tracks calls
        let audio_call_count = Arc::new(AtomicUsize::new(0));
        let error_call_count = Arc::new(AtomicUsize::new(0));
        let complete_call_count = Arc::new(AtomicUsize::new(0));

        let audio_counter = audio_call_count.clone();
        let error_counter = error_call_count.clone();
        let complete_counter = complete_call_count.clone();

        struct TestCallback {
            audio_counter: Arc<AtomicUsize>,
            error_counter: Arc<AtomicUsize>,
            complete_counter: Arc<AtomicUsize>,
        }

        impl AudioCallback for TestCallback {
            fn on_audio(
                &self,
                _audio_data: AudioData,
            ) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
                let counter = self.audio_counter.clone();
                Box::pin(async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                })
            }

            fn on_error(&self, _error: TTSError) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
                let counter = self.error_counter.clone();
                Box::pin(async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                })
            }

            fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
                let counter = self.complete_counter.clone();
                Box::pin(async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                })
            }
        }

        let callback = Arc::new(TestCallback {
            audio_counter,
            error_counter,
            complete_counter,
        });

        // Register the callback
        let result = tts.on_audio(callback);
        assert!(result.is_ok());

        // Verify initial counts
        assert_eq!(audio_call_count.load(Ordering::SeqCst), 0);
        assert_eq!(error_call_count.load(Ordering::SeqCst), 0);
        assert_eq!(complete_call_count.load(Ordering::SeqCst), 0);
    }
}
