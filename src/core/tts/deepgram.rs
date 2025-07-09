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

/// Deepgram TTS WebSocket message types
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DeepgramTTSMessage {
    /// Send text to synthesize
    Speak { text: String },
    /// Clear the synthesis queue
    Clear,
    /// Flush the synthesis queue
    Flush,
    /// Close the connection
    Close,
}

/// Deepgram TTS response message types
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

/// Deepgram TTS provider implementation
pub struct DeepgramTTS {
    /// Current connection state
    state: Arc<RwLock<ConnectionState>>,
    /// WebSocket connection
    connection: Arc<RwLock<Option<WebSocketConnection>>>,
    /// Audio callback for handling audio data
    audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
    /// Configuration
    config: TTSConfig,
    /// Message sender for WebSocket
    message_sender: Arc<RwLock<Option<mpsc::UnboundedSender<DeepgramTTSMessage>>>>,
    /// Task handles for cleanup
    task_handles: Arc<RwLock<Vec<tokio::task::JoinHandle<()>>>>,
}

impl DeepgramTTS {
    /// Create a new Deepgram TTS instance
    pub fn new(config: TTSConfig) -> TTSResult<Self> {
        Ok(Self {
            state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
            connection: Arc::new(RwLock::new(None)),
            audio_callback: Arc::new(RwLock::new(None)),
            config,
            message_sender: Arc::new(RwLock::new(None)),
            task_handles: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Build the WebSocket URL with query parameters
    fn build_websocket_url(config: &TTSConfig) -> TTSResult<Url> {
        let mut url = Url::parse("wss://api.deepgram.com/v1/speak")
            .map_err(|e| TTSError::InvalidConfiguration(format!("Invalid base URL: {e}")))?;

        // Add query parameters
        if let Some(encoding) = config.audio_format.as_ref() {
            url.query_pairs_mut().append_pair("encoding", encoding);
        }

        if let Some(sample_rate) = config.sample_rate {
            url.query_pairs_mut()
                .append_pair("sample_rate", &sample_rate.to_string());
        }

        if let Some(model) = config.voice_id.as_ref() {
            url.query_pairs_mut().append_pair("model", model);
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

    /// Handle incoming WebSocket messages
    async fn handle_websocket_message(
        message: Message,
        audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
        config: TTSConfig,
    ) {
        match message {
            Message::Binary(data) => {
                // Deepgram TTS sends raw binary audio data
                debug!("Received binary audio data: {} bytes", data.len());

                if let Some(callback) = audio_callback.read().await.as_ref() {
                    let audio_format = config
                        .audio_format
                        .clone()
                        .unwrap_or_else(|| "linear16".to_string());

                    let sample_rate = config.sample_rate.unwrap_or(24000);

                    let audio_data = AudioData {
                        data: data.to_vec(),
                        sample_rate,
                        format: audio_format,
                        duration_ms: None, // Calculate if needed
                    };

                    callback.on_audio(audio_data).await;
                }
            }
            Message::Text(text) => {
                // Handle text control messages (errors, metadata, etc.)
                debug!("Received text message: {}", text);

                // Try to parse as JSON for control messages
                if let Ok(response) = serde_json::from_str::<DeepgramTTSResponse>(&text) {
                    match response {
                        DeepgramTTSResponse::Error { error } => {
                            if let Some(callback) = audio_callback.read().await.as_ref() {
                                let tts_error = TTSError::ProviderError(error);
                                callback.on_error(tts_error).await;
                            }
                        }
                        DeepgramTTSResponse::Close { reason } => {
                            info!("WebSocket closed by server: {}", reason);
                            if let Some(callback) = audio_callback.read().await.as_ref() {
                                callback.on_complete().await;
                            }
                        }
                        DeepgramTTSResponse::Control { message } => {
                            debug!("Control message: {}", message);
                        }
                        DeepgramTTSResponse::Audio { data: _ } => {
                            // This shouldn't happen with the new API - audio comes as binary
                            warn!("Received unexpected JSON audio data");
                        }
                    }
                } else {
                    // Handle non-JSON text messages
                    debug!("Received non-JSON text message: {}", text);
                }
            }
            Message::Close(_) => {
                info!("WebSocket connection closed");
                if let Some(callback) = audio_callback.read().await.as_ref() {
                    callback.on_complete().await;
                }
            }
            Message::Ping(data) => {
                debug!("Received ping: {} bytes", data.len());
            }
            Message::Pong(data) => {
                debug!("Received pong: {} bytes", data.len());
            }
            Message::Frame(_) => {
                debug!("Received raw frame");
            }
        }
    }

    /// Start the WebSocket message handling task
    async fn start_message_handler(
        mut ws_stream: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
        audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
        config: TTSConfig,
        mut message_receiver: mpsc::UnboundedReceiver<DeepgramTTSMessage>,
        state: Arc<RwLock<ConnectionState>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            debug!("TTS WebSocket message handler started");

            loop {
                tokio::select! {
                    // Handle incoming messages from WebSocket
                    message = ws_stream.next() => {
                        match message {
                            Some(Ok(msg)) => {
                                debug!("Processing WebSocket message: {:?}", msg);
                                Self::handle_websocket_message(
                                    msg,
                                    audio_callback.clone(),
                                    config.clone(),
                                ).await;
                            }
                            Some(Err(e)) => {
                                error!("WebSocket error: {}", e);
                                *state.write().await = ConnectionState::Error(e.to_string());
                                break;
                            }
                            None => {
                                info!("WebSocket stream ended");
                                break;
                            }
                        }
                    }
                    // Handle outgoing messages to WebSocket
                    outgoing_message = message_receiver.recv() => {
                        match outgoing_message {
                            Some(msg) => {
                                debug!("Sending TTS message: {:?}", msg);
                                let json_msg = match serde_json::to_string(&msg) {
                                    Ok(json) => json,
                                    Err(e) => {
                                        error!("Failed to serialize message: {}", e);
                                        continue;
                                    }
                                };

                                debug!("Sending JSON: {}", json_msg);
                                if let Err(e) = ws_stream.send(Message::Text(json_msg.into())).await {
                                    error!("Failed to send message: {}", e);
                                    *state.write().await = ConnectionState::Error(e.to_string());
                                    break;
                                }
                            }
                            None => {
                                info!("Message receiver closed");
                                break;
                            }
                        }
                    }
                }
            }

            // Clean up connection state
            *state.write().await = ConnectionState::Disconnected;
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
            state: Arc::new(RwLock::new(ConnectionState::Disconnected)),
            connection: Arc::new(RwLock::new(None)),
            audio_callback: Arc::new(RwLock::new(None)),
            config,
            message_sender: Arc::new(RwLock::new(None)),
            task_handles: Arc::new(RwLock::new(Vec::new())),
        })
    }

    async fn connect(&mut self) -> TTSResult<()> {
        // Update state to connecting
        *self.state.write().await = ConnectionState::Connecting;

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

        // Create message channel for outgoing messages
        let (message_sender, message_receiver) = mpsc::unbounded_channel();

        // Start message handler task
        let task_handle = Self::start_message_handler(
            ws_stream,
            self.audio_callback.clone(),
            self.config.clone(),
            message_receiver,
            self.state.clone(),
        )
        .await;

        // Store task handle for cleanup
        self.task_handles.write().await.push(task_handle);

        // Store message sender
        *self.message_sender.write().await = Some(message_sender);

        // Update state to connected
        *self.state.write().await = ConnectionState::Connected;
        info!("Deepgram TTS WebSocket connection ready");

        Ok(())
    }

    async fn disconnect(&mut self) -> TTSResult<()> {
        // Send close message if connected
        if let Some(sender) = self.message_sender.read().await.as_ref() {
            let _ = sender.send(DeepgramTTSMessage::Close);
        }

        // Clean up task handles
        let mut handles = self.task_handles.write().await;
        for handle in handles.drain(..) {
            handle.abort();
        }

        // Clear state
        *self.state.write().await = ConnectionState::Disconnected;
        *self.connection.write().await = None;
        *self.message_sender.write().await = None;
        *self.audio_callback.write().await = None;

        Ok(())
    }

    fn is_ready(&self) -> bool {
        // Use try_read to avoid blocking in tests
        if let Ok(state) = self.state.try_read() {
            matches!(*state, ConnectionState::Connected)
        } else {
            false // If we can't read state, assume not ready
        }
    }

    fn get_connection_state(&self) -> ConnectionState {
        // Use try_read to avoid blocking in tests
        if let Ok(state) = self.state.try_read() {
            state.clone()
        } else {
            ConnectionState::Disconnected // If we can't read state, assume disconnected
        }
    }

    async fn speak(&self, text: &str, flush: bool) -> TTSResult<()> {
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "Deepgram TTS provider not connected".to_string(),
            ));
        }

        let sender = self.message_sender.read().await;
        if let Some(sender) = sender.as_ref() {
            // Send the speak message
            let speak_message = DeepgramTTSMessage::Speak {
                text: text.to_string(),
            };

            sender.send(speak_message).map_err(|e| {
                TTSError::InternalError(format!("Failed to send speak message: {e}"))
            })?;

            // Conditionally flush to trigger audio generation if requested
            if flush {
                let flush_message = DeepgramTTSMessage::Flush;
                sender.send(flush_message).map_err(|e| {
                    TTSError::InternalError(format!("Failed to send flush message: {e}"))
                })?;
                debug!("Sent speak message with immediate flush for text: {}", text);
            } else {
                debug!("Sent speak message (queued, no flush) for text: {}", text);
            }
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
            sender.send(DeepgramTTSMessage::Clear).map_err(|e| {
                TTSError::InternalError(format!("Failed to send clear message: {e}"))
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
            sender.send(DeepgramTTSMessage::Flush).map_err(|e| {
                TTSError::InternalError(format!("Failed to send flush message: {e}"))
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
            "documentation": "https://developers.deepgram.com/reference/text-to-speech-api/speak-streaming"
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
    async fn test_tts_message_format() {
        // Test that our messages serialize correctly to the expected Deepgram format
        let speak_msg = DeepgramTTSMessage::Speak {
            text: "Hello, world!".to_string(),
        };

        let json = serde_json::to_string(&speak_msg).unwrap();
        assert_eq!(json, r#"{"type":"Speak","text":"Hello, world!"}"#);

        let flush_msg = DeepgramTTSMessage::Flush;
        let json = serde_json::to_string(&flush_msg).unwrap();
        assert_eq!(json, r#"{"type":"Flush"}"#);

        let clear_msg = DeepgramTTSMessage::Clear;
        let json = serde_json::to_string(&clear_msg).unwrap();
        assert_eq!(json, r#"{"type":"Clear"}"#);
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
