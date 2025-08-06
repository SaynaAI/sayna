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

use parking_lot::RwLock as SyncRwLock;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use tokio::sync::{Notify, RwLock};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::debug;

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

/// Callback type for audio clear operations (e.g., LiveKit audio buffer clearing)
pub type AudioClearCallback =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Internal state for managing speech final timing
/// Uses parking_lot RwLock for faster synchronization and pre-allocated string buffers
struct SpeechFinalState {
    /// Combined text buffer from STT results - pre-allocated with capacity
    text_buffer: String,
    /// Last seen text for comparison - pre-allocated with capacity
    last_text: String,
    /// Timer handle for speech final timeout
    timer_handle: Option<JoinHandle<()>>,
    /// Whether we're currently waiting for speech_final - atomic for lock-free reads
    waiting_for_speech_final: AtomicBool,
    /// User callback to call when timer expires
    user_callback: Option<STTCallback>,
}

/// State for managing interruption control
/// Uses atomic types for lock-free access in hot paths
struct InterruptionState {
    /// Whether interruptions are currently allowed - atomic for lock-free reads
    allow_interruption: AtomicBool,
    /// Time when the current non-interruptible audio will finish playing
    /// Stored as milliseconds since epoch for atomic access
    non_interruptible_until_ms: AtomicUsize,
    /// Sample rate of the current TTS audio - atomic for lock-free access
    current_sample_rate: AtomicU32,
    /// Accumulated audio duration for the current non-interruptible segment
    accumulated_duration: parking_lot::Mutex<f32>,
    /// Start time of the non-interruptible segment as milliseconds since epoch
    start_time_ms: AtomicUsize,
    /// Whether we're currently clearing audio (blocks audio callbacks) - atomic for lock-free reads
    is_clearing: AtomicBool,
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
/// Optimized for extreme low-latency with lock-free atomics and pre-allocated buffers
pub struct VoiceManager {
    tts: Arc<RwLock<Box<dyn BaseTTS>>>,
    stt: Arc<RwLock<Box<dyn BaseSTT>>>,

    // Callbacks - using parking_lot RwLock for faster synchronization
    stt_callback: Arc<SyncRwLock<Option<STTCallback>>>,
    tts_audio_callback: Arc<SyncRwLock<Option<TTSAudioCallback>>>,
    tts_error_callback: Arc<SyncRwLock<Option<TTSErrorCallback>>>,
    audio_clear_callback: Arc<SyncRwLock<Option<AudioClearCallback>>>,

    // Speech final timing control - using parking_lot for faster access
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,

    // Interruption control - mostly lock-free with atomics
    interruption_state: Arc<InterruptionState>,

    // Configuration
    config: VoiceManagerConfig,

    // Notification for audio clear completion instead of sleep
    clear_notify: Arc<Notify>,
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

        // Pre-allocate string buffers with reasonable capacity
        const TEXT_BUFFER_CAPACITY: usize = 1024;
        let text_buffer = String::with_capacity(TEXT_BUFFER_CAPACITY);
        let last_text = String::with_capacity(TEXT_BUFFER_CAPACITY);

        Ok(Self {
            tts: Arc::new(RwLock::new(tts)),
            stt: Arc::new(RwLock::new(stt)),
            stt_callback: Arc::new(SyncRwLock::new(None)),
            tts_audio_callback: Arc::new(SyncRwLock::new(None)),
            tts_error_callback: Arc::new(SyncRwLock::new(None)),
            audio_clear_callback: Arc::new(SyncRwLock::new(None)),
            speech_final_state: Arc::new(SyncRwLock::new(SpeechFinalState {
                text_buffer,
                last_text,
                timer_handle: None,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
            })),
            interruption_state: Arc::new(InterruptionState {
                allow_interruption: AtomicBool::new(true),
                non_interruptible_until_ms: AtomicUsize::new(0),
                current_sample_rate: AtomicU32::new(24000),
                accumulated_duration: parking_lot::Mutex::new(0.0),
                start_time_ms: AtomicUsize::new(0),
                is_clearing: AtomicBool::new(false),
            }),
            config,
            clear_notify: Arc::new(Notify::new()),
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

        // Set up internal TTS callback - using parking_lot for faster access
        {
            let mut tts = self.tts.write().await;
            let tts_callback = Arc::new(VoiceManagerTTSCallback {
                audio_callback: self.tts_audio_callback.read().clone(),
                error_callback: self.tts_error_callback.read().clone(),
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
            let mut state = self.speech_final_state.write();
            if let Some(handle) = state.timer_handle.take() {
                handle.abort();
            }
            // Reset speech final state - reuse allocated capacity
            state.text_buffer.clear();
            state.last_text.clear();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);
            state.user_callback = None;
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
        let mut stt = self.stt.write().await;
        stt.send_audio(audio)
            .await
            .map_err(VoiceManagerError::STTError)?;
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
            let mut tts = self.tts.write().await;
            tts.speak(text, flush)
                .await
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Send text to the TTS provider with interruption control
    ///
    /// # Arguments
    /// * `text` - Text to synthesize
    /// * `flush` - Whether to immediately flush and start processing the text
    /// * `allow_interruption` - Whether this audio can be interrupted by STT or clear commands
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    pub async fn speak_with_interruption(
        &self,
        text: &str,
        flush: bool,
        allow_interruption: bool,
    ) -> VoiceManagerResult<()> {
        // Update interruption state using atomics for lock-free performance
        if !allow_interruption {
            self.interruption_state
                .allow_interruption
                .store(false, Ordering::Release);
            self.interruption_state
                .is_clearing
                .store(false, Ordering::Release);

            // Update sample rate from TTS config
            if let Some(sample_rate) = self.config.tts_config.sample_rate {
                self.interruption_state
                    .current_sample_rate
                    .store(sample_rate, Ordering::Release);
            }

            // Reset accumulated duration and set start time
            {
                let mut duration = self.interruption_state.accumulated_duration.lock();
                *duration = 0.0;
            }

            // Get current timestamp in milliseconds
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as usize;
            self.interruption_state
                .start_time_ms
                .store(now, Ordering::Release);

            // Calculate audio duration based on text length and sample rate
            // Estimate: ~150 words per minute average speech rate
            let word_count = text.split_whitespace().count() as f32;
            let estimated_duration_seconds = (word_count / 150.0) * 60.0;
            let until_ms = now + (estimated_duration_seconds * 1000.0) as usize;
            self.interruption_state
                .non_interruptible_until_ms
                .store(until_ms, Ordering::Release);
        } else {
            self.interruption_state
                .allow_interruption
                .store(true, Ordering::Release);
            self.interruption_state
                .non_interruptible_until_ms
                .store(0, Ordering::Release);
            self.interruption_state
                .start_time_ms
                .store(0, Ordering::Release);
            self.interruption_state
                .is_clearing
                .store(false, Ordering::Release);

            let mut duration = self.interruption_state.accumulated_duration.lock();
            *duration = 0.0;
        }

        // Send text to TTS provider
        {
            let mut tts = self.tts.write().await;
            tts.speak(text, flush)
                .await
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Check if interruption is currently blocked
    ///
    /// # Returns
    /// * `bool` - True if interruption is currently blocked (allow_interruption=false and audio still playing)
    pub async fn is_interruption_blocked(&self) -> bool {
        // Lock-free check using atomics
        if !self
            .interruption_state
            .allow_interruption
            .load(Ordering::Acquire)
        {
            let until_ms = self
                .interruption_state
                .non_interruptible_until_ms
                .load(Ordering::Acquire);
            if until_ms > 0 {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as usize;
                if now_ms < until_ms {
                    return true;
                }
            }
        }
        false
    }

    /// Clear any queued text from the TTS provider and audio buffers
    ///
    /// This method clears both the TTS text queue and any audio buffers
    /// (e.g., LiveKit audio source) if an audio clear callback is registered.
    /// It also blocks any incoming audio callbacks during the clearing process
    /// to prevent audio chunks from being forwarded after clear is initiated.
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    pub async fn clear_tts(&self) -> VoiceManagerResult<()> {
        // Check if we're allowed to clear
        if self.is_interruption_blocked().await {
            // Still within non-interruptible period, ignore clear
            return Ok(());
        }

        // Set clearing flag to immediately block audio callbacks (lock-free)
        self.interruption_state
            .is_clearing
            .store(true, Ordering::Release);
        debug!("Started audio clearing process - blocking audio callbacks");

        // Clear TTS text queue
        let mut tts = self.tts.write().await;
        tts.clear().await.map_err(VoiceManagerError::TTSError)?;
        drop(tts); // Release the lock

        // Call audio clear callback to clear any audio buffers (e.g., LiveKit)
        {
            let callback_opt = self.audio_clear_callback.read().clone();
            if let Some(callback) = callback_opt {
                callback().await;
            }
        }

        // Use notification instead of sleep for better latency
        // Wait for pending audio to be processed with timeout
        let _ = tokio::time::timeout(
            Duration::from_millis(50), // Reduced from 100ms
            self.clear_notify.notified(),
        )
        .await;

        // Reset interruption state using atomics
        self.interruption_state
            .allow_interruption
            .store(true, Ordering::Release);
        self.interruption_state
            .non_interruptible_until_ms
            .store(0, Ordering::Release);
        self.interruption_state
            .start_time_ms
            .store(0, Ordering::Release);
        self.interruption_state
            .is_clearing
            .store(false, Ordering::Release);

        let mut duration = self.interruption_state.accumulated_duration.lock();
        *duration = 0.0;
        drop(duration);

        debug!("Completed audio clearing process - audio callbacks resumed");

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

        // Store the callback for later use - using parking_lot for faster access
        {
            let mut stt_callback = self.stt_callback.write();
            *stt_callback = Some(callback.clone());
        }

        // Also store in speech final state for timer access
        {
            let mut state = self.speech_final_state.write();
            state.user_callback = Some(callback.clone());
        }

        // Create wrapper callback that processes timing and interruption control before forwarding to user
        let speech_final_state_clone = self.speech_final_state.clone();
        let interruption_state_clone = self.interruption_state.clone();

        let wrapper_callback: STTResultCallback = Arc::new(move |result| {
            let callback = callback.clone();
            let speech_final_state = speech_final_state_clone.clone();
            let interruption_state = interruption_state_clone.clone();

            Box::pin(async move {
                // Check if we should ignore STT results due to non-interruptible audio (lock-free)
                if !interruption_state
                    .allow_interruption
                    .load(Ordering::Acquire)
                {
                    let until_ms = interruption_state
                        .non_interruptible_until_ms
                        .load(Ordering::Acquire);
                    if until_ms > 0 {
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as usize;
                        if now_ms < until_ms {
                            // Still within non-interruptible period, ignore STT result
                            return;
                        }
                    }
                }

                // Process result with timing control inline
                let processed_result =
                    Self::process_stt_result_with_timing_static(result, speech_final_state).await;

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
        let user_callback = Arc::new(callback);
        let interruption_state_clone = self.interruption_state.clone();

        // Create wrapper that checks clearing state and updates interruption timing
        let wrapper_callback = Arc::new(move |audio_data: AudioData| {
            let user_cb = user_callback.clone();
            let int_state = interruption_state_clone.clone();

            Box::pin(async move {
                // Check if we're in clearing state - lock-free check
                if int_state.is_clearing.load(Ordering::Acquire) {
                    debug!(
                        "Discarding audio chunk during clearing process: {} bytes",
                        audio_data.data.len()
                    );
                    return; // Don't forward audio to user callback
                }

                // Update non-interruptible timing based on actual audio data
                if !int_state.allow_interruption.load(Ordering::Acquire) {
                    // Calculate actual audio duration from audio data
                    // For PCM/linear16: bytes / (sample_rate * bytes_per_sample * channels)
                    // Assuming 16-bit audio (2 bytes per sample) and mono (1 channel)
                    let bytes_per_sample = 2;
                    let channels = 1;
                    let sample_rate = int_state.current_sample_rate.load(Ordering::Acquire);

                    let chunk_duration_seconds = audio_data.data.len() as f32
                        / (sample_rate as f32 * bytes_per_sample as f32 * channels as f32);

                    // Accumulate the duration
                    {
                        let mut duration = int_state.accumulated_duration.lock();
                        *duration += chunk_duration_seconds;

                        // Update the non-interruptible period based on accumulated audio duration
                        let start_ms = int_state.start_time_ms.load(Ordering::Acquire);
                        if start_ms > 0 {
                            let until_ms = start_ms + (*duration * 1000.0) as usize;
                            int_state
                                .non_interruptible_until_ms
                                .store(until_ms, Ordering::Release);
                        }
                    }
                }

                // Call the user's callback (only if not clearing)
                user_cb(audio_data).await;
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        // Store callback and release lock before await
        let audio_callback = {
            let mut tts_audio_callback = self.tts_audio_callback.write();
            *tts_audio_callback = Some(wrapper_callback.clone());
            tts_audio_callback.clone()
        };

        // Update the internal TTS callback
        {
            let mut tts = self.tts.write().await;
            let tts_callback = Arc::new(VoiceManagerTTSCallback {
                audio_callback,
                error_callback: self.tts_error_callback.read().clone(),
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
        // Store callback and then release lock before await
        let error_callback = {
            let mut tts_error_callback = self.tts_error_callback.write();
            *tts_error_callback = Some(Arc::new(callback));
            tts_error_callback.clone()
        };

        // Update the internal TTS callback
        {
            let mut tts = self.tts.write().await;
            let tts_callback = Arc::new(VoiceManagerTTSCallback {
                audio_callback: self.tts_audio_callback.read().clone(),
                error_callback,
            });

            tts.on_audio(tts_callback)
                .map_err(VoiceManagerError::TTSError)?;
        }

        Ok(())
    }

    /// Register a callback for audio clear operations
    ///
    /// This callback is called when the TTS queue is cleared and any audio
    /// buffers (e.g., LiveKit audio source) need to be cleared as well.
    ///
    /// # Arguments
    /// * `callback` - Closure that returns a Future for clearing audio
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
    /// voice_manager.on_audio_clear(|| {
    ///     Box::pin(async move {
    ///         // Clear LiveKit audio buffer or other audio sources
    ///         println!("Clearing audio buffers");
    ///     })
    /// }).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn on_audio_clear<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        let mut audio_clear_callback = self.audio_clear_callback.write();
        *audio_clear_callback = Some(Arc::new(callback));
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
    /// This method implements the following behavior matching Python logic:
    /// - Returns results immediately to callback (no waiting)
    /// - Starts background timer for is_final=true results  
    /// - Timer generates additional callback invocation when it expires
    /// - Cancels timer when real is_speech_final=true arrives
    /// - Buffers text from final results for forced speech_final
    async fn process_stt_result_with_timing_static(
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    ) -> Option<STTResult> {
        let mut state = speech_final_state.write();

        // Skip empty final results that aren't speech_final
        if result.transcript.trim().is_empty() && result.is_final && !result.is_speech_final {
            return None;
        }

        // Case 1: Real is_speech_final=true received
        if result.is_speech_final {
            // Cancel any existing timer
            if let Some(handle) = state.timer_handle.take() {
                handle.abort();
            }

            // Reset state completely for next speech segment
            state.text_buffer.clear();
            state.last_text.clear();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);

            return Some(result);
        }

        // Case 2: is_final=true (but not speech_final) with non-empty text
        if result.is_final && !result.transcript.trim().is_empty() {
            // Update buffer with current transcript
            state.text_buffer = result.transcript.clone();

            // Cancel existing timer and restart it
            if let Some(handle) = state.timer_handle.take() {
                handle.abort();
            }

            // Start new background timer
            let speech_final_state_clone = speech_final_state.clone();

            let timer_handle = tokio::spawn(async move {
                // Wait for timeout period (using 2300ms to match Python's 2.3 seconds)
                tokio::time::sleep(Duration::from_millis(2300)).await;

                // Check if we should force speech_final and get callback before locking
                let callback_opt = {
                    let timer_state = speech_final_state_clone.read();
                    if timer_state.waiting_for_speech_final.load(Ordering::Acquire) {
                        timer_state.user_callback.clone()
                    } else {
                        None
                    }
                };

                if let Some(callback) = callback_opt {
                    // Now lock for write to reset state
                    {
                        let mut timer_state = speech_final_state_clone.write();
                        if timer_state.waiting_for_speech_final.load(Ordering::Acquire) {
                            // Reset state
                            timer_state.text_buffer.clear();
                            timer_state.last_text.clear();
                            timer_state
                                .waiting_for_speech_final
                                .store(false, Ordering::Release);
                            timer_state.timer_handle = None;
                        }
                    }

                    // Create forced result and call callback outside lock
                    let forced_result = STTResult {
                        transcript: String::new(),
                        is_final: true,
                        is_speech_final: true,
                        confidence: 1.0, // High confidence for forced result
                    };
                    callback(forced_result).await;
                }
            });

            state.timer_handle = Some(timer_handle);
            state
                .waiting_for_speech_final
                .store(true, Ordering::Release);
        }

        // Always return the original result immediately
        Some(result)
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

        // Test Case 1: is_final result should return immediately and start timer
        {
            let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
                text_buffer: String::with_capacity(1024),
                last_text: String::with_capacity(1024),
                timer_handle: None,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
            }));

            // Reset state first
            {
                let mut state = speech_final_state.write();
                *state = SpeechFinalState {
                    text_buffer: String::with_capacity(1024),
                    last_text: String::with_capacity(1024),
                    timer_handle: None,
                    waiting_for_speech_final: AtomicBool::new(false),
                    user_callback: None,
                };
            }

            // Send is_final=true without is_speech_final=true
            let result1 = STTResult::new("Hello".to_string(), true, false, 0.9);
            let processed = VoiceManager::process_stt_result_with_timing_static(
                result1,
                speech_final_state.clone(),
            )
            .await;

            // Should return original result immediately
            assert!(processed.is_some());
            let processed_result = processed.unwrap();
            assert_eq!(processed_result.transcript, "Hello");
            assert!(processed_result.is_final);
            assert!(!processed_result.is_speech_final);

            // Timer should be started and state updated
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert_eq!(state.text_buffer, "Hello");
            assert!(state.timer_handle.is_some());
        }

        // Test Case 2: Timer should be started for final results and state should be set correctly
        {
            let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
                text_buffer: String::with_capacity(1024),
                last_text: String::with_capacity(1024),
                timer_handle: None,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
            }));

            // Reset state first
            {
                let mut state = speech_final_state.write();
                *state = SpeechFinalState {
                    text_buffer: String::with_capacity(1024),
                    last_text: String::with_capacity(1024),
                    timer_handle: None,
                    waiting_for_speech_final: AtomicBool::new(false),
                    user_callback: None,
                };
            }

            // Send is_final=true without is_speech_final=true
            let result1 = STTResult::new("Test message".to_string(), true, false, 0.8);
            let processed = VoiceManager::process_stt_result_with_timing_static(
                result1,
                speech_final_state.clone(),
            )
            .await;

            // Should return the original result immediately
            assert!(processed.is_some());
            let processed_result = processed.unwrap();
            assert_eq!(processed_result.transcript, "Test message");
            assert!(processed_result.is_final);
            assert!(!processed_result.is_speech_final);

            // Check that timer was started and state is correct
            let state = speech_final_state.read();
            assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
            assert_eq!(state.text_buffer, "Test message");
            assert!(state.timer_handle.is_some());
        }

        // Test Case 3: Real speech_final should cancel timer and reset state
        {
            let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
                text_buffer: String::with_capacity(1024),
                last_text: String::with_capacity(1024),
                timer_handle: None,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
            }));

            // Reset state first
            {
                let mut state = speech_final_state.write();
                *state = SpeechFinalState {
                    text_buffer: String::with_capacity(1024),
                    last_text: String::with_capacity(1024),
                    timer_handle: None,
                    waiting_for_speech_final: AtomicBool::new(false),
                    user_callback: None,
                };
            }

            // Send is_final=true result to start timer
            let result1 = STTResult::new("Hello world".to_string(), true, false, 0.9);
            let _processed1 = VoiceManager::process_stt_result_with_timing_static(
                result1,
                speech_final_state.clone(),
            )
            .await;

            // Verify timer was started
            {
                let state = speech_final_state.read();
                assert!(state.waiting_for_speech_final.load(Ordering::Acquire));
                assert!(state.timer_handle.is_some());
                assert_eq!(state.text_buffer, "Hello world");
            }

            // Send is_speech_final=true (should cancel timer and reset state)
            let result2 = STTResult::new("final result".to_string(), true, true, 0.95);
            let processed2 = VoiceManager::process_stt_result_with_timing_static(
                result2,
                speech_final_state.clone(),
            )
            .await;

            // Should return the original speech_final result
            assert!(processed2.is_some());
            let final_result = processed2.unwrap();
            assert!(final_result.is_speech_final);
            assert!(final_result.is_final);
            assert_eq!(final_result.transcript, "final result");
            assert_eq!(final_result.confidence, 0.95);

            // State should be reset
            let state = speech_final_state.read();
            assert!(!state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.text_buffer.is_empty());
            assert!(state.last_text.is_empty());
            assert!(state.timer_handle.is_none());
        }

        // Test Case 4: Direct speech_final with no prior timer should return original result
        {
            let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
                text_buffer: String::with_capacity(1024),
                last_text: String::with_capacity(1024),
                timer_handle: None,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
            }));

            // Reset state first
            {
                let mut state = speech_final_state.write();
                *state = SpeechFinalState {
                    text_buffer: String::with_capacity(1024),
                    last_text: String::with_capacity(1024),
                    timer_handle: None,
                    waiting_for_speech_final: AtomicBool::new(false),
                    user_callback: None,
                };
            }

            // Send is_speech_final=true with no prior timer (direct speech final)
            let result = STTResult::new("Direct speech final".to_string(), true, true, 0.85);
            let processed = VoiceManager::process_stt_result_with_timing_static(
                result,
                speech_final_state.clone(),
            )
            .await;

            assert!(processed.is_some());
            let final_result = processed.unwrap();
            assert!(final_result.is_speech_final);
            assert!(final_result.is_final);
            // Should return original text as-is
            assert_eq!(final_result.transcript, "Direct speech final");
            assert_eq!(final_result.confidence, 0.85);

            // State should remain reset since no timer was running
            let state = speech_final_state.read();
            assert!(!state.waiting_for_speech_final.load(Ordering::Acquire));
            assert!(state.text_buffer.is_empty());
        }
    }
}
