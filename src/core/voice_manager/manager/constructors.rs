//! VoiceManager constructors and initialization logic.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize};
use tokio::sync::RwLock;
#[cfg(feature = "stt-vad")]
use tokio::sync::mpsc;
#[cfg(feature = "stt-vad")]
use tracing::{debug, warn};

#[cfg(feature = "stt-vad")]
use crate::core::vad::{SilenceTracker, SilenceTrackerConfig, SileroVAD};
use crate::core::{create_stt_provider, create_tts_provider, turn_detect::TurnDetector};

#[cfg(feature = "stt-vad")]
use super::super::vad_processor::VADState;
use super::super::{
    config::VoiceManagerConfig,
    errors::{VoiceManagerError, VoiceManagerResult},
    state::{InterruptionState, SpeechFinalState},
    stt_config::STTProcessingConfig,
    stt_result::STTResultProcessor,
};
#[cfg(feature = "stt-vad")]
use super::{VAD_AUDIO_BUFFER_CAPACITY, VAD_CHANNEL_CAPACITY, VAD_FRAME_BUFFER_CAPACITY};
use super::{VoiceManager, VoiceManagerComponents};

const TEXT_BUFFER_CAPACITY: usize = 1024;

impl VoiceManager {
    /// Create a new VoiceManager with the given configuration.
    ///
    /// # Arguments
    /// * `config` - Configuration for both STT and TTS providers
    /// * `turn_detector` - Optional turn detector for end-of-speech detection
    /// * `vad` - Optional VAD instance (only when `stt-vad` feature is enabled)
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
    ///     let voice_manager = VoiceManager::new(config, None, None)?;
    ///     Ok(())
    /// }
    /// ```
    #[cfg(feature = "stt-vad")]
    pub fn new(
        config: VoiceManagerConfig,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        vad: Option<Arc<RwLock<SileroVAD>>>,
    ) -> VoiceManagerResult<Self> {
        Self::new_internal(config, turn_detector, vad)
    }

    /// Create a new VoiceManager with the given configuration.
    ///
    /// # Arguments
    /// * `config` - Configuration for both STT and TTS providers
    /// * `turn_detector` - Optional turn detector for end-of-speech detection
    ///
    /// # Returns
    /// * `VoiceManagerResult<Self>` - A new VoiceManager instance or error
    #[cfg(not(feature = "stt-vad"))]
    pub fn new(
        config: VoiceManagerConfig,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) -> VoiceManagerResult<Self> {
        Self::new_internal(config, turn_detector)
    }

    /// Create STTProcessingConfig from VoiceManagerConfig.
    pub(super) fn create_stt_processing_config(config: &VoiceManagerConfig) -> STTProcessingConfig {
        #[cfg(feature = "stt-vad")]
        {
            let vad_config = &config.vad_config.silero_config;
            STTProcessingConfig::with_vad(vad_config.silence_duration_ms)
                .set_turn_detection_inference_timeout_ms(
                    config
                        .speech_final_config
                        .turn_detection_inference_timeout_ms,
                )
                .set_duplicate_window_ms(config.speech_final_config.duplicate_window_ms)
                .set_retry_silence_duration_ms(config.speech_final_config.retry_silence_duration_ms)
                .set_backup_silence_timeout_ms(config.speech_final_config.backup_silence_timeout_ms)
        }

        #[cfg(not(feature = "stt-vad"))]
        {
            STTProcessingConfig::default()
                .set_turn_detection_inference_timeout_ms(
                    config
                        .speech_final_config
                        .turn_detection_inference_timeout_ms,
                )
                .set_duplicate_window_ms(config.speech_final_config.duplicate_window_ms)
                .set_retry_silence_duration_ms(config.speech_final_config.retry_silence_duration_ms)
                .set_backup_silence_timeout_ms(config.speech_final_config.backup_silence_timeout_ms)
        }
    }

    /// Initialize all STT processing components (VAD path).
    #[cfg(feature = "stt-vad")]
    pub(super) fn initialize_components(
        config: &VoiceManagerConfig,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) -> VoiceManagerComponents {
        let vad_config = &config.vad_config.silero_config;
        let silence_tracker_config = SilenceTrackerConfig::from_sample_rate(
            vad_config.sample_rate.as_hz(),
            vad_config.threshold,
            vad_config.silence_duration_ms,
            vad_config.min_speech_duration_ms,
        );
        let silence_tracker = Arc::new(SilenceTracker::new(silence_tracker_config));

        let vad_state = Arc::new(VADState::new(
            VAD_AUDIO_BUFFER_CAPACITY,
            VAD_FRAME_BUFFER_CAPACITY,
        ));

        let processing_config = Self::create_stt_processing_config(config);
        let stt_result_processor = Arc::new(STTResultProcessor::with_vad_components(
            processing_config,
            turn_detector,
            silence_tracker.clone(),
            vad_state.clone(),
        ));

        VoiceManagerComponents {
            stt_result_processor,
            silence_tracker,
            vad_state,
        }
    }

    /// Initialize all STT processing components (non-VAD path).
    #[cfg(not(feature = "stt-vad"))]
    pub(super) fn initialize_components(config: &VoiceManagerConfig) -> VoiceManagerComponents {
        let processing_config = Self::create_stt_processing_config(config);
        VoiceManagerComponents {
            stt_result_processor: Arc::new(STTResultProcessor::new(processing_config)),
        }
    }

    fn new_internal(
        config: VoiceManagerConfig,
        #[cfg(feature = "stt-vad")] turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        #[cfg(not(feature = "stt-vad"))] _turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        #[cfg(feature = "stt-vad")] vad: Option<Arc<RwLock<SileroVAD>>>,
    ) -> VoiceManagerResult<Self> {
        let tts = create_tts_provider(&config.tts_config.provider, config.tts_config.clone())
            .map_err(VoiceManagerError::TTSError)?;
        let stt = create_stt_provider(&config.stt_config.provider, config.stt_config.clone())
            .map_err(VoiceManagerError::STTError)?;

        let text_buffer = String::with_capacity(TEXT_BUFFER_CAPACITY);

        #[cfg(feature = "stt-vad")]
        let components = Self::initialize_components(&config, turn_detector);

        #[cfg(not(feature = "stt-vad"))]
        let components = Self::initialize_components(&config);

        let speech_final_state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: None,
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::with_capacity(1024),
            vad_turn_end_detected: AtomicBool::new(false),
            vad_turn_detection_handle: None,
            turn_in_progress: AtomicBool::new(false),
            backup_timeout_handle: None,
        }));

        #[cfg(feature = "stt-vad")]
        let vad_task_tx = if vad.is_some() {
            let (tx, mut rx) = mpsc::channel::<Vec<u8>>(VAD_CHANNEL_CAPACITY);

            let vad_clone = vad.clone();
            let vad_state_clone = components.vad_state.clone();
            let silence_tracker_clone = components.silence_tracker.clone();
            let stt_processor_clone = components.stt_result_processor.clone();
            let speech_final_state_clone = speech_final_state.clone();

            tokio::spawn(async move {
                debug!("VAD worker task started");
                while let Some(audio) = rx.recv().await {
                    if let Some(ref vad) = vad_clone {
                        match super::super::vad_processor::process_audio_chunk(
                            &audio,
                            vad,
                            &vad_state_clone,
                            &silence_tracker_clone,
                        )
                        .await
                        {
                            Ok(events) => {
                                for event in events {
                                    stt_processor_clone
                                        .process_vad_event(event, speech_final_state_clone.clone());
                                }
                            }
                            Err(e) => {
                                warn!("VAD processing error (STT continues): {:?}", e);
                            }
                        }
                    }
                }
                debug!("VAD worker task exiting");
            });

            Some(tx)
        } else {
            None
        };

        Ok(Self {
            tts: Arc::new(RwLock::new(tts)),
            stt: Arc::new(RwLock::new(stt)),
            tts_audio_callback: Arc::new(SyncRwLock::new(None)),
            tts_error_callback: Arc::new(SyncRwLock::new(None)),
            audio_clear_callback: Arc::new(SyncRwLock::new(None)),
            tts_complete_callback: Arc::new(SyncRwLock::new(None)),
            speech_final_state,
            stt_result_processor: components.stt_result_processor,
            #[cfg(feature = "stt-vad")]
            vad,
            #[cfg(feature = "stt-vad")]
            silence_tracker: components.silence_tracker,
            #[cfg(feature = "stt-vad")]
            vad_state: components.vad_state,
            #[cfg(feature = "stt-vad")]
            vad_task_tx,
            interruption_state: Arc::new(InterruptionState {
                allow_interruption: AtomicBool::new(true),
                non_interruptible_until_ms: AtomicUsize::new(0),
                current_sample_rate: AtomicU32::new(24000),
                is_completed: AtomicBool::new(true),
            }),
            config,
            noise_processor: Arc::new(RwLock::new(None)),
        })
    }
}
