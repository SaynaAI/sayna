//! Cartesia STT WebSocket client implementation.
//!
//! This module contains the main `CartesiaSTT` struct that implements the
//! `BaseSTT` trait for real-time speech-to-text streaming.
//!
//! # Architecture
//!
//! The client uses a low-latency design pattern similar to Deepgram:
//! - Bounded channel (size 32) for audio data with backpressure
//! - Unbounded channels for results and errors (never block on output)
//! - Single event loop for handling both incoming and outgoing WebSocket messages
//! - Separate tasks for result and error forwarding to callbacks
//!
//! # Audio Format
//!
//! Audio is sent as raw binary WebSocket frames (NOT base64 encoded):
//! - PCM signed 16-bit little-endian (pcm_s16le)
//! - Sample rates: 8000, 16000, 22050, 24000, 44100, or 48000 Hz

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::sync::{Mutex, Notify, mpsc, oneshot};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, error, info, warn};

use super::config::CartesiaSTTConfig;
use super::messages::CartesiaMessage;
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
pub(crate) enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    /// Error state - variant is constructed but not pattern-matched
    Error,
}

// =============================================================================
// CartesiaSTT Client
// =============================================================================

/// Cartesia Speech-to-Text WebSocket client.
///
/// Implements the `BaseSTT` trait for real-time streaming transcription
/// using the Cartesia WebSocket API.
///
/// # Low-Latency Design
///
/// This client is designed for minimal latency:
/// - Audio data is sent as raw binary (no base64 encoding overhead)
/// - Bounded channel (32 items) for audio provides backpressure without blocking
/// - Unbounded channels for results ensure output never blocks the main loop
/// - Single event loop handles both send and receive operations
///
/// # Example
///
/// ```rust,no_run
/// use sayna::core::stt::{BaseSTT, STTConfig, CartesiaSTT};
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = STTConfig {
///         api_key: "your-cartesia-api-key".to_string(),
///         language: "en".to_string(),
///         sample_rate: 16000,
///         ..Default::default()
///     };
///
///     let mut stt = CartesiaSTT::new(config)?;
///     stt.connect().await?;
///
///     // Register callback for results
///     stt.on_result(Arc::new(|result| {
///         Box::pin(async move {
///             println!("Transcription: {}", result.transcript);
///         })
///     })).await?;
///
///     // Send audio data (raw PCM S16LE bytes)
///     let audio_data = vec![0u8; 3200]; // 100ms at 16kHz
///     stt.send_audio(audio_data).await?;
///
///     Ok(())
/// }
/// ```
pub struct CartesiaSTT {
    /// Cartesia-specific configuration.
    config: Option<CartesiaSTTConfig>,

    /// Current connection state.
    state: ConnectionState,

    /// State change notification.
    state_notify: Arc<Notify>,

    /// WebSocket sender for audio data (bounded channel for backpressure).
    ws_sender: Option<mpsc::Sender<Bytes>>,

    /// Shutdown signal sender.
    shutdown_tx: Option<oneshot::Sender<()>>,

    /// Result channel sender.
    result_tx: Option<mpsc::UnboundedSender<STTResult>>,

    /// Error channel sender for streaming errors.
    error_tx: Option<mpsc::UnboundedSender<STTError>>,

    /// Connection task handle.
    connection_handle: Option<tokio::task::JoinHandle<()>>,

    /// Result forwarding task handle.
    result_forward_handle: Option<tokio::task::JoinHandle<()>>,

    /// Error forwarding task handle.
    error_forward_handle: Option<tokio::task::JoinHandle<()>>,

    /// Shared callback storage for async access.
    result_callback: Arc<Mutex<Option<AsyncSTTCallback>>>,

    /// Error callback storage for streaming errors.
    error_callback: Arc<Mutex<Option<AsyncErrorCallback>>>,
}

impl CartesiaSTT {
    /// Handle incoming WebSocket messages and convert to STTResult.
    ///
    /// This is optimized for the hot path:
    /// - Parse JSON once
    /// - Branch on message type
    /// - Non-blocking send via channel
    fn handle_websocket_message(
        message: Message,
        result_tx: &mpsc::UnboundedSender<STTResult>,
        error_tx: &mpsc::UnboundedSender<STTError>,
    ) -> Result<bool, STTError> {
        match message {
            Message::Text(text) => {
                debug!("Received text message: {}", text);

                match CartesiaMessage::parse(&text) {
                    Ok(msg) => match msg {
                        CartesiaMessage::Transcript(transcript) => {
                            let stt_result = transcript.to_stt_result();

                            // Send result (non-blocking)
                            if result_tx.send(stt_result).is_err() {
                                warn!("Failed to send result - channel closed");
                            }
                        }
                        CartesiaMessage::FlushDone => {
                            debug!("Received flush_done acknowledgment");
                        }
                        CartesiaMessage::Done => {
                            debug!("Received done acknowledgment - session ending");
                            return Ok(true); // Signal to close connection
                        }
                        CartesiaMessage::Error(e) => {
                            let stt_error = STTError::ProviderError(e.message);
                            if error_tx.send(stt_error.clone()).is_err() {
                                warn!("Failed to send error - channel closed");
                            }
                            return Err(stt_error);
                        }
                        CartesiaMessage::Unknown(raw) => {
                            debug!("Received unknown message type: {}", raw);
                        }
                    },
                    Err(e) => {
                        warn!("Failed to parse Cartesia message: {} - raw: {}", e, text);
                    }
                }
            }
            Message::Close(close_frame) => {
                info!("WebSocket connection closed: {:?}", close_frame);
                return Ok(true); // Signal to exit loop
            }
            Message::Binary(_) => {
                // Cartesia doesn't send binary messages, ignore
                debug!("Received unexpected binary message from Cartesia");
            }
            Message::Ping(_) | Message::Pong(_) => {
                // Handled automatically by tokio-tungstenite
            }
            Message::Frame(_) => {
                // Raw frames, ignore
            }
        }

        Ok(false) // Continue processing
    }

    /// Start the WebSocket connection task.
    ///
    /// This spawns the main event loop that handles:
    /// - Sending audio data from the channel
    /// - Receiving and processing messages from Cartesia
    /// - Shutdown signaling
    async fn start_connection(&mut self, config: CartesiaSTTConfig) -> Result<(), STTError> {
        // Validate configuration before connecting
        config.validate()?;

        let ws_url = config.build_websocket_url(&config.base.api_key);

        // Create channels for communication (bounded for backpressure on audio)
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

        // Clone error_tx for the connection task
        let error_tx_for_task = error_tx.clone();

        // Start the connection task
        let connection_handle = tokio::spawn(async move {
            // Connect to Cartesia WebSocket
            let (ws_stream, _) = match connect_async(&ws_url).await {
                Ok(result) => result,
                Err(e) => {
                    let stt_error =
                        STTError::ConnectionFailed(format!("Failed to connect to Cartesia: {e}"));
                    error!("{}", stt_error);
                    let _ = error_tx_for_task.send(stt_error);
                    return;
                }
            };

            info!("Connected to Cartesia WebSocket");

            // Signal successful connection
            let _ = connected_tx.send(());

            let (mut ws_sink, mut ws_stream) = ws_stream.split();

            // Main event loop
            loop {
                tokio::select! {
                    // Handle outgoing audio data
                    Some(audio_data) = ws_rx.recv() => {
                        // Send audio as raw binary (NOT base64 encoded)
                        let message = Message::Binary(audio_data);
                        if let Err(e) = ws_sink.send(message).await {
                            let stt_error = STTError::NetworkError(format!(
                                "Failed to send WebSocket message: {e}"
                            ));
                            error!("{}", stt_error);
                            let _ = error_tx.send(stt_error);
                            break;
                        }
                    }

                    // Handle incoming messages
                    message = ws_stream.next() => {
                        match message {
                            Some(Ok(msg)) => {
                                match Self::handle_websocket_message(msg, &result_tx, &error_tx) {
                                    Ok(should_close) => {
                                        if should_close {
                                            debug!("Closing WebSocket connection as requested");
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        error!("Streaming error from Cartesia: {}", e);
                                        break;
                                    }
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
                                info!("WebSocket stream ended");
                                break;
                            }
                        }
                    }

                    // Handle shutdown signal
                    _ = &mut shutdown_rx => {
                        info!("Received shutdown signal");
                        // Send "done" command to gracefully close the session
                        let done_message = Message::Text("\"done\"".into());
                        if let Err(e) = ws_sink.send(done_message).await {
                            warn!("Failed to send done command: {}", e);
                        }
                        break;
                    }
                }
            }

            info!("Cartesia WebSocket connection closed");
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
                        "Received STT result: {} (final: {})",
                        result.transcript, result.is_final
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
                    error!(
                        "STT streaming error but no error callback registered: {}",
                        error
                    );
                }
            }
        });

        self.error_forward_handle = Some(error_forwarding_handle);

        // Update state and wait for connection
        self.state = ConnectionState::Connecting;

        // Wait for connection to be established with timeout (10 seconds like Deepgram)
        match timeout(Duration::from_secs(10), connected_rx).await {
            Ok(Ok(())) => {
                self.state = ConnectionState::Connected;
                self.state_notify.notify_waiters();
                info!("Successfully connected to Cartesia STT");
                Ok(())
            }
            Ok(Err(_)) => {
                let error_msg = "Connection channel closed".to_string();
                self.state = ConnectionState::Error;
                Err(STTError::ConnectionFailed(error_msg))
            }
            Err(_) => {
                let error_msg = "Connection timeout".to_string();
                self.state = ConnectionState::Error;
                Err(STTError::ConnectionFailed(error_msg))
            }
        }
    }

    /// Send a finalize command to flush buffered audio.
    ///
    /// This triggers Cartesia to finalize any pending transcription
    /// without closing the connection.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the command was sent, or an error if not connected.
    ///
    /// # Note
    ///
    /// This method is not yet fully implemented - consider disconnecting
    /// and reconnecting for now. The public API is preserved for future use.
    pub async fn finalize(&mut self) -> Result<(), STTError> {
        if !self.is_ready() {
            return Err(STTError::ConnectionFailed(
                "Not connected to Cartesia STT".to_string(),
            ));
        }

        // The finalize command is sent as a text message
        // We'll send it through a separate channel or directly
        // For now, we can use the ws_sender with a special marker
        // But since ws_sender only accepts Bytes (audio), we need
        // to handle this differently.
        //
        // Note: In a full implementation, we might want a separate
        // command channel. For now, this is left as a TODO for
        // advanced use cases.
        warn!("finalize() is not yet fully implemented - consider disconnecting and reconnecting");
        Ok(())
    }
}

impl Default for CartesiaSTT {
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
        }
    }
}

// =============================================================================
// BaseSTT Trait Implementation
// =============================================================================

#[async_trait::async_trait]
impl BaseSTT for CartesiaSTT {
    fn new(config: STTConfig) -> Result<Self, STTError>
    where
        Self: Sized,
    {
        // Validate API key
        if config.api_key.is_empty() {
            return Err(STTError::AuthenticationFailed(
                "API key is required for Cartesia STT".to_string(),
            ));
        }

        // Create Cartesia-specific configuration from base config
        let cartesia_config = CartesiaSTTConfig::from_base(config);

        Ok(Self {
            config: Some(cartesia_config),
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
        })
    }

    async fn connect(&mut self) -> Result<(), STTError> {
        // Get the stored configuration
        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

        // Start the connection
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

        // Clean up channels and callbacks
        self.ws_sender = None;
        self.result_tx = None;
        self.error_tx = None;
        *self.result_callback.lock().await = None;
        *self.error_callback.lock().await = None;

        // Update state
        self.state = ConnectionState::Disconnected;
        self.state_notify.notify_waiters();

        info!("Disconnected from Cartesia STT");
        Ok(())
    }

    fn is_ready(&self) -> bool {
        matches!(self.state, ConnectionState::Connected) && self.ws_sender.is_some()
    }

    async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
        if !self.is_ready() {
            return Err(STTError::ConnectionFailed(
                "Not connected to Cartesia STT".to_string(),
            ));
        }

        if let Some(ws_sender) = &self.ws_sender {
            let data_len = audio_data.len();
            let audio_bytes = Bytes::from(audio_data);

            // Send audio data with backpressure handling
            ws_sender
                .send(audio_bytes)
                .await
                .map_err(|e| STTError::NetworkError(format!("Failed to send audio data: {e}")))?;

            debug!("Sent {} bytes of audio data to Cartesia", data_len);
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
        // For Cartesia, we need to reconnect to update configuration
        if self.is_ready() {
            self.disconnect().await?;
        }

        // Update stored configuration
        self.config = Some(CartesiaSTTConfig::from_base(config));

        // Reconnect with new config
        self.connect().await?;
        Ok(())
    }

    fn get_provider_info(&self) -> &'static str {
        "Cartesia STT (ink-whisper)"
    }
}

// =============================================================================
// Drop Implementation
// =============================================================================

impl Drop for CartesiaSTT {
    fn drop(&mut self) {
        // Send shutdown signal if still connected
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }

        // Abort any running tasks
        if let Some(handle) = self.connection_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.result_forward_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.error_forward_handle.take() {
            handle.abort();
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_cartesia_stt_creation() {
        let config = STTConfig {
            provider: "cartesia".to_string(),
            api_key: "test_key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "ink-whisper".to_string(),
        };

        let stt = <CartesiaSTT as BaseSTT>::new(config).unwrap();
        assert!(!stt.is_ready());
        assert_eq!(stt.get_provider_info(), "Cartesia STT (ink-whisper)");
    }

    #[tokio::test]
    async fn test_cartesia_stt_empty_api_key() {
        let config = STTConfig {
            provider: "cartesia".to_string(),
            api_key: String::new(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "ink-whisper".to_string(),
        };

        let result = <CartesiaSTT as BaseSTT>::new(config);
        assert!(result.is_err());
        if let Err(STTError::AuthenticationFailed(msg)) = result {
            assert!(msg.contains("API key is required"));
        } else {
            panic!("Expected AuthenticationFailed error");
        }
    }

    #[tokio::test]
    async fn test_cartesia_stt_send_audio_not_connected() {
        let config = STTConfig {
            provider: "cartesia".to_string(),
            api_key: "test_key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "ink-whisper".to_string(),
        };

        let mut stt = <CartesiaSTT as BaseSTT>::new(config).unwrap();
        let audio_data = vec![0u8; 1024];
        let result = stt.send_audio(audio_data).await;

        assert!(result.is_err());
        if let Err(STTError::ConnectionFailed(msg)) = result {
            assert!(msg.contains("Not connected"));
        } else {
            panic!("Expected ConnectionFailed error");
        }
    }

    #[tokio::test]
    async fn test_cartesia_stt_default() {
        let stt = CartesiaSTT::default();
        assert!(!stt.is_ready());
        assert!(stt.config.is_none());
        assert!(stt.ws_sender.is_none());
    }

    #[tokio::test]
    async fn test_cartesia_stt_disconnect_when_not_connected() {
        let config = STTConfig {
            provider: "cartesia".to_string(),
            api_key: "test_key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "ink-whisper".to_string(),
        };

        let mut stt = <CartesiaSTT as BaseSTT>::new(config).unwrap();
        // Disconnect should succeed even when not connected
        let result = stt.disconnect().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_cartesia_stt_callback_registration() {
        let config = STTConfig {
            provider: "cartesia".to_string(),
            api_key: "test_key".to_string(),
            language: "en".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            model: "ink-whisper".to_string(),
        };

        let mut stt = <CartesiaSTT as BaseSTT>::new(config).unwrap();

        // Register result callback
        let result_callback = Arc::new(|result: STTResult| {
            Box::pin(async move {
                println!("Result: {}", result.transcript);
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        });

        let result = stt.on_result(result_callback).await;
        assert!(result.is_ok());

        // Register error callback
        let error_callback = Arc::new(|error: STTError| {
            Box::pin(async move {
                println!("Error: {}", error);
            }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        });

        let result = stt.on_error(error_callback).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_handle_websocket_message_transcript() {
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();
        let (error_tx, _error_rx) = mpsc::unbounded_channel::<STTError>();

        let json = r#"{"type": "transcript", "text": "Hello world", "is_final": true}"#;
        let message = Message::Text(json.to_string().into());

        let should_close =
            CartesiaSTT::handle_websocket_message(message, &result_tx, &error_tx).unwrap();
        assert!(!should_close);

        // Check that result was sent
        let received = tokio::time::timeout(Duration::from_millis(100), result_rx.recv())
            .await
            .expect("Should receive result within timeout")
            .expect("Should receive a result");

        assert_eq!(received.transcript, "Hello world");
        assert!(received.is_final);
    }

    #[tokio::test]
    async fn test_handle_websocket_message_done() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let (error_tx, _error_rx) = mpsc::unbounded_channel::<STTError>();

        let json = r#"{"type": "done"}"#;
        let message = Message::Text(json.to_string().into());

        let should_close =
            CartesiaSTT::handle_websocket_message(message, &result_tx, &error_tx).unwrap();
        assert!(should_close);
    }

    #[tokio::test]
    async fn test_handle_websocket_message_error() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let (error_tx, mut error_rx) = mpsc::unbounded_channel::<STTError>();

        let json = r#"{"type": "error", "message": "Invalid audio format"}"#;
        let message = Message::Text(json.to_string().into());

        let result = CartesiaSTT::handle_websocket_message(message, &result_tx, &error_tx);
        assert!(result.is_err());

        // Check that error was sent via channel
        let received = tokio::time::timeout(Duration::from_millis(100), error_rx.recv())
            .await
            .expect("Should receive error within timeout")
            .expect("Should receive an error");

        if let STTError::ProviderError(msg) = received {
            assert_eq!(msg, "Invalid audio format");
        } else {
            panic!("Expected ProviderError");
        }
    }

    #[tokio::test]
    async fn test_handle_websocket_message_flush_done() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let (error_tx, _error_rx) = mpsc::unbounded_channel::<STTError>();

        let json = r#"{"type": "flush_done"}"#;
        let message = Message::Text(json.to_string().into());

        let should_close =
            CartesiaSTT::handle_websocket_message(message, &result_tx, &error_tx).unwrap();
        assert!(!should_close);
    }

    #[tokio::test]
    async fn test_handle_websocket_message_close() {
        let (result_tx, _result_rx) = mpsc::unbounded_channel::<STTResult>();
        let (error_tx, _error_rx) = mpsc::unbounded_channel::<STTError>();

        let message = Message::Close(None);

        let should_close =
            CartesiaSTT::handle_websocket_message(message, &result_tx, &error_tx).unwrap();
        assert!(should_close);
    }
}
