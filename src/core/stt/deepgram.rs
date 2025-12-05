use bytes::Bytes;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::time::{Instant, interval, timeout};
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, error, info, warn};

use super::base::{BaseSTT, STTConfig, STTError, STTErrorCallback, STTResult, STTResultCallback};

/// Type alias for the complex callback function type
type AsyncSTTCallback = Box<
    dyn Fn(STTResult) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Type alias for the error callback function type
type AsyncErrorCallback = Box<
    dyn Fn(STTError) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Simplified connection state with atomic updates
#[derive(Debug, Clone)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    /// Error state - variant is constructed but not pattern-matched
    Error,
}

/// Configuration specific to Deepgram STT
#[derive(Debug, Clone)]
pub struct DeepgramSTTConfig {
    /// Base STT configuration
    pub base: STTConfig,
    /// Enable speaker diarization
    pub diarize: bool,
    /// Enable interim results
    pub interim_results: bool,
    /// Enable filler words detection
    pub filler_words: bool,
    /// Enable profanity filtering
    pub profanity_filter: bool,
    /// Enable smart formatting
    pub smart_format: bool,
    /// Keywords to boost recognition
    pub keywords: Vec<String>,
    /// Redaction settings
    pub redact: Vec<String>,
    /// Voice activity detection events
    pub vad_events: bool,
    /// Endpointing timeout in seconds
    pub endpointing: Option<u32>,
    /// Custom tags for request identification
    pub tag: Option<String>,
    /// Utterance end timeout in milliseconds
    pub utterance_end_ms: Option<u32>,
}

impl Default for DeepgramSTTConfig {
    fn default() -> Self {
        Self {
            base: STTConfig::default(),
            diarize: false,
            interim_results: true,
            filler_words: false,
            profanity_filter: false,
            smart_format: true,
            keywords: Vec::new(),
            redact: Vec::new(),
            vad_events: true,
            endpointing: Some(200),
            tag: None,
            utterance_end_ms: Some(500),
        }
    }
}

/// Deepgram transcription response structure
#[derive(Debug, Deserialize, Serialize)]
pub struct DeepgramResponse {
    #[serde(rename = "type")]
    pub response_type: String,
    pub channel: Option<DeepgramChannel>,
    pub metadata: Option<DeepgramMetadata>,
    pub is_final: Option<bool>,
    pub speech_final: Option<bool>,
    pub duration: Option<f64>,
    pub start: Option<f64>,
    pub channel_index: Option<Vec<u32>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeepgramChannel {
    pub alternatives: Vec<DeepgramAlternative>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeepgramAlternative {
    pub transcript: String,
    pub confidence: f32,
    pub words: Option<Vec<DeepgramWord>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeepgramWord {
    pub word: String,
    pub start: f64,
    pub end: f64,
    pub confidence: f32,
    pub punctuated_word: Option<String>,
    pub speaker: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeepgramMetadata {
    pub transaction_key: Option<String>,
    pub request_id: Option<String>,
    pub sha256: Option<String>,
    pub created: Option<String>,
    pub duration: Option<f64>,
    pub channels: Option<u32>,
    pub models: Option<Vec<String>>,
    pub model_info: Option<DeepgramModelInfo>,
    pub model_uuid: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DeepgramModelInfo {
    pub name: String,
    pub version: String,
    pub arch: String,
}

/// Deepgram error response structure
#[derive(Debug, Deserialize, Serialize)]
pub struct DeepgramError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub description: String,
    pub message: String,
    pub variant: Option<String>,
}

/// Deepgram STT WebSocket client
pub struct DeepgramSTT {
    /// Configuration for the STT client
    config: Option<DeepgramSTTConfig>,
    /// Current connection state
    state: ConnectionState,
    /// State change notification
    state_notify: Arc<Notify>,
    /// WebSocket sender for audio data (bounded channel for backpressure)
    ws_sender: Option<mpsc::Sender<Bytes>>,
    /// Shutdown signal sender
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Result channel sender
    result_tx: Option<mpsc::UnboundedSender<STTResult>>,
    /// Error channel sender for streaming errors
    error_tx: Option<mpsc::UnboundedSender<STTError>>,
    /// Connection handle
    connection_handle: Option<tokio::task::JoinHandle<()>>,
    /// Result forwarding task handle
    result_forward_handle: Option<tokio::task::JoinHandle<()>>,
    /// Error forwarding task handle
    error_forward_handle: Option<tokio::task::JoinHandle<()>>,
    /// Shared callback storage for async access
    result_callback: Arc<Mutex<Option<AsyncSTTCallback>>>,
    /// Error callback storage for streaming errors
    error_callback: Arc<Mutex<Option<AsyncErrorCallback>>>,
}

impl DeepgramSTT {
    /// Build the WebSocket URL with query parameters (optimized string building)
    fn build_websocket_url(&self, config: &DeepgramSTTConfig) -> Result<String, STTError> {
        let mut url = String::with_capacity(256); // Pre-allocate expected size
        url.push_str("wss://api.deepgram.com/v1/listen?model=");
        url.push_str(&config.base.model);
        url.push_str("&language=");
        url.push_str(&config.base.language);
        url.push_str("&sample_rate=");
        url.push_str(&config.base.sample_rate.to_string());
        url.push_str("&channels=");
        url.push_str(&config.base.channels.to_string());
        url.push_str("&punctuate=");
        url.push_str(&config.base.punctuation.to_string());
        url.push_str("&interim_results=");
        url.push_str(&config.interim_results.to_string());
        url.push_str("&smart_format=");
        url.push_str(&config.smart_format.to_string());
        url.push_str("&encoding=");
        url.push_str(&config.base.encoding);

        // Add optional parameters only if they're set
        if let Some(endpointing) = config.endpointing {
            url.push_str("&endpointing=");
            url.push_str(&endpointing.to_string());
        }

        if let Some(tag) = &config.tag {
            url.push_str("&tag=");
            url.push_str(tag);
        }

        if !config.keywords.is_empty() {
            url.push_str("&keywords=");
            url.push_str(&config.keywords.join(","));
        }

        Ok(url)
    }

    /// Handle incoming WebSocket messages (optimized for hot path)
    fn handle_websocket_message(
        message: Message,
        result_tx: &mpsc::UnboundedSender<STTResult>,
    ) -> Result<(), STTError> {
        match message {
            Message::Text(text) => {
                debug!("Received text message: {}", text);

                // Parse the response once and branch on type
                match serde_json::from_str::<DeepgramResponse>(&text) {
                    Ok(response) => {
                        match response.response_type.as_str() {
                            "Results" => {
                                if let Some(channel) = response.channel
                                    && let Some(alternative) = channel.alternatives.first()
                                {
                                    let stt_result = STTResult::new(
                                        alternative.transcript.clone(),
                                        response.is_final.unwrap_or(false),
                                        response.speech_final.unwrap_or(false),
                                        alternative.confidence,
                                    );

                                    // Send result (non-blocking)
                                    if result_tx.send(stt_result).is_err() {
                                        warn!("Failed to send result - channel closed");
                                    }
                                }
                            }
                            "Metadata" => {
                                // Log metadata for debugging but don't process
                                debug!("Received metadata response");
                            }
                            "Error" => {
                                // Try to parse as error for better message
                                let error_msg = if let Ok(error) =
                                    serde_json::from_str::<DeepgramError>(&text)
                                {
                                    format!("{}: {}", error.error_type, error.description)
                                } else {
                                    "Unknown error from Deepgram".to_string()
                                };
                                return Err(STTError::ProviderError(error_msg));
                            }
                            _ => {
                                // Ignore unknown message types
                                debug!("Received unknown message type: {}", response.response_type);
                            }
                        }
                    }
                    Err(_) => {
                        // Not a DeepgramResponse, might be a simple error or other message
                        if text.contains("\"type\":\"Error\"") {
                            let error_msg =
                                if let Ok(error) = serde_json::from_str::<DeepgramError>(&text) {
                                    format!("{}: {}", error.error_type, error.description)
                                } else {
                                    "Unknown error from Deepgram".to_string()
                                };
                            return Err(STTError::ProviderError(error_msg));
                        }
                        // Ignore other non-parseable messages
                    }
                }
            }
            Message::Close(close_frame) => {
                info!("WebSocket connection closed: {:?}", close_frame);
            }
            _ => {
                // Ignore other message types for performance
            }
        }

        Ok(())
    }

    /// Start the WebSocket connection task (optimized for minimal latency)
    async fn start_connection(&mut self, config: DeepgramSTTConfig) -> Result<(), STTError> {
        let ws_url = self.build_websocket_url(&config)?;

        // Create channels for communication (bounded for backpressure)
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

        // Start the connection task
        let connection_handle = tokio::spawn(async move {
            // Update state to connecting (this will be set by the main thread)
            // state_notify.notify_waiters() - don't notify here, main thread handles connecting state

            // Connect to Deepgram
            let host = "api.deepgram.com";
            let request = match tokio_tungstenite::tungstenite::http::Request::builder()
                .method("GET")
                .uri(&ws_url)
                .header("Host", host)
                .header("Upgrade", "websocket")
                .header("Connection", "upgrade")
                .header("Sec-WebSocket-Key", generate_key())
                .header("Sec-WebSocket-Version", "13")
                .header("Authorization", format!("token {api_key}"))
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

            let (ws_stream, _) = match connect_async(request).await {
                Ok(result) => result,
                Err(e) => {
                    let stt_error =
                        STTError::ConnectionFailed(format!("Failed to connect to Deepgram: {e}"));
                    error!("{}", stt_error);
                    let _ = error_tx.send(stt_error);
                    return;
                }
            };

            info!("Connected to Deepgram WebSocket");

            // Signal successful connection
            let _ = connected_tx.send(());

            let (mut ws_sink, mut ws_stream) = ws_stream.split();

            // Keep-alive mechanism to prevent connection timeout
            // Deepgram connections timeout after 10 seconds of inactivity
            // Send keep-alive messages every 1 second when no audio is being sent
            let mut keepalive_timer = interval(Duration::from_secs(1));
            let mut last_activity = Instant::now();

            // Main event loop
            loop {
                tokio::select! {
                    // Handle outgoing audio data
                    Some(audio_data) = ws_rx.recv() => {
                        let message = Message::Binary(audio_data);
                        if let Err(e) = ws_sink.send(message).await {
                            let stt_error = STTError::NetworkError(format!(
                                "Failed to send WebSocket message: {e}"
                            ));
                            error!("{}", stt_error);
                            let _ = error_tx.send(stt_error);
                            break;
                        }
                        // Update last activity time when audio is sent
                        last_activity = Instant::now();
                    }

                    // Handle incoming messages
                    message = ws_stream.next() => {
                        match message {
                            Some(Ok(msg)) => {
                                if let Err(e) = Self::handle_websocket_message(msg, &result_tx) {
                                    error!("Streaming error from Deepgram: {}", e);
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
                                info!("WebSocket stream ended");
                                break;
                            }
                        }
                    }

                    // Handle keep-alive timer
                    _ = keepalive_timer.tick() => {
                        // Check if we need to send a keep-alive message
                        // Only send if no audio was sent in the last second
                        if last_activity.elapsed() >= Duration::from_secs(1) {
                            let keepalive_message = Message::Text(r#"{"type":"KeepAlive"}"#.into());
                            if let Err(e) = ws_sink.send(keepalive_message).await {
                                let stt_error = STTError::NetworkError(format!(
                                    "Failed to send keep-alive message: {e}"
                                ));
                                error!("{}", stt_error);
                                let _ = error_tx.send(stt_error);
                                break;
                            }
                            debug!("Sent keep-alive message to Deepgram");
                        }
                    }

                    // Handle shutdown signal
                    _ = &mut shutdown_rx => {
                        info!("Received shutdown signal");
                        break;
                    }
                }
            }

            info!("Deepgram WebSocket connection closed");
        });

        self.connection_handle = Some(connection_handle);

        // Start result forwarding task with shared callback
        let callback_ref = self.result_callback.clone();
        let result_forwarding_handle = tokio::spawn(async move {
            while let Some(result) = result_rx.recv().await {
                if let Some(callback) = callback_ref.lock().await.as_ref() {
                    callback(result).await;
                } else {
                    // Log the result if no callback is set
                    debug!(
                        "Received STT result: {} (confidence: {})",
                        result.transcript, result.confidence
                    );
                }
            }
        });

        // Store the result forwarding handle for cleanup
        self.result_forward_handle = Some(result_forwarding_handle);

        // Start error forwarding task with shared callback
        let error_callback_ref = self.error_callback.clone();
        let error_forwarding_handle = tokio::spawn(async move {
            while let Some(error) = error_rx.recv().await {
                if let Some(callback) = error_callback_ref.lock().await.as_ref() {
                    callback(error).await;
                } else {
                    // Log the error if no callback is set
                    error!(
                        "STT streaming error but no error callback registered: {}",
                        error
                    );
                }
            }
        });

        // Store the error forwarding handle for cleanup
        self.error_forward_handle = Some(error_forwarding_handle);

        // Update state and wait for connection
        self.state = ConnectionState::Connecting;

        // Wait for connection to be established with timeout
        match timeout(Duration::from_secs(10), connected_rx).await {
            Ok(Ok(())) => {
                self.state = ConnectionState::Connected;
                self.state_notify.notify_waiters();
                info!("Successfully connected to Deepgram STT");
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
}

impl Default for DeepgramSTT {
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

#[async_trait::async_trait]
impl BaseSTT for DeepgramSTT {
    fn new(config: STTConfig) -> Result<Self, STTError> {
        // Validate API key
        if config.api_key.is_empty() {
            return Err(STTError::AuthenticationFailed(
                "API key is required".to_string(),
            ));
        }

        // Create Deepgram-specific configuration, preserving config values
        let deepgram_config = DeepgramSTTConfig {
            base: config,
            diarize: false,
            interim_results: true,
            filler_words: false,
            profanity_filter: false,
            smart_format: true,
            keywords: Vec::new(),
            redact: Vec::new(),
            vad_events: true,
            endpointing: Some(200),
            tag: None,
            utterance_end_ms: Some(500),
        };

        Ok(Self {
            config: Some(deepgram_config),
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

        // Wait for connection task to finish
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

        info!("Disconnected from Deepgram STT");
        Ok(())
    }

    fn is_ready(&self) -> bool {
        matches!(self.state, ConnectionState::Connected) && self.ws_sender.is_some()
    }

    async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
        if !self.is_ready() {
            return Err(STTError::ConnectionFailed(
                "Not connected to Deepgram".to_string(),
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

            debug!("Sent {} bytes of audio data", data_len);
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
        // For Deepgram, we need to reconnect to update configuration
        if self.is_ready() {
            self.disconnect().await?;
        }

        // Update stored configuration, preserving config values
        let deepgram_config = DeepgramSTTConfig {
            base: config,
            diarize: false,
            interim_results: true,
            filler_words: false,
            profanity_filter: false,
            smart_format: true,
            keywords: Vec::new(),
            redact: Vec::new(),
            vad_events: true,
            endpointing: Some(200),
            tag: None,
            utterance_end_ms: Some(500),
        };
        self.config = Some(deepgram_config);

        self.connect().await?;
        Ok(())
    }

    fn get_provider_info(&self) -> &'static str {
        "Deepgram STT WebSocket"
    }
}

impl Drop for DeepgramSTT {
    fn drop(&mut self) {
        // Send shutdown signal if still connected
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_deepgram_stt_creation() {
        let config = STTConfig {
            model: "nova-3".to_string(),
            provider: "deepgram".to_string(),
            api_key: "test_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            ..Default::default()
        };

        let stt = <DeepgramSTT as BaseSTT>::new(config).unwrap();
        assert!(!stt.is_ready());
        assert_eq!(stt.get_provider_info(), "Deepgram STT WebSocket");
    }

    #[tokio::test]
    async fn test_deepgram_stt_config_validation() {
        // Test with empty API key
        let config = STTConfig {
            model: "nova-3".to_string(),
            provider: "deepgram".to_string(),
            api_key: String::new(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            ..Default::default()
        };

        let result = <DeepgramSTT as BaseSTT>::new(config);
        assert!(result.is_err());
        if let Err(STTError::AuthenticationFailed(msg)) = result {
            assert!(msg.contains("API key is required"));
        } else {
            panic!("Expected AuthenticationFailed error");
        }
    }

    #[tokio::test]
    async fn test_deepgram_config_url_building() {
        let stt = DeepgramSTT::default();
        let config = DeepgramSTTConfig {
            base: STTConfig {
                model: "nova-3".to_string(),
                provider: "deepgram".to_string(),
                api_key: "test_key".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                channels: 1,
                punctuation: false, // Set to false here for testing
                encoding: "linear16".to_string(),
                ..Default::default()
            },
            interim_results: true,
            smart_format: false,
            endpointing: Some(300),
            utterance_end_ms: Some(1000),
            tag: Some("test-tag".to_string()),
            keywords: vec!["hello".to_string(), "world".to_string()],
            ..Default::default()
        };

        let url = stt.build_websocket_url(&config).unwrap();
        assert!(url.contains("wss://api.deepgram.com/v1/listen"));
        assert!(url.contains("model=nova-3"));
        assert!(url.contains("language=en-US"));
        assert!(url.contains("sample_rate=16000"));
        assert!(url.contains("channels=1"));
        assert!(url.contains("punctuate=false"));
        assert!(url.contains("interim_results=true"));
        assert!(url.contains("smart_format=false"));
        assert!(url.contains("keywords=hello,world"));
        assert!(url.contains("endpointing=300"));
        assert!(url.contains("tag=test-tag"));
    }

    #[tokio::test]
    async fn test_message_handling() {
        let (result_tx, mut result_rx) = mpsc::unbounded_channel::<STTResult>();

        let json_response = r#"
        {
            "type": "Results",
            "channel": {
                "alternatives": [
                    {
                        "transcript": "Hello world",
                        "confidence": 0.98,
                        "words": []
                    }
                ]
            },
            "is_final": true,
            "speech_final": true,
            "duration": 2.0,
            "start": 0.0
        }
        "#;

        let message = Message::Text(json_response.to_string().into());
        let result = DeepgramSTT::handle_websocket_message(message, &result_tx);

        assert!(result.is_ok());

        // Check that result was sent
        let received_result = tokio::time::timeout(Duration::from_millis(100), result_rx.recv())
            .await
            .expect("Should receive result within timeout")
            .expect("Should receive a result");
        assert_eq!(received_result.transcript, "Hello world");
        assert_eq!(received_result.confidence, 0.98);
        assert!(received_result.is_final);
        assert!(received_result.is_speech_final);
    }

    #[tokio::test]
    async fn test_keepalive_message_format() {
        // Test that the keep-alive message format is correct
        let keepalive_json = r#"{"type":"KeepAlive"}"#;
        let parsed: serde_json::Value = serde_json::from_str(keepalive_json).unwrap();

        assert_eq!(parsed["type"], "KeepAlive");

        // Test that the message can be created successfully
        let message = Message::Text(keepalive_json.into());
        match message {
            Message::Text(_) => {} // Success
            _ => panic!("Expected Text message"),
        }
    }
}
