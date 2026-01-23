//! Test-only module for injecting stub providers into VoiceManager.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize};
use tokio::sync::RwLock;

use crate::core::stt::BaseSTT;
use crate::core::tts::BaseTTS;
use crate::core::turn_detect::TurnDetector;
#[cfg(feature = "stt-vad")]
use crate::core::vad::{SilenceTracker, SilenceTrackerConfig, SileroVAD};

#[cfg(feature = "stt-vad")]
use super::super::stt_result::STTResultProcessor;
#[cfg(feature = "stt-vad")]
use super::super::vad_processor::VADState;
use super::super::{
    config::VoiceManagerConfig,
    errors::VoiceManagerResult,
    state::{InterruptionState, SpeechFinalState},
};
use super::VoiceManager;

const TEXT_BUFFER_CAPACITY: usize = 1024;

impl VoiceManager {
    /// Test-only: Get a reference to the VAD state for setting test flags.
    #[cfg(feature = "stt-vad")]
    pub fn get_vad_state_for_testing(&self) -> Arc<VADState> {
        self.vad_state.clone()
    }

    /// Test-only constructor with forced VAD error support.
    ///
    /// This variant allows tests to inject:
    /// - A VAD instance (so the VAD processing path is taken)
    /// - A pre-configured VADState with `force_error` set (so VAD returns an error)
    ///
    /// This enables testing the VAD error path without needing a real VAD model.
    #[cfg(feature = "stt-vad")]
    pub fn new_with_providers_and_vad_state(
        config: VoiceManagerConfig,
        stt: Box<dyn BaseSTT>,
        tts: Box<dyn BaseTTS>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        vad: Option<Arc<RwLock<SileroVAD>>>,
        vad_state: Arc<VADState>,
    ) -> VoiceManagerResult<Self> {
        let text_buffer = String::with_capacity(TEXT_BUFFER_CAPACITY);

        let processing_config = Self::create_stt_processing_config(&config);
        let vad_config = &config.vad_config.silero_config;
        let silence_tracker_config = SilenceTrackerConfig::from_sample_rate(
            vad_config.sample_rate.as_hz(),
            vad_config.threshold,
            vad_config.silence_duration_ms,
            vad_config.min_speech_duration_ms,
        );
        let silence_tracker = Arc::new(SilenceTracker::new(silence_tracker_config));

        let stt_result_processor = Arc::new(STTResultProcessor::with_vad_components(
            processing_config,
            turn_detector,
            silence_tracker.clone(),
            vad_state.clone(),
        ));

        Ok(Self {
            tts: Arc::new(RwLock::new(tts)),
            stt: Arc::new(RwLock::new(stt)),
            tts_audio_callback: Arc::new(SyncRwLock::new(None)),
            tts_error_callback: Arc::new(SyncRwLock::new(None)),
            audio_clear_callback: Arc::new(SyncRwLock::new(None)),
            tts_complete_callback: Arc::new(SyncRwLock::new(None)),
            speech_final_state: Arc::new(SyncRwLock::new(SpeechFinalState {
                text_buffer,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
                turn_detection_last_fired_ms: AtomicUsize::new(0),
                last_forced_text: String::with_capacity(1024),
                vad_turn_end_detected: AtomicBool::new(false),
                vad_turn_detection_handle: None,
                turn_in_progress: AtomicBool::new(false),
                backup_timeout_handle: None,
            })),
            stt_result_processor,
            vad,
            silence_tracker,
            vad_state,
            vad_task_tx: None,
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

    /// Test-only constructor that accepts pre-built STT and TTS providers.
    ///
    /// This allows tests to inject stub implementations for verifying
    /// behavior in isolation (e.g., testing that STT receives audio
    /// even when VAD errors).
    #[cfg(feature = "stt-vad")]
    pub fn new_with_providers(
        config: VoiceManagerConfig,
        stt: Box<dyn BaseSTT>,
        tts: Box<dyn BaseTTS>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        vad: Option<Arc<RwLock<SileroVAD>>>,
    ) -> VoiceManagerResult<Self> {
        Self::new_with_providers_internal(config, stt, tts, turn_detector, vad)
    }

    #[cfg(not(feature = "stt-vad"))]
    #[allow(dead_code)]
    pub fn new_with_providers(
        config: VoiceManagerConfig,
        stt: Box<dyn BaseSTT>,
        tts: Box<dyn BaseTTS>,
        _turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) -> VoiceManagerResult<Self> {
        Self::new_with_providers_internal(config, stt, tts, _turn_detector)
    }

    fn new_with_providers_internal(
        config: VoiceManagerConfig,
        stt: Box<dyn BaseSTT>,
        tts: Box<dyn BaseTTS>,
        #[cfg(feature = "stt-vad")] turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        #[cfg(not(feature = "stt-vad"))] _turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        #[cfg(feature = "stt-vad")] vad: Option<Arc<RwLock<SileroVAD>>>,
    ) -> VoiceManagerResult<Self> {
        let text_buffer = String::with_capacity(TEXT_BUFFER_CAPACITY);

        #[cfg(feature = "stt-vad")]
        let components = Self::initialize_components(&config, turn_detector);

        #[cfg(not(feature = "stt-vad"))]
        let components = Self::initialize_components(&config);

        Ok(Self {
            tts: Arc::new(RwLock::new(tts)),
            stt: Arc::new(RwLock::new(stt)),
            tts_audio_callback: Arc::new(SyncRwLock::new(None)),
            tts_error_callback: Arc::new(SyncRwLock::new(None)),
            audio_clear_callback: Arc::new(SyncRwLock::new(None)),
            tts_complete_callback: Arc::new(SyncRwLock::new(None)),
            speech_final_state: Arc::new(SyncRwLock::new(SpeechFinalState {
                text_buffer,
                waiting_for_speech_final: AtomicBool::new(false),
                user_callback: None,
                turn_detection_last_fired_ms: AtomicUsize::new(0),
                last_forced_text: String::with_capacity(1024),
                vad_turn_end_detected: AtomicBool::new(false),
                vad_turn_detection_handle: None,
                turn_in_progress: AtomicBool::new(false),
                backup_timeout_handle: None,
            })),
            stt_result_processor: components.stt_result_processor,
            #[cfg(feature = "stt-vad")]
            vad,
            #[cfg(feature = "stt-vad")]
            silence_tracker: components.silence_tracker,
            #[cfg(feature = "stt-vad")]
            vad_state: components.vad_state,
            #[cfg(feature = "stt-vad")]
            vad_task_tx: None,
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
