//! Main VoiceManager implementation

use parking_lot::RwLock as SyncRwLock;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use tokio::sync::{Notify, RwLock};
use tokio::time::Duration;
use tracing::debug;

use crate::core::cache::store::CacheStore;
use crate::core::{
    create_stt_provider, create_tts_provider,
    stt::{BaseSTT, STTResult, STTResultCallback},
    tts::{AudioData, BaseTTS, TTSError},
    turn_detect::TurnDetector,
};

use super::{
    callbacks::{
        AudioClearCallback, STTCallback, TTSAudioCallback, TTSCompleteCallback, TTSErrorCallback,
        VoiceManagerTTSCallback,
    },
    config::VoiceManagerConfig,
    errors::{VoiceManagerError, VoiceManagerResult},
    state::{InterruptionState, SpeechFinalState},
    stt_result::{STTProcessingConfig, STTResultProcessor},
};

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
    tts_complete_callback: Arc<SyncRwLock<Option<TTSCompleteCallback>>>,

    // Speech final timing control - using parking_lot for faster access
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,

    // Turn detection for better end-of-speech detection
    turn_detector: Option<Arc<RwLock<TurnDetector>>>,

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
    ///     let stt_config = STTConfig {
    ///         provider: "deepgram".to_string(),
    ///         api_key: "your-api-key".to_string(),
    ///         ..Default::default()
    ///     };
    ///     let tts_config = TTSConfig {
    ///         provider: "deepgram".to_string(),
    ///         api_key: "your-api-key".to_string(),
    ///         ..Default::default()
    ///     };
    ///
    ///     let config = VoiceManagerConfig::new(stt_config, tts_config);
    ///     let voice_manager = VoiceManager::new(config, None)?;
    ///     Ok(())
    /// }
    /// ```
    pub fn new(
        config: VoiceManagerConfig,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) -> VoiceManagerResult<Self> {
        let tts = create_tts_provider(&config.tts_config.provider, config.tts_config.clone())
            .map_err(VoiceManagerError::TTSError)?;
        let stt = create_stt_provider(&config.stt_config.provider, config.stt_config.clone())
            .map_err(VoiceManagerError::STTError)?;

        // Pre-allocate string buffers with reasonable capacity
        const TEXT_BUFFER_CAPACITY: usize = 1024;
        let text_buffer = String::with_capacity(TEXT_BUFFER_CAPACITY);

        Ok(Self {
            tts: Arc::new(RwLock::new(tts)),
            stt: Arc::new(RwLock::new(stt)),
            stt_callback: Arc::new(SyncRwLock::new(None)),
            tts_audio_callback: Arc::new(SyncRwLock::new(None)),
            tts_error_callback: Arc::new(SyncRwLock::new(None)),
            audio_clear_callback: Arc::new(SyncRwLock::new(None)),
            tts_complete_callback: Arc::new(SyncRwLock::new(None)),
            speech_final_state: Arc::new(SyncRwLock::new(SpeechFinalState {
                text_buffer,
                turn_detection_handle: None,
                hard_timeout_handle: None,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
                turn_detection_last_fired_ms: AtomicUsize::new(0),
                last_forced_text: String::with_capacity(1024),
                segment_start_ms: AtomicUsize::new(0),
                hard_timeout_deadline_ms: AtomicUsize::new(0),
            })),
            turn_detector,
            interruption_state: Arc::new(InterruptionState {
                allow_interruption: AtomicBool::new(true),
                non_interruptible_until_ms: AtomicUsize::new(0),
                current_sample_rate: AtomicU32::new(24000),
                is_completed: AtomicBool::new(true), // Start as completed
            }),
            config,
            clear_notify: Arc::new(Notify::new()),
        })
    }

    /// Set the TTS cache store and optionally the precomputed TTS config hash
    pub async fn set_tts_cache(
        &self,
        cache: Arc<CacheStore>,
        config_hash: Option<String>,
    ) -> VoiceManagerResult<()> {
        let mut tts = self.tts.write().await;
        if let Some(provider) = tts.get_provider() {
            provider.set_cache(cache).await;
            if let Some(hash) = config_hash {
                provider.set_tts_config_hash(hash).await;
            }
            Ok(())
        } else {
            Err(VoiceManagerError::InitializationError(
                "TTS provider does not support cache".to_string(),
            ))
        }
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
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
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
                interruption_state: Some(self.interruption_state.clone()),
                complete_callback: self.tts_complete_callback.read().clone(),
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
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
    /// voice_manager.stop().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop(&self) -> VoiceManagerResult<()> {
        // Cancel any pending speech final timer
        {
            let mut state = self.speech_final_state.write();
            if let Some(handle) = state.turn_detection_handle.take() {
                handle.abort();
            }
            // Cancel hard timeout handle
            if let Some(handle) = state.hard_timeout_handle.take() {
                handle.abort();
            }
            // Reset speech final state - reuse allocated capacity
            state.text_buffer.clear();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);
            state.user_callback = None;
            state
                .turn_detection_last_fired_ms
                .store(0, Ordering::Release);
            state.last_forced_text.clear();
            state.segment_start_ms.store(0, Ordering::Release);
            state.hard_timeout_deadline_ms.store(0, Ordering::Release);
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
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
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
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
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
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
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
        // Update interruption state
        self.interruption_state
            .allow_interruption
            .store(allow_interruption, Ordering::Release);

        if !allow_interruption {
            // Update sample rate from TTS config
            if let Some(sample_rate) = self.config.tts_config.sample_rate {
                self.interruption_state
                    .current_sample_rate
                    .store(sample_rate, Ordering::Release);
            }

            // Get current timestamp in milliseconds
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as usize;

            // Initialize non_interruptible_until_ms to current time
            // The actual duration will be calculated as TTS chunks arrive
            self.interruption_state
                .non_interruptible_until_ms
                .store(now, Ordering::Release);

            // Mark as not completed since new audio is starting
            self.interruption_state
                .is_completed
                .store(false, Ordering::SeqCst);
        } else {
            // For interruptible audio, just reset to defaults
            self.interruption_state.reset();
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
    /// * `bool` - True if interruption is currently blocked
    pub async fn is_interruption_blocked(&self) -> bool {
        !self.interruption_state.can_interrupt()
    }

    /// Clear any queued text from the TTS provider and audio buffers
    ///
    /// This method clears both the TTS text queue and any audio buffers
    /// (e.g., LiveKit audio source) if an audio clear callback is registered.
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    pub async fn clear_tts(&self) -> VoiceManagerResult<()> {
        // Check if we're allowed to clear
        if !self.interruption_state.can_interrupt() {
            // Not allowed to interrupt yet
            return Ok(());
        }

        debug!("Starting audio clearing process");

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

        // Reset interruption state since we interrupted
        self.interruption_state.reset();

        debug!("Completed audio clearing process");

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
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
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

        // Pre-clone Arc references outside the callback to reduce per-invocation overhead
        let speech_final_state_clone = self.speech_final_state.clone();
        let interruption_state_clone = self.interruption_state.clone();
        let turn_detector_clone = self.turn_detector.clone();

        // Create STT processor with configured timeouts from VoiceManagerConfig
        let processing_config = STTProcessingConfig::new(
            self.config.speech_final_config.stt_speech_final_wait_ms,
            self.config
                .speech_final_config
                .turn_detection_inference_timeout_ms,
            self.config.speech_final_config.speech_final_hard_timeout_ms,
            self.config.speech_final_config.duplicate_window_ms,
        );
        let stt_processor = STTResultProcessor::new(processing_config);

        let wrapper_callback: STTResultCallback = Arc::new(move |result| {
            // Clone Arc references per invocation (lightweight operation)
            let callback = callback.clone();
            let speech_final_state = speech_final_state_clone.clone();
            let interruption_state = interruption_state_clone.clone();
            let turn_detector = turn_detector_clone.clone();
            let stt_processor = stt_processor.clone();

            Box::pin(async move {
                // Fast synchronous check for interruption - execute before any async ops
                if !interruption_state.can_interrupt() {
                    // Still within non-interruptible period, ignore STT result
                    return;
                }

                // Process result with timing control - now non-blocking for result delivery
                let processed_result = stt_processor
                    .process_result(result, speech_final_state, turn_detector)
                    .await;

                if let Some(processed_result) = processed_result {
                    // Call user callback with processed result
                    callback(processed_result).await;
                }
                // If None returned, result was suppressed (empty interim result)
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
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
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
                // Check if this is new audio after completion
                if int_state.is_completed.load(Ordering::Acquire) {
                    // New audio starting after completion
                    int_state.is_completed.store(false, Ordering::SeqCst);

                    // Reset the non_interruptible_until_ms to current time if we're in non-interruptible mode
                    if !int_state.allow_interruption.load(Ordering::Acquire) {
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as usize;
                        int_state
                            .non_interruptible_until_ms
                            .store(now_ms, Ordering::Release);
                    }
                }

                // Calculate audio duration and update non_interruptible_until_ms
                if !int_state.allow_interruption.load(Ordering::Acquire) {
                    // Calculate actual audio duration from audio data
                    // For PCM/linear16: bytes / (sample_rate * bytes_per_sample * channels)
                    // Assuming 16-bit audio (2 bytes per sample) and mono (1 channel)
                    let bytes_per_sample = 2;
                    let channels = 1;
                    let sample_rate = int_state.current_sample_rate.load(Ordering::Acquire);

                    let chunk_duration_seconds = audio_data.data.len() as f32
                        / (sample_rate as f32 * bytes_per_sample as f32 * channels as f32);

                    let chunk_duration_ms = (chunk_duration_seconds * 1000.0) as usize;

                    // Add duration to non_interruptible_until_ms
                    let current_until =
                        int_state.non_interruptible_until_ms.load(Ordering::Acquire);
                    int_state
                        .non_interruptible_until_ms
                        .store(current_until + chunk_duration_ms, Ordering::Release);
                }

                // Call the user's callback
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
                interruption_state: Some(self.interruption_state.clone()),
                complete_callback: self.tts_complete_callback.read().clone(),
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
                interruption_state: Some(self.interruption_state.clone()),
                complete_callback: self.tts_complete_callback.read().clone(),
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
    /// # let config = VoiceManagerConfig::new(STTConfig::default(), TTSConfig::default());
    /// # let voice_manager = VoiceManager::new(config, None)?;
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

    /// Register a callback to be invoked when TTS playback completes
    ///
    /// The completion callback is triggered after the TTS provider finishes generating
    /// all audio chunks for a given `speak()` command. This is useful for:
    /// - Updating UI state (hiding loading indicators)
    /// - Coordinating sequential actions
    /// - Analytics and monitoring
    /// - Knowing when it's safe to perform operations
    ///
    /// # Important Notes
    /// - Callback fires once per `speak()` call
    /// - Callback fires after all audio chunks are generated
    /// - Callback timing indicates server-side generation completion, not client playback
    /// - Multiple `speak()` calls will trigger multiple callbacks in FIFO order
    ///
    /// # Arguments
    /// * `callback` - Async function to call when TTS playback completes
    ///
    /// # Returns
    /// * `VoiceManagerResult<()>` - Success or error
    ///
    /// # Example
    /// ```rust,ignore
    /// use sayna::core::voice_manager::VoiceManager;
    ///
    /// let voice_manager = VoiceManager::new(config, None)?;
    /// voice_manager.start().await?;
    ///
    /// // Register completion callback
    /// voice_manager.on_tts_complete(|| {
    ///     Box::pin(async move {
    ///         println!("TTS playback completed!");
    ///         // Update UI state, trigger next action, etc.
    ///     })
    /// }).await?;
    ///
    /// voice_manager.speak("Hello world", true, true).await?;
    /// // Callback will fire after "Hello world" is fully generated
    /// ```
    pub async fn on_tts_complete<F>(&self, callback: F) -> VoiceManagerResult<()>
    where
        F: Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static,
    {
        // Store the callback
        *self.tts_complete_callback.write() = Some(Arc::new(callback));

        // Update the TTS provider's callback to include completion callback
        let mut tts = self.tts.write().await;
        let audio_callback = self.tts_audio_callback.read().clone();
        let error_callback = self.tts_error_callback.read().clone();
        let complete_callback = self.tts_complete_callback.read().clone();

        let callback = Arc::new(VoiceManagerTTSCallback {
            audio_callback,
            error_callback,
            interruption_state: Some(self.interruption_state.clone()),
            complete_callback,
        });

        tts.on_audio(callback)
            .map_err(VoiceManagerError::TTSError)?;

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
