//! ElevenLabs STT WebSocket client implementation.
//!
//! This module contains the main `ElevenLabsSTT` struct that implements the
//! `BaseSTT` trait for real-time speech-to-text streaming.

use base64::prelude::*;
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, error, info, warn};

use super::config::{CommitStrategy, ElevenLabsAudioFormat, ElevenLabsRegion, ElevenLabsSTTConfig};
use super::messages::{ElevenLabsMessage, InputAudioChunk};
use crate::core::stt::base::{
    BaseSTT, STTConfig, STTError, STTErrorCallback, STTResult, STTResultCallback,
};

// =============================================================================
// Type Aliases
// =============================================================================

/// Type alias for the async result callback function.
type AsyncSTTCallback = Box<
    dyn Fn(STTResult) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Type alias for the async error callback function.
type AsyncErrorCallback = Box<
    dyn Fn(STTError) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

// =============================================================================
// Connection State
// =============================================================================

/// Connection state for the WebSocket client.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Error variant will be used for connection error reporting
pub(crate) enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

// =============================================================================
// ElevenLabsSTT Client
// =============================================================================

/// ElevenLabs STT WebSocket client.
///
/// This struct implements real-time speech-to-text using the ElevenLabs
/// WebSocket API. It manages:
/// - WebSocket connection lifecycle
/// - Audio data streaming to the API
/// - Transcription result callbacks
/// - Error handling and recovery
///
/// # Architecture
///
/// The implementation uses a multi-channel architecture for low-latency processing:
///
/// ```text
/// ┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
/// │   send_audio()  │────▶│  ws_sender (mpsc)│────▶│  WebSocket Task │
/// └─────────────────┘     └──────────────────┘     └────────┬────────┘
///                                                           │
///                         ┌──────────────────┐              │
///                         │  result_tx (mpsc)│◀─────────────┘
///                         └────────┬─────────┘
///                                  │
///                         ┌────────▼─────────┐
///                         │ Result Forward   │────▶ User Callback
///                         │      Task        │
///                         └──────────────────┘
/// ```
///
/// # Thread Safety
///
/// All shared state is protected by either:
/// - `tokio::sync::Mutex` for async-safe access to callbacks
/// - `Arc<Notify>` for state change notifications
/// - Bounded `mpsc` channels for backpressure control
pub struct ElevenLabsSTT {
    /// Configuration for the STT client
    pub(crate) config: Option<ElevenLabsSTTConfig>,

    /// Current connection state
    pub(crate) state: ConnectionState,

    /// State change notification
    state_notify: Arc<Notify>,

    /// WebSocket sender for audio data
    /// Uses bounded channel (32 items) to provide backpressure
    ws_sender: Option<mpsc::Sender<Bytes>>,

    /// Shutdown signal sender
    shutdown_tx: Option<oneshot::Sender<()>>,

    /// Result channel sender
    result_tx: Option<mpsc::UnboundedSender<STTResult>>,

    /// Error channel sender
    error_tx: Option<mpsc::UnboundedSender<STTError>>,

    /// Connection task handle
    connection_handle: Option<tokio::task::JoinHandle<()>>,

    /// Result forwarding task handle
    result_forward_handle: Option<tokio::task::JoinHandle<()>>,

    /// Error forwarding task handle
    error_forward_handle: Option<tokio::task::JoinHandle<()>>,

    /// Shared callback storage for async access
    pub(crate) result_callback: Arc<Mutex<Option<AsyncSTTCallback>>>,

    /// Error callback storage
    error_callback: Arc<Mutex<Option<AsyncErrorCallback>>>,

    /// Session ID from the ElevenLabs connection
    session_id: Option<String>,
}

impl Default for ElevenLabsSTT {
    fn default() -> Self {
        Self {
            config: None,
            state: ConnectionState::Disconnected,
            state_notify: Arc::new(Notify::new()),
            ws_sender: None,
            shutdown_tx: None,
            result_tx: None,
            error_tx: None,
            connection_handle: None,
            result_forward_handle: None,
            error_forward_handle: None,
            result_callback: Arc::new(Mutex::new(None)),
            error_callback: Arc::new(Mutex::new(None)),
            session_id: None,
        }
    }
}

impl ElevenLabsSTT {
    /// Build the WebSocket URL with query parameters.
    pub(crate) fn build_websocket_url(
        &self,
        config: &ElevenLabsSTTConfig,
    ) -> Result<String, STTError> {
        // Pre-allocate with estimated capacity
        let mut url = String::with_capacity(512);

        // Base URL from region
        url.push_str(config.region.websocket_base_url());
        url.push_str("/v1/speech-to-text/realtime?");

        // Required: model_id
        url.push_str("model_id=");
        url.push_str(&config.model_id);

        // Required: audio_format
        url.push_str("&audio_format=");
        url.push_str(config.audio_format.as_str());

        // Optional: language_code (extract just the language code part)
        if !config.base.language.is_empty() {
            url.push_str("&language_code=");
            let lang = if config.base.language.contains('-') {
                config
                    .base
                    .language
                    .split('-')
                    .next()
                    .unwrap_or(&config.base.language)
            } else {
                &config.base.language
            };
            url.push_str(lang);
        }

        // Commit strategy
        url.push_str("&commit_strategy=");
        url.push_str(config.commit_strategy.as_str());

        // Include timestamps
        url.push_str("&include_timestamps=");
        url.push_str(if config.include_timestamps {
            "true"
        } else {
            "false"
        });

        // Optional VAD parameters
        if let Some(threshold) = config.vad_silence_threshold_secs {
            url.push_str("&vad_silence_threshold_secs=");
            url.push_str(&threshold.to_string());
        }

        if let Some(threshold) = config.vad_threshold {
            url.push_str("&vad_threshold=");
            url.push_str(&threshold.to_string());
        }

        if let Some(duration) = config.min_speech_duration_ms {
            url.push_str("&min_speech_duration_ms=");
            url.push_str(&duration.to_string());
        }

        if let Some(duration) = config.min_silence_duration_ms {
            url.push_str("&min_silence_duration_ms=");
            url.push_str(&duration.to_string());
        }

        // Enable logging (for debugging)
        if config.enable_logging {
            url.push_str("&enable_logging=true");
        }

        Ok(url)
    }

    /// Get the host name from the region for HTTP headers.
    pub(crate) fn get_host_from_region(region: &ElevenLabsRegion) -> &'static str {
        region.host()
    }

    /// Encode audio data for transmission to ElevenLabs.
    ///
    /// ElevenLabs expects base64-encoded audio in JSON messages.
    #[inline]
    #[cfg_attr(not(test), allow(dead_code))] // Used in tests and can be used externally
    pub(crate) fn encode_audio_for_transmission(audio_data: &[u8]) -> String {
        BASE64_STANDARD.encode(audio_data)
    }

    /// Handle incoming WebSocket messages from ElevenLabs.
    ///
    /// This method is optimized for the hot path of message processing:
    /// - Parses JSON once
    /// - Branches on message type
    /// - Converts to internal STTResult format
    /// - Non-blocking result transmission
    pub(crate) fn handle_websocket_message(
        message: Message,
        result_tx: &mpsc::UnboundedSender<STTResult>,
        session_id: &mut Option<String>,
    ) -> Result<(), STTError> {
        match message {
            Message::Text(text) => {
                debug!("Received ElevenLabs message: {}", text);

                match ElevenLabsMessage::parse(&text) {
                    Ok(parsed_msg) => match parsed_msg {
                        ElevenLabsMessage::SessionStarted(session) => {
                            info!("ElevenLabs STT session started: {}", session.session_id);
                            *session_id = Some(session.session_id);
                        }

                        ElevenLabsMessage::PartialTranscript(partial) => {
                            if !partial.text.is_empty() {
                                let stt_result = STTResult::new(
                                    partial.text,
                                    false, // is_final = false (interim)
                                    0.0,   // No confidence for partials
                                );

                                if result_tx.send(stt_result).is_err() {
                                    warn!("Failed to send partial result - channel closed");
                                }
                            }
                        }

                        ElevenLabsMessage::CommittedTranscript(committed) => {
                            let stt_result = STTResult::new(
                                committed.text,
                                true, // is_final = true
                                1.0,  // High confidence for committed
                            );

                            if result_tx.send(stt_result).is_err() {
                                warn!("Failed to send committed result - channel closed");
                            }
                        }

                        ElevenLabsMessage::CommittedTranscriptWithTimestamps(committed) => {
                            // Calculate average confidence from word logprobs
                            let confidence = Self::calculate_confidence(&committed.words);

                            let stt_result = STTResult::new(
                                committed.text,
                                true, // is_final = true
                                confidence.clamp(0.0, 1.0),
                            );

                            if result_tx.send(stt_result).is_err() {
                                warn!("Failed to send timestamped result - channel closed");
                            }
                        }

                        ElevenLabsMessage::Error(err) => {
                            let error_msg = format!(
                                "ElevenLabs STT error ({}): {}",
                                err.message_type, err.error
                            );
                            error!("{}", error_msg);

                            return match err.message_type.as_str() {
                                "auth_error" => Err(STTError::AuthenticationFailed(err.error)),
                                "quota_exceeded_error" => Err(STTError::ProviderError(format!(
                                    "Quota exceeded: {}",
                                    err.error
                                ))),
                                _ => Err(STTError::ProviderError(err.error)),
                            };
                        }

                        ElevenLabsMessage::Unknown(raw) => {
                            debug!("Received unknown ElevenLabs message type: {}", raw);
                        }
                    },
                    Err(e) => {
                        warn!("Failed to parse ElevenLabs message: {}", e);
                    }
                }
            }

            Message::Close(close_frame) => {
                info!("ElevenLabs WebSocket closed: {:?}", close_frame);
            }

            Message::Ping(_) => {
                debug!("Received ping from ElevenLabs");
            }

            Message::Pong(_) => {
                debug!("Received pong from ElevenLabs");
            }

            _ => {
                debug!("Received unexpected message type");
            }
        }

        Ok(())
    }

    /// Calculate confidence from word logprobs.
    fn calculate_confidence(words: &[super::messages::WordTiming]) -> f32 {
        if words.is_empty() {
            return 1.0;
        }

        let sum: f64 = words
            .iter()
            .filter_map(|w| w.logprob)
            .map(|lp| lp.exp())
            .sum();

        let count = words.iter().filter(|w| w.logprob.is_some()).count();

        if count > 0 {
            (sum / count as f64) as f32
        } else {
            1.0
        }
    }

    /// Start the WebSocket connection to ElevenLabs STT API.
    async fn start_connection(&mut self, config: ElevenLabsSTTConfig) -> Result<(), STTError> {
        let ws_url = self.build_websocket_url(&config)?;

        // Create channels for communication
        let (ws_tx, mut ws_rx) = mpsc::channel::<Bytes>(32);
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let (error_tx, mut error_rx) = mpsc::unbounded_channel::<STTError>();
        let (connected_tx, connected_rx) = oneshot::channel::<()>();

        // Store channels
        self.ws_sender = Some(ws_tx);
        self.shutdown_tx = Some(shutdown_tx);
        self.result_tx = Some(result_tx.clone());
        self.error_tx = Some(error_tx.clone());

        // Clone necessary data for the connection task
        let api_key = config.base.api_key.clone();
        let host = Self::get_host_from_region(&config.region);

        // Start the connection task
        let connection_handle = tokio::spawn(async move {
            // Build WebSocket request with ElevenLabs authentication
            let request = match tokio_tungstenite::tungstenite::http::Request::builder()
                .method("GET")
                .uri(&ws_url)
                .header("Host", host)
                .header("Upgrade", "websocket")
                .header("Connection", "upgrade")
                .header("Sec-WebSocket-Key", generate_key())
                .header("Sec-WebSocket-Version", "13")
                .header("xi-api-key", &api_key)
                .body(())
            {
                Ok(request) => request,
                Err(e) => {
                    let stt_error = STTError::ConnectionFailed(format!(
                        "Failed to create WebSocket request: {e}"
                    ));
                    error!("{}", stt_error);
                    let _ = error_tx.send(stt_error);
                    return;
                }
            };

            // Connect to ElevenLabs
            let (ws_stream, _response) = match connect_async(request).await {
                Ok(result) => result,
                Err(e) => {
                    let stt_error =
                        STTError::ConnectionFailed(format!("Failed to connect to ElevenLabs: {e}"));
                    error!("{}", stt_error);
                    let _ = error_tx.send(stt_error);
                    return;
                }
            };

            info!("Connected to ElevenLabs STT WebSocket");

            let (mut ws_sink, mut ws_stream) = ws_stream.split();

            // Track session ID
            let mut session_id: Option<String> = None;
            let mut connected_tx = Some(connected_tx);

            // Main event loop
            loop {
                tokio::select! {
                    // Handle outgoing audio data
                    Some(audio_data) = ws_rx.recv() => {
                        let audio_base64 = BASE64_STANDARD.encode(&audio_data);
                        let input_msg = InputAudioChunk::new(audio_base64);

                        let json_msg = match serde_json::to_string(&input_msg) {
                            Ok(json) => json,
                            Err(e) => {
                                let stt_error = STTError::AudioProcessingError(format!(
                                    "Failed to serialize audio chunk: {e}"
                                ));
                                error!("{}", stt_error);
                                let _ = error_tx.send(stt_error);
                                continue;
                            }
                        };

                        let message = Message::Text(json_msg.into());
                        if let Err(e) = ws_sink.send(message).await {
                            let stt_error = STTError::NetworkError(format!(
                                "Failed to send audio to ElevenLabs: {e}"
                            ));
                            error!("{}", stt_error);
                            let _ = error_tx.send(stt_error);
                            break;
                        }

                        debug!("Sent {} bytes of audio to ElevenLabs", audio_data.len());
                    }

                    // Handle incoming messages
                    message = ws_stream.next() => {
                        match message {
                            Some(Ok(msg)) => {
                                if let Err(e) = Self::handle_websocket_message(
                                    msg,
                                    &result_tx,
                                    &mut session_id,
                                ) {
                                    error!("ElevenLabs streaming error: {}", e);
                                    let _ = error_tx.send(e);
                                    break;
                                }

                                // Signal connection ready after receiving session_started
                                if session_id.is_some() && let Some(tx) = connected_tx.take() {
                                    let _ = tx.send(());
                                }
                            }
                            Some(Err(e)) => {
                                let stt_error = STTError::NetworkError(format!(
                                    "WebSocket error: {e}"
                                ));
                                error!("{}", stt_error);
                                let _ = error_tx.send(stt_error);
                                break;
                            }
                            None => {
                                info!("ElevenLabs WebSocket stream ended");
                                break;
                            }
                        }
                    }

                    // Handle shutdown signal
                    _ = &mut shutdown_rx => {
                        info!("Received shutdown signal for ElevenLabs STT");
                        let _ = ws_sink.send(Message::Close(None)).await;
                        break;
                    }
                }
            }

            info!("ElevenLabs STT WebSocket connection closed");
        });

        self.connection_handle = Some(connection_handle);

        // Start result forwarding task
        let callback_ref = self.result_callback.clone();
        let result_forwarding_handle = tokio::spawn(async move {
            while let Some(result) = result_rx.recv().await {
                if let Some(callback) = callback_ref.lock().await.as_ref() {
                    callback(result).await;
                } else {
                    debug!(
                        "ElevenLabs STT result (no callback): {} (final: {}, confidence: {})",
                        result.transcript, result.is_final, result.confidence
                    );
                }
            }
        });

        self.result_forward_handle = Some(result_forwarding_handle);

        // Start error forwarding task
        let error_callback_ref = self.error_callback.clone();
        let error_forwarding_handle = tokio::spawn(async move {
            while let Some(error) = error_rx.recv().await {
                if let Some(callback) = error_callback_ref.lock().await.as_ref() {
                    callback(error).await;
                } else {
                    error!("ElevenLabs STT error (no callback registered): {}", error);
                }
            }
        });

        self.error_forward_handle = Some(error_forwarding_handle);

        // Update state and wait for connection
        self.state = ConnectionState::Connecting;

        // Wait for session_started message with timeout
        match timeout(Duration::from_secs(10), connected_rx).await {
            Ok(Ok(())) => {
                self.state = ConnectionState::Connected;
                self.state_notify.notify_waiters();
                info!("Successfully connected to ElevenLabs STT");
                Ok(())
            }
            Ok(Err(_)) => {
                let error_msg = "Connection channel closed before session started".to_string();
                self.state = ConnectionState::Error(error_msg.clone());
                Err(STTError::ConnectionFailed(error_msg))
            }
            Err(_) => {
                let error_msg = "Connection timeout waiting for session_started".to_string();
                self.state = ConnectionState::Error(error_msg.clone());
                Err(STTError::ConnectionFailed(error_msg))
            }
        }
    }
}

impl Drop for ElevenLabsSTT {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

// =============================================================================
// BaseSTT Trait Implementation
// =============================================================================

#[async_trait::async_trait]
impl BaseSTT for ElevenLabsSTT {
    fn new(config: STTConfig) -> Result<Self, STTError> {
        if config.api_key.is_empty() {
            return Err(STTError::AuthenticationFailed(
                "API key is required for ElevenLabs STT".to_string(),
            ));
        }

        let audio_format = ElevenLabsAudioFormat::from_sample_rate(config.sample_rate);

        let elevenlabs_config = ElevenLabsSTTConfig {
            base: config,
            model_id: "scribe_v2_realtime".to_string(),
            audio_format,
            commit_strategy: CommitStrategy::Vad,
            include_timestamps: false,
            // Set sensible VAD defaults for responsive end-of-speech detection
            vad_silence_threshold_secs: Some(0.5), // 500ms silence triggers commit
            vad_threshold: None,                   // Use API default sensitivity
            min_speech_duration_ms: Some(50),      // Minimum 50ms of speech to count
            min_silence_duration_ms: Some(300),    // 300ms silence for end-of-speech
            enable_logging: false,
            region: ElevenLabsRegion::Default,
        };

        Ok(Self {
            config: Some(elevenlabs_config),
            state: ConnectionState::Disconnected,
            state_notify: Arc::new(Notify::new()),
            ws_sender: None,
            shutdown_tx: None,
            result_tx: None,
            error_tx: None,
            connection_handle: None,
            result_forward_handle: None,
            error_forward_handle: None,
            result_callback: Arc::new(Mutex::new(None)),
            error_callback: Arc::new(Mutex::new(None)),
            session_id: None,
        })
    }

    async fn connect(&mut self) -> Result<(), STTError> {
        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

        self.start_connection(config.clone()).await
    }

    async fn disconnect(&mut self) -> Result<(), STTError> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        if let Some(handle) = self.connection_handle.take() {
            let _ = timeout(Duration::from_secs(5), handle).await;
        }

        if let Some(handle) = self.result_forward_handle.take() {
            handle.abort();
            let _ = handle.await;
        }

        if let Some(handle) = self.error_forward_handle.take() {
            handle.abort();
            let _ = handle.await;
        }

        self.ws_sender = None;
        self.result_tx = None;
        self.error_tx = None;
        *self.result_callback.lock().await = None;
        *self.error_callback.lock().await = None;
        self.session_id = None;

        self.state = ConnectionState::Disconnected;
        self.state_notify.notify_waiters();

        info!("Disconnected from ElevenLabs STT");
        Ok(())
    }

    fn is_ready(&self) -> bool {
        matches!(self.state, ConnectionState::Connected) && self.ws_sender.is_some()
    }

    async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
        if !self.is_ready() {
            return Err(STTError::ConnectionFailed(
                "Not connected to ElevenLabs STT".to_string(),
            ));
        }

        if let Some(ws_sender) = &self.ws_sender {
            let data_len = audio_data.len();
            let audio_bytes = Bytes::from(audio_data);

            ws_sender
                .send(audio_bytes)
                .await
                .map_err(|e| STTError::NetworkError(format!("Failed to send audio data: {e}")))?;

            debug!("Queued {} bytes of audio for ElevenLabs", data_len);
        }

        Ok(())
    }

    async fn on_result(&mut self, callback: STTResultCallback) -> Result<(), STTError> {
        *self.result_callback.lock().await = Some(Box::new(move |result| {
            let cb = callback.clone();
            Box::pin(async move {
                cb(result).await;
            })
        }));
        Ok(())
    }

    async fn on_error(&mut self, callback: STTErrorCallback) -> Result<(), STTError> {
        *self.error_callback.lock().await = Some(Box::new(move |error| {
            let cb = callback.clone();
            Box::pin(async move {
                cb(error).await;
            })
        }));
        Ok(())
    }

    fn get_config(&self) -> Option<&STTConfig> {
        self.config.as_ref().map(|c| &c.base)
    }

    async fn update_config(&mut self, config: STTConfig) -> Result<(), STTError> {
        if self.is_ready() {
            self.disconnect().await?;
        }

        let audio_format = ElevenLabsAudioFormat::from_sample_rate(config.sample_rate);
        let existing = self.config.take();

        let elevenlabs_config = ElevenLabsSTTConfig {
            base: config,
            model_id: existing
                .as_ref()
                .map(|c| c.model_id.clone())
                .unwrap_or_else(|| "scribe_v2_realtime".to_string()),
            audio_format,
            commit_strategy: existing
                .as_ref()
                .map(|c| c.commit_strategy)
                .unwrap_or(CommitStrategy::Vad),
            include_timestamps: existing.as_ref().is_some_and(|c| c.include_timestamps),
            vad_silence_threshold_secs: existing
                .as_ref()
                .and_then(|c| c.vad_silence_threshold_secs),
            vad_threshold: existing.as_ref().and_then(|c| c.vad_threshold),
            min_speech_duration_ms: existing.as_ref().and_then(|c| c.min_speech_duration_ms),
            min_silence_duration_ms: existing.as_ref().and_then(|c| c.min_silence_duration_ms),
            enable_logging: existing.as_ref().is_some_and(|c| c.enable_logging),
            region: existing
                .as_ref()
                .map(|c| c.region)
                .unwrap_or(ElevenLabsRegion::Default),
        };

        self.config = Some(elevenlabs_config);

        self.connect().await?;
        Ok(())
    }

    fn get_provider_info(&self) -> &'static str {
        "ElevenLabs STT Real-Time WebSocket"
    }
}

// =============================================================================
// ElevenLabs-Specific Helper Methods
// =============================================================================

impl ElevenLabsSTT {
    /// Get the current session ID.
    ///
    /// The session ID is assigned by ElevenLabs when the connection is established.
    pub fn get_session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Update ElevenLabs-specific settings.
    ///
    /// This allows updating ElevenLabs-specific parameters without
    /// affecting the base STT configuration. Requires reconnection.
    pub async fn update_elevenlabs_settings(
        &mut self,
        model_id: Option<String>,
        include_timestamps: Option<bool>,
        region: Option<ElevenLabsRegion>,
    ) -> Result<(), STTError> {
        if self.is_ready() {
            self.disconnect().await?;
        }

        if let Some(config) = &mut self.config {
            if let Some(model) = model_id {
                config.model_id = model;
            }
            if let Some(timestamps) = include_timestamps {
                config.include_timestamps = timestamps;
            }
            if let Some(reg) = region {
                config.region = reg;
            }
        }

        self.connect().await
    }

    /// Set VAD (Voice Activity Detection) parameters.
    ///
    /// These settings control how ElevenLabs detects speech boundaries.
    pub async fn set_vad_settings(
        &mut self,
        silence_threshold_secs: Option<f32>,
        threshold: Option<f32>,
        min_speech_duration_ms: Option<u32>,
        min_silence_duration_ms: Option<u32>,
    ) -> Result<(), STTError> {
        if self.is_ready() {
            self.disconnect().await?;
        }

        if let Some(config) = &mut self.config {
            config.vad_silence_threshold_secs = silence_threshold_secs;
            config.vad_threshold = threshold;
            config.min_speech_duration_ms = min_speech_duration_ms;
            config.min_silence_duration_ms = min_silence_duration_ms;
        }

        self.connect().await
    }
}
