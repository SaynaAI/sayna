//! # TTS Base Trait Implementation
//!
//! This module provides the base trait abstraction for Text-to-Speech (TTS) providers.
//! It allows for a unified interface to interact with different TTS services like
//! Deepgram, ElevenLabs, Google TTS, and other providers.
//!
//! ## Usage Example
//!
//! ```rust,ignore
//! use sayna::core::tts::{BaseTTS, TTSConfig, AudioCallback, AudioData, TTSError};
//! use std::sync::Arc;
//! use std::pin::Pin;
//! use std::future::Future;
//!
//! // Create a custom audio callback
//! struct MyAudioCallback;
//!
//! impl AudioCallback for MyAudioCallback {
//!     fn on_audio(&self, audio_data: AudioData) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
//!         Box::pin(async move {
//!             println!("Received audio: {} bytes, format: {}",
//!                      audio_data.data.len(), audio_data.format);
//!             // Process the audio data here
//!         })
//!     }
//!     
//!     fn on_error(&self, error: TTSError) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
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
//! // Usage with a TTS provider
//! async fn example_usage() -> Result<(), TTSError> {
//!     // Configure the provider
//!     let config = TTSConfig {
//!         voice_id: Some("aura-luna-en".to_string()),
//!         speaking_rate: Some(1.0),
//!         audio_format: Some("pcm".to_string()),
//!         sample_rate: Some(22050),
//!         ..Default::default()
//!     };
//!     
//!     // Create your TTS provider (e.g., DeepgramTTS, ElevenLabsTTS, etc.)
//!     let mut tts_provider = create_tts_provider("deepgram", config)?;
//!     
//!     // Connect to the provider
//!     tts_provider.connect().await?;
//!     
//!     // Register audio callback
//!     let callback = Arc::new(MyAudioCallback);
//!     tts_provider.on_audio(callback)?;
//!     
//!     // Synthesize text
//!     tts_provider.speak("Hello, world! This is a test of the TTS system.").await?;
//!     
//!     // Flush to ensure immediate processing
//!     tts_provider.flush().await?;
//!     
//!     // Clean up
//!     tts_provider.disconnect().await?;
//!     
//!     Ok(())
//! }
//!
//! // Factory function is available in the parent module
//! # use crate::core::tts::create_tts_provider;
//! ```

use async_trait::async_trait;
use futures::Future;
use std::pin::Pin;
use std::sync::Arc;

/// Audio data structure for TTS output
#[derive(Debug, Clone)]
pub struct AudioData {
    /// Audio bytes in the format specified by the provider
    pub data: Vec<u8>,
    /// Sample rate of the audio
    pub sample_rate: u32,
    /// Audio format (e.g., "wav", "mp3", "pcm")
    pub format: String,
    /// Duration of the audio chunk in milliseconds
    pub duration_ms: Option<u32>,
}

/// TTS-specific error types
#[derive(Debug, Clone, thiserror::Error)]
pub enum TTSError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Provider not ready: {0}")]
    ProviderNotReady(String),

    #[error("Audio generation failed: {0}")]
    AudioGenerationFailed(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Provider error: {0}")]
    ProviderError(String),

    #[error("Timeout error: {0}")]
    TimeoutError(String),

    #[error("Internal error: {0}")]
    InternalError(String),
}

/// Result type for TTS operations
pub type TTSResult<T> = Result<T, TTSError>;

/// Connection state for TTS providers
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    /// Not connected to the provider
    Disconnected,
    /// Currently connecting to the provider
    Connecting,
    /// Connected and ready to receive requests
    Connected,
    /// Provider is processing requests
    Processing,
    /// Error state - connection failed or lost
    Error(String),
}

/// Audio callback trait for handling audio data from TTS providers
pub trait AudioCallback: Send + Sync {
    /// Called when audio data is received from the TTS provider
    fn on_audio(&self, audio_data: AudioData) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Called when an error occurs during audio generation
    fn on_error(&self, error: TTSError) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;

    /// Called when the TTS provider finishes processing all queued text
    fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// Pronunciation replacement configuration
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Pronunciation {
    /// Word to replace
    pub word: String,
    /// Pronunciation to use instead
    pub pronunciation: String,
}

/// Configuration for TTS providers
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct TTSConfig {
    pub provider: String,
    /// API key for the TTS provider
    pub api_key: String,
    /// Voice ID or name to use for synthesis
    pub voice_id: Option<String>,
    /// Model to use for TTS
    pub model: String,
    /// Speaking rate (0.25 to 4.0, 1.0 is normal)
    pub speaking_rate: Option<f32>,
    /// Audio format preference
    pub audio_format: Option<String>,
    /// Sample rate preference
    pub sample_rate: Option<u32>,
    /// Connection timeout in seconds
    pub connection_timeout: Option<u64>,
    /// Request timeout in seconds
    pub request_timeout: Option<u64>,
    /// Pronunciation replacements to apply before TTS
    pub pronunciations: Vec<Pronunciation>,
    /// Request pool size for concurrent HTTP requests
    pub request_pool_size: Option<usize>,
}

impl Default for TTSConfig {
    fn default() -> Self {
        Self {
            model: "".to_string(),
            provider: String::new(),
            api_key: String::new(),
            voice_id: Some("aura-asteria-en".to_string()),
            speaking_rate: Some(1.0),
            audio_format: Some("linear16".to_string()),
            sample_rate: Some(24000),
            connection_timeout: Some(30),
            request_timeout: Some(60),
            pronunciations: Vec::new(),
            request_pool_size: Some(4),
        }
    }
}

/// Base trait for Text-to-Speech providers
#[async_trait]
pub trait BaseTTS: Send + Sync {
    /// Create a new instance of the TTS provider
    ///
    /// This method creates a new instance of the TTS provider with the given configuration.
    /// The configuration is stored as a struct field and used later during connection.
    ///
    /// # Arguments
    /// * `config` - Configuration for the TTS provider
    ///
    /// # Returns
    /// * `TTSResult<Self>` - A new instance of the TTS provider
    fn new(config: TTSConfig) -> TTSResult<Self>
    where
        Self: Sized;

    /// Get the underlying TTSProvider for HTTP-based providers.
    /// Returns None for non-HTTP providers.
    fn get_provider(&mut self) -> Option<&mut crate::core::tts::provider::TTSProvider> {
        None
    }

    /// Connect to the TTS provider
    ///
    /// This method establishes a connection to the TTS provider (e.g., Deepgram, ElevenLabs)
    /// and prepares it for text synthesis requests. Uses the configuration provided during
    /// initialization.
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of the connection attempt
    async fn connect(&mut self) -> TTSResult<()> {
        // Default implementation - derived implementations should override to use config
        Err(TTSError::InternalError(
            "Connect method should be overridden by provider implementation".to_string(),
        ))
    }

    /// Disconnect from the TTS provider
    ///
    /// This method cleanly closes the connection to the TTS provider and releases any resources.
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of the disconnection attempt
    async fn disconnect(&mut self) -> TTSResult<()> {
        if let Some(provider) = self.get_provider() {
            provider.generic_disconnect().await
        } else {
            Err(TTSError::InternalError(
                "Provider not available for disconnection".to_string(),
            ))
        }
    }

    /// Check if the TTS provider is ready to process requests
    ///
    /// This method indicates whether the connection is established and ready to be used
    /// for text synthesis.
    ///
    /// # Returns
    /// * `bool` - True if ready, false otherwise
    fn is_ready(&self) -> bool {
        false
    }

    /// Get the current connection state
    ///
    /// # Returns
    /// * `ConnectionState` - Current state of the connection
    fn get_connection_state(&self) -> ConnectionState {
        ConnectionState::Disconnected
    }

    /// Send text to the TTS provider for synthesis
    ///
    /// This method sends text to the target TTS provider for conversion to speech.
    /// The audio will be delivered via the registered audio callback.
    /// If the provider is not ready, it will attempt to reconnect automatically.
    ///
    /// # Arguments
    /// * `text` - The text to synthesize
    /// * `flush` - Whether to immediately flush and start processing the text
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of the synthesis request
    async fn speak(&mut self, text: &str, flush: bool) -> TTSResult<()>;

    /// Clear any queued text from the TTS provider
    ///
    /// This method sends a clear command to the target provider to remove any
    /// pending text from the synthesis queue.
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of the clear operation
    async fn clear(&mut self) -> TTSResult<()> {
        if let Some(provider) = self.get_provider() {
            provider.generic_clear().await
        } else {
            Err(TTSError::InternalError(
                "Provider not available for clear".to_string(),
            ))
        }
    }

    /// Flush the TTS provider
    ///
    /// This method forces the TTS provider to start processing any queued text
    /// immediately, rather than waiting for additional text or timeout.
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of the flush operation
    async fn flush(&self) -> TTSResult<()>;

    /// Register an audio callback
    ///
    /// This method registers a callback that will be triggered when the TTS provider
    /// responds with audio data.
    ///
    /// # Arguments
    /// * `callback` - The callback to register for audio events
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of callback registration
    fn on_audio(&mut self, callback: Arc<dyn AudioCallback>) -> TTSResult<()> {
        if let Some(provider) = self.get_provider() {
            provider.generic_on_audio(callback)
        } else {
            Err(TTSError::InternalError(
                "Provider not available for audio callback".to_string(),
            ))
        }
    }

    /// Remove the registered audio callback
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of callback removal
    fn remove_audio_callback(&mut self) -> TTSResult<()> {
        if let Some(provider) = self.get_provider() {
            provider.generic_remove_audio_callback()
        } else {
            Err(TTSError::InternalError(
                "Provider not available for callback removal".to_string(),
            ))
        }
    }

    /// Get provider-specific information
    ///
    /// # Returns
    /// * `serde_json::Value` - Provider-specific information (e.g., supported voices, formats)
    fn get_provider_info(&self) -> serde_json::Value {
        serde_json::json!({
            "provider": "unknown",
            "version": "1.0.0"
        })
    }
}

/// Helper function to create a boxed TTS trait object
pub type BoxedTTS = Box<dyn BaseTTS>;

/// TTS factory trait for creating provider-specific implementations
pub trait TTSFactory: Send + Sync {
    /// Create a new TTS provider instance
    ///
    /// # Arguments
    /// * `provider_type` - The type of TTS provider to create
    /// * `config` - Configuration for the TTS provider
    ///
    /// # Returns
    /// * `TTSResult<BoxedTTS>` - A boxed TTS provider instance
    fn create_tts(&self, provider_type: &str, config: TTSConfig) -> TTSResult<BoxedTTS>;

    /// Get list of supported TTS providers
    ///
    /// # Returns
    /// * `Vec<String>` - List of supported provider names
    fn get_supported_providers(&self) -> Vec<String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::time::{Duration, sleep};

    struct MockTTS {
        state: ConnectionState,
        config: TTSConfig,
        callback: Option<Arc<dyn AudioCallback>>,
    }

    #[async_trait]
    impl BaseTTS for MockTTS {
        fn new(config: TTSConfig) -> TTSResult<Self> {
            Ok(Self {
                state: ConnectionState::Disconnected,
                config,
                callback: None,
            })
        }

        async fn connect(&mut self) -> TTSResult<()> {
            self.state = ConnectionState::Connecting;
            self.config
                .audio_format
                .as_ref()
                .expect("Audio format is required");
            sleep(Duration::from_millis(100)).await;
            self.state = ConnectionState::Connected;
            Ok(())
        }

        async fn disconnect(&mut self) -> TTSResult<()> {
            self.state = ConnectionState::Disconnected;
            self.callback = None;
            Ok(())
        }

        fn is_ready(&self) -> bool {
            matches!(self.state, ConnectionState::Connected)
        }

        fn get_connection_state(&self) -> ConnectionState {
            self.state.clone()
        }

        async fn speak(&mut self, text: &str, _flush: bool) -> TTSResult<()> {
            if !self.is_ready() {
                return Err(TTSError::ProviderNotReady(
                    "TTS provider not connected".to_string(),
                ));
            }

            if let Some(callback) = &self.callback {
                let audio_data = AudioData {
                    data: format!("Generated audio for: {text}").into_bytes(),
                    sample_rate: 22050,
                    format: "pcm".to_string(),
                    duration_ms: Some(1000),
                };
                callback.on_audio(audio_data).await;
            }

            Ok(())
        }

        async fn clear(&mut self) -> TTSResult<()> {
            if !self.is_ready() {
                return Err(TTSError::ProviderNotReady(
                    "TTS provider not connected".to_string(),
                ));
            }
            Ok(())
        }

        async fn flush(&self) -> TTSResult<()> {
            if !self.is_ready() {
                return Err(TTSError::ProviderNotReady(
                    "TTS provider not connected".to_string(),
                ));
            }
            Ok(())
        }

        fn on_audio(&mut self, callback: Arc<dyn AudioCallback>) -> TTSResult<()> {
            self.callback = Some(callback);
            Ok(())
        }

        fn remove_audio_callback(&mut self) -> TTSResult<()> {
            self.callback = None;
            Ok(())
        }

        fn get_provider_info(&self) -> serde_json::Value {
            serde_json::json!({
                "provider": "mock",
                "version": "1.0.0",
                "supported_formats": ["pcm", "wav"]
            })
        }
    }

    #[tokio::test]
    async fn test_tts_connection_lifecycle() {
        let config = TTSConfig::default();
        let mut tts = MockTTS::new(config).unwrap();

        // Initially disconnected
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);

        // Connect
        tts.connect().await.unwrap();
        assert!(tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Connected);

        // Disconnect
        tts.disconnect().await.unwrap();
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_tts_error_when_not_ready() {
        let config = TTSConfig::default();
        let mut tts = MockTTS::new(config).unwrap();

        // Should fail when not connected
        let result = tts.speak("Hello", false).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TTSError::ProviderNotReady(_)));
    }
}
