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
//! - **Speech Final Timing Control**: Automatic fallback mechanism for delayed `is_speech_final` signals in noisy environments
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
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant};

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

/// Internal state for managing speech final timing
#[derive(Debug)]
struct SpeechFinalState {
    /// Combined text buffer from STT results
    text_buffer: String,
    /// Last seen text for comparison
    last_text: String,
    /// Timer handle for speech final timeout
    timer_handle: Option<JoinHandle<()>>,
    /// Timestamp when we last saw is_final=true without speech_final
    last_final_time: Option<Instant>,
    /// Whether we're currently waiting for speech_final
    waiting_for_speech_final: bool,
}

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

    // Speech final timing control
    speech_final_state: Arc<RwLock<SpeechFinalState>>,

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
            speech_final_state: Arc::new(RwLock::new(SpeechFinalState {
                text_buffer: String::new(),
                last_text: String::new(),
                timer_handle: None,
                last_final_time: None,
                waiting_for_speech_final: false,
            })),
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
        // Cancel any pending speech final timer
        {
            let mut state = self.speech_final_state.write().await;
            if let Some(handle) = state.timer_handle.take() {
                handle.abort();
            }
            // Reset speech final state
            state.text_buffer.clear();
            state.last_text.clear();
            state.waiting_for_speech_final = false;
            state.last_final_time = None;
        }

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
        let mut tts = self.tts.write().await;
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

        // Create wrapper callback that processes timing before forwarding to user
        let speech_final_state_clone = self.speech_final_state.clone();
        let stt_callback_clone = self.stt_callback.clone();

        let wrapper_callback: STTResultCallback = Arc::new(move |result| {
            let callback = callback.clone();
            let speech_final_state = speech_final_state_clone.clone();
            let stt_callback_for_timer = stt_callback_clone.clone();

            Box::pin(async move {
                // Process result with timing control inline
                let processed_result = Self::process_stt_result_with_timing_static(
                    result,
                    speech_final_state,
                    stt_callback_for_timer,
                )
                .await;

                if let Some(processed_result) = processed_result {
                    // Call user callback with processed result
                    callback(processed_result).await;
                }
                // If None returned, result was suppressed (interim result while waiting)
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

    /// Static method to process STT results with speech final timing control
    ///
    /// This method implements the following behavior:
    /// - Buffers text results until `is_speech_final=true` is received
    /// - Starts a 1.3s timer when `is_final=true` but no `is_speech_final=true` and text hasn't changed
    /// - Forces `is_speech_final=true` with buffered text if timer expires
    /// - Cancels timer if real `is_speech_final=true` arrives or text changes
    /// - **Important**: `is_speech_final=true` results always contain the full buffered text, then buffer is cleared
    async fn process_stt_result_with_timing_static(
        result: STTResult,
        speech_final_state: Arc<RwLock<SpeechFinalState>>,
        stt_callback: Arc<RwLock<Option<STTCallback>>>,
    ) -> Option<STTResult> {
        let mut state = speech_final_state.write().await;

        // If we get is_speech_final=true, cancel any timer and forward the buffered result
        if result.is_speech_final {
            // Cancel any existing timer
            if let Some(handle) = state.timer_handle.take() {
                handle.abort();
            }

            // Create result with full buffered text (if any), otherwise use original
            let final_transcript = if !state.text_buffer.is_empty() {
                state.text_buffer.clone()
            } else {
                result.transcript.clone()
            };

            // Reset state for next speech segment
            state.text_buffer.clear();
            state.last_text.clear();
            state.waiting_for_speech_final = false;
            state.last_final_time = None;

            // Return result with buffered text
            let final_result = STTResult::new(
                final_transcript,
                result.is_final,
                true, // is_speech_final
                result.confidence,
            );

            return Some(final_result);
        }

        // Update text buffer
        if !result.transcript.trim().is_empty() {
            if result.is_final {
                // Append final text to buffer
                if !state.text_buffer.is_empty() && !state.text_buffer.ends_with(' ') {
                    state.text_buffer.push(' ');
                }
                state.text_buffer.push_str(&result.transcript);
            } else {
                // For interim results, we don't add to buffer but still process
            }
        }

        // Check if we should start/update timer logic
        if result.is_final && !result.is_speech_final {
            let current_text = state.text_buffer.clone();

            // Check if text has changed since last time
            let text_changed = current_text != state.last_text;

            if text_changed {
                // Text changed, cancel existing timer if any
                if let Some(handle) = state.timer_handle.take() {
                    handle.abort();
                }
                state.waiting_for_speech_final = false;
            }

            // Update last seen text
            state.last_text = current_text.clone();

            if !state.waiting_for_speech_final && !current_text.trim().is_empty() {
                // Start timer for speech final timeout
                state.waiting_for_speech_final = true;
                state.last_final_time = Some(Instant::now());

                let speech_final_state_clone = speech_final_state.clone();
                let stt_callback_clone = stt_callback.clone();
                let buffered_text = current_text.clone();
                let result_confidence = result.confidence; // Capture confidence for timer closure

                let timer_handle = tokio::spawn(async move {
                    // Wait for 1.3 seconds
                    tokio::time::sleep(Duration::from_millis(1300)).await;

                    // Check if we should still fire the timeout
                    let mut state = speech_final_state_clone.write().await;

                    // Only fire timeout if we're still waiting and text hasn't changed
                    if state.waiting_for_speech_final && state.text_buffer == buffered_text {
                        // Create forced speech final result
                        let forced_result = STTResult::new(
                            buffered_text.clone(),
                            true,
                            true, // Force is_speech_final
                            result_confidence,
                        );

                        // Reset state
                        state.text_buffer.clear();
                        state.last_text.clear();
                        state.waiting_for_speech_final = false;
                        state.last_final_time = None;
                        state.timer_handle = None;

                        // Forward to user callback
                        if let Some(callback) = stt_callback_clone.read().await.as_ref() {
                            callback(forced_result).await;
                        }
                    } else {
                        // Timer was cancelled or text changed, just clean up
                        state.timer_handle = None;
                    }
                });

                state.timer_handle = Some(timer_handle);
            }
        }

        // Always forward the original result (unless it would be redundant)
        // We only suppress interim results when we're building up the buffer
        if result.is_final || !state.waiting_for_speech_final {
            Some(result)
        } else {
            // Suppress interim results while waiting for speech final
            None
        }
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

    #[tokio::test]
    async fn test_speech_final_timing_control() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::sync::mpsc;

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

        // Channel to collect results
        let (tx, _rx) = mpsc::unbounded_channel();
        let result_counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = result_counter.clone();

        // Register callback to collect results
        voice_manager
            .on_stt_result(move |result| {
                let tx = tx.clone();
                let counter = counter_clone.clone();
                Box::pin(async move {
                    counter.fetch_add(1, Ordering::Relaxed);
                    let _ = tx.send(result);
                })
            })
            .await
            .unwrap();

        // Test Case 1: Normal speech final behavior
        {
            let speech_final_state = voice_manager.speech_final_state.clone();
            let stt_callback = voice_manager.stt_callback.clone();

            // Send is_final=true without is_speech_final=true
            let result1 = STTResult::new("Hello".to_string(), true, false, 0.9);
            let processed = VoiceManager::process_stt_result_with_timing_static(
                result1,
                speech_final_state.clone(),
                stt_callback.clone(),
            )
            .await;

            assert!(processed.is_some());
            let processed_result = processed.unwrap();
            assert_eq!(processed_result.transcript, "Hello");
            assert!(processed_result.is_final);
            assert!(!processed_result.is_speech_final); // Should still be false, timer started
        }

        // Test Case 2: Timer should fire after 1.3 seconds if no speech_final received
        {
            let speech_final_state = voice_manager.speech_final_state.clone();
            let stt_callback = voice_manager.stt_callback.clone();

            // Send is_final=true without is_speech_final=true
            let result1 = STTResult::new("Test message".to_string(), true, false, 0.8);
            let _processed = VoiceManager::process_stt_result_with_timing_static(
                result1,
                speech_final_state.clone(),
                stt_callback.clone(),
            )
            .await;

            // Wait slightly longer than the timer (1.3s + buffer)
            tokio::time::sleep(tokio::time::Duration::from_millis(1400)).await;

            // Check if timer fired - the callback should have been called
            // Note: In a real test environment, we'd collect the callback results
            // For now, we just verify the state was reset
            let state = speech_final_state.read().await;
            assert!(!state.waiting_for_speech_final); // Should be reset after timer fires
        }

        // Test Case 3: Real speech_final should cancel timer and return buffered text
        {
            let speech_final_state = voice_manager.speech_final_state.clone();
            let stt_callback = voice_manager.stt_callback.clone();

            // Reset state first
            {
                let mut state = speech_final_state.write().await;
                *state = SpeechFinalState {
                    text_buffer: String::new(),
                    last_text: String::new(),
                    timer_handle: None,
                    last_final_time: None,
                    waiting_for_speech_final: false,
                };
            }

            // Send multiple is_final=true results to build up buffer
            let result1 = STTResult::new("Hello".to_string(), true, false, 0.9);
            let _processed1 = VoiceManager::process_stt_result_with_timing_static(
                result1,
                speech_final_state.clone(),
                stt_callback.clone(),
            )
            .await;

            let result2 = STTResult::new("world".to_string(), true, false, 0.8);
            let _processed2 = VoiceManager::process_stt_result_with_timing_static(
                result2,
                speech_final_state.clone(),
                stt_callback.clone(),
            )
            .await;

            // Send is_speech_final=true (should cancel timer and return full buffered text)
            let result3 = STTResult::new("final".to_string(), true, true, 0.95);
            let processed3 = VoiceManager::process_stt_result_with_timing_static(
                result3,
                speech_final_state.clone(),
                stt_callback.clone(),
            )
            .await;

            assert!(processed3.is_some());
            let final_result = processed3.unwrap();
            assert!(final_result.is_speech_final);
            assert!(final_result.is_final);
            // Should contain the full buffered text, not just the current result
            assert_eq!(final_result.transcript, "Hello world");
            assert_eq!(final_result.confidence, 0.95);

            // State should be reset
            let state = speech_final_state.read().await;
            assert!(!state.waiting_for_speech_final);
            assert!(state.text_buffer.is_empty());
            assert!(state.last_text.is_empty());
        }

        // Test Case 4: is_speech_final with empty buffer should use original text
        {
            let speech_final_state = voice_manager.speech_final_state.clone();
            let stt_callback = voice_manager.stt_callback.clone();

            // Reset state first
            {
                let mut state = speech_final_state.write().await;
                *state = SpeechFinalState {
                    text_buffer: String::new(),
                    last_text: String::new(),
                    timer_handle: None,
                    last_final_time: None,
                    waiting_for_speech_final: false,
                };
            }

            // Send is_speech_final=true with no buffered content (direct speech final)
            let result = STTResult::new("Direct speech final".to_string(), true, true, 0.85);
            let processed = VoiceManager::process_stt_result_with_timing_static(
                result,
                speech_final_state.clone(),
                stt_callback.clone(),
            )
            .await;

            assert!(processed.is_some());
            let final_result = processed.unwrap();
            assert!(final_result.is_speech_final);
            assert!(final_result.is_final);
            // Should use original text since buffer was empty
            assert_eq!(final_result.transcript, "Direct speech final");
            assert_eq!(final_result.confidence, 0.85);
        }
    }
}
