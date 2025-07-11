//! # Voice Manager
//!
//! This module provides a unified interface for managing both Speech-to-Text (STT) and
//! Text-to-Speech (TTS) providers in a single, coordinated system. The VoiceManager
//! abstracts away the complexity of managing multiple providers and provides a clean
//! API for real-time voice processing.
//!
//! ## Features
//!
//! - **Unified Management**: Coordinate STT and TTS providers through a single interface
//! - **Real-time Processing**: Optimized for low-latency voice processing
//! - **Error Handling**: Comprehensive error handling with proper error propagation
//! - **Callback System**: Event-driven architecture for handling results
//! - **Thread Safety**: Safe concurrent access using Arc<RwLock<>>
//! - **Provider Abstraction**: Support for multiple STT and TTS providers
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
//! use sayna::core::stt::STTConfig;
//! use sayna::core::tts::TTSConfig;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Configure STT and TTS providers
//!     let config = VoiceManagerConfig {
//!         stt_config: STTConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-stt-api-key".to_string(),
//!             language: "en-US".to_string(),
//!             sample_rate: 16000,
//!             channels: 1,
//!             punctuation: true,
//!             encoding: "linear16".to_string(),
//!             model: "nova-3".to_string(),
//!         },
//!         tts_config: TTSConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-tts-api-key".to_string(),
//!             voice_id: Some("aura-luna-en".to_string()),
//!             speaking_rate: Some(1.0),
//!             audio_format: Some("pcm".to_string()),
//!             sample_rate: Some(22050),
//!             ..Default::default()
//!         },
//!     };
//!
//!     // Create and start the voice manager
//!     let voice_manager = VoiceManager::new(config)?;
//!     voice_manager.start().await?;
//!
//!     // Wait for both providers to be ready
//!     while !voice_manager.is_ready().await {
//!         tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
//!     }
//!
//!     // Register callbacks for STT results
//!     voice_manager.on_stt_result(|result| {
//!         Box::pin(async move {
//!             println!("STT Result: {} (confidence: {:.2})", result.transcript, result.confidence);
//!             if result.is_final {
//!                 println!("Final transcription: {}", result.transcript);
//!             }
//!         })
//!     }).await?;
//!
//!     // Register callbacks for TTS audio
//!     voice_manager.on_tts_audio(|audio_data| {
//!         Box::pin(async move {
//!             println!("Received {} bytes of audio in {} format",
//!                      audio_data.data.len(), audio_data.format);
//!             // Process audio data here (e.g., play through speakers)
//!         })
//!     }).await?;
//!
//!     // Register callbacks for TTS errors
//!     voice_manager.on_tts_error(|error| {
//!         Box::pin(async move {
//!             eprintln!("TTS Error: {}", error);
//!         })
//!     }).await?;
//!
//!     // Example: Send audio for transcription
//!     let audio_data = vec![0u8; 1024]; // Your audio data here
//!     voice_manager.receive_audio(audio_data).await?;
//!
//!     // Example: Synthesize speech
//!     voice_manager.speak("Hello, this is a test message", true).await?;
//!     voice_manager.flush_tts().await?; // Ensure immediate processing
//!
//!     // Example: Multiple speech requests
//!     voice_manager.speak("First message.", false).await?;
//!     voice_manager.speak("Second message.", false).await?;
//!     voice_manager.speak("Third message.", true).await?;
//!
//!     // Clear any pending TTS requests
//!     voice_manager.clear_tts().await?;
//!
//!     // Check individual provider status
//!     println!("STT ready: {}", voice_manager.is_stt_ready().await);
//!     println!("TTS ready: {}", voice_manager.is_tts_ready().await);
//!
//!     // Get provider information
//!     println!("STT provider: {}", voice_manager.get_stt_provider_info().await);
//!     println!("TTS provider: {}", voice_manager.get_tts_provider_info().await);
//!
//!     // Stop the voice manager
//!     voice_manager.stop().await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Advanced Usage
//!
//! ### Real-time Voice Processing
//!
//! ```rust,no_run
//! use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
//! use sayna::core::stt::STTConfig;
//! use sayna::core::tts::TTSConfig;
//! use tokio::sync::mpsc;
//! use std::sync::Arc;
//!
//! async fn realtime_voice_processing() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = VoiceManagerConfig {
//!         stt_config: STTConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-stt-api-key".to_string(),
//!             ..Default::default()
//!         },
//!         tts_config: TTSConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-tts-api-key".to_string(),
//!             ..Default::default()
//!         },
//!     };
//!     let voice_manager = Arc::new(VoiceManager::new(config)?);
//!     let vm = voice_manager.clone();
//!     
//!     // Start the voice manager
//!     vm.start().await?;
//!
//!     // Channel for audio input
//!     let (audio_tx, mut audio_rx) = mpsc::unbounded_channel::<Vec<u8>>();
//!
//!     // Set up STT callback for real-time transcription
//!     let vm_clone = vm.clone();
//!     vm.on_stt_result(move |result| {
//!         let vm = vm_clone.clone();
//!         Box::pin(async move {
//!             if result.is_final && !result.transcript.trim().is_empty() {
//!                 // Echo the transcription back as speech
//!                 let response = format!("You said: {}", result.transcript);
//!                 if let Err(e) = vm.speak(&response, true).await {
//!                     eprintln!("Failed to speak response: {}", e);
//!                 }
//!             }
//!         })
//!     }).await?;
//!
//!     // Audio processing loop
//!     tokio::spawn(async move {
//!         while let Some(audio_data) = audio_rx.recv().await {
//!             if let Err(e) = voice_manager.receive_audio(audio_data).await {
//!                 eprintln!("Failed to process audio: {}", e);
//!             }
//!         }
//!     });
//!
//!     // Simulate audio input (in real app, this would come from microphone)
//!     for i in 0..10 {
//!         let audio_chunk = vec![0u8; 1024]; // Mock audio data
//!         audio_tx.send(audio_chunk)?;
//!         tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Error Handling and Recovery
//!
//! ```rust,no_run
//! use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
//! use sayna::core::stt::STTConfig;
//! use sayna::core::tts::TTSConfig;
//!
//! async fn robust_voice_processing() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = VoiceManagerConfig {
//!         stt_config: STTConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-stt-api-key".to_string(),
//!             ..Default::default()
//!         },
//!         tts_config: TTSConfig {
//!             provider: "deepgram".to_string(),
//!             api_key: "your-tts-api-key".to_string(),
//!             ..Default::default()
//!         },
//!     };
//!     let voice_manager = VoiceManager::new(config)?;
//!     
//!     // Start with error handling
//!     match voice_manager.start().await {
//!         Ok(_) => println!("Voice manager started successfully"),
//!         Err(e) => {
//!             eprintln!("Failed to start voice manager: {}", e);
//!             return Err(e.into());
//!         }
//!     }
//!
//!     // Set up error callback for TTS
//!     voice_manager.on_tts_error(|error| {
//!         Box::pin(async move {
//!             eprintln!("TTS Error occurred: {}", error);
//!             // Implement recovery logic here
//!         })
//!     }).await?;
//!
//!     // Check provider readiness with timeout
//!     let timeout = tokio::time::Duration::from_secs(30);
//!     let start_time = tokio::time::Instant::now();
//!     
//!     while !voice_manager.is_ready().await {
//!         if start_time.elapsed() > timeout {
//!             return Err("Timeout waiting for providers to be ready".into());
//!         }
//!         tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
//!     }
//!
//!     // Process with error recovery
//!     match voice_manager.speak("Test message", true).await {
//!         Ok(_) => println!("Speech synthesis successful"),
//!         Err(e) => {
//!             eprintln!("Speech synthesis failed: {}", e);
//!             // Implement retry logic or fallback
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::core::{
    create_stt_provider, create_tts_provider,
    stt::{BaseSTT, STTConfig, STTError, STTResult, STTResultCallback},
    tts::{AudioCallback, AudioData, BaseTTS, TTSConfig, TTSError},
};

/// Configuration for the VoiceManager
#[derive(Debug, Clone)]
pub struct VoiceManagerConfig {
    /// Configuration for the STT provider
    pub stt_config: STTConfig,
    /// Configuration for the TTS provider
    pub tts_config: TTSConfig,
}

/// Error types for VoiceManager operations
#[derive(Debug, thiserror::Error)]
pub enum VoiceManagerError {
    #[error("TTS error: {0}")]
    TTSError(#[from] TTSError),
    #[error("STT error: {0}")]
    STTError(#[from] STTError),
    #[error("Initialization error: {0}")]
    InitializationError(String),
    #[error("Provider not ready: {0}")]
    ProviderNotReady(String),
    #[error("Callback registration error: {0}")]
    CallbackRegistrationError(String),
    #[error("Internal error: {0}")]
    InternalError(String),
}

/// Result type for VoiceManager operations
pub type VoiceManagerResult<T> = Result<T, VoiceManagerError>;

/// Callback type for STT results
pub type STTCallback =
    Arc<dyn Fn(STTResult) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Callback type for TTS audio data
pub type TTSAudioCallback =
    Arc<dyn Fn(AudioData) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Callback type for TTS errors
pub type TTSErrorCallback =
    Arc<dyn Fn(TTSError) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Internal TTS callback implementation for the VoiceManager
struct VoiceManagerTTSCallback {
    audio_callback: Option<TTSAudioCallback>,
    error_callback: Option<TTSErrorCallback>,
}

impl AudioCallback for VoiceManagerTTSCallback {
    fn on_audio(&self, audio_data: AudioData) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let callback = self.audio_callback.clone();

        Box::pin(async move {
            // Call user callback if registered
            if let Some(callback) = callback {
                callback(audio_data).await;
            }
        })
    }

    fn on_error(&self, error: TTSError) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let callback = self.error_callback.clone();

        Box::pin(async move {
            if let Some(callback) = callback {
                callback(error).await;
            }
        })
    }

    fn on_complete(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            // Handle completion if needed
        })
    }
}

/// VoiceManager provides a unified interface for managing STT and TTS providers
pub struct VoiceManager {
    tts: Arc<RwLock<Box<dyn BaseTTS>>>,
    stt: Arc<RwLock<Box<dyn BaseSTT>>>,

    // Callbacks
    stt_callback: Arc<RwLock<Option<STTCallback>>>,
    tts_audio_callback: Arc<RwLock<Option<TTSAudioCallback>>>,
    tts_error_callback: Arc<RwLock<Option<TTSErrorCallback>>>,

    // Configuration
    config: VoiceManagerConfig,
}

impl VoiceManager {
    /// Create a new VoiceManager with the given configuration
    ///
    /// # Arguments
    /// * `config` - Configuration for both STT and TTS providers
    ///
    /// # Returns
    /// * `VoiceManagerResult<Self>` - A new VoiceManager instance or error
    ///
    /// # Example
    /// ```rust,no_run
    /// use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// use sayna::core::stt::STTConfig;
    /// use sayna::core::tts::TTSConfig;
    ///
    /// fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = VoiceManagerConfig {
    ///         stt_config: STTConfig {
    ///             provider: "deepgram".to_string(),
    ///             api_key: "your-api-key".to_string(),
    ///             ..Default::default()
    ///         },
    ///         tts_config: TTSConfig {
    ///             provider: "deepgram".to_string(),
    ///             api_key: "your-api-key".to_string(),
    ///             ..Default::default()
    ///         },
    ///     };
    ///
    ///     let voice_manager = VoiceManager::new(config)?;
    ///     Ok(())
    /// }
    /// ```
    pub fn new(config: VoiceManagerConfig) -> VoiceManagerResult<Self> {
        let tts = create_tts_provider(&config.tts_config.provider, config.tts_config.clone())
            .map_err(VoiceManagerError::TTSError)?;
        let stt = create_stt_provider(&config.stt_config.provider, config.stt_config.clone())
            .map_err(VoiceManagerError::STTError)?;

        Ok(Self {
            tts: Arc::new(RwLock::new(tts)),
            stt: Arc::new(RwLock::new(stt)),
            stt_callback: Arc::new(RwLock::new(None)),
            tts_audio_callback: Arc::new(RwLock::new(None)),
            tts_error_callback: Arc::new(RwLock::new(None)),
            config,
        })
    }

    /// Start the VoiceManager by connecting to both STT and TTS providers
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = VoiceManagerConfig {
    /// #     stt_config: STTConfig::default(),
    /// #     tts_config: TTSConfig::default(),
    /// # };
    /// # let voice_manager = VoiceManager::new(config)?;
    /// voice_manager.start().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start(&self) -> VoiceManagerResult<()> {
        // Connect STT provider
        {
            let mut stt = self.stt.write().await;
            stt.connect().await.map_err(VoiceManagerError::STTError)?;
        }

        // Connect TTS provider
        {
            let mut tts = self.tts.write().await;
            tts.connect().await.map_err(VoiceManagerError::TTSError)?;
        }

        // Set up internal TTS callback
        {
            let mut tts = self.tts.write().await;
            let tts_callback = Arc::new(VoiceManagerTTSCallback {
                audio_callback: self.tts_audio_callback.read().await.clone(),
                error_callback: self.tts_error_callback.read().await.clone(),
            });

            tts.on_audio(tts_callback)
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Stop the VoiceManager by disconnecting from both providers
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = VoiceManagerConfig {
    /// #     stt_config: STTConfig::default(),
    /// #     tts_config: TTSConfig::default(),
    /// # };
    /// # let voice_manager = VoiceManager::new(config)?;
    /// voice_manager.stop().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop(&self) -> VoiceManagerResult<()> {
        // Disconnect STT provider
        {
            let mut stt = self.stt.write().await;
            stt.disconnect()
                .await
                .map_err(VoiceManagerError::STTError)?;
        }

        // Disconnect TTS provider
        {
            let mut tts = self.tts.write().await;
            tts.disconnect()
                .await
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Check if both STT and TTS providers are ready
    ///
    /// # Returns
    /// * `bool` - True if both providers are ready, false otherwise
    ///
    /// # Example
    /// ```rust,no_run
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = VoiceManagerConfig {
    /// #     stt_config: STTConfig::default(),
    /// #     tts_config: TTSConfig::default(),
    /// # };
    /// # let voice_manager = VoiceManager::new(config)?;
    /// if voice_manager.is_ready().await {
    ///     println!("VoiceManager is ready!");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn is_ready(&self) -> bool {
        let stt_ready = {
            let stt = self.stt.read().await;
            stt.is_ready()
        };

        let tts_ready = {
            let tts = self.tts.read().await;
            tts.is_ready()
        };

        stt_ready && tts_ready
    }

    /// Send audio data to the STT provider for transcription
    ///
    /// # Arguments
    /// * `audio` - Audio bytes to process
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = VoiceManagerConfig {
    /// #     stt_config: STTConfig::default(),
    /// #     tts_config: TTSConfig::default(),
    /// # };
    /// # let voice_manager = VoiceManager::new(config)?;
    /// let audio_data = vec![0u8; 1024]; // Your audio data
    /// voice_manager.receive_audio(audio_data).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receive_audio(&self, audio: Vec<u8>) -> VoiceManagerResult<()> {
        // Send audio to STT provider
        {
            let mut stt = self.stt.write().await;
            stt.send_audio(audio)
                .await
                .map_err(VoiceManagerError::STTError)?;
        }

        Ok(())
    }

    /// Send text to the TTS provider for synthesis
    ///
    /// # Arguments
    /// * `text` - Text to synthesize
    /// * `flush` - Whether to immediately flush and start processing the text
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = VoiceManagerConfig {
    /// #     stt_config: STTConfig::default(),
    /// #     tts_config: TTSConfig::default(),
    /// # };
    /// # let voice_manager = VoiceManager::new(config)?;
    /// // Queue text without immediate processing
    /// voice_manager.speak("Hello, world!", false).await?;
    /// voice_manager.speak("How are you?", false).await?;
    ///
    /// // Send and immediately process
    /// voice_manager.speak("Final message", true).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn speak(&self, text: &str, flush: bool) -> VoiceManagerResult<()> {
        // Send text to TTS provider
        {
            let tts = self.tts.read().await;
            tts.speak(text, flush)
                .await
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Clear any queued text from the TTS provider
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    pub async fn clear_tts(&self) -> VoiceManagerResult<()> {
        let tts = self.tts.read().await;
        tts.clear().await.map_err(VoiceManagerError::TTSError)?;
        Ok(())
    }

    /// Flush the TTS provider to process queued text immediately
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    pub async fn flush_tts(&self) -> VoiceManagerResult<()> {
        let tts = self.tts.read().await;
        tts.flush().await.map_err(VoiceManagerError::TTSError)?;
        Ok(())
    }

    /// Register a callback for STT results
    ///
    /// # Arguments
    /// * `callback` - Callback function to handle STT results
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = VoiceManagerConfig {
    /// #     stt_config: STTConfig::default(),
    /// #     tts_config: TTSConfig::default(),
    /// # };
    /// # let voice_manager = VoiceManager::new(config)?;
    /// voice_manager.on_stt_result(|result| {
    ///     Box::pin(async move {
    ///         println!("Transcription: {}", result.transcript);
    ///     })
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn on_stt_result<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn(STTResult) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        let callback = Arc::new(callback);

        // Store the callback for later use
        {
            let mut stt_callback = self.stt_callback.write().await;
            *stt_callback = Some(callback.clone());
        }

        // Create wrapper callback
        let wrapper_callback: STTResultCallback = Arc::new(move |result| {
            let callback = callback.clone();

            Box::pin(async move {
                // Call user callback
                callback(result).await;
            })
        });

        // Register callback with STT provider
        {
            let mut stt = self.stt.write().await;
            stt.on_result(wrapper_callback)
                .await
                .map_err(VoiceManagerError::STTError)?;
        }

        Ok(())
    }

    /// Register a callback for TTS audio data
    ///
    /// # Arguments
    /// * `callback` - Callback function to handle TTS audio data
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,no_run
    /// # use sayna::core::voice_manager::{VoiceManager, VoiceManagerConfig};
    /// # use sayna::core::stt::STTConfig;
    /// # use sayna::core::tts::TTSConfig;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let config = VoiceManagerConfig {
    /// #     stt_config: STTConfig::default(),
    /// #     tts_config: TTSConfig::default(),
    /// # };
    /// # let voice_manager = VoiceManager::new(config)?;
    /// voice_manager.on_tts_audio(|audio_data| {
    ///     Box::pin(async move {
    ///         println!("Received {} bytes of audio", audio_data.data.len());
    ///     })
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn on_tts_audio<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn(AudioData) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        let mut tts_audio_callback = self.tts_audio_callback.write().await;
        *tts_audio_callback = Some(Arc::new(callback));

        // Update the internal TTS callback
        {
            let mut tts = self.tts.write().await;
            let tts_callback = Arc::new(VoiceManagerTTSCallback {
                audio_callback: tts_audio_callback.clone(),
                error_callback: self.tts_error_callback.read().await.clone(),
            });

            tts.on_audio(tts_callback)
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Register a callback for TTS errors
    ///
    /// # Arguments
    /// * `callback` - Callback function to handle TTS errors
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    pub async fn on_tts_error<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn(TTSError) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        let mut tts_error_callback = self.tts_error_callback.write().await;
        *tts_error_callback = Some(Arc::new(callback));

        // Update the internal TTS callback
        {
            let mut tts = self.tts.write().await;
            let tts_callback = Arc::new(VoiceManagerTTSCallback {
                audio_callback: self.tts_audio_callback.read().await.clone(),
                error_callback: tts_error_callback.clone(),
            });

            tts.on_audio(tts_callback)
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Get the current configuration
    ///
    /// # Returns
    /// * `&VoiceManagerConfig` - Current configuration
    pub fn get_config(&self) -> &VoiceManagerConfig {
        &self.config
    }

    /// Check if STT provider is ready
    ///
    /// # Returns
    /// * `bool` - True if STT provider is ready
    pub async fn is_stt_ready(&self) -> bool {
        let stt = self.stt.read().await;
        stt.is_ready()
    }

    /// Check if TTS provider is ready
    ///
    /// # Returns
    /// * `bool` - True if TTS provider is ready
    pub async fn is_tts_ready(&self) -> bool {
        let tts = self.tts.read().await;
        tts.is_ready()
    }

    /// Get STT provider information
    ///
    /// # Returns
    /// * `&'static str` - STT provider information
    pub async fn get_stt_provider_info(&self) -> &'static str {
        let stt = self.stt.read().await;
        stt.get_provider_info()
    }

    /// Get TTS provider information
    ///
    /// # Returns
    /// * `serde_json::Value` - TTS provider information
    pub async fn get_tts_provider_info(&self) -> serde_json::Value {
        let tts = self.tts.read().await;
        tts.get_provider_info()
    }
}

// Ensure VoiceManager is thread-safe
unsafe impl Send for VoiceManager {}
unsafe impl Sync for VoiceManager {}

#[cfg(test)]
mod tests {
    use super::*;
    // No additional imports needed for current tests

    #[tokio::test]
    async fn test_voice_manager_creation() {
        let config = VoiceManagerConfig {
            stt_config: STTConfig {
                provider: "deepgram".to_string(),
                api_key: "test_key".to_string(),
                ..Default::default()
            },
            tts_config: TTSConfig {
                provider: "deepgram".to_string(),
                api_key: "test_key".to_string(),
                ..Default::default()
            },
        };

        let result = VoiceManager::new(config);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_voice_manager_config_access() {
        let config = VoiceManagerConfig {
            stt_config: STTConfig {
                provider: "deepgram".to_string(),
                api_key: "test_key".to_string(),
                ..Default::default()
            },
            tts_config: TTSConfig {
                provider: "deepgram".to_string(),
                api_key: "test_key".to_string(),
                ..Default::default()
            },
        };

        let voice_manager = VoiceManager::new(config).unwrap();
        let retrieved_config = voice_manager.get_config();

        assert_eq!(retrieved_config.stt_config.provider, "deepgram");
        assert_eq!(retrieved_config.tts_config.provider, "deepgram");
    }

    #[tokio::test]
    async fn test_voice_manager_callback_registration() {
        let config = VoiceManagerConfig {
            stt_config: STTConfig {
                provider: "deepgram".to_string(),
                api_key: "test_key".to_string(),
                ..Default::default()
            },
            tts_config: TTSConfig {
                provider: "deepgram".to_string(),
                api_key: "test_key".to_string(),
                ..Default::default()
            },
        };

        let voice_manager = VoiceManager::new(config).unwrap();

        // Test STT callback registration
        let stt_result = voice_manager
            .on_stt_result(|result| {
                Box::pin(async move {
                    println!("STT Result: {}", result.transcript);
                })
            })
            .await;

        assert!(stt_result.is_ok());

        // Test TTS callback registration
        let tts_result = voice_manager
            .on_tts_audio(|audio_data| {
                Box::pin(async move {
                    println!("TTS Audio: {} bytes", audio_data.data.len());
                })
            })
            .await;

        assert!(tts_result.is_ok());
    }
}
