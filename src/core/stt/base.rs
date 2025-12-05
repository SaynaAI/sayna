use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Result structure containing transcription data from STT providers
#[derive(Debug, Clone, PartialEq)]
pub struct STTResult {
    /// The transcribed text from the audio
    pub transcript: String,
    /// Whether this is a final transcription result (not an interim result)
    pub is_final: bool,
    /// Whether this marks the end of a speech segment
    pub is_speech_final: bool,
    /// Confidence score of the transcription (0.0 to 1.0)
    pub confidence: f32,
}

impl STTResult {
    /// Creates a new STTResult
    pub fn new(transcript: String, is_final: bool, is_speech_final: bool, confidence: f32) -> Self {
        Self {
            transcript,
            is_final,
            is_speech_final,
            confidence: confidence.clamp(0.0, 1.0), // Ensure confidence is within valid range
        }
    }
}

/// Configuration for STT providers
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct STTConfig {
    pub provider: String,
    /// API key for the STT provider
    pub api_key: String,
    /// Language code for transcription (e.g., "en-US", "es-ES")
    pub language: String,
    /// Sample rate of the audio in Hz
    pub sample_rate: u32,
    /// Number of audio channels (1 for mono, 2 for stereo)
    pub channels: u16,
    /// Enable punctuation in results
    pub punctuation: bool,
    /// Encoding of the audio
    pub encoding: String,
    /// Model to use for transcription
    pub model: String,
    /// Cache directory for local models (e.g., Whisper)
    /// Must be set from ServerConfig.cache_path for local providers
    #[serde(default)]
    pub cache_path: Option<std::path::PathBuf>,
}

impl Default for STTConfig {
    fn default() -> Self {
        Self {
            model: "nova-3".to_string(),
            provider: String::new(),
            api_key: String::new(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            cache_path: None,
        }
    }
}

/// Error types for STT operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum STTError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    #[error("Audio processing error: {0}")]
    AudioProcessingError(String),
    #[error("Provider error: {0}")]
    ProviderError(String),
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Invalid audio format: {0}")]
    InvalidAudioFormat(String),
}

/// Type alias for STT result callback
pub type STTResultCallback =
    Arc<dyn Fn(STTResult) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Type alias for STT error callback
pub type STTErrorCallback =
    Arc<dyn Fn(STTError) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Base trait for Speech-to-Text providers
#[async_trait::async_trait]
pub trait BaseSTT: Send + Sync {
    /// Create a new instance of the STT provider with the given configuration
    ///
    /// # Arguments
    /// * `config` - Configuration for the STT provider
    ///
    /// # Returns
    /// * `Result<Self, STTError>` - New instance or error
    fn new(config: STTConfig) -> Result<Self, STTError>
    where
        Self: Sized;

    /// Connect to the STT provider
    ///
    /// # Returns
    /// * `Result<(), STTError>` - Success or error
    async fn connect(&mut self) -> Result<(), STTError>;

    /// Disconnect from the STT provider
    ///
    /// # Returns
    /// * `Result<(), STTError>` - Success or error
    async fn disconnect(&mut self) -> Result<(), STTError>;

    /// Check if the connection is ready to be used
    ///
    /// # Returns
    /// * `bool` - True if ready, false otherwise
    fn is_ready(&self) -> bool;

    /// Send audio data to the STT provider for transcription
    ///
    /// # Arguments
    /// * `audio_data` - Audio bytes to process
    ///
    /// # Returns
    /// * `Result<(), STTError>` - Success or error
    async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError>;

    /// Register a callback function that gets triggered when transcription results are available
    ///
    /// # Arguments
    /// * `callback` - Callback function to handle STT results
    ///
    /// # Returns
    /// * `Result<(), STTError>` - Success or error
    async fn on_result(&mut self, callback: STTResultCallback) -> Result<(), STTError>;

    /// Register a callback function that gets triggered when errors occur during streaming
    ///
    /// This is critical for propagating streaming errors (e.g., permission denied, rate limits)
    /// that occur after the initial connection is established.
    ///
    /// # Arguments
    /// * `callback` - Callback function to handle STT errors
    ///
    /// # Returns
    /// * `Result<(), STTError>` - Success or error
    async fn on_error(&mut self, callback: STTErrorCallback) -> Result<(), STTError>;

    /// Get the current configuration
    fn get_config(&self) -> Option<&STTConfig>;

    /// Update configuration while maintaining connection
    ///
    /// # Arguments
    /// * `config` - New configuration
    ///
    /// # Returns
    /// * `Result<(), STTError>` - Success or error
    async fn update_config(&mut self, config: STTConfig) -> Result<(), STTError>;

    /// Get provider-specific information
    fn get_provider_info(&self) -> &'static str;
}

/// Factory trait for creating STT providers
pub trait STTFactory {
    /// Create a new STT provider instance
    fn create_stt() -> Box<dyn BaseSTT>;
}

/// Helper trait for common STT operations
pub trait STTHelper {
    /// Validate audio format
    fn validate_audio_format(&self, sample_rate: u32, channels: u16) -> Result<(), STTError>;

    /// Convert audio to required format
    fn convert_audio_format(
        &self,
        audio_data: Vec<u8>,
        target_format: &str,
    ) -> Result<Vec<u8>, STTError>;
}

/// Connection state for STT providers
#[derive(Debug, Clone, PartialEq)]
pub enum STTConnectionState {
    /// Not connected
    Disconnected,
    /// In the process of connecting
    Connecting,
    /// Connected and ready to receive audio
    Connected,
    /// Error state
    Error(String),
}

/// Statistics for STT operations
#[derive(Debug, Default, Clone)]
pub struct STTStats {
    /// Total audio bytes processed
    pub total_audio_bytes: u64,
    /// Number of transcription results received
    pub results_count: u32,
    /// Number of final results received
    pub final_results_count: u32,
    /// Average confidence score
    pub average_confidence: f32,
    /// Connection uptime in seconds
    pub uptime_seconds: u64,
}

impl STTStats {
    /// Update statistics with a new result
    pub fn update_with_result(&mut self, result: &STTResult) {
        self.results_count += 1;
        if result.is_final {
            self.final_results_count += 1;
        }

        // Update average confidence
        let total_confidence =
            self.average_confidence * (self.results_count - 1) as f32 + result.confidence;
        self.average_confidence = total_confidence / self.results_count as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Mock implementation for testing
    struct MockSTT {
        config: Option<STTConfig>,
        connected: AtomicBool,
        callback: Option<STTResultCallback>,
    }

    #[async_trait::async_trait]
    impl BaseSTT for MockSTT {
        fn new(config: STTConfig) -> Result<Self, STTError> {
            Ok(Self {
                config: Some(config),
                connected: AtomicBool::new(false),
                callback: None,
            })
        }

        async fn connect(&mut self) -> Result<(), STTError> {
            self.connected.store(true, Ordering::Relaxed);
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), STTError> {
            self.connected.store(false, Ordering::Relaxed);
            Ok(())
        }

        fn is_ready(&self) -> bool {
            self.connected.load(Ordering::Relaxed)
        }

        async fn send_audio(&mut self, audio_data: Vec<u8>) -> Result<(), STTError> {
            if !self.is_ready() {
                return Err(STTError::ConnectionFailed("Not connected".to_string()));
            }

            // Mock processing - simulate transcription result
            if let Some(ref callback) = self.callback {
                let result = STTResult::new(
                    format!("Transcribed {} bytes of audio", audio_data.len()),
                    true,
                    true,
                    0.95,
                );
                callback(result).await;
            }

            Ok(())
        }

        async fn on_result(&mut self, callback: STTResultCallback) -> Result<(), STTError> {
            self.callback = Some(callback);
            Ok(())
        }

        async fn on_error(&mut self, _callback: STTErrorCallback) -> Result<(), STTError> {
            // Mock implementation - errors not simulated
            Ok(())
        }

        fn get_config(&self) -> Option<&STTConfig> {
            self.config.as_ref()
        }

        async fn update_config(&mut self, config: STTConfig) -> Result<(), STTError> {
            if self.is_ready() {
                self.config = Some(config);
                Ok(())
            } else {
                Err(STTError::ConnectionFailed("Not connected".to_string()))
            }
        }

        fn get_provider_info(&self) -> &'static str {
            "MockSTT v1.0"
        }
    }

    #[tokio::test]
    async fn test_stt_new_function() {
        let config = STTConfig {
            model: "nova-3".to_string(),
            provider: "mock".to_string(),
            api_key: "test_key".to_string(),
            language: "en-US".to_string(),
            sample_rate: 16000,
            channels: 1,
            punctuation: true,
            encoding: "linear16".to_string(),
            ..Default::default()
        };

        let stt = MockSTT::new(config.clone()).unwrap();

        // Should have config set but not be connected
        assert!(stt.get_config().is_some());
        assert!(!stt.is_ready());

        // Config should match what we passed
        let stored_config = stt.get_config().unwrap();
        assert_eq!(stored_config.api_key, "test_key");
        assert_eq!(stored_config.language, "en-US");
        assert_eq!(stored_config.sample_rate, 16000);
        assert_eq!(stored_config.channels, 1);
        assert!(stored_config.punctuation);
    }

    #[tokio::test]
    async fn test_mock_stt_implementation() {
        // Test creation with config
        let config = STTConfig::default();
        let mut stt = MockSTT::new(config.clone()).unwrap();

        // Test initial state - should not be connected yet
        assert!(!stt.is_ready());
        assert!(stt.get_config().is_some());

        // Test connection
        stt.connect().await.unwrap();
        assert!(stt.is_ready());
        assert!(stt.get_config().is_some());

        // Test callback registration - simplified for testing
        let callback = Arc::new(|result: STTResult| {
            Box::pin(async move {
                println!("Received result: {result:?}");
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        stt.on_result(callback).await.unwrap();

        // Test audio processing
        let audio_data = vec![0u8; 1024];
        stt.send_audio(audio_data).await.unwrap();

        // Give some time for async callback to execute
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Test disconnect
        stt.disconnect().await.unwrap();
        assert!(!stt.is_ready());

        // Test provider info
        assert_eq!(stt.get_provider_info(), "MockSTT v1.0");
    }

    #[test]
    fn test_stt_result_creation() {
        let result = STTResult::new("Hello world".to_string(), true, true, 0.95);
        assert_eq!(result.transcript, "Hello world");
        assert!(result.is_final);
        assert!(result.is_speech_final);
        assert_eq!(result.confidence, 0.95);
    }

    #[test]
    fn test_stt_result_confidence_clamping() {
        let result = STTResult::new("Test".to_string(), true, false, 1.5);
        assert_eq!(result.confidence, 1.0);

        let result = STTResult::new("Test".to_string(), true, false, -0.5);
        assert_eq!(result.confidence, 0.0);
    }

    #[test]
    fn test_stt_config_default() {
        let config = STTConfig::default();
        assert_eq!(config.language, "en-US");
        assert_eq!(config.sample_rate, 16000);
        assert_eq!(config.channels, 1);
        assert!(config.punctuation);
    }

    #[test]
    fn test_stt_stats_update() {
        let mut stats = STTStats::default();
        let result = STTResult::new("Test".to_string(), true, false, 0.8);

        stats.update_with_result(&result);

        assert_eq!(stats.results_count, 1);
        assert_eq!(stats.final_results_count, 1);
        assert_eq!(stats.average_confidence, 0.8);
    }

    #[test]
    fn test_stt_connection_states() {
        let disconnected = STTConnectionState::Disconnected;
        let connecting = STTConnectionState::Connecting;
        let connected = STTConnectionState::Connected;
        let error = STTConnectionState::Error("Test error".to_string());

        assert_eq!(disconnected, STTConnectionState::Disconnected);
        assert_eq!(connecting, STTConnectionState::Connecting);
        assert_eq!(connected, STTConnectionState::Connected);
        assert_eq!(error, STTConnectionState::Error("Test error".to_string()));
    }
}
