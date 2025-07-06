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
//!         provider_config: serde_json::json!({
//!             "api_key": "your_deepgram_api_key",
//!             "model": "aura-asteria-en",
//!             "encoding": "mp3",
//!             "sample_rate": 24000
//!         }),
//!         voice_id: Some("aura-asteria-en".to_string()),
//!         audio_format: Some("mp3".to_string()),
//!         sample_rate: Some(24000),
//!         ..Default::default()
//!     };
//!     
//!     let mut tts = DeepgramTTS::new(config)?;
//!     tts.connect().await?;
//!     tts.speak("Hello from Deepgram!").await?;
//!     tts.disconnect().await?;
//!     
//!     Ok(())
//! }
//! ```

use crate::core::tts::base::{
    AudioCallback, AudioData, BaseTTS, ConnectionState, TTSConfig, TTSError, TTSResult,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{
        http::header::{AUTHORIZATION, USER_AGENT},
        http::{HeaderValue, Request},
        protocol::Message,
    },
};
use url::Url;

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

        // Extract configuration from provider_config
        let provider_config = &config.provider_config;

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

        // Add provider-specific parameters
        if let Some(mip_opt_out) = provider_config.get("mip_opt_out") {
            if let Some(value) = mip_opt_out.as_str() {
                url.query_pairs_mut().append_pair("mip_opt_out", value);
            }
        }

        Ok(url)
    }

    /// Get authorization header value
    fn get_auth_header(config: &TTSConfig) -> TTSResult<String> {
        let provider_config = &config.provider_config;

        if let Some(api_key) = provider_config.get("api_key") {
            if let Some(key) = api_key.as_str() {
                return Ok(format!("Token {key}"));
            }
        }

        if let Some(jwt_token) = provider_config.get("jwt_token") {
            if let Some(token) = jwt_token.as_str() {
                return Ok(format!("Bearer {token}"));
            }
        }

        Err(TTSError::InvalidConfiguration(
            "Missing API key or JWT token in provider configuration".to_string(),
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
                // Handle binary audio data
                if let Some(callback) = audio_callback.read().await.as_ref() {
                    let audio_format = config
                        .audio_format
                        .clone()
                        .unwrap_or_else(|| "mp3".to_string());

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
                // Handle text control messages
                if let Ok(response) = serde_json::from_str::<DeepgramTTSResponse>(&text) {
                    match response {
                        DeepgramTTSResponse::Audio { data } => {
                            // Handle base64 encoded audio data
                            if let Ok(decoded_data) = STANDARD.decode(&data) {
                                if let Some(callback) = audio_callback.read().await.as_ref() {
                                    let audio_format = config
                                        .audio_format
                                        .clone()
                                        .unwrap_or_else(|| "mp3".to_string());

                                    let sample_rate = config.sample_rate.unwrap_or(24000);

                                    let audio_data = AudioData {
                                        data: decoded_data,
                                        sample_rate,
                                        format: audio_format,
                                        duration_ms: None,
                                    };

                                    callback.on_audio(audio_data).await;
                                }
                            }
                        }
                        DeepgramTTSResponse::Error { error } => {
                            if let Some(callback) = audio_callback.read().await.as_ref() {
                                let tts_error = TTSError::ProviderError(error);
                                callback.on_error(tts_error).await;
                            }
                        }
                        DeepgramTTSResponse::Close { reason } => {
                            tracing::info!("WebSocket closed by server: {}", reason);
                            if let Some(callback) = audio_callback.read().await.as_ref() {
                                callback.on_complete().await;
                            }
                        }
                        DeepgramTTSResponse::Control { message } => {
                            tracing::debug!("Control message: {}", message);
                        }
                    }
                }
            }
            Message::Close(_) => {
                tracing::info!("WebSocket connection closed");
                if let Some(callback) = audio_callback.read().await.as_ref() {
                    callback.on_complete().await;
                }
            }
            _ => {
                tracing::debug!("Received unexpected message type: {:?}", message);
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
            loop {
                tokio::select! {
                    // Handle incoming messages from WebSocket
                    message = ws_stream.next() => {
                        match message {
                            Some(Ok(msg)) => {
                                Self::handle_websocket_message(
                                    msg,
                                    audio_callback.clone(),
                                    config.clone(),
                                ).await;
                            }
                            Some(Err(e)) => {
                                tracing::error!("WebSocket error: {}", e);
                                *state.write().await = ConnectionState::Error(e.to_string());
                                break;
                            }
                            None => {
                                tracing::info!("WebSocket stream ended");
                                break;
                            }
                        }
                    }
                    // Handle outgoing messages to WebSocket
                    outgoing_message = message_receiver.recv() => {
                        match outgoing_message {
                            Some(msg) => {
                                let json_msg = match serde_json::to_string(&msg) {
                                    Ok(json) => json,
                                    Err(e) => {
                                        tracing::error!("Failed to serialize message: {}", e);
                                        continue;
                                    }
                                };

                                if let Err(e) = ws_stream.send(Message::Text(json_msg.into())).await {
                                    tracing::error!("Failed to send message: {}", e);
                                    *state.write().await = ConnectionState::Error(e.to_string());
                                    break;
                                }
                            }
                            None => {
                                tracing::info!("Message receiver closed");
                                break;
                            }
                        }
                    }
                }
            }

            // Clean up connection state
            *state.write().await = ConnectionState::Disconnected;
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

        // Get authorization header
        let auth_header = Self::get_auth_header(&self.config)?;

        // Create request with headers
        let mut request = Request::builder()
            .uri(url.as_str())
            .header("Sec-WebSocket-Protocol", "")
            .body(())
            .map_err(|e| TTSError::ConnectionFailed(format!("Failed to create request: {e}")))?;

        request.headers_mut().insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth_header)
                .map_err(|e| TTSError::InvalidConfiguration(format!("Invalid auth header: {e}")))?,
        );

        request
            .headers_mut()
            .insert(USER_AGENT, HeaderValue::from_static("sayna-tts/1.0"));

        // Connect to WebSocket
        let (ws_stream, _) = connect_async(request)
            .await
            .map_err(|e| TTSError::ConnectionFailed(format!("WebSocket connection failed: {e}")))?;

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

    async fn speak(&self, text: &str) -> TTSResult<()> {
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "Deepgram TTS provider not connected".to_string(),
            ));
        }

        let sender = self.message_sender.read().await;
        if let Some(sender) = sender.as_ref() {
            let message = DeepgramTTSMessage::Speak {
                text: text.to_string(),
            };

            sender.send(message).map_err(|e| {
                TTSError::InternalError(format!("Failed to send speak message: {e}"))
            })?;
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

    async fn set_options(&mut self, options: serde_json::Value) -> TTSResult<()> {
        // Merge options into provider_config
        if let serde_json::Value::Object(ref mut provider_config) = self.config.provider_config {
            if let serde_json::Value::Object(new_options) = options {
                for (key, value) in new_options {
                    provider_config.insert(key, value);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tts::base::ChannelAudioCallback;
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
            provider_config: serde_json::json!({
                "api_key": "test_key"
            }),
            ..Default::default()
        };

        let auth_header = DeepgramTTS::get_auth_header(&config).unwrap();
        assert_eq!(auth_header, "Token test_key");

        let jwt_config = TTSConfig {
            provider_config: serde_json::json!({
                "jwt_token": "test_jwt"
            }),
            ..Default::default()
        };

        let jwt_header = DeepgramTTS::get_auth_header(&jwt_config).unwrap();
        assert_eq!(jwt_header, "Bearer test_jwt");
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
        let config = TTSConfig::default();
        let tts = DeepgramTTS::new(config).unwrap();
        let result = tts.speak("Hello").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TTSError::ProviderNotReady(_)));
    }
}
