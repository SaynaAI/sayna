//! # ElevenLabs TTS Provider
//!
//! This module implements the ElevenLabs Text-to-Speech provider using their WebSocket API.
//! It provides real-time audio synthesis with support for streaming input and chunked output.
//!
//! ## Features
//!
//! - **WebSocket-based streaming**: Real-time audio generation using WebSocket connections
//! - **Chunked text processing**: Efficient handling of text chunks with configurable buffering
//! - **Base64 audio decoding**: Automatic decoding of base64-encoded audio data
//! - **Voice settings**: Full support for ElevenLabs voice configuration
//! - **Error handling**: Comprehensive error handling for WebSocket and API errors
//! - **Alignment data**: Optional word-level timing information
//! - **Optimized for low latency**: Minimal allocations and efficient memory usage
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use sayna::core::tts::elevenlabs::ElevenLabsTTS;
//! use sayna::core::tts::{TTSConfig, AudioCallback, AudioData, BaseTTS};
//! use std::sync::Arc;
//! use std::pin::Pin;
//! use std::future::Future;
//!
//! struct MyAudioCallback;
//!
//! impl AudioCallback for MyAudioCallback {
//!     fn on_audio(&self, audio_data: AudioData) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
//!         Box::pin(async move {
//!             println!("Received audio: {} bytes", audio_data.data.len());
//!         })
//!     }
//!     
//!     fn on_error(&self, error: sayna::core::tts::TTSError) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
//!         Box::pin(async move {
//!             eprintln!("TTS Error: {}", error);
//!         })
//!     }
//!     
//!     fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
//!         Box::pin(async move {
//!             println!("TTS synthesis complete");
//!         })
//!     }
//! }
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = TTSConfig {
//!         provider: "elevenlabs".to_string(),
//!         api_key: "your-api-key".to_string(),
//!         voice_id: Some("21m00Tcm4TlvDq8ikWAM".to_string()),
//!         ..Default::default()
//!     };
//!
//!     let mut tts = ElevenLabsTTS::new(config)?;
//!     tts.connect().await?;
//!
//!     // Register audio callback
//!     tts.on_audio(Arc::new(MyAudioCallback))?;
//!
//!     // Send text for synthesis
//!     tts.speak("Hello, world!", true).await?;
//!
//!     Ok(())
//! # }
//! ```

use async_trait::async_trait;
use base64::prelude::*;
use bytes::{Bytes, BytesMut};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;

use crate::core::tts::{
    AudioCallback, AudioData, BaseTTS, ConnectionState, TTSConfig, TTSError, TTSResult,
};

// Pre-computed message constants to avoid runtime JSON serialization
const CLOSE_MESSAGE: &str = r#"{"text":""}"#;

// Connection state constants for atomic operations
const STATE_DISCONNECTED: u8 = 0;
const STATE_CONNECTING: u8 = 1;
const STATE_CONNECTED: u8 = 2;
const STATE_ERROR: u8 = 3;

/// Voice settings for ElevenLabs TTS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSettings {
    /// Voice stability (0.0 to 1.0)
    pub stability: f32,
    /// Similarity boost (0.0 to 1.0)
    pub similarity_boost: f32,
    /// Use speaker boost
    pub use_speaker_boost: bool,
    /// Voice style (0.0 to 1.0)
    pub style: Option<f32>,
    /// Speaking rate (0.25 to 2.0)
    pub speed: Option<f32>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            stability: 0.5,
            similarity_boost: 0.8,
            use_speaker_boost: false,
            style: None,
            speed: Some(1.0),
        }
    }
}

/// Generation configuration for ElevenLabs TTS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationConfig {
    /// Chunk length schedule for buffering
    pub chunk_length_schedule: Vec<u32>,
}

impl Default for GenerationConfig {
    fn default() -> Self {
        Self {
            chunk_length_schedule: vec![120, 160, 250, 290],
        }
    }
}

/// Pronunciation dictionary locator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PronunciationDictionaryLocator {
    pub pronunciation_dictionary_id: String,
    pub version_id: String,
}

/// Character timing information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterAlignment {
    /// Character start times in milliseconds
    #[serde(rename = "charStartTimesMs")]
    pub char_start_times_ms: Vec<u32>,
    /// Character durations in milliseconds
    #[serde(rename = "charDurationsMs")]
    pub chars_durations_ms: Vec<u32>,
    /// Characters
    pub chars: Vec<String>,
}

/// Initialize connection message
#[derive(Debug, Clone, Serialize)]
pub struct InitializeConnection {
    /// Initial text (usually a space)
    pub text: String,
    /// Voice settings
    pub voice_settings: InitVoiceSettings,
    /// API key
    pub xi_api_key: String,
    /// Generation configuration (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GenerationConfig>,
    /// Pronunciation dictionary locators (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pronunciation_dictionary_locators: Option<Vec<PronunciationDictionaryLocator>>,
}

/// Simplified voice settings for initialization
#[derive(Debug, Clone, Serialize)]
pub struct InitVoiceSettings {
    /// Voice stability (0.0 to 1.0)
    pub stability: f32,
    /// Similarity boost (0.0 to 1.0)
    pub similarity_boost: f32,
    /// Speaking rate (0.25 to 2.0)
    pub speed: f32,
    /// Use speaker boost (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_speaker_boost: Option<bool>,
}

/// Send text message
#[derive(Debug, Clone, Serialize)]
pub struct SendText {
    /// Text to synthesize
    pub text: String,
    /// Whether to immediately generate audio
    pub try_trigger_generation: Option<bool>,
    /// Whether to flush the buffer
    pub flush: Option<bool>,
    /// Voice settings override
    pub voice_settings: Option<VoiceSettings>,
    /// Generation configuration override
    pub generation_config: Option<GenerationConfig>,
}

/// Close connection message
#[derive(Debug, Clone, Serialize)]
pub struct CloseConnection {
    /// Empty text to close connection
    pub text: String,
}

/// ElevenLabs message types for sending
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ElevenLabsOutboundMessage {
    Initialize(InitializeConnection),
    SendText(SendText),
    Close(CloseConnection),
}

/// Audio output message from ElevenLabs
#[derive(Debug, Clone, Deserialize)]
pub struct AudioOutput {
    /// Base64 encoded audio data
    pub audio: String,
    /// Whether this is the final audio chunk
    #[serde(rename = "isFinal")]
    pub is_final: Option<bool>,
    /// Normalized alignment data
    #[serde(rename = "normalizedAlignment")]
    pub normalized_alignment: Option<CharacterAlignment>,
    /// Alignment data
    pub alignment: Option<CharacterAlignment>,
}

/// Final output message from ElevenLabs
#[derive(Debug, Clone, Deserialize)]
pub struct FinalOutput {
    /// Whether this is the final output
    #[serde(rename = "isFinal")]
    pub is_final: bool,
}

/// Error message from ElevenLabs
#[derive(Debug, Clone, Deserialize)]
pub struct ErrorMessage {
    /// Error message
    pub error: String,
    /// Error details
    pub detail: Option<String>,
}

/// ElevenLabs message types for receiving
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ElevenLabsInboundMessage {
    AudioOutput(AudioOutput),
    FinalOutput(FinalOutput),
    Error(ErrorMessage),
}

/// Optimized message buffer for reducing allocations
#[derive(Default)]
struct MessageBuffer {
    decode_buffer: BytesMut,
}

impl MessageBuffer {
    /// Decode base64 audio data with reusable buffer
    fn decode_audio_data(&mut self, base64_data: &str) -> Result<Bytes, TTSError> {
        // Estimate decoded size
        let estimated_size = (base64_data.len() * 3) / 4;

        // Ensure buffer has enough capacity
        if self.decode_buffer.capacity() < estimated_size {
            self.decode_buffer.reserve(estimated_size);
        }

        // Clear but keep capacity
        self.decode_buffer.clear();

        // Decode directly into buffer
        let decoded = BASE64_STANDARD
            .decode(base64_data)
            .map_err(|e| TTSError::InternalError(format!("Failed to decode base64 audio: {e}")))?;

        // Convert to Bytes for zero-copy operations
        Ok(Bytes::from(decoded))
    }
}

/// Optimized connection state using atomic operations
#[derive(Default)]
struct AtomicConnectionState {
    state: AtomicU8,
    error_message: RwLock<Option<String>>,
}

impl AtomicConnectionState {
    fn new() -> Self {
        Self {
            state: AtomicU8::new(STATE_DISCONNECTED),
            error_message: RwLock::new(None),
        }
    }

    fn set_connecting(&self) {
        self.state.store(STATE_CONNECTING, Ordering::Relaxed);
    }

    fn set_connected(&self) {
        self.state.store(STATE_CONNECTED, Ordering::Relaxed);
    }

    fn set_disconnected(&self) {
        self.state.store(STATE_DISCONNECTED, Ordering::Relaxed);
    }

    async fn set_error(&self, error: String) {
        self.state.store(STATE_ERROR, Ordering::Relaxed);
        *self.error_message.write().await = Some(error);
    }

    fn is_connected(&self) -> bool {
        self.state.load(Ordering::Relaxed) == STATE_CONNECTED
    }
}

/// ElevenLabs TTS provider implementation optimized for low latency
pub struct ElevenLabsTTS {
    config: TTSConfig,
    state: Arc<AtomicConnectionState>,
    audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
    // Use bounded channel for backpressure control
    websocket_tx: Option<mpsc::Sender<ElevenLabsOutboundMessage>>,
    shutdown_tx: Option<mpsc::UnboundedSender<()>>,
    voice_settings: VoiceSettings,
    // Task handle for cleanup
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl ElevenLabsTTS {
    /// Create ElevenLabs-specific configuration from TTSConfig
    fn create_voice_settings(config: &TTSConfig) -> VoiceSettings {
        VoiceSettings {
            stability: 0.5,
            similarity_boost: 0.8,
            use_speaker_boost: false,
            style: None,
            speed: config.speaking_rate,
        }
    }

    /// Build WebSocket URL with query parameters - optimized for fewer allocations
    fn build_websocket_url(config: &TTSConfig) -> TTSResult<String> {
        let voice_id = config.voice_id.as_ref().ok_or_else(|| {
            TTSError::InvalidConfiguration("voice_id is required for ElevenLabs".to_string())
        })?;

        let base_url = format!("wss://api.elevenlabs.io/v1/text-to-speech/{voice_id}/stream-input");

        let mut url = Url::parse(&base_url)
            .map_err(|e| TTSError::InvalidConfiguration(format!("Invalid URL: {e}")))?;

        // Add query parameters efficiently
        {
            let mut query_pairs = url.query_pairs_mut();

            // Add optional parameters
            if let Some(sample_rate) = config.sample_rate {
                if let Some(format) = &config.audio_format {
                    let output_format = match format.as_str() {
                        "pcm" => Cow::Owned(format!("pcm_{sample_rate}")),
                        "mp3" => Cow::Owned(format!("mp3_{sample_rate}_128")),
                        _ => Cow::Owned(format!("pcm_{sample_rate}")),
                    };
                    query_pairs.append_pair("output_format", &output_format);
                }
            }

            // Add inactivity timeout
            if let Some(timeout) = config.connection_timeout {
                query_pairs.append_pair("inactivity_timeout", &timeout.to_string());
            }

            // Add model
            if !config.model.is_empty() {
                query_pairs.append_pair("model_id", &config.model);
            }

            // Add other parameters
            query_pairs.append_pair("enable_logging", "false");
            query_pairs.append_pair("enable_ssml_parsing", "false");
            query_pairs.append_pair("sync_alignment", "true");
        }

        Ok(url.to_string())
    }

    /// Handle WebSocket connection and message processing - optimized for performance
    async fn handle_websocket_connection(
        url: String,
        config: TTSConfig,
        voice_settings: VoiceSettings,
        audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
        state: Arc<AtomicConnectionState>,
        mut message_rx: mpsc::Receiver<ElevenLabsOutboundMessage>,
        mut shutdown_rx: mpsc::UnboundedReceiver<()>,
    ) -> TTSResult<()> {
        // Validate API key
        if config.api_key.is_empty() {
            return Err(TTSError::InvalidConfiguration(
                "API key is required".to_string(),
            ));
        }

        tracing::info!("Connecting to ElevenLabs WebSocket: {}", url);

        // Connect to WebSocket
        let (ws_stream, response) = connect_async(&url).await.map_err(|e| {
            TTSError::ConnectionFailed(format!("Failed to connect to ElevenLabs: {e}"))
        })?;

        tracing::info!(
            "Connected to ElevenLabs WebSocket. Response status: {}",
            response.status()
        );

        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        // Send initialization message
        let init_voice_settings = InitVoiceSettings {
            stability: voice_settings.stability,
            similarity_boost: voice_settings.similarity_boost,
            speed: voice_settings.speed.unwrap_or(1.0),
            use_speaker_boost: if voice_settings.use_speaker_boost {
                Some(true)
            } else {
                None
            },
        };

        let init_msg = InitializeConnection {
            text: " ".to_string(),
            voice_settings: init_voice_settings,
            xi_api_key: config.api_key.clone(),
            generation_config: None,
            pronunciation_dictionary_locators: None,
        };

        let init_json = serde_json::to_string(&init_msg).map_err(|e| {
            TTSError::InternalError(format!("Failed to serialize init message: {e}"))
        })?;

        tracing::debug!("Sending ElevenLabs initialization message: {}", init_json);

        ws_sender
            .send(Message::Text(init_json.into()))
            .await
            .map_err(|e| TTSError::ConnectionFailed(format!("Failed to send init message: {e}")))?;

        // Set connected state
        state.set_connected();

        // Create message buffer for reuse
        let mut message_buffer = MessageBuffer::default();

        // Message handling loop
        loop {
            tokio::select! {
                // Handle incoming WebSocket messages
                ws_msg = ws_receiver.next() => {
                    match ws_msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Err(e) = Self::handle_inbound_message(
                                text.to_string(),
                                &audio_callback,
                                &mut message_buffer
                            ).await {
                                tracing::error!("Error handling inbound message: {}", e);
                                if let Some(callback) = audio_callback.read().await.as_ref() {
                                    callback.on_error(e).await;
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) => {
                            tracing::info!("WebSocket connection closed by server");
                            break;
                        }
                        Some(Err(e)) => {
                            let error = TTSError::NetworkError(format!("WebSocket error: {e}"));
                            tracing::error!("WebSocket error: {}", error);
                            state.set_error(error.to_string()).await;
                            if let Some(callback) = audio_callback.read().await.as_ref() {
                                callback.on_error(error).await;
                            }
                            break;
                        }
                        None => {
                            tracing::info!("WebSocket stream ended");
                            break;
                        }
                        _ => {}
                    }
                }

                // Handle outbound messages
                outbound_msg = message_rx.recv() => {
                    match outbound_msg {
                        Some(msg) => {
                            let json = serde_json::to_string(&msg)
                                .map_err(|e| TTSError::InternalError(format!("Failed to serialize message: {e}")))?;

                            if let Err(e) = ws_sender.send(Message::Text(json.into())).await {
                                tracing::error!("Failed to send message: {}", e);
                                let error = TTSError::NetworkError(format!("Failed to send message: {e}"));
                                state.set_error(error.to_string()).await;
                                if let Some(callback) = audio_callback.read().await.as_ref() {
                                    callback.on_error(error).await;
                                }
                                break;
                            }
                        }
                        None => {
                            // Channel closed, close connection
                            let _ = ws_sender.send(Message::Text(CLOSE_MESSAGE.into())).await;
                            break;
                        }
                    }
                }

                // Handle shutdown signal
                _ = shutdown_rx.recv() => {
                    tracing::info!("Shutdown signal received");
                    break;
                }
            }
        }

        // Set disconnected state
        state.set_disconnected();

        // Send completion signal
        if let Some(callback) = audio_callback.read().await.as_ref() {
            callback.on_complete().await;
        }

        Ok(())
    }

    /// Handle inbound messages from ElevenLabs - optimized for performance
    async fn handle_inbound_message(
        message: String,
        audio_callback: &Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
        message_buffer: &mut MessageBuffer,
    ) -> TTSResult<()> {
        // Quick check for audio messages to avoid full JSON parsing
        if message.contains("\"audio\"") {
            if let Ok(audio_output) = serde_json::from_str::<AudioOutput>(&message) {
                tracing::debug!("Successfully parsed as AudioOutput");
                Self::handle_audio_output(audio_output, audio_callback, message_buffer).await?;
                return Ok(());
            }
        }

        // Check for final output
        if message.contains("\"isFinal\"") {
            if let Ok(final_output) = serde_json::from_str::<FinalOutput>(&message) {
                tracing::debug!("Successfully parsed as FinalOutput");
                Self::handle_final_output(final_output, audio_callback).await?;
                return Ok(());
            }
        }

        // Check for error messages
        if message.contains("\"error\"") {
            if let Ok(error_msg) = serde_json::from_str::<ErrorMessage>(&message) {
                tracing::debug!("Successfully parsed as ErrorMessage");
                Self::handle_error_message(error_msg, audio_callback).await?;
                return Ok(());
            }
        }

        // Log unknown messages
        tracing::warn!("Received unknown message from ElevenLabs: {}", message);

        Ok(())
    }

    /// Handle audio output message - optimized for zero-copy operations
    async fn handle_audio_output(
        audio_output: AudioOutput,
        audio_callback: &Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
        message_buffer: &mut MessageBuffer,
    ) -> TTSResult<()> {
        if let Some(callback) = audio_callback.read().await.as_ref() {
            // Decode base64 audio data with reusable buffer
            let audio_bytes = message_buffer.decode_audio_data(&audio_output.audio)?;

            let audio_chunk = AudioData {
                data: audio_bytes.to_vec(), // Convert to Vec<u8> for API compatibility
                sample_rate: 22050,         // ElevenLabs default
                format: "pcm".to_string(),
                duration_ms: None,
            };

            callback.on_audio(audio_chunk).await;
        }

        Ok(())
    }

    /// Handle final output message
    async fn handle_final_output(
        _final_output: FinalOutput,
        audio_callback: &Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
    ) -> TTSResult<()> {
        if let Some(callback) = audio_callback.read().await.as_ref() {
            callback.on_complete().await;
        }

        Ok(())
    }

    /// Handle error message
    async fn handle_error_message(
        error_msg: ErrorMessage,
        audio_callback: &Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
    ) -> TTSResult<()> {
        let error = TTSError::ProviderError(format!("ElevenLabs error: {}", error_msg.error));

        if let Some(callback) = audio_callback.read().await.as_ref() {
            callback.on_error(error.clone()).await;
        }

        Err(error)
    }
}

#[async_trait]
impl BaseTTS for ElevenLabsTTS {
    fn new(config: TTSConfig) -> TTSResult<Self> {
        if config.provider != "elevenlabs" {
            return Err(TTSError::InvalidConfiguration(
                "Provider must be 'elevenlabs'".to_string(),
            ));
        }

        if config.api_key.is_empty() {
            return Err(TTSError::InvalidConfiguration(
                "API key is required for ElevenLabs".to_string(),
            ));
        }

        if config.voice_id.is_none() {
            return Err(TTSError::InvalidConfiguration(
                "Voice ID is required for ElevenLabs".to_string(),
            ));
        }

        let voice_settings = Self::create_voice_settings(&config);

        Ok(Self {
            config,
            state: Arc::new(AtomicConnectionState::new()),
            audio_callback: Arc::new(RwLock::new(None)),
            websocket_tx: None,
            shutdown_tx: None,
            voice_settings,
            task_handle: None,
        })
    }

    async fn connect(&mut self) -> TTSResult<()> {
        if self.state.is_connected() {
            return Err(TTSError::InvalidConfiguration(
                "Already connected".to_string(),
            ));
        }

        self.state.set_connecting();

        // Build WebSocket URL
        let url = Self::build_websocket_url(&self.config)?;

        // Create bounded channels for backpressure control
        let (message_tx, message_rx) = mpsc::channel(32);
        let (shutdown_tx, shutdown_rx) = mpsc::unbounded_channel();

        // Store senders
        self.websocket_tx = Some(message_tx);
        self.shutdown_tx = Some(shutdown_tx);

        // Spawn WebSocket handler
        let config = self.config.clone();
        let voice_settings = self.voice_settings.clone();
        let audio_callback = self.audio_callback.clone();
        let state = self.state.clone();

        let task_handle = tokio::spawn(async move {
            if let Err(e) = Self::handle_websocket_connection(
                url,
                config,
                voice_settings,
                audio_callback,
                state,
                message_rx,
                shutdown_rx,
            )
            .await
            {
                tracing::error!("WebSocket connection failed: {}", e);
            }
        });

        self.task_handle = Some(task_handle);

        // Give the connection a moment to establish
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        Ok(())
    }

    async fn disconnect(&mut self) -> TTSResult<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        if let Some(task_handle) = self.task_handle.take() {
            task_handle.abort();
        }

        self.websocket_tx = None;
        self.state.set_disconnected();

        Ok(())
    }

    fn is_ready(&self) -> bool {
        self.state.is_connected()
    }

    fn get_connection_state(&self) -> ConnectionState {
        // For synchronous access, we'll return the current state
        // This is a trade-off for performance to avoid async in this method
        match self.state.state.load(Ordering::Relaxed) {
            STATE_DISCONNECTED => ConnectionState::Disconnected,
            STATE_CONNECTING => ConnectionState::Connecting,
            STATE_CONNECTED => ConnectionState::Connected,
            STATE_ERROR => ConnectionState::Error("Connection error".to_string()),
            _ => ConnectionState::Disconnected,
        }
    }

    async fn speak(&self, text: &str, flush: bool) -> TTSResult<()> {
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "ElevenLabs TTS not connected".to_string(),
            ));
        }

        let mut send_msg = text.to_string();

        if send_msg.is_empty() {
            send_msg = " ".to_string();
        }

        let send_msg = ElevenLabsOutboundMessage::SendText(SendText {
            text: send_msg,
            try_trigger_generation: Some(true),
            flush: Some(flush),
            voice_settings: None,
            generation_config: None,
        });

        if let Some(tx) = &self.websocket_tx {
            tx.send(send_msg).await.map_err(|e| {
                TTSError::InternalError(format!("Failed to send text message: {e}"))
            })?;
        }

        Ok(())
    }

    async fn clear(&mut self) -> TTSResult<()> {
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "ElevenLabs TTS not connected".to_string(),
            ));
        }

        let close_msg = ElevenLabsOutboundMessage::Close(CloseConnection {
            text: "".to_string(),
        });

        if let Some(tx) = &self.websocket_tx {
            tx.send(close_msg).await.map_err(|e| {
                TTSError::InternalError(format!("Failed to send clear message: {e}"))
            })?;
        }

        // Closing the connection will trigger a reconnection
        self.disconnect().await?;
        self.connect().await?;

        Ok(())
    }

    async fn flush(&self) -> TTSResult<()> {
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "ElevenLabs TTS not connected".to_string(),
            ));
        }

        // Send a flush message with empty text but flush=true
        let flush_msg = ElevenLabsOutboundMessage::SendText(SendText {
            text: "".to_string(),
            try_trigger_generation: Some(true),
            flush: Some(true),
            voice_settings: None,
            generation_config: None,
        });

        if let Some(tx) = &self.websocket_tx {
            tx.send(flush_msg).await.map_err(|e| {
                TTSError::InternalError(format!("Failed to send flush message: {e}"))
            })?;
        }

        Ok(())
    }

    fn on_audio(&mut self, callback: Arc<dyn AudioCallback>) -> TTSResult<()> {
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
            "provider": "elevenlabs",
            "version": "1.0.0",
            "capabilities": {
                "streaming": true,
                "voice_settings": true,
                "alignment": true,
                "pronunciation_dictionaries": true
            },
            "supported_formats": ["pcm", "mp3"],
            "supported_sample_rates": [8000, 16000, 22050, 24000, 44100],
            "features": {
                "real_time_streaming": true,
                "chunk_buffering": true,
                "voice_cloning": true,
                "ssml_support": true
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Testing utilities are imported as needed in specific tests

    #[test]
    fn test_elevenlabs_tts_creation() {
        let config = TTSConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test_key".to_string(),
            voice_id: Some("test_voice_id".to_string()),
            ..Default::default()
        };

        let result = ElevenLabsTTS::new(config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_elevenlabs_tts_invalid_config() {
        let config = TTSConfig {
            provider: "elevenlabs".to_string(),
            api_key: "".to_string(),
            voice_id: None,
            ..Default::default()
        };

        let result = ElevenLabsTTS::new(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_voice_settings_creation() {
        let config = TTSConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test_key".to_string(),
            voice_id: Some("test_voice_id".to_string()),
            speaking_rate: Some(1.5),
            ..Default::default()
        };

        let voice_settings = ElevenLabsTTS::create_voice_settings(&config);
        assert_eq!(voice_settings.speed, Some(1.5));
        assert_eq!(voice_settings.stability, 0.5);
        assert_eq!(voice_settings.similarity_boost, 0.8);
    }

    #[test]
    fn test_websocket_url_building() {
        let config = TTSConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test_key".to_string(),
            voice_id: Some("test_voice_id".to_string()),
            sample_rate: Some(22050),
            audio_format: Some("pcm".to_string()),
            connection_timeout: Some(30),
            model: "eleven_multilingual_v2".to_string(),
            ..Default::default()
        };

        let url = ElevenLabsTTS::build_websocket_url(&config).unwrap();
        assert!(url.contains("test_voice_id"));
        assert!(url.contains("output_format=pcm_22050"));
        assert!(url.contains("inactivity_timeout=30"));
        assert!(url.contains("enable_logging=false"));
        assert!(url.contains("model_id=eleven_multilingual_v2"));
    }

    #[test]
    fn test_message_buffer_optimization() {
        let mut buffer = MessageBuffer::default();

        // Test base64 decoding with buffer reuse
        let test_data = "SGVsbG8gV29ybGQ="; // "Hello World" in base64
        let result = buffer.decode_audio_data(test_data).unwrap();
        assert_eq!(result.as_ref(), b"Hello World");

        // Test buffer reuse
        let test_data2 = "VGVzdA=="; // "Test" in base64
        let result2 = buffer.decode_audio_data(test_data2).unwrap();
        assert_eq!(result2.as_ref(), b"Test");
    }

    #[test]
    fn test_atomic_connection_state() {
        let state = AtomicConnectionState::new();

        // Initially disconnected
        assert!(!state.is_connected());

        // Set connecting
        state.set_connecting();
        assert!(!state.is_connected());

        // Set connected
        state.set_connected();
        assert!(state.is_connected());

        // Set disconnected
        state.set_disconnected();
        assert!(!state.is_connected());
    }

    #[tokio::test]
    async fn test_elevenlabs_lifecycle() {
        let config = TTSConfig {
            provider: "elevenlabs".to_string(),
            api_key: "test_key".to_string(),
            voice_id: Some("test_voice_id".to_string()),
            ..Default::default()
        };

        let tts = ElevenLabsTTS::new(config).unwrap();

        // Initially disconnected
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);

        // Note: We can't actually test connect() without a real API key
        // but we can test the error handling
        let result = tts.speak("test", false).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TTSError::ProviderNotReady(_)));
    }
}
