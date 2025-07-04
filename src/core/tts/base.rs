//! # TTS Base Trait Implementation
//!
//! This module provides the base trait abstraction for Text-to-Speech (TTS) providers.
//! It allows for a unified interface to interact with different TTS services like
//! Deepgram, ElevenLabs, Google TTS, and other providers.
//!
//! ## Usage Example
//!
//! ```rust
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
//!     // Create your TTS provider (e.g., DeepgramTTS, ElevenLabsTTS, etc.)
//!     let mut tts_provider = create_tts_provider("deepgram")?;
//!     
//!     // Configure the provider
//!     let config = TTSConfig {
//!         voice_id: Some("aura-luna-en".to_string()),
//!         speaking_rate: Some(1.0),
//!         audio_format: Some("pcm".to_string()),
//!         sample_rate: Some(22050),
//!         ..Default::default()
//!     };
//!     
//!     // Connect to the provider
//!     tts_provider.connect(config).await?;
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
//! // Factory function example (would be implemented by your provider)
//! fn create_tts_provider(provider_type: &str) -> Result<Box<dyn BaseTTS>, TTSError> {
//!     match provider_type {
//!         "deepgram" => {
//!             // Return DeepgramTTS implementation
//!             todo!("Implement DeepgramTTS")
//!         },
//!         "elevenlabs" => {
//!             // Return ElevenLabsTTS implementation
//!             todo!("Implement ElevenLabsTTS")
//!         },
//!         _ => Err(TTSError::InvalidConfiguration(
//!             format!("Unsupported TTS provider: {}", provider_type)
//!         ))
//!     }
//! }
//! ```

use async_trait::async_trait;
use futures::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

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
#[derive(Debug, thiserror::Error)]
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

/// Configuration for TTS providers
#[derive(Debug, Clone)]
pub struct TTSConfig {
    /// Provider-specific configuration (e.g., API key, endpoint URL)
    pub provider_config: serde_json::Value,
    /// Voice ID or name to use for synthesis
    pub voice_id: Option<String>,
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
}

impl Default for TTSConfig {
    fn default() -> Self {
        Self {
            provider_config: serde_json::Value::Object(serde_json::Map::new()),
            voice_id: None,
            speaking_rate: Some(1.0),
            audio_format: Some("pcm".to_string()),
            sample_rate: Some(22050),
            connection_timeout: Some(30),
            request_timeout: Some(60),
        }
    }
}

/// Base trait for Text-to-Speech providers
#[async_trait]
pub trait BaseTTS: Send + Sync {
    /// Connect to the TTS provider
    ///
    /// This method establishes a connection to the TTS provider (e.g., Deepgram, ElevenLabs)
    /// and prepares it for text synthesis requests.
    ///
    /// # Arguments
    /// * `config` - Configuration for the TTS provider
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of the connection attempt
    async fn connect(&mut self, config: TTSConfig) -> TTSResult<()>;

    /// Disconnect from the TTS provider
    ///
    /// This method cleanly closes the connection to the TTS provider and releases any resources.
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of the disconnection attempt
    async fn disconnect(&mut self) -> TTSResult<()>;

    /// Check if the TTS provider is ready to process requests
    ///
    /// This method indicates whether the connection is established and ready to be used
    /// for text synthesis.
    ///
    /// # Returns
    /// * `bool` - True if ready, false otherwise
    fn is_ready(&self) -> bool;

    /// Get the current connection state
    ///
    /// # Returns
    /// * `ConnectionState` - Current state of the connection
    fn get_connection_state(&self) -> ConnectionState;

    /// Send text to the TTS provider for synthesis
    ///
    /// This method sends text to the target TTS provider for conversion to speech.
    /// The audio will be delivered via the registered audio callback.
    ///
    /// # Arguments
    /// * `text` - The text to synthesize
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of the synthesis request
    async fn speak(&self, text: &str) -> TTSResult<()>;

    /// Clear any queued text from the TTS provider
    ///
    /// This method sends a clear command to the target provider to remove any
    /// pending text from the synthesis queue.
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of the clear operation
    async fn clear(&self) -> TTSResult<()>;

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
    fn on_audio(&mut self, callback: Arc<dyn AudioCallback>) -> TTSResult<()>;

    /// Remove the registered audio callback
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of callback removal
    fn remove_audio_callback(&mut self) -> TTSResult<()>;

    /// Get provider-specific information
    ///
    /// # Returns
    /// * `serde_json::Value` - Provider-specific information (e.g., supported voices, formats)
    fn get_provider_info(&self) -> serde_json::Value;

    /// Set provider-specific options
    ///
    /// # Arguments
    /// * `options` - Provider-specific options to set
    ///
    /// # Returns
    /// * `TTSResult<()>` - Success or failure of setting options
    async fn set_options(&mut self, options: serde_json::Value) -> TTSResult<()>;
}

/// Channel-based audio callback implementation
pub struct ChannelAudioCallback {
    audio_sender: mpsc::UnboundedSender<AudioData>,
    error_sender: mpsc::UnboundedSender<TTSError>,
    complete_sender: mpsc::UnboundedSender<()>,
}

impl ChannelAudioCallback {
    /// Create a new channel-based audio callback
    ///
    /// # Returns
    /// * `(Self, Receivers)` - The callback and receiver channels
    pub fn new() -> (
        Self,
        mpsc::UnboundedReceiver<AudioData>,
        mpsc::UnboundedReceiver<TTSError>,
        mpsc::UnboundedReceiver<()>,
    ) {
        let (audio_sender, audio_receiver) = mpsc::unbounded_channel();
        let (error_sender, error_receiver) = mpsc::unbounded_channel();
        let (complete_sender, complete_receiver) = mpsc::unbounded_channel();

        (
            Self {
                audio_sender,
                error_sender,
                complete_sender,
            },
            audio_receiver,
            error_receiver,
            complete_receiver,
        )
    }
}

impl AudioCallback for ChannelAudioCallback {
    fn on_audio(&self, audio_data: AudioData) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if let Err(e) = self.audio_sender.send(audio_data) {
                tracing::error!("Failed to send audio data: {}", e);
            }
        })
    }

    fn on_error(&self, error: TTSError) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if let Err(e) = self.error_sender.send(error) {
                tracing::error!("Failed to send error: {}", e);
            }
        })
    }

    fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if let Err(e) = self.complete_sender.send(()) {
                tracing::error!("Failed to send completion signal: {}", e);
            }
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
    ///
    /// # Returns
    /// * `TTSResult<BoxedTTS>` - A boxed TTS provider instance
    fn create_tts(&self, provider_type: &str) -> TTSResult<BoxedTTS>;

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
        config: Option<TTSConfig>,
        callback: Option<Arc<dyn AudioCallback>>,
    }

    impl MockTTS {
        fn new() -> Self {
            Self {
                state: ConnectionState::Disconnected,
                config: None,
                callback: None,
            }
        }
    }

    #[async_trait]
    impl BaseTTS for MockTTS {
        async fn connect(&mut self, config: TTSConfig) -> TTSResult<()> {
            self.state = ConnectionState::Connecting;
            sleep(Duration::from_millis(100)).await;
            self.config = Some(config);
            self.state = ConnectionState::Connected;
            Ok(())
        }

        async fn disconnect(&mut self) -> TTSResult<()> {
            self.state = ConnectionState::Disconnected;
            self.config = None;
            self.callback = None;
            Ok(())
        }

        fn is_ready(&self) -> bool {
            matches!(self.state, ConnectionState::Connected)
        }

        fn get_connection_state(&self) -> ConnectionState {
            self.state.clone()
        }

        async fn speak(&self, text: &str) -> TTSResult<()> {
            if !self.is_ready() {
                return Err(TTSError::ProviderNotReady(
                    "TTS provider not connected".to_string(),
                ));
            }

            if let Some(callback) = &self.callback {
                let audio_data = AudioData {
                    data: format!("Generated audio for: {}", text).into_bytes(),
                    sample_rate: 22050,
                    format: "pcm".to_string(),
                    duration_ms: Some(1000),
                };
                callback.on_audio(audio_data).await;
            }

            Ok(())
        }

        async fn clear(&self) -> TTSResult<()> {
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

        async fn set_options(&mut self, _options: serde_json::Value) -> TTSResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_tts_connection_lifecycle() {
        let mut tts = MockTTS::new();

        // Initially disconnected
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);

        // Connect
        let config = TTSConfig::default();
        tts.connect(config).await.unwrap();
        assert!(tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Connected);

        // Disconnect
        tts.disconnect().await.unwrap();
        assert!(!tts.is_ready());
        assert_eq!(tts.get_connection_state(), ConnectionState::Disconnected);
    }

    #[tokio::test]
    async fn test_tts_audio_callback() {
        let mut tts = MockTTS::new();
        let (callback, mut audio_receiver, _error_receiver, _complete_receiver) =
            ChannelAudioCallback::new();

        tts.connect(TTSConfig::default()).await.unwrap();
        tts.on_audio(Arc::new(callback)).unwrap();

        tts.speak("Hello, world!").await.unwrap();

        // Check that audio data was received
        let audio_data = audio_receiver.recv().await.unwrap();
        assert_eq!(audio_data.format, "pcm");
        assert_eq!(audio_data.sample_rate, 22050);
        assert!(audio_data.data.len() > 0);
    }

    #[tokio::test]
    async fn test_tts_error_when_not_ready() {
        let tts = MockTTS::new();

        // Should fail when not connected
        let result = tts.speak("Hello").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TTSError::ProviderNotReady(_)));
    }
}
