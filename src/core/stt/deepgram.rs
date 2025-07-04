use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast, mpsc};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{debug, error, info, warn};
use url::Url;

use super::base::{
    BaseSTT, STTConfig, STTConnectionState, STTError, STTResult, STTResultCallback, STTStats,
};

/// Configuration specific to Deepgram STT
#[derive(Debug, Clone)]
pub struct DeepgramSTTConfig {
    /// Base STT configuration
    pub base: STTConfig,
    /// Deepgram model to use (e.g., "nova-2", "whisper")
    pub model: String,
    /// Enable punctuation in transcription
    pub punctuation: bool,
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
}

impl Default for DeepgramSTTConfig {
    fn default() -> Self {
        Self {
            base: STTConfig::default(),
            model: "nova-2".to_string(),
            punctuation: true,
            diarize: false,
            interim_results: true,
            filler_words: false,
            profanity_filter: false,
            smart_format: true,
            keywords: Vec::new(),
            redact: Vec::new(),
            vad_events: false,
            endpointing: Some(10),
            tag: None,
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
    /// Connection state
    state: Arc<RwLock<STTConnectionState>>,
    /// Statistics
    stats: Arc<RwLock<STTStats>>,
    /// WebSocket sender for audio data
    ws_sender: Option<mpsc::UnboundedSender<Message>>,
    /// Shutdown signal sender
    shutdown_tx: Option<broadcast::Sender<()>>,
    /// Result callback
    result_callback: Option<STTResultCallback>,
    /// Connection handle
    connection_handle: Option<tokio::task::JoinHandle<()>>,
}

impl DeepgramSTT {
    /// Build the WebSocket URL with query parameters
    fn build_websocket_url(&self, config: &DeepgramSTTConfig) -> Result<String, STTError> {
        let mut url = Url::parse("wss://api.deepgram.com/v1/listen")
            .map_err(|e| STTError::ConfigurationError(format!("Invalid WebSocket URL: {e}")))?;

        // Add query parameters
        {
            let mut query_pairs = url.query_pairs_mut();

            // Model
            query_pairs.append_pair("model", &config.model);

            // Language
            query_pairs.append_pair("language", &config.base.language);

            // Audio settings
            query_pairs.append_pair("sample_rate", &config.base.sample_rate.to_string());
            query_pairs.append_pair("channels", &config.base.channels.to_string());

            // Feature flags
            query_pairs.append_pair("punctuation", &config.punctuation.to_string());
            query_pairs.append_pair("diarize", &config.diarize.to_string());
            query_pairs.append_pair("interim_results", &config.interim_results.to_string());
            query_pairs.append_pair("filler_words", &config.filler_words.to_string());
            query_pairs.append_pair("profanity_filter", &config.profanity_filter.to_string());
            query_pairs.append_pair("smart_format", &config.smart_format.to_string());
            query_pairs.append_pair("vad_events", &config.vad_events.to_string());

            // Optional parameters
            if let Some(endpointing) = config.endpointing {
                query_pairs.append_pair("endpointing", &endpointing.to_string());
            }

            if let Some(tag) = &config.tag {
                query_pairs.append_pair("tag", tag);
            }

            // Keywords
            if !config.keywords.is_empty() {
                query_pairs.append_pair("keywords", &config.keywords.join(","));
            }

            // Redaction
            if !config.redact.is_empty() {
                query_pairs.append_pair("redact", &config.redact.join(","));
            }
        }

        Ok(url.to_string())
    }

    /// Handle incoming WebSocket messages
    async fn handle_websocket_message(
        &self,
        message: Message,
        callback: Option<STTResultCallback>,
        stats: Arc<RwLock<STTStats>>,
    ) -> Result<(), STTError> {
        match message {
            Message::Text(text) => {
                debug!("Received text message: {}", text);

                // Parse the JSON response
                let response: DeepgramResponse = serde_json::from_str(&text).map_err(|e| {
                    STTError::ProviderError(format!("Failed to parse response: {e}"))
                })?;

                // Handle different response types
                match response.response_type.as_str() {
                    "Results" => {
                        if let Some(channel) = response.channel {
                            if let Some(alternative) = channel.alternatives.first() {
                                let stt_result = STTResult::new(
                                    alternative.transcript.clone(),
                                    response.is_final.unwrap_or(false),
                                    response.speech_final.unwrap_or(false),
                                    alternative.confidence,
                                );

                                // Update statistics
                                {
                                    let mut stats_guard = stats.write().await;
                                    stats_guard.update_with_result(&stt_result);
                                }

                                // Call the result callback
                                if let Some(callback) = callback {
                                    callback(stt_result).await;
                                }
                            }
                        }
                    }
                    "Metadata" => {
                        debug!("Received metadata: {:?}", response.metadata);
                    }
                    "Error" => {
                        let error_msg =
                            if let Ok(error) = serde_json::from_str::<DeepgramError>(&text) {
                                format!("{}: {}", error.error_type, error.description)
                            } else {
                                "Unknown error from Deepgram".to_string()
                            };
                        return Err(STTError::ProviderError(error_msg));
                    }
                    _ => {
                        warn!("Unknown response type: {}", response.response_type);
                    }
                }
            }
            Message::Binary(data) => {
                debug!("Received binary message: {} bytes", data.len());
                // Deepgram STT typically doesn't send binary data back
                warn!("Unexpected binary message from Deepgram");
            }
            Message::Close(close_frame) => {
                info!("WebSocket connection closed: {:?}", close_frame);
            }
            Message::Ping(data) => {
                debug!("Received ping: {} bytes", data.len());
                // Pong will be handled automatically by the WebSocket library
            }
            Message::Pong(data) => {
                debug!("Received pong: {} bytes", data.len());
            }
            Message::Frame(_) => {
                // Raw frames are handled by the WebSocket library
            }
        }

        Ok(())
    }

    /// Start the WebSocket connection task
    async fn start_connection(&mut self, config: DeepgramSTTConfig) -> Result<(), STTError> {
        let ws_url = self.build_websocket_url(&config)?;

        // Create channels for communication
        let (ws_tx, mut ws_rx) = mpsc::unbounded_channel::<Message>();
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

        // Store channels
        self.ws_sender = Some(ws_tx);
        self.shutdown_tx = Some(shutdown_tx);

        // Clone necessary data for the connection task
        let state = self.state.clone();
        let stats = self.stats.clone();
        let callback = self.result_callback.clone();
        let api_key = config.base.api_key.clone();

        // Start the connection task
        let connection_handle = tokio::spawn(async move {
            // Update state to connecting
            {
                let mut state_guard = state.write().await;
                *state_guard = STTConnectionState::Connecting;
            }

            // Connect to Deepgram
            let request = tokio_tungstenite::tungstenite::http::Request::builder()
                .uri(&ws_url)
                .header("Authorization", format!("token {api_key}"))
                .header("Sec-WebSocket-Protocol", "token")
                .body(())
                .unwrap();

            let (ws_stream, _) = match connect_async(request).await {
                Ok(result) => result,
                Err(e) => {
                    error!("Failed to connect to Deepgram: {}", e);
                    let mut state_guard = state.write().await;
                    *state_guard = STTConnectionState::Error(format!("Connection failed: {e}"));
                    return;
                }
            };

            info!("Connected to Deepgram WebSocket");

            // Update state to connected
            {
                let mut state_guard = state.write().await;
                *state_guard = STTConnectionState::Connected;
            }

            let (mut ws_sink, mut ws_stream) = ws_stream.split();

            // Handle incoming and outgoing messages
            loop {
                tokio::select! {
                    // Handle outgoing messages
                    Some(message) = ws_rx.recv() => {
                        if let Err(e) = ws_sink.send(message).await {
                            error!("Failed to send WebSocket message: {}", e);
                            break;
                        }
                    }

                    // Handle incoming messages
                    message = ws_stream.next() => {
                        match message {
                            Some(Ok(msg)) => {
                                // Create a temporary instance for handling the message
                                let stt_instance = DeepgramSTT {
                                    config: None,
                                    state: state.clone(),
                                    stats: stats.clone(),
                                    ws_sender: None,
                                    shutdown_tx: None,
                                    result_callback: None,
                                    connection_handle: None,
                                };
                                if let Err(e) = stt_instance.handle_websocket_message(msg, callback.clone(), stats.clone()).await {
                                    error!("Failed to handle WebSocket message: {}", e);
                                    break;
                                }
                            }
                            Some(Err(e)) => {
                                error!("WebSocket error: {}", e);
                                break;
                            }
                            None => {
                                info!("WebSocket stream ended");
                                break;
                            }
                        }
                    }

                    // Handle shutdown signal
                    _ = shutdown_rx.recv() => {
                        info!("Received shutdown signal");
                        break;
                    }
                }
            }

            // Update state to disconnected
            {
                let mut state_guard = state.write().await;
                *state_guard = STTConnectionState::Disconnected;
            }

            info!("Deepgram WebSocket connection closed");
        });

        self.connection_handle = Some(connection_handle);

        // Wait for connection to be established
        let mut attempts = 0;
        while attempts < 50 {
            let state = self.state.read().await;
            match *state {
                STTConnectionState::Connected => break,
                STTConnectionState::Error(_) => {
                    return Err(STTError::ConnectionFailed(
                        "Failed to connect to Deepgram".to_string(),
                    ));
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    attempts += 1;
                }
            }
        }

        if attempts >= 50 {
            return Err(STTError::ConnectionFailed("Connection timeout".to_string()));
        }

        Ok(())
    }
}

impl Default for DeepgramSTT {
    fn default() -> Self {
        Self {
            config: None,
            state: Arc::new(RwLock::new(STTConnectionState::Disconnected)),
            stats: Arc::new(RwLock::new(STTStats::default())),
            ws_sender: None,
            shutdown_tx: None,
            result_callback: None,
            connection_handle: None,
        }
    }
}

#[async_trait::async_trait]
impl BaseSTT for DeepgramSTT {
    async fn new(config: STTConfig) -> Result<Self, STTError> {
        // Validate API key
        if config.api_key.is_empty() {
            return Err(STTError::AuthenticationFailed(
                "API key is required".to_string(),
            ));
        }

        // Create Deepgram-specific configuration
        let deepgram_config = DeepgramSTTConfig {
            base: config,
            ..Default::default()
        };

        Ok(Self {
            config: Some(deepgram_config),
            state: Arc::new(RwLock::new(STTConnectionState::Disconnected)),
            stats: Arc::new(RwLock::new(STTStats::default())),
            ws_sender: None,
            shutdown_tx: None,
            result_callback: None,
            connection_handle: None,
        })
    }

    async fn connect(&mut self) -> Result<(), STTError> {
        // Get the stored configuration
        let config = self.config.as_ref().ok_or_else(|| {
            STTError::ConfigurationError("No configuration available".to_string())
        })?;

        // Start the connection
        self.start_connection(config.clone()).await?;

        info!("Successfully connected to Deepgram STT");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), STTError> {
        // Send shutdown signal
        if let Some(shutdown_tx) = &self.shutdown_tx {
            let _ = shutdown_tx.send(());
        }

        // Wait for connection task to finish
        if let Some(handle) = self.connection_handle.take() {
            let _ = timeout(Duration::from_secs(5), handle).await;
        }

        // Clean up
        self.ws_sender = None;
        self.shutdown_tx = None;
        self.result_callback = None;

        // Update state
        {
            let mut state = self.state.write().await;
            *state = STTConnectionState::Disconnected;
        }

        info!("Disconnected from Deepgram STT");
        Ok(())
    }

    fn is_ready(&self) -> bool {
        // Check if we have a WebSocket sender (connection is established)
        self.ws_sender.is_some()
    }

    async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
        if !self.is_ready() {
            return Err(STTError::ConnectionFailed(
                "Not connected to Deepgram".to_string(),
            ));
        }

        if let Some(ws_sender) = &self.ws_sender {
            // Send audio data as binary message
            let message = Message::Binary(audio_data.clone().into());
            ws_sender
                .send(message)
                .map_err(|e| STTError::NetworkError(format!("Failed to send audio data: {e}")))?;

            // Update statistics
            {
                let mut stats = self.stats.write().await;
                stats.total_audio_bytes += audio_data.len() as u64;
            }

            debug!("Sent {} bytes of audio data", audio_data.len());
        }

        Ok(())
    }

    async fn on_result(&mut self, callback: STTResultCallback) -> Result<(), STTError> {
        self.result_callback = Some(callback);
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

        // Update stored configuration
        let deepgram_config = DeepgramSTTConfig {
            base: config,
            ..Default::default()
        };
        self.config = Some(deepgram_config);

        self.connect().await?;
        Ok(())
    }

    fn get_provider_info(&self) -> &'static str {
        "Deepgram STT WebSocket v1.0"
    }
}

impl Drop for DeepgramSTT {
    fn drop(&mut self) {
        // Send shutdown signal if still connected
        if let Some(shutdown_tx) = &self.shutdown_tx {
            let _ = shutdown_tx.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{Duration, sleep};

    #[tokio::test]
    async fn test_deepgram_stt_creation() {
        let config = STTConfig {
            api_key: "test_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
        };

        let stt = <DeepgramSTT as BaseSTT>::new(config).await.unwrap();
        assert!(!stt.is_ready());
        assert_eq!(stt.get_provider_info(), "Deepgram STT WebSocket v1.0");
    }

    #[tokio::test]
    async fn test_deepgram_stt_config_validation() {
        // Test with empty API key
        let config = STTConfig {
            api_key: String::new(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
        };

        let result = <DeepgramSTT as BaseSTT>::new(config).await;
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
                api_key: "test_key".to_string(),
                language: "en-US".to_string(),
                sample_rate: 16000,
                channels: 1,
                punctuation: true,
            },
            model: "nova-2".to_string(),
            punctuation: true,
            diarize: false,
            interim_results: true,
            filler_words: false,
            profanity_filter: false,
            smart_format: true,
            keywords: vec!["hello".to_string(), "world".to_string()],
            redact: vec!["ssn".to_string()],
            vad_events: false,
            endpointing: Some(10),
            tag: Some("test-tag".to_string()),
        };

        let url = stt.build_websocket_url(&config).unwrap();
        assert!(url.contains("wss://api.deepgram.com/v1/listen"));
        assert!(url.contains("model=nova-2"));
        assert!(url.contains("language=en-US"));
        assert!(url.contains("sample_rate=16000"));
        assert!(url.contains("channels=1"));
        assert!(url.contains("punctuation=true"));
        assert!(url.contains("interim_results=true"));
        assert!(url.contains("keywords=hello%2Cworld") || url.contains("keywords=hello,world"));
        assert!(url.contains("redact=ssn"));
        assert!(url.contains("endpointing=10"));
        assert!(url.contains("tag=test-tag"));
    }

    #[tokio::test]
    async fn test_deepgram_response_parsing() {
        let stt = DeepgramSTT::default();
        let stats = Arc::new(RwLock::new(STTStats::default()));
        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        let callback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                assert_eq!(result.transcript, "Hello world");
                assert_eq!(result.confidence, 0.98);
                assert!(result.is_final);
                assert!(result.is_speech_final);
            }) as std::pin::Pin<Box<dyn futures::Future<Output = ()> + Send>>
        });

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
        let result = stt
            .handle_websocket_message(message, Some(callback), stats.clone())
            .await;

        assert!(result.is_ok());

        // Give the callback time to execute
        sleep(Duration::from_millis(100)).await;

        // Check that callback was called
        assert_eq!(callback_count.load(Ordering::SeqCst), 1);

        // Check that stats were updated
        let stats_guard = stats.read().await;
        assert_eq!(stats_guard.results_count, 1);
        assert_eq!(stats_guard.final_results_count, 1);
        assert_eq!(stats_guard.average_confidence, 0.98);
    }

    #[tokio::test]
    async fn test_deepgram_error_handling() {
        let stt = DeepgramSTT::default();
        let stats = Arc::new(RwLock::new(STTStats::default()));

        let error_response = r#"
        {
            "type": "Error",
            "error_type": "authentication_error",
            "description": "Invalid API key",
            "message": "The provided API key is invalid"
        }
        "#;

        let message = Message::Text(error_response.to_string().into());
        let result = stt.handle_websocket_message(message, None, stats).await;

        assert!(result.is_err());
        if let Err(STTError::ProviderError(msg)) = result {
            assert!(msg.contains("Error"));
            assert!(msg.contains("Invalid API key"));
        } else {
            panic!("Expected ProviderError");
        }
    }
}
