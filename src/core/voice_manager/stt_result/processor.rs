//! Core STT result processing.
//!
//! Contains the main `STTResultProcessor` struct and core STT processing logic
//! including duplicate detection, result filtering, and turn detection handling.
//!
//! VAD event handling is split into a separate module (`vad_events`) when the
//! `stt-vad` feature is enabled.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
#[cfg(feature = "stt-vad")]
use tokio::sync::RwLock;
use tracing::debug;

use crate::core::stt::STTResult;
#[cfg(feature = "stt-vad")]
use crate::core::turn_detect::TurnDetector;
#[cfg(feature = "stt-vad")]
use crate::core::vad::SilenceTracker;

use super::super::state::SpeechFinalState;
use super::super::stt_config::STTProcessingConfig;
#[cfg(not(feature = "stt-vad"))]
use super::super::utils::get_current_time_ms;
#[cfg(feature = "stt-vad")]
use super::super::vad_processor::VADState;

/// Processor for STT results with timing control.
///
/// This struct handles the core STT result processing including:
/// - Result filtering (empty results, duplicates)
/// - Turn detection state management
/// - Speech final event handling
///
/// When `stt-vad` is compiled, VAD event handling methods are available
/// via the `vad_events` module integration.
#[derive(Clone)]
pub struct STTResultProcessor {
    pub(super) config: STTProcessingConfig,

    /// Optional turn detector for smart-turn fallback (feature-gated: `stt-vad`)
    #[cfg(feature = "stt-vad")]
    pub(super) turn_detector: Option<Arc<RwLock<TurnDetector>>>,

    /// Optional silence tracker for smart-turn fallback (feature-gated: `stt-vad`)
    #[cfg(feature = "stt-vad")]
    pub(super) silence_tracker: Option<Arc<SilenceTracker>>,

    /// Optional VAD state containing audio buffer for smart-turn fallback (feature-gated: `stt-vad`)
    #[cfg(feature = "stt-vad")]
    pub(super) vad_state: Option<Arc<VADState>>,
}

impl STTResultProcessor {
    /// Create a new STTResultProcessor with just configuration (no VAD components).
    #[cfg(not(feature = "stt-vad"))]
    pub fn new(config: STTProcessingConfig) -> Self {
        Self { config }
    }

    /// Create a new STTResultProcessor with just configuration (no VAD components).
    #[cfg(feature = "stt-vad")]
    pub fn new(config: STTProcessingConfig) -> Self {
        Self {
            config,
            turn_detector: None,
            silence_tracker: None,
            vad_state: None,
        }
    }

    /// Create a new STTResultProcessor with VAD components for smart-turn fallback.
    #[cfg(feature = "stt-vad")]
    pub fn with_vad_components(
        config: STTProcessingConfig,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        silence_tracker: Arc<SilenceTracker>,
        vad_state: Arc<VADState>,
    ) -> Self {
        Self {
            config,
            turn_detector,
            silence_tracker: Some(silence_tracker),
            vad_state: Some(vad_state),
        }
    }

    /// Process an STT result, handling turn detection and speech_final events.
    pub async fn process_result(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    ) -> Option<STTResult> {
        // Fast synchronous checks - no awaits
        if !self.should_deliver_result(&result) {
            return None;
        }

        // Handle real speech_final from STT provider.
        // When stt-vad is enabled, IGNORE this and let VAD + Smart Turn control turn completion.
        #[cfg(not(feature = "stt-vad"))]
        if result.is_speech_final {
            let now_ms = get_current_time_ms();
            return self.handle_real_speech_final(result, speech_final_state, now_ms);
        }

        // When stt-vad is enabled, override is_speech_final to false so user only
        // receives is_speech_final=true from the VAD + Smart Turn path.
        #[cfg(feature = "stt-vad")]
        let result = if result.is_speech_final {
            debug!(
                "STT provider sent is_speech_final=true, but stt-vad is enabled - overriding to false (VAD controls turn completion)"
            );
            STTResult {
                is_speech_final: false,
                ..result
            }
        } else {
            result
        };

        // Handle is_final (but not speech_final) - spawn turn detection in background
        if result.is_final {
            self.handle_turn_detection(&result.transcript, speech_final_state);
        }

        // Always return the original result immediately - no awaits in critical path
        Some(result)
    }

    /// Check if VAD-based silence detection is enabled.
    ///
    /// When `stt-vad` is compiled, VAD is always active and cannot be disabled at runtime.
    #[cfg(feature = "stt-vad")]
    pub fn is_vad_enabled(&self) -> bool {
        true
    }

    /// Check if VAD-based silence detection is enabled.
    #[cfg(not(feature = "stt-vad"))]
    pub fn is_vad_enabled(&self) -> bool {
        false
    }

    /// Get the configured VAD silence duration threshold in milliseconds.
    pub fn vad_silence_duration_ms(&self) -> u64 {
        self.config.vad_silence_duration_ms
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Core STT processing methods (internal)
    // ─────────────────────────────────────────────────────────────────────────────

    pub(super) fn should_deliver_result(&self, result: &STTResult) -> bool {
        // Skip empty final results that aren't speech_final
        !(result.transcript.trim().is_empty() && result.is_final && !result.is_speech_final)
    }

    /// Handle turn detection by accumulating text and setting waiting flag.
    ///
    /// No timeout tasks are spawned. The actual speech_final emission depends on feature:
    /// - When `stt-vad` enabled: VAD + Smart-Turn path controls turn completion
    /// - When `stt-vad` NOT enabled: STT provider's `is_speech_final` controls turn completion
    ///
    /// Takes a borrowed transcript to avoid cloning the full STTResult just to buffer text.
    pub(super) fn handle_turn_detection(
        &self,
        transcript: &str,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    ) {
        // Update text buffer and set waiting flag
        let mut state = speech_final_state.write();
        state.text_buffer.push_str(transcript);
        state
            .waiting_for_speech_final
            .store(true, Ordering::Release);

        debug!(
            "Accumulated text for turn detection: '{}' (waiting for VAD TurnEnd or real speech_final)",
            state.text_buffer
        );
    }

    #[cfg(not(feature = "stt-vad"))]
    pub(super) fn handle_real_speech_final(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        now_ms: usize,
    ) -> Option<STTResult> {
        let mut state = speech_final_state.write();

        // Check for duplicate within the configured window
        if self.is_duplicate_speech_final(&state, &result.transcript, now_ms) {
            debug!(
                "Ignoring duplicate real speech_final - turn detection fired {}ms ago",
                now_ms.saturating_sub(state.turn_detection_last_fired_ms.load(Ordering::Acquire))
            );
            return None;
        }

        // Cancel pending detection tasks and reset state for next speech segment
        state.reset_for_next_segment();

        Some(result)
    }

    #[cfg(not(feature = "stt-vad"))]
    pub(super) fn is_duplicate_speech_final(
        &self,
        state: &SpeechFinalState,
        transcript: &str,
        now_ms: usize,
    ) -> bool {
        let last_fired_ms = state.turn_detection_last_fired_ms.load(Ordering::Acquire);

        last_fired_ms > 0
            && now_ms.saturating_sub(last_fired_ms) < self.config.duplicate_window_ms
            && state.last_forced_text == transcript
    }
}

impl Default for STTResultProcessor {
    fn default() -> Self {
        Self::new(STTProcessingConfig::default())
    }
}
