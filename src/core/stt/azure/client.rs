//! Azure STT WebSocket client implementation.
//!
//! This module contains the main `AzureSTT` struct that implements the
//! `BaseSTT` trait for real-time speech-to-text streaming using Microsoft
//! Azure Cognitive Services Speech-to-Text API.
//!
//! # Architecture
//!
//! The implementation follows the same patterns as DeepgramSTT and ElevenLabsSTT:
//!
//! ```text
//! ┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
//! │   send_audio()  │────▶│  ws_sender (mpsc)│────▶│  WebSocket Task │
//! └─────────────────┘     └──────────────────┘     └────────┬────────┘
//!                                                           │
//!                         ┌──────────────────┐              │
//!                         │  result_tx (mpsc)│◀─────────────┘
//!                         └────────┬─────────┘
//!                                  │
//!                         ┌────────▼─────────┐
//!                         │ Result Forward   │────▶ User Callback
//!                         │      Task        │
//!                         └──────────────────┘
//! ```
//!
//! # Azure-Specific Details
//!
//! - Uses `Ocp-Apim-Subscription-Key` header for authentication
//! - Includes `X-ConnectionId` header for debugging
//! - Content-Type specifies PCM audio format
//! - Messages may have header prefixes before JSON content

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use tokio::time::{Instant, interval, timeout};
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, error, info, warn};

use super::config::AzureSTTConfig;
use super::messages::{AzureMessage, RecognitionStatus};
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
enum ConnectionState {
    /// Not connected to Azure.
    Disconnected,
    /// In the process of establishing connection.
    Connecting,
    /// Connected and ready to receive audio.
    Connected,
    /// An error occurred (stores error message for debugging).
    #[allow(dead_code)]
    Error(String),
}

// =============================================================================
// AzureSTT Client
// =============================================================================

/// Microsoft Azure Speech-to-Text WebSocket client.
///
/// This struct implements real-time speech-to-text using the Azure Cognitive
/// Services Speech-to-Text WebSocket API. It manages:
///
/// - WebSocket connection lifecycle
/// - Audio data streaming to Azure
/// - Transcription result callbacks (both interim and final)
/// - Error handling and recovery
/// - Keep-alive mechanism to prevent timeout during silence
///
/// # Thread Safety
///
/// All shared state is protected by:
/// - `tokio::sync::Mutex` for async-safe access to callbacks
/// - `Arc<Notify>` for state change notifications
/// - Bounded `mpsc` channels for backpressure control
///
/// # Example
///
/// ```rust,no_run
/// use sayna::core::stt::{BaseSTT, STTConfig};
/// use sayna::core::stt::azure::AzureSTT;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = STTConfig {
///         api_key: "your-azure-subscription-key".to_string(),
///         language: "en-US".to_string(),
///         sample_rate: 16000,
///         ..Default::default()
///     };
///
///     let mut stt = AzureSTT::new(config)?;
///
///     // Register result callback
///     stt.on_result(Arc::new(|result| {
///         Box::pin(async move {
///             println!("Transcript: {}", result.transcript);
///         })
///     })).await?;
///
///     // Connect to Azure
///     stt.connect().await?;
///
///     // Send audio data
///     let audio_data = vec![0u8; 1024]; // Your PCM audio data
///     stt.send_audio(audio_data).await?;
///
///     // Disconnect when done
///     stt.disconnect().await?;
///
///     Ok(())
/// }
/// ```
pub struct AzureSTT {
    /// Configuration for the STT client.
    config: Option<AzureSTTConfig>,

    /// Current connection state.
    state: ConnectionState,

    /// State change notification.
    state_notify: Arc<Notify>,

    /// WebSocket sender for audio data.
    /// Uses bounded channel (32 items) to provide backpressure.
    ws_sender: Option<mpsc::Sender<Bytes>>,

    /// Shutdown signal sender.
    shutdown_tx: Option<oneshot::Sender<()>>,

    /// Result channel sender.
    result_tx: Option<mpsc::UnboundedSender<STTResult>>,

    /// Error channel sender.
    error_tx: Option<mpsc::UnboundedSender<STTError>>,

    /// Connection task handle.
    connection_handle: Option<tokio::task::JoinHandle<()>>,

    /// Result forwarding task handle.
    result_forward_handle: Option<tokio::task::JoinHandle<()>>,

    /// Error forwarding task handle.
    error_forward_handle: Option<tokio::task::JoinHandle<()>>,

    /// Shared callback storage for async access.
    result_callback: Arc<Mutex<Option<AsyncSTTCallback>>>,

    /// Error callback storage.
    error_callback: Arc<Mutex<Option<AsyncErrorCallback>>>,

    /// Connection ID for debugging (sent to Azure in headers).
    connection_id: String,
}

impl Default for AzureSTT {
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
            connection_id: generate_key(),
        }
    }
}

impl AzureSTT {
    /// Build the Content-Type header value for audio format.
    ///
    /// Azure expects a specific format like:
    /// `audio/wav; codecs=audio/pcm; samplerate=16000`
    fn build_content_type(config: &AzureSTTConfig) -> String {
        format!(
            "audio/wav; codecs=audio/pcm; samplerate={}",
            config.base.sample_rate
        )
    }

    /// Handle incoming WebSocket messages from Azure.
    ///
    /// This method parses Azure messages and routes them appropriately:
    /// - SpeechHypothesis -> interim results (if enabled)
    /// - SpeechPhrase -> final results
    /// - Errors -> error channel
    fn handle_websocket_message(
        message: Message,
        result_tx: &mpsc::UnboundedSender<STTResult>,
        interim_results_enabled: bool,
    ) -> Result<(), STTError> {
        match message {
            Message::Text(text) => {
                debug!("Received Azure message: {}", text);

                match AzureMessage::parse(&text) {
                    Ok(parsed_msg) => match parsed_msg {
                        AzureMessage::SpeechStartDetected(start) => {
                            debug!("Speech started at offset: {}s", start.offset_seconds());
                        }

                        AzureMessage::SpeechHypothesis(hypothesis) => {
                            if interim_results_enabled && !hypothesis.text.is_empty() {
                                let stt_result = hypothesis.to_stt_result();
                                if result_tx.send(stt_result).is_err() {
                                    warn!("Failed to send hypothesis result - channel closed");
                                }
                            }
                        }

                        AzureMessage::SpeechPhrase(phrase) => {
                            // Check recognition status
                            if phrase.recognition_status.is_error() {
                                let error_msg = format!(
                                    "Azure recognition error: {:?}",
                                    phrase.recognition_status
                                );
                                return Err(STTError::ProviderError(error_msg));
                            }

                            // Only send results for successful recognition
                            if let Some(stt_result) = phrase.to_stt_result() {
                                if result_tx.send(stt_result).is_err() {
                                    warn!("Failed to send phrase result - channel closed");
                                }
                            } else if phrase.recognition_status == RecognitionStatus::NoMatch {
                                debug!("No speech detected in audio segment");
                            }
                        }

                        AzureMessage::SpeechEndDetected(end) => {
                            debug!("Speech ended at offset: {}s", end.offset_seconds());
                        }

                        AzureMessage::TurnStart => {
                            debug!("Azure recognition turn started");
                        }

                        AzureMessage::TurnEnd => {
                            debug!("Azure recognition turn ended");
                        }

                        AzureMessage::Unknown(raw) => {
                            debug!("Received unknown Azure message: {}", raw);
                        }
                    },
                    Err(e) => {
                        warn!("Failed to parse Azure message: {}", e);
                    }
                }
            }

            Message::Binary(data) => {
                // Azure may send binary data for certain responses
                debug!("Received binary message from Azure: {} bytes", data.len());
            }

            Message::Close(close_frame) => {
                info!("Azure WebSocket closed: {:?}", close_frame);
            }

            Message::Ping(_) => {
                debug!("Received ping from Azure");
            }

            Message::Pong(_) => {
                debug!("Received pong from Azure");
            }

            _ => {
                debug!("Received unexpected message type from Azure");
            }
        }

        Ok(())
    }

    /// Start the WebSocket connection to Azure Speech Services.
    async fn start_connection(&mut self, config: AzureSTTConfig) -> Result<(), STTError> {
        let ws_url = config.build_websocket_url();

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
        let host = config.region.stt_hostname();
        let content_type = Self::build_content_type(&config);
        let connection_id = self.connection_id.clone();
        let interim_results_enabled = config.interim_results;

        // Start the connection task
        let connection_handle = tokio::spawn(async move {
            // Build WebSocket request with Azure authentication headers
            let request = match tokio_tungstenite::tungstenite::http::Request::builder()
                .method("GET")
                .uri(&ws_url)
                .header("Host", &host)
                .header("Upgrade", "websocket")
                .header("Connection", "upgrade")
                .header("Sec-WebSocket-Key", generate_key())
                .header("Sec-WebSocket-Version", "13")
                // Azure-specific authentication header
                .header("Ocp-Apim-Subscription-Key", &api_key)
                // Connection ID for debugging
                .header("X-ConnectionId", &connection_id)
                // Audio format specification
                .header("Content-Type", &content_type)
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

            // Connect to Azure with timeout
            let connect_result =
                match timeout(Duration::from_secs(30), connect_async(request)).await {
                    Ok(result) => result,
                    Err(_) => {
                        let stt_error = STTError::ConnectionFailed(
                            "Connection to Azure timed out after 30 seconds".to_string(),
                        );
                        error!("{}", stt_error);
                        let _ = error_tx.send(stt_error);
                        return;
                    }
                };

            let (ws_stream, _response) = match connect_result {
                Ok(result) => result,
                Err(e) => {
                    let error_msg = format!("Failed to connect to Azure: {e}");
                    // Check for authentication errors
                    let stt_error = if error_msg.contains("401")
                        || error_msg.contains("Unauthorized")
                    {
                        STTError::AuthenticationFailed(
                            "Azure authentication failed. Check subscription key and region."
                                .to_string(),
                        )
                    } else if error_msg.contains("403") || error_msg.contains("Forbidden") {
                        STTError::AuthenticationFailed(
                            "Azure access forbidden. Subscription may be inactive or region mismatch.".to_string()
                        )
                    } else {
                        STTError::ConnectionFailed(error_msg)
                    };
                    error!("{}", stt_error);
                    let _ = error_tx.send(stt_error);
                    return;
                }
            };

            info!(
                "Connected to Azure Speech-to-Text WebSocket (connection_id: {})",
                connection_id
            );

            // Signal successful connection
            let _ = connected_tx.send(());

            let (mut ws_sink, mut ws_stream) = ws_stream.split();

            // Keep-alive mechanism: Azure connections may timeout during silence
            // Send silence frames every 5 seconds if no audio was sent
            let mut keepalive_timer = interval(Duration::from_secs(1));
            let mut last_audio_time = Instant::now();

            // Main event loop
            loop {
                tokio::select! {
                    // Prioritize audio sending for lowest latency
                    biased;

                    // Handle outgoing audio data
                    Some(audio_data) = ws_rx.recv() => {
                        // Azure expects raw PCM audio as binary messages
                        let message = Message::Binary(audio_data);
                        if let Err(e) = ws_sink.send(message).await {
                            let stt_error = STTError::NetworkError(format!(
                                "Failed to send audio to Azure: {e}"
                            ));
                            error!("{}", stt_error);
                            let _ = error_tx.send(stt_error);
                            break;
                        }
                        // Update last audio time for keep-alive tracking
                        last_audio_time = Instant::now();
                    }

                    // Handle incoming messages
                    message = ws_stream.next() => {
                        match message {
                            Some(Ok(msg)) => {
                                if let Err(e) = Self::handle_websocket_message(
                                    msg,
                                    &result_tx,
                                    interim_results_enabled,
                                ) {
                                    error!("Azure streaming error: {}", e);
                                    let _ = error_tx.send(e);
                                    break;
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
                                info!("Azure WebSocket stream ended");
                                break;
                            }
                        }
                    }

                    // Handle keep-alive timer
                    _ = keepalive_timer.tick() => {
                        // Check if we need to send silence to keep connection alive
                        // Azure may timeout after extended silence (typically 30-60 seconds)
                        if last_audio_time.elapsed() >= Duration::from_secs(5) {
                            // Send a small buffer of silence (32 samples at 16kHz = 2ms)
                            // This is minimal overhead but keeps the connection alive
                            let silence_frame = vec![0u8; 64]; // 32 16-bit samples
                            let message = Message::Binary(silence_frame.into());
                            if let Err(e) = ws_sink.send(message).await {
                                let stt_error = STTError::NetworkError(format!(
                                    "Failed to send keep-alive: {e}"
                                ));
                                error!("{}", stt_error);
                                let _ = error_tx.send(stt_error);
                                break;
                            }
                            debug!("Sent keep-alive silence frame to Azure");
                            last_audio_time = Instant::now();
                        }
                    }

                    // Handle shutdown signal
                    _ = &mut shutdown_rx => {
                        info!("Received shutdown signal for Azure STT");
                        // Send close message
                        let _ = ws_sink.send(Message::Close(None)).await;
                        break;
                    }
                }
            }

            info!("Azure STT WebSocket connection closed");
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
                        "Azure STT result (no callback): {} (final: {}, confidence: {})",
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
                    error!("Azure STT error (no callback registered): {}", error);
                }
            }
        });

        self.error_forward_handle = Some(error_forwarding_handle);

        // Update state and wait for connection
        self.state = ConnectionState::Connecting;

        // Wait for connection with timeout
        match timeout(Duration::from_secs(30), connected_rx).await {
            Ok(Ok(())) => {
                self.state = ConnectionState::Connected;
                self.state_notify.notify_waiters();
                info!("Successfully connected to Azure Speech-to-Text");
                Ok(())
            }
            Ok(Err(_)) => {
                let error_msg = "Connection channel closed before confirmation".to_string();
                self.state = ConnectionState::Error(error_msg.clone());
                Err(STTError::ConnectionFailed(error_msg))
            }
            Err(_) => {
                let error_msg = "Connection timeout waiting for Azure".to_string();
                self.state = ConnectionState::Error(error_msg.clone());
                Err(STTError::ConnectionFailed(error_msg))
            }
        }
    }

    /// Get the connection ID for debugging.
    ///
    /// This ID is sent to Azure in the `X-ConnectionId` header and can be
    /// used to correlate logs between client and server.
    pub fn get_connection_id(&self) -> &str {
        &self.connection_id
    }

    /// Get the Azure-specific configuration.
    ///
    /// Returns `None` if the client was not properly initialized.
    pub fn get_azure_config(&self) -> Option<&AzureSTTConfig> {
        self.config.as_ref()
    }
}

impl Drop for AzureSTT {
    fn drop(&mut self) {
        // Send shutdown signal if still connected
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

// =============================================================================
// BaseSTT Trait Implementation
// =============================================================================

#[async_trait::async_trait]
impl BaseSTT for AzureSTT {
    fn new(config: STTConfig) -> Result<Self, STTError> {
        // Validate API key (subscription key)
        if config.api_key.is_empty() {
            return Err(STTError::AuthenticationFailed(
                "Azure subscription key is required".to_string(),
            ));
        }

        // Create Azure-specific configuration with defaults
        let azure_config = AzureSTTConfig::from_base(config);

        Ok(Self {
            config: Some(azure_config),
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
            connection_id: generate_key(),
        })
    }

    async fn connect(&mut self) -> Result<(), STTError> {
        // Check if already connected
        if matches!(self.state, ConnectionState::Connected) {
            return Err(STTError::ConnectionFailed(
                "Already connected to Azure".to_string(),
            ));
        }

        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

        // Generate new connection ID for this connection attempt
        self.connection_id = generate_key();

        self.start_connection(config.clone()).await
    }

    async fn disconnect(&mut self) -> Result<(), STTError> {
        // Send shutdown signal
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        // Wait for connection task to finish with timeout
        if let Some(handle) = self.connection_handle.take() {
            let _ = timeout(Duration::from_secs(5), handle).await;
        }

        // Clean up result forwarding task
        if let Some(handle) = self.result_forward_handle.take() {
            handle.abort();
            let _ = handle.await;
        }

        // Clean up error forwarding task
        if let Some(handle) = self.error_forward_handle.take() {
            handle.abort();
            let _ = handle.await;
        }

        // Clear all channels
        self.ws_sender = None;
        self.result_tx = None;
        self.error_tx = None;

        // Clear callbacks
        *self.result_callback.lock().await = None;
        *self.error_callback.lock().await = None;

        // Update state
        self.state = ConnectionState::Disconnected;
        self.state_notify.notify_waiters();

        info!("Disconnected from Azure Speech-to-Text");
        Ok(())
    }

    fn is_ready(&self) -> bool {
        matches!(self.state, ConnectionState::Connected) && self.ws_sender.is_some()
    }

    async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
        if !self.is_ready() {
            return Err(STTError::ConnectionFailed(
                "Not connected to Azure Speech-to-Text".to_string(),
            ));
        }

        if let Some(ws_sender) = &self.ws_sender {
            let data_len = audio_data.len();
            let audio_bytes = Bytes::from(audio_data);

            // Send with backpressure handling
            ws_sender
                .send(audio_bytes)
                .await
                .map_err(|e| STTError::NetworkError(format!("Failed to send audio data: {e}")))?;

            debug!("Queued {} bytes of audio for Azure STT", data_len);
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
        // Disconnect if currently connected
        if self.is_ready() {
            self.disconnect().await?;
        }

        // Preserve Azure-specific settings from existing config
        let existing = self.config.take();

        let azure_config = AzureSTTConfig {
            base: config,
            region: existing
                .as_ref()
                .map(|c| c.region.clone())
                .unwrap_or_default(),
            output_format: existing
                .as_ref()
                .map(|c| c.output_format)
                .unwrap_or_default(),
            profanity: existing.as_ref().map(|c| c.profanity).unwrap_or_default(),
            interim_results: existing.as_ref().map(|c| c.interim_results).unwrap_or(true),
            word_level_timing: existing.as_ref().is_some_and(|c| c.word_level_timing),
            endpoint_id: existing.as_ref().and_then(|c| c.endpoint_id.clone()),
            auto_detect_languages: existing
                .as_ref()
                .and_then(|c| c.auto_detect_languages.clone()),
        };

        self.config = Some(azure_config);

        // Reconnect with new configuration
        self.connect().await
    }

    fn get_provider_info(&self) -> &'static str {
        "Microsoft Azure Speech-to-Text"
    }
}

// =============================================================================
// Azure-Specific Helper Methods
// =============================================================================

impl AzureSTT {
    /// Update Azure-specific settings.
    ///
    /// This allows updating Azure-specific parameters without affecting
    /// the base STT configuration. Requires reconnection.
    ///
    /// # Arguments
    ///
    /// * `region` - Optional new Azure region
    /// * `output_format` - Optional new output format (Simple or Detailed)
    /// * `profanity` - Optional profanity handling setting
    /// * `interim_results` - Optional interim results toggle
    pub async fn update_azure_settings(
        &mut self,
        region: Option<super::config::AzureRegion>,
        output_format: Option<super::config::AzureOutputFormat>,
        profanity: Option<super::config::AzureProfanityOption>,
        interim_results: Option<bool>,
    ) -> Result<(), STTError> {
        if self.is_ready() {
            self.disconnect().await?;
        }

        if let Some(config) = &mut self.config {
            if let Some(r) = region {
                config.region = r;
            }
            if let Some(f) = output_format {
                config.output_format = f;
            }
            if let Some(p) = profanity {
                config.profanity = p;
            }
            if let Some(ir) = interim_results {
                config.interim_results = ir;
            }
        }

        self.connect().await
    }

    /// Set a Custom Speech endpoint.
    ///
    /// Use this to specify a Custom Speech model trained on your specific
    /// domain or vocabulary.
    ///
    /// # Arguments
    ///
    /// * `endpoint_id` - The Custom Speech endpoint ID, or None to use default
    pub async fn set_custom_endpoint(
        &mut self,
        endpoint_id: Option<String>,
    ) -> Result<(), STTError> {
        if self.is_ready() {
            self.disconnect().await?;
        }

        if let Some(config) = &mut self.config {
            config.endpoint_id = endpoint_id;
        }

        self.connect().await
    }

    /// Enable automatic language detection.
    ///
    /// When enabled, Azure will automatically detect which of the specified
    /// languages is being spoken.
    ///
    /// # Arguments
    ///
    /// * `languages` - List of BCP-47 language codes to detect, or None to disable
    pub async fn set_auto_detect_languages(
        &mut self,
        languages: Option<Vec<String>>,
    ) -> Result<(), STTError> {
        if self.is_ready() {
            self.disconnect().await?;
        }

        if let Some(config) = &mut self.config {
            config.auto_detect_languages = languages;
        }

        self.connect().await
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_azure_stt_creation() {
        let config = STTConfig {
            provider: "azure".to_string(),
            api_key: "test_subscription_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "default".to_string(),
        };

        let stt = <AzureSTT as BaseSTT>::new(config).unwrap();
        assert!(!stt.is_ready());
        assert_eq!(stt.get_provider_info(), "Microsoft Azure Speech-to-Text");
    }

    #[test]
    fn test_azure_stt_empty_api_key_error() {
        let config = STTConfig {
            provider: "azure".to_string(),
            api_key: String::new(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "default".to_string(),
        };

        let result = <AzureSTT as BaseSTT>::new(config);
        assert!(result.is_err());
        if let Err(STTError::AuthenticationFailed(msg)) = result {
            assert!(msg.contains("subscription key"));
        } else {
            panic!("Expected AuthenticationFailed error");
        }
    }

    #[test]
    fn test_azure_stt_config_access() {
        let config = STTConfig {
            provider: "azure".to_string(),
            api_key: "test_key".to_string(),
            language: "de-DE".to_string(),
            sample_rate: 8000,
            channels: 1,
            punctuation: false,
            encoding: "linear16".to_string(),
            model: "default".to_string(),
        };

        let stt = <AzureSTT as BaseSTT>::new(config).unwrap();

        let stored_config = stt.get_config().unwrap();
        assert_eq!(stored_config.api_key, "test_key");
        assert_eq!(stored_config.language, "de-DE");
        assert_eq!(stored_config.sample_rate, 8000);
    }

    #[test]
    fn test_azure_stt_azure_config_access() {
        let config = STTConfig {
            provider: "azure".to_string(),
            api_key: "test_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let stt = <AzureSTT as BaseSTT>::new(config).unwrap();

        let azure_config = stt.get_azure_config().unwrap();
        // Default region should be EastUS
        assert_eq!(
            azure_config.region,
            super::super::config::AzureRegion::EastUS
        );
        // Default interim_results should be true
        assert!(azure_config.interim_results);
    }

    #[test]
    fn test_azure_stt_connection_id() {
        let config = STTConfig {
            provider: "azure".to_string(),
            api_key: "test_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let stt = <AzureSTT as BaseSTT>::new(config).unwrap();

        // Connection ID should be a non-empty string (base64-encoded random bytes)
        let conn_id = stt.get_connection_id();
        assert!(!conn_id.is_empty());
        // The key is base64-encoded, so it should contain only valid base64 characters
        assert!(
            conn_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        );
    }

    #[test]
    fn test_content_type_building() {
        let config = AzureSTTConfig {
            base: STTConfig {
                sample_rate: 16000,
                ..Default::default()
            },
            ..Default::default()
        };

        let content_type = AzureSTT::build_content_type(&config);
        assert_eq!(
            content_type,
            "audio/wav; codecs=audio/pcm; samplerate=16000"
        );

        let config_8k = AzureSTTConfig {
            base: STTConfig {
                sample_rate: 8000,
                ..Default::default()
            },
            ..Default::default()
        };

        let content_type_8k = AzureSTT::build_content_type(&config_8k);
        assert_eq!(
            content_type_8k,
            "audio/wav; codecs=audio/pcm; samplerate=8000"
        );
    }

    #[test]
    fn test_default_state() {
        let stt = AzureSTT::default();
        assert!(!stt.is_ready());
        assert!(stt.config.is_none());
    }

    #[tokio::test]
    async fn test_send_audio_not_connected_error() {
        let config = STTConfig {
            provider: "azure".to_string(),
            api_key: "test_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let mut stt = <AzureSTT as BaseSTT>::new(config).unwrap();

        let result = stt.send_audio(vec![0u8; 1024]).await;
        assert!(result.is_err());
        if let Err(STTError::ConnectionFailed(msg)) = result {
            assert!(msg.contains("Not connected"));
        } else {
            panic!("Expected ConnectionFailed error");
        }
    }

    #[tokio::test]
    async fn test_callback_registration() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let config = STTConfig {
            provider: "azure".to_string(),
            api_key: "test_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let mut stt = <AzureSTT as BaseSTT>::new(config).unwrap();

        let callback_registered = Arc::new(AtomicBool::new(false));
        let callback_flag = callback_registered.clone();

        let callback: STTResultCallback = Arc::new(move |_result| {
            callback_flag.store(true, Ordering::SeqCst);
            Box::pin(async {})
        });

        stt.on_result(callback).await.unwrap();

        // Callback should be stored (we can't easily test invocation without a real connection)
        assert!(stt.result_callback.lock().await.is_some());
    }

    #[tokio::test]
    async fn test_error_callback_registration() {
        let config = STTConfig {
            provider: "azure".to_string(),
            api_key: "test_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let mut stt = <AzureSTT as BaseSTT>::new(config).unwrap();

        let callback: STTErrorCallback = Arc::new(move |_error| Box::pin(async {}));

        stt.on_error(callback).await.unwrap();

        // Error callback should be stored
        assert!(stt.error_callback.lock().await.is_some());
    }

    #[tokio::test]
    async fn test_connect_already_connected_error() {
        // This test verifies the error message for double-connect
        // We can't actually connect in unit tests without Azure credentials
        let config = STTConfig {
            provider: "azure".to_string(),
            api_key: "test_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            ..Default::default()
        };

        let mut stt = <AzureSTT as BaseSTT>::new(config).unwrap();

        // Manually set state to Connected to simulate
        stt.state = ConnectionState::Connected;
        stt.ws_sender = Some(mpsc::channel(1).0); // Dummy sender

        let result = stt.connect().await;
        assert!(result.is_err());
        if let Err(STTError::ConnectionFailed(msg)) = result {
            assert!(msg.contains("Already connected"));
        } else {
            panic!("Expected ConnectionFailed error");
        }
    }
}
