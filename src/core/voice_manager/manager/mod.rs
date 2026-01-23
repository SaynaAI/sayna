//! Main VoiceManager implementation
//!
//! This module is split into focused submodules following Rust best practices
//! for code organization (see `.cursor/rules/rust.mdc` section 1.3).

mod audio;
mod callbacks;
mod constructors;
mod info;
mod lifecycle;
#[cfg(test)]
pub mod test_support;

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use tokio::sync::RwLock;
#[cfg(feature = "stt-vad")]
use tokio::sync::mpsc;

#[cfg(feature = "stt-vad")]
use super::vad_processor::VADState;
use crate::core::noise_filter::StreamNoiseProcessor;
#[cfg(feature = "stt-vad")]
use crate::core::vad::{SilenceTracker, SileroVAD};
use crate::core::{stt::BaseSTT, tts::BaseTTS};

use super::{
    callbacks::{AudioClearCallback, TTSAudioCallback, TTSCompleteCallback, TTSErrorCallback},
    config::VoiceManagerConfig,
    state::{InterruptionState, SpeechFinalState},
    stt_result::STTResultProcessor,
};

#[cfg(feature = "stt-vad")]
pub(super) const VAD_AUDIO_BUFFER_CAPACITY: usize = 16000 * 5;
#[cfg(feature = "stt-vad")]
pub(super) const VAD_FRAME_BUFFER_CAPACITY: usize = 512;
#[cfg(feature = "stt-vad")]
pub(super) const VAD_CHANNEL_CAPACITY: usize = 16;

/// VoiceManager provides a unified interface for managing STT and TTS providers.
/// Optimized for extreme low-latency with lock-free atomics and pre-allocated buffers.
pub struct VoiceManager {
    pub(super) tts: Arc<RwLock<Box<dyn BaseTTS>>>,
    pub(super) stt: Arc<RwLock<Box<dyn BaseSTT>>>,

    pub(super) tts_audio_callback: Arc<SyncRwLock<Option<TTSAudioCallback>>>,
    pub(super) tts_error_callback: Arc<SyncRwLock<Option<TTSErrorCallback>>>,
    pub(super) audio_clear_callback: Arc<SyncRwLock<Option<AudioClearCallback>>>,
    pub(super) tts_complete_callback: Arc<SyncRwLock<Option<TTSCompleteCallback>>>,

    pub(super) speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,

    pub(super) stt_result_processor: Arc<STTResultProcessor>,

    #[cfg(feature = "stt-vad")]
    pub(super) vad: Option<Arc<RwLock<SileroVAD>>>,

    #[cfg(feature = "stt-vad")]
    pub(super) silence_tracker: Arc<SilenceTracker>,

    #[cfg(feature = "stt-vad")]
    pub(super) vad_state: Arc<VADState>,

    #[cfg(feature = "stt-vad")]
    pub(super) vad_task_tx: Option<mpsc::Sender<Vec<u8>>>,

    pub(super) interruption_state: Arc<InterruptionState>,

    pub(super) config: VoiceManagerConfig,

    /// Noise processor wrapped in Arc for lock-free async access.
    /// The outer RwLock protects initialization (write in start()), the inner Arc
    /// allows cloning the processor out of the lock before async calls.
    pub(super) noise_processor: Arc<RwLock<Option<Arc<StreamNoiseProcessor>>>>,
}

/// Components returned from initialization.
pub(super) struct VoiceManagerComponents {
    pub(super) stt_result_processor: Arc<STTResultProcessor>,
    #[cfg(feature = "stt-vad")]
    pub(super) silence_tracker: Arc<SilenceTracker>,
    #[cfg(feature = "stt-vad")]
    pub(super) vad_state: Arc<VADState>,
}

// Compile-time assertion that VoiceManager is Send + Sync.
// This replaces the previous unsafe impls, relying on all fields being thread-safe.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    // The assertion is evaluated at compile time when this const is instantiated.
    // If VoiceManager doesn't implement Send + Sync, this will fail to compile.
    let _ = assert_send_sync::<VoiceManager>;
};
