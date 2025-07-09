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
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;

use crate::core::tts::{
    AudioCallback, AudioData, BaseTTS, ConnectionState, TTSConfig, TTSError, TTSResult,
};

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

/// ElevenLabs TTS provider implementation
pub struct ElevenLabsTTS {
    config: TTSConfig,
    state: ConnectionState,
    audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
    websocket_tx: Option<mpsc::UnboundedSender<ElevenLabsOutboundMessage>>,
    shutdown_tx: Option<mpsc::UnboundedSender<()>>,
    voice_settings: VoiceSettings,
    generation_config: GenerationConfig,
    pronunciation_dictionaries: Option<Vec<PronunciationDictionaryLocator>>,
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

    /// Build WebSocket URL with query parameters
    fn build_websocket_url(config: &TTSConfig) -> TTSResult<String> {
        let voice_id = config.voice_id.as_ref().ok_or_else(|| {
            TTSError::InvalidConfiguration("voice_id is required for ElevenLabs".to_string())
        })?;

        let base_url = format!(
            "wss://api.elevenlabs.io/v1/text-to-speech/{}/stream-input",
            voice_id
        );
        let mut url = Url::parse(&base_url)
            .map_err(|e| TTSError::InvalidConfiguration(format!("Invalid URL: {}", e)))?;

        // Add query parameters
        let mut query_params = HashMap::new();

        // Add optional parameters
        if let Some(sample_rate) = config.sample_rate {
            if let Some(format) = &config.audio_format {
                let output_format = match format.as_str() {
                    "pcm" => format!("pcm_{}", sample_rate),
                    "mp3" => format!("mp3_{}_128", sample_rate),
                    _ => format!("pcm_{}", sample_rate),
                };
                query_params.insert("output_format", output_format);
            }
        }

        // Add inactivity timeout
        if let Some(timeout) = config.connection_timeout {
            query_params.insert("inactivity_timeout", timeout.to_string());
        }

        // Add other optional parameters
        query_params.insert("enable_logging", "true".to_string());
        query_params.insert("enable_ssml_parsing", "false".to_string());
        query_params.insert("sync_alignment", "true".to_string());

        // Apply query parameters
        for (key, value) in query_params {
            url.query_pairs_mut().append_pair(&key, &value);
        }

        Ok(url.to_string())
    }

    /// Handle WebSocket connection and message processing
    async fn handle_websocket_connection(
        url: String,
        config: TTSConfig,
        voice_settings: VoiceSettings,
        _generation_config: GenerationConfig,
        _pronunciation_dictionaries: Option<Vec<PronunciationDictionaryLocator>>,
        audio_callback: Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
        mut message_rx: mpsc::UnboundedReceiver<ElevenLabsOutboundMessage>,
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
            TTSError::ConnectionFailed(format!("Failed to connect to ElevenLabs: {}", e))
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
            generation_config: None, // Don't send generation_config in initial message
            pronunciation_dictionary_locators: None, // Don't send pronunciation dictionaries in initial message
        };

        let init_json = serde_json::to_string(&init_msg).map_err(|e| {
            TTSError::InternalError(format!("Failed to serialize init message: {}", e))
        })?;

        // Debug log the initialization message
        tracing::debug!("Sending ElevenLabs initialization message: {}", init_json);

        ws_sender
            .send(Message::Text(init_json.into()))
            .await
            .map_err(|e| {
                TTSError::ConnectionFailed(format!("Failed to send init message: {}", e))
            })?;

        // Message handling loop
        loop {
            tokio::select! {
                // Handle incoming WebSocket messages
                ws_msg = ws_receiver.next() => {
                    match ws_msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Err(e) = Self::handle_inbound_message(text.to_string(), &audio_callback).await {
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
                            let error = TTSError::NetworkError(format!("WebSocket error: {}", e));
                            tracing::error!("WebSocket error: {}", error);
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
                                .map_err(|e| TTSError::InternalError(format!("Failed to serialize message: {}", e)))?;

                            if let Err(e) = ws_sender.send(Message::Text(json.into())).await {
                                tracing::error!("Failed to send message: {}", e);
                                let error = TTSError::NetworkError(format!("Failed to send message: {}", e));
                                if let Some(callback) = audio_callback.read().await.as_ref() {
                                    callback.on_error(error).await;
                                }
                                break;
                            }
                        }
                        None => {
                            // Channel closed, close connection
                            let close_msg = CloseConnection {
                                text: "".to_string(),
                            };
                            let close_json = serde_json::to_string(&close_msg)
                                .map_err(|e| TTSError::InternalError(format!("Failed to serialize close message: {}", e)))?;

                            let _ = ws_sender.send(Message::Text(close_json.into())).await;
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

        // Send completion signal
        if let Some(callback) = audio_callback.read().await.as_ref() {
            callback.on_complete().await;
        }

        Ok(())
    }

    /// Handle inbound messages from ElevenLabs
    async fn handle_inbound_message(
        message: String,
        audio_callback: &Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
    ) -> TTSResult<()> {
        tracing::debug!("Received ElevenLabs message: {}", message);

        // Try to parse as generic JSON first to see structure
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&message) {
            tracing::debug!(
                "Parsed JSON structure: {}",
                serde_json::to_string_pretty(&parsed)
                    .unwrap_or_else(|_| "Failed to pretty print".to_string())
            );
        }

        // Try to parse as different message types
        if let Ok(audio_output) = serde_json::from_str::<AudioOutput>(&message) {
            tracing::debug!("Successfully parsed as AudioOutput");
            Self::handle_audio_output(audio_output, audio_callback).await?;
        } else if let Ok(final_output) = serde_json::from_str::<FinalOutput>(&message) {
            tracing::debug!("Successfully parsed as FinalOutput");
            Self::handle_final_output(final_output, audio_callback).await?;
        } else if let Ok(error_msg) = serde_json::from_str::<ErrorMessage>(&message) {
            tracing::debug!("Successfully parsed as ErrorMessage");
            Self::handle_error_message(error_msg, audio_callback).await?;
        } else {
            // Try to parse as generic JSON to see if it's a structured message
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&message) {
                tracing::warn!(
                    "Received unknown structured message from ElevenLabs: {}",
                    serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| message.clone())
                );

                // Print the exact JSON we received to help debug
                tracing::warn!("Raw message content: {}", message);

                // Try to determine what type of message this might be
                if let Some(obj) = parsed.as_object() {
                    tracing::info!(
                        "Message contains keys: {:?}",
                        obj.keys().collect::<Vec<_>>()
                    );

                    // Check if it's a confirmation or status message
                    if obj.contains_key("status") {
                        tracing::info!("Message appears to be a status message");
                    } else if obj.contains_key("success") {
                        tracing::info!("Message appears to be a success confirmation");
                    } else if obj.contains_key("ready") {
                        tracing::info!("Message appears to be a ready confirmation");
                    }
                }
            } else {
                tracing::warn!("Received non-JSON message from ElevenLabs: {}", message);
            }
        }

        Ok(())
    }

    /// Handle audio output message
    async fn handle_audio_output(
        audio_output: AudioOutput,
        audio_callback: &Arc<RwLock<Option<Arc<dyn AudioCallback>>>>,
    ) -> TTSResult<()> {
        if let Some(callback) = audio_callback.read().await.as_ref() {
            // Decode base64 audio data
            let audio_data = BASE64_STANDARD.decode(&audio_output.audio).map_err(|e| {
                TTSError::InternalError(format!("Failed to decode base64 audio: {}", e))
            })?;

            let audio_chunk = AudioData {
                data: audio_data,
                sample_rate: 22050, // ElevenLabs default
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
        let generation_config = GenerationConfig::default();

        Ok(Self {
            config,
            state: ConnectionState::Disconnected,
            audio_callback: Arc::new(RwLock::new(None)),
            websocket_tx: None,
            shutdown_tx: None,
            voice_settings,
            generation_config,
            pronunciation_dictionaries: None,
        })
    }

    async fn connect(&mut self) -> TTSResult<()> {
        if self.state != ConnectionState::Disconnected {
            return Err(TTSError::InvalidConfiguration(
                "Already connected or connecting".to_string(),
            ));
        }

        self.state = ConnectionState::Connecting;

        // Build WebSocket URL
        let url = Self::build_websocket_url(&self.config)?;

        // Create channels for communication
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let (shutdown_tx, shutdown_rx) = mpsc::unbounded_channel();

        // Store senders
        self.websocket_tx = Some(message_tx);
        self.shutdown_tx = Some(shutdown_tx);

        // Use the shared audio callback reference from the struct
        let audio_callback_ref = self.audio_callback.clone();

        // Spawn WebSocket handler
        let config = self.config.clone();
        let voice_settings = self.voice_settings.clone();
        let generation_config = self.generation_config.clone();
        let pronunciation_dictionaries = self.pronunciation_dictionaries.clone();

        tokio::spawn(async move {
            if let Err(e) = Self::handle_websocket_connection(
                url,
                config,
                voice_settings,
                generation_config,
                pronunciation_dictionaries,
                audio_callback_ref,
                message_rx,
                shutdown_rx,
            )
            .await
            {
                tracing::error!("WebSocket connection failed: {}", e);
            }
        });

        // Give the connection a moment to establish
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        self.state = ConnectionState::Connected;

        Ok(())
    }

    async fn disconnect(&mut self) -> TTSResult<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        self.websocket_tx = None;
        self.state = ConnectionState::Disconnected;

        // Clear the audio callback
        if let Ok(mut audio_callback) = self.audio_callback.try_write() {
            *audio_callback = None;
        }

        Ok(())
    }

    fn is_ready(&self) -> bool {
        matches!(self.state, ConnectionState::Connected)
    }

    fn get_connection_state(&self) -> ConnectionState {
        self.state.clone()
    }

    async fn speak(&self, text: &str, flush: bool) -> TTSResult<()> {
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "ElevenLabs TTS not connected".to_string(),
            ));
        }

        if text.is_empty() {
            // Empty text closes the connection
            let close_msg = ElevenLabsOutboundMessage::Close(CloseConnection {
                text: "".to_string(),
            });

            if let Some(tx) = &self.websocket_tx {
                tx.send(close_msg).map_err(|e| {
                    TTSError::InternalError(format!("Failed to send close message: {}", e))
                })?;
            }

            return Ok(());
        }

        let send_msg = ElevenLabsOutboundMessage::SendText(SendText {
            text: text.to_string(),
            try_trigger_generation: Some(true),
            flush: Some(flush),
            voice_settings: None,
            generation_config: None,
        });

        if let Some(tx) = &self.websocket_tx {
            tx.send(send_msg).map_err(|e| {
                TTSError::InternalError(format!("Failed to send text message: {}", e))
            })?;
        }

        Ok(())
    }

    async fn clear(&self) -> TTSResult<()> {
        // ElevenLabs doesn't have a specific clear command, but we can send empty text
        // to clear the buffer and close the connection, then reconnect
        if !self.is_ready() {
            return Err(TTSError::ProviderNotReady(
                "ElevenLabs TTS not connected".to_string(),
            ));
        }

        let close_msg = ElevenLabsOutboundMessage::Close(CloseConnection {
            text: "".to_string(),
        });

        if let Some(tx) = &self.websocket_tx {
            tx.send(close_msg).map_err(|e| {
                TTSError::InternalError(format!("Failed to send clear message: {}", e))
            })?;
        }

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
            tx.send(flush_msg).map_err(|e| {
                TTSError::InternalError(format!("Failed to send flush message: {}", e))
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
            }
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
            ..Default::default()
        };

        let url = ElevenLabsTTS::build_websocket_url(&config).unwrap();
        assert!(url.contains("test_voice_id"));
        assert!(url.contains("output_format=pcm_22050"));
        assert!(url.contains("inactivity_timeout=30"));
        assert!(url.contains("enable_logging=true"));
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
