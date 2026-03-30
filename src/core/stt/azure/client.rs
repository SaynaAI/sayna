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
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use tokio::time::{Instant, interval, timeout};
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use super::config::AzureSTTConfig;
use super::messages::{AzureMessage, RecognitionStatus};
use crate::core::providers::azure::AzureRegion;
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

    // =========================================================================
    // USP (Unified Speech Protocol) helpers
    // =========================================================================

    /// Generate an ISO-8601 UTC timestamp for USP message headers.
    fn generate_iso8601_timestamp() -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = now.as_secs();
        let millis = now.subsec_millis();

        // Calculate UTC date/time components
        let days = total_secs / 86400;
        let time_secs = total_secs % 86400;
        let hours = time_secs / 3600;
        let minutes = (time_secs % 3600) / 60;
        let seconds = time_secs % 60;

        // Days since epoch to Y/M/D (simplified Gregorian)
        let mut y = 1970i32;
        let mut remaining_days = days as i32;
        loop {
            let year_days = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                366
            } else {
                365
            };
            if remaining_days < year_days {
                break;
            }
            remaining_days -= year_days;
            y += 1;
        }
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let month_days = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        let mut m = 0usize;
        for &md in &month_days {
            if remaining_days < md {
                break;
            }
            remaining_days -= md;
            m += 1;
        }

        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            y,
            m + 1,
            remaining_days + 1,
            hours,
            minutes,
            seconds,
            millis,
        )
    }

    /// Build a USP text-framed message with path, request ID, and JSON body.
    fn build_usp_text_message(
        path: &str,
        request_id: &str,
        content_type: &str,
        body: &str,
    ) -> String {
        format!(
            "path:{}\r\nx-requestid:{}\r\nx-timestamp:{}\r\ncontent-type:{}\r\n\r\n{}",
            path,
            request_id,
            Self::generate_iso8601_timestamp(),
            content_type,
            body,
        )
    }

    /// Build a USP binary-framed message for audio data.
    ///
    /// Format: [2-byte big-endian header length][header bytes][audio payload]
    fn build_usp_binary_message(request_id: &str, audio_payload: &[u8]) -> Vec<u8> {
        let header = format!(
            "path:audio\r\nx-requestid:{}\r\nx-timestamp:{}\r\ncontent-type:audio/x-wav\r\n",
            request_id,
            Self::generate_iso8601_timestamp(),
        );
        let header_bytes = header.as_bytes();
        let header_len = header_bytes.len() as u16;

        let mut message = Vec::with_capacity(2 + header_bytes.len() + audio_payload.len());
        message.extend_from_slice(&header_len.to_be_bytes());
        message.extend_from_slice(header_bytes);
        message.extend_from_slice(audio_payload);
        message
    }

    /// Build a standard 44-byte WAV RIFF header for streaming PCM audio.
    fn build_wav_riff_header(sample_rate: u32, bits_per_sample: u16, channels: u16) -> [u8; 44] {
        let byte_rate = sample_rate * (bits_per_sample as u32 / 8) * channels as u32;
        let block_align = channels * (bits_per_sample / 8);

        let mut header = [0u8; 44];
        header[0..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&0u32.to_le_bytes()); // unknown size for streaming
        header[8..12].copy_from_slice(b"WAVE");
        header[12..16].copy_from_slice(b"fmt ");
        header[16..20].copy_from_slice(&16u32.to_le_bytes()); // PCM sub-chunk size
        header[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM format tag
        header[22..24].copy_from_slice(&channels.to_le_bytes());
        header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
        header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
        header[32..34].copy_from_slice(&block_align.to_le_bytes());
        header[34..36].copy_from_slice(&bits_per_sample.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&0u32.to_le_bytes()); // unknown data size
        header
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
        let sample_rate = config.base.sample_rate;
        let channels = config.base.channels;

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

            let (mut ws_sink, mut ws_stream) = ws_stream.split();

            // === USP Protocol: Generate session-scoped request ID ===
            let request_id = Uuid::new_v4().simple().to_string();

            // === USP Protocol: Send speech.config as the first message ===
            let speech_config_body = serde_json::json!({
                "context": {
                    "system": {
                        "name": "SpeechSDK",
                        "version": "1.0.0",
                        "build": "Rust",
                        "lang": "Rust"
                    },
                    "os": {
                        "platform": std::env::consts::OS,
                        "name": "Sayna",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "audio": {
                        "source": {
                            "samplerate": sample_rate,
                            "bitspersample": 16,
                            "channelcount": channels
                        }
                    }
                },
                "recognition": "conversation"
            });

            let config_message = Self::build_usp_text_message(
                "speech.config",
                &request_id,
                "application/json",
                &speech_config_body.to_string(),
            );

            if let Err(e) = ws_sink.send(Message::Text(config_message.into())).await {
                let stt_error =
                    STTError::ConnectionFailed(format!("Failed to send speech.config: {e}"));
                error!("{}", stt_error);
                let _ = error_tx.send(stt_error);
                return;
            }
            debug!("Sent speech.config USP message to Azure");

            // === USP Protocol: Send initial audio message with WAV RIFF header ===
            let wav_header = Self::build_wav_riff_header(sample_rate, 16, channels);
            let initial_audio = Self::build_usp_binary_message(&request_id, &wav_header);
            if let Err(e) = ws_sink.send(Message::Binary(initial_audio.into())).await {
                let stt_error =
                    STTError::ConnectionFailed(format!("Failed to send initial WAV header: {e}"));
                error!("{}", stt_error);
                let _ = error_tx.send(stt_error);
                return;
            }
            debug!("Sent initial WAV RIFF header to Azure");

            // Signal successful connection (after USP handshake)
            let _ = connected_tx.send(());

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
                        // Wrap audio in USP binary frame
                        let usp_frame = Self::build_usp_binary_message(&request_id, &audio_data);
                        let message = Message::Binary(usp_frame.into());
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
                            // wrapped in USP binary frame
                            let silence_frame = vec![0u8; 64]; // 32 16-bit samples
                            let usp_frame = Self::build_usp_binary_message(&request_id, &silence_frame);
                            let message = Message::Binary(usp_frame.into());
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
        let region = config
            .azure_region
            .as_deref()
            .map(|r| r.parse::<AzureRegion>().unwrap_or_default())
            .unwrap_or_default();
        let azure_config = AzureSTTConfig::with_region(config, region);

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
            azure_region: None,
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
            azure_region: None,
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
            azure_region: None,
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

    // =========================================================================
    // USP helper tests
    // =========================================================================

    #[test]
    fn test_generate_iso8601_timestamp_format() {
        let ts = AzureSTT::generate_iso8601_timestamp();
        // Should match: YYYY-MM-DDTHH:MM:SS.mmmZ
        assert!(ts.ends_with('Z'), "Timestamp must end with Z: {ts}");
        assert_eq!(ts.len(), 24, "Timestamp length should be 24: {ts}");
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[16..17], ":");
        assert_eq!(&ts[19..20], ".");
    }

    #[test]
    fn test_build_usp_text_message_format() {
        let msg = AzureSTT::build_usp_text_message(
            "speech.config",
            "abc123",
            "application/json",
            r#"{"test": true}"#,
        );

        assert!(msg.starts_with("path:speech.config\r\n"));
        assert!(msg.contains("x-requestid:abc123\r\n"));
        assert!(msg.contains("x-timestamp:"));
        assert!(msg.contains("content-type:application/json\r\n"));
        // Headers and body separated by \r\n\r\n
        assert!(msg.contains("\r\n\r\n{\"test\": true}"));
    }

    #[test]
    fn test_build_usp_binary_message_format() {
        let payload = b"hello audio";
        let msg = AzureSTT::build_usp_binary_message("req123", payload);

        // First 2 bytes are big-endian header length
        let header_len = u16::from_be_bytes([msg[0], msg[1]]) as usize;
        assert!(header_len > 0);
        assert!(msg.len() > 2 + header_len);

        // Extract header string
        let header_str = std::str::from_utf8(&msg[2..2 + header_len]).unwrap();
        assert!(header_str.starts_with("path:audio\r\n"));
        assert!(header_str.contains("x-requestid:req123\r\n"));
        assert!(header_str.contains("x-timestamp:"));
        assert!(header_str.contains("content-type:audio/x-wav\r\n"));

        // Payload follows header
        let extracted_payload = &msg[2 + header_len..];
        assert_eq!(extracted_payload, payload);
    }

    #[test]
    fn test_build_wav_riff_header() {
        let header = AzureSTT::build_wav_riff_header(16000, 16, 1);
        assert_eq!(header.len(), 44);

        // RIFF magic
        assert_eq!(&header[0..4], b"RIFF");
        // WAVE magic
        assert_eq!(&header[8..12], b"WAVE");
        // fmt sub-chunk
        assert_eq!(&header[12..16], b"fmt ");
        // PCM format tag = 1
        assert_eq!(u16::from_le_bytes([header[20], header[21]]), 1);
        // Channels = 1
        assert_eq!(u16::from_le_bytes([header[22], header[23]]), 1);
        // Sample rate = 16000
        assert_eq!(
            u32::from_le_bytes([header[24], header[25], header[26], header[27]]),
            16000
        );
        // Byte rate = 16000 * 1 * 2 = 32000
        assert_eq!(
            u32::from_le_bytes([header[28], header[29], header[30], header[31]]),
            32000
        );
        // Block align = 1 * 2 = 2
        assert_eq!(u16::from_le_bytes([header[32], header[33]]), 2);
        // Bits per sample = 16
        assert_eq!(u16::from_le_bytes([header[34], header[35]]), 16);
        // data sub-chunk
        assert_eq!(&header[36..40], b"data");
    }

    #[test]
    fn test_build_wav_riff_header_stereo() {
        let header = AzureSTT::build_wav_riff_header(48000, 16, 2);
        // Channels = 2
        assert_eq!(u16::from_le_bytes([header[22], header[23]]), 2);
        // Sample rate = 48000
        assert_eq!(
            u32::from_le_bytes([header[24], header[25], header[26], header[27]]),
            48000
        );
        // Byte rate = 48000 * 2 * 2 = 192000
        assert_eq!(
            u32::from_le_bytes([header[28], header[29], header[30], header[31]]),
            192000
        );
        // Block align = 2 * 2 = 4
        assert_eq!(u16::from_le_bytes([header[32], header[33]]), 4);
    }
}
