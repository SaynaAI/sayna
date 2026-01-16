//! STT result processing with timing control
//!
//! This module provides STT result processing with configurable timing control,
//! supporting both timeout-based and VAD-based silence detection modes.
//!
//! ## Feature Behavior
//!
//! - **When `stt-vad` is compiled**: VAD-based silence detection is **always active**.
//!   The `use_vad_silence_detection` field is ignored, and there is no runtime toggle
//!   to disable VAD. This ensures consistent behavior when turn detection is built.
//!
//! - **When `stt-vad` is NOT compiled**: VAD-based detection is unavailable, and
//!   timeout-based mode is used.
//!
//! ## Timeout-based Mode
//!
//! The traditional approach waits for STT provider to send `speech_final`:
//! 1. Wait for STT provider to send real `speech_final` (default: 2000ms)
//! 2. If silent, run turn detection ML model on accumulated text
//! 3. Fire artificial `speech_final` if turn detection confirms
//!
//! ## VAD-based Mode (always active under `stt-vad`)
//!
//! Uses Silero-VAD for more accurate silence detection:
//! 1. Monitor audio with VAD for configurable silence threshold (default: 300ms)
//! 2. When silence detected, run turn detection ML model on accumulated text
//! 3. Fire artificial `speech_final` if turn detection confirms
//!
//! VAD mode provides faster and more accurate turn detection by analyzing
//! actual audio-level silence rather than relying on timeouts.
//!
//! ## VAD + Smart-Turn Integration Pattern
//!
//! The integration follows best practices from the smart-turn documentation:
//!
//! ### Why Use Both VAD and Smart-Turn?
//!
//! **VAD (Silero)** provides:
//! - Fast, efficient silence detection (~2ms per frame)
//! - Low latency feedback on speech activity
//! - Immediate signal when user stops speaking
//!
//! **Smart-Turn** provides:
//! - Semantic understanding of turn completion
//! - Higher accuracy (>90% for most languages)
//! - Context-aware decision making
//!
//! **Combined Pattern**: VAD detects silence first (fast, cheap), then smart-turn
//! confirms if the turn is semantically complete (more accurate, slightly slower).
//!
//! ### How It Works
//!
//! ```text
//! Audio In ─► VAD ─► SilenceTracker ─► TurnEnd Event ─► Smart-Turn ─► speech_final
//!                          ▲                               │
//!                          │                               │
//!                          └── If false, reset and wait ───┘
//! ```
//!
//! 1. **VAD monitors silence**: Silero-VAD continuously processes audio frames
//! 2. **TurnEnd triggers smart-turn**: When silence exceeds threshold, we run smart-turn
//! 3. **Smart-turn sees full context**: The model receives ALL accumulated audio
//! 4. **Rejection allows retry**: If smart-turn returns false, we reset and wait
//!
//! ### Audio Buffer Management
//!
//! - `SpeechStart`: Clear buffer (new utterance starting)
//! - `SpeechResumed`: Keep buffer (same utterance continues)
//! - `TurnEnd`: Pass buffer to smart-turn
//! - After `speech_final`: Clear buffer
//!
//! ### Why This Pattern?
//!
//! Smart-turn needs context and "is not designed to run on very short audio segments."
//! Running it on the entire accumulated turn (up to 8 seconds) provides the best accuracy.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::{debug, info};

use crate::core::vad::{SilenceTracker, VADEvent};
use crate::core::{stt::STTResult, turn_detect::TurnDetector};

use super::state::SpeechFinalState;

/// Configuration for STT result processing
#[derive(Clone, Copy)]
pub struct STTProcessingConfig {
    /// Use VAD-based silence detection instead of timeout.
    ///
    /// # Feature-dependent behavior
    ///
    /// - **When `stt-vad` is compiled**: This field is **ignored**. VAD-based
    ///   silence detection is always active and cannot be disabled at runtime.
    ///   The bundled turn detection model requires VAD to function correctly.
    ///
    /// - **When `stt-vad` is NOT compiled**: This field is also ignored since
    ///   VAD functionality is unavailable; timeout-based mode is always used.
    ///
    /// # Behavior when active (under `stt-vad`)
    ///
    /// Silence is detected by processing audio through Silero-VAD and tracking
    /// when speech probability drops below threshold for the configured
    /// `vad_silence_duration_ms`.
    ///
    /// Default: false (for backward compatibility, but ignored under `stt-vad`)
    pub use_vad_silence_detection: bool,

    /// Time to wait for STT provider to send real speech_final (ms).
    ///
    /// This is the primary window in timeout-based mode - we trust STT provider
    /// during this time. Only used when `stt-vad` feature is NOT compiled.
    ///
    /// Default: 2000ms
    pub stt_speech_final_wait_ms: u64,

    /// VAD silence duration threshold (ms).
    ///
    /// When `stt-vad` is compiled, this is how long continuous silence (speech
    /// probability below threshold) must be observed before triggering turn
    /// detection. VAD is always active under `stt-vad`.
    ///
    /// Default: 300ms
    pub vad_silence_duration_ms: u64,

    /// Maximum time to wait for turn detection inference to complete (ms).
    ///
    /// If the turn detection ML model takes longer than this, we fire
    /// speech_final as a fallback.
    ///
    /// Default: 100ms
    pub turn_detection_inference_timeout_ms: u64,

    /// Hard upper bound timeout for any user utterance (ms).
    ///
    /// This guarantees that no utterance will wait longer than this value
    /// even if neither the STT provider, VAD, nor turn detector fire.
    /// Acts as a safety net for both modes.
    ///
    /// Default: 5000ms
    pub speech_final_hard_timeout_ms: u64,

    /// Window to prevent duplicate speech_final events (ms).
    ///
    /// If a real speech_final arrives within this window after we've already
    /// fired an artificial one, we ignore the duplicate.
    ///
    /// Default: 500ms
    pub duplicate_window_ms: usize,
}

impl Default for STTProcessingConfig {
    fn default() -> Self {
        Self {
            // Field value ignored under stt-vad (VAD always active when compiled)
            use_vad_silence_detection: false,
            stt_speech_final_wait_ms: 2000, // Wait 2s for real speech_final from STT
            vad_silence_duration_ms: 300,   // 300ms silence threshold for VAD
            turn_detection_inference_timeout_ms: 100, // 100ms max for model inference
            speech_final_hard_timeout_ms: 5000, // 5s hard upper bound for any utterance
            duplicate_window_ms: 500,       // 500ms duplicate prevention window
        }
    }
}

impl STTProcessingConfig {
    /// Create a new STTProcessingConfig with explicit timeout values (legacy API).
    ///
    /// This constructor maintains backward compatibility. For VAD-enabled configs,
    /// use the builder methods instead.
    pub fn new(
        stt_speech_final_wait_ms: u64,
        turn_detection_inference_timeout_ms: u64,
        speech_final_hard_timeout_ms: u64,
        duplicate_window_ms: usize,
    ) -> Self {
        Self {
            use_vad_silence_detection: false,
            stt_speech_final_wait_ms,
            vad_silence_duration_ms: 300,
            turn_detection_inference_timeout_ms,
            speech_final_hard_timeout_ms,
            duplicate_window_ms,
        }
    }

    /// Create a new config with VAD-based silence detection settings.
    ///
    /// # Feature-dependent behavior
    ///
    /// When `stt-vad` is compiled, VAD is always active regardless of this call.
    /// This method is primarily useful for setting the `vad_silence_duration_ms`.
    ///
    /// # Arguments
    /// * `vad_silence_duration_ms` - Silence duration threshold in milliseconds (default: 300)
    pub fn with_vad(vad_silence_duration_ms: u64) -> Self {
        Self {
            use_vad_silence_detection: true,
            vad_silence_duration_ms,
            ..Self::default()
        }
    }

    /// Set the VAD enable flag (no-op under `stt-vad` feature).
    ///
    /// # Feature-dependent behavior
    ///
    /// - **When `stt-vad` is compiled**: This method is a **no-op**. VAD is always
    ///   active and cannot be disabled at runtime. The field value is ignored.
    ///
    /// - **When `stt-vad` is NOT compiled**: This method sets the field, but VAD
    ///   functionality is unavailable anyway.
    ///
    /// This method is retained for API compatibility but has no effect on runtime
    /// behavior when the `stt-vad` feature is enabled.
    #[cfg_attr(feature = "stt-vad", allow(unused_variables, unused_mut))]
    pub fn set_use_vad(mut self, use_vad: bool) -> Self {
        // Under stt-vad, this field is ignored - VAD is always active.
        // We still set it for API compatibility when the feature is not compiled.
        #[cfg(not(feature = "stt-vad"))]
        {
            self.use_vad_silence_detection = use_vad;
        }
        self
    }

    /// Set the VAD silence duration threshold.
    pub fn set_vad_silence_duration_ms(mut self, duration_ms: u64) -> Self {
        self.vad_silence_duration_ms = duration_ms;
        self
    }

    /// Set the STT speech final wait timeout (timeout mode only).
    pub fn set_stt_speech_final_wait_ms(mut self, wait_ms: u64) -> Self {
        self.stt_speech_final_wait_ms = wait_ms;
        self
    }

    /// Set the hard timeout for any utterance.
    pub fn set_hard_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.speech_final_hard_timeout_ms = timeout_ms;
        self
    }
}

/// Processor for STT results with timing control
#[derive(Clone)]
pub struct STTResultProcessor {
    config: STTProcessingConfig,
}

impl STTResultProcessor {
    pub fn new(config: STTProcessingConfig) -> Self {
        Self { config }
    }

    /// Process an STT result with timing control
    ///
    /// This method implements:
    /// - Immediate return of results (no waiting)
    /// - Turn detection ML model with intelligent timeout selection
    /// - Fast-path synchronous checks before async operations
    /// - Prevention of duplicate speech_final events
    pub async fn process_result(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) -> Option<STTResult> {
        // Fast synchronous checks - no awaits
        if !self.should_deliver_result(&result) {
            return None;
        }

        let now_ms = self.get_current_time_ms();

        // Handle real speech_final
        if result.is_speech_final {
            return self.handle_real_speech_final(result, speech_final_state, now_ms);
        }

        // Handle is_final (but not speech_final) - spawn turn detection in background
        if result.is_final {
            self.handle_turn_detection(result.clone(), speech_final_state, turn_detector);
        }

        // Always return the original result immediately - no awaits in critical path
        Some(result)
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // VAD-based silence detection methods
    // ─────────────────────────────────────────────────────────────────────────────

    /// Process a VAD event for silence detection.
    ///
    /// This method should be called for every VAD event from the SilenceTracker.
    /// When VAD detects silence exceeding the threshold (TurnEnd event), it triggers
    /// turn detection on the accumulated text.
    ///
    /// # Arguments
    /// * `event` - VAD event from SilenceTracker
    /// * `speech_final_state` - Shared state for speech final tracking
    /// * `turn_detector` - Optional turn detector for text-level confirmation
    /// * `silence_tracker` - SilenceTracker to reset after firing speech_final
    /// * `vad_audio_buffer` - VAD audio buffer to clear after firing speech_final
    ///
    /// # VAD Events Handled
    ///
    /// - `SpeechStart`: Resets VAD state and clears audio buffer for new utterance
    /// - `SpeechResumed`: Cancels pending VAD turn detection (user still talking)
    /// - `SilenceDetected`: Logged but no action (waiting for threshold)
    /// - `TurnEnd`: Triggers turn detection on accumulated audio samples
    pub fn process_vad_event(
        &self,
        event: VADEvent,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        silence_tracker: Arc<SilenceTracker>,
        vad_audio_buffer: Arc<SyncRwLock<Vec<i16>>>,
    ) {
        // When stt-vad is not compiled, VAD processing can be disabled via config.
        // When stt-vad IS compiled, VAD is always active (no runtime disable toggle).
        #[cfg(not(feature = "stt-vad"))]
        if !self.config.use_vad_silence_detection {
            return;
        }

        match event {
            VADEvent::SpeechStart => {
                debug!("VAD: Speech started - resetting VAD state and clearing audio buffer");
                let mut state = speech_final_state.write();
                state.reset_vad_state();
                // Clear audio buffer for new utterance
                let mut buffer = vad_audio_buffer.write();
                buffer.clear();
            }

            VADEvent::SpeechResumed => {
                // User resumed speaking before turn end threshold
                // Cancel any pending VAD turn detection
                debug!("VAD: Speech resumed - cancelling pending VAD turn detection");
                let mut state = speech_final_state.write();
                state.reset_vad_state();
            }

            VADEvent::SilenceDetected => {
                // Short silence detected, not yet at threshold
                debug!("VAD: Silence detected - waiting for turn end threshold");
            }

            VADEvent::TurnEnd => {
                // Silence exceeded threshold - trigger turn detection
                self.handle_vad_turn_end(
                    speech_final_state,
                    turn_detector,
                    silence_tracker,
                    vad_audio_buffer,
                );
            }
        }
    }

    /// Handle VAD TurnEnd event by spawning turn detection.
    ///
    /// Called when SilenceTracker detects that silence has exceeded the configured
    /// threshold after sufficient speech. Spawns a task to run turn detection
    /// on the accumulated audio buffer.
    fn handle_vad_turn_end(
        &self,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        silence_tracker: Arc<SilenceTracker>,
        vad_audio_buffer: Arc<SyncRwLock<Vec<i16>>>,
    ) {
        // Get audio buffer for turn detection
        let (audio_samples, buffered_text, should_trigger) = {
            let mut state = speech_final_state.write();

            // Check if already handled or not waiting
            if state.vad_turn_end_detected.load(Ordering::Acquire) {
                debug!("VAD: TurnEnd already detected for this segment - skipping");
                return;
            }

            // Only trigger if we have audio and are waiting for speech_final
            if !state.waiting_for_speech_final.load(Ordering::Acquire) {
                debug!("VAD: TurnEnd received but not waiting for speech_final - skipping");
                return;
            }

            // Get audio samples from buffer
            let audio_buffer = vad_audio_buffer.read();
            if audio_buffer.is_empty() {
                debug!("VAD: TurnEnd received but audio buffer is empty - skipping");
                return;
            }

            // Clone audio samples for turn detection
            let audio = audio_buffer.clone();

            // Mark as detected to prevent duplicates
            state.vad_turn_end_detected.store(true, Ordering::Release);

            // Cancel any timeout-based detection task since VAD is handling it
            if let Some(old_handle) = state.turn_detection_handle.take() {
                debug!("VAD: Cancelling timeout-based turn detection task");
                old_handle.abort();
            }

            (audio, state.text_buffer.clone(), true)
        };

        if !should_trigger {
            return;
        }

        info!(
            "VAD: TurnEnd after {}ms silence - spawning turn detection with {} audio samples",
            self.config.vad_silence_duration_ms,
            audio_samples.len()
        );

        // Spawn VAD-triggered turn detection task with audio samples
        let handle = self.spawn_vad_turn_detection_audio(
            audio_samples,
            buffered_text,
            speech_final_state.clone(),
            turn_detector,
            silence_tracker,
            vad_audio_buffer,
        );

        // Store the handle
        let mut state = speech_final_state.write();
        state.vad_turn_detection_handle = Some(handle);
    }

    /// Spawn turn detection task with audio input.
    ///
    /// This task runs the turn detection ML model on the provided audio samples
    /// and fires speech_final if the turn is confirmed.
    ///
    /// # Arguments
    /// * `audio_samples` - Audio samples to pass to turn detector
    /// * `buffered_text` - Text buffer for logging/backward compatibility
    /// * `speech_final_state` - Shared state for speech final tracking
    /// * `turn_detector` - Optional turn detector for audio-level confirmation
    /// * `silence_tracker` - SilenceTracker to reset after firing speech_final
    /// * `vad_audio_buffer` - VAD audio buffer to clear after firing speech_final
    fn spawn_vad_turn_detection_audio(
        &self,
        audio_samples: Vec<i16>,
        buffered_text: String,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
        silence_tracker: Arc<SilenceTracker>,
        vad_audio_buffer: Arc<SyncRwLock<Vec<i16>>>,
    ) -> JoinHandle<()> {
        let inference_timeout_ms = self.config.turn_detection_inference_timeout_ms;

        tokio::spawn(async move {
            // Check if we should still fire (not cancelled by new speech)
            let should_continue = {
                let state = speech_final_state.read();
                state.waiting_for_speech_final.load(Ordering::Acquire)
                    && state.vad_turn_end_detected.load(Ordering::Acquire)
            };

            if !should_continue {
                debug!("VAD turn detection cancelled - speech resumed or already handled");
                return;
            }

            // Run turn detection with audio samples
            let detection_method = if let Some(detector) = turn_detector {
                let sample_count = audio_samples.len();
                let turn_result =
                    tokio::time::timeout(Duration::from_millis(inference_timeout_ms), async {
                        let detector_guard = detector.read().await;
                        detector_guard.is_turn_complete(&audio_samples).await
                    })
                    .await;

                match turn_result {
                    Ok(Ok(true)) => {
                        info!(
                            "VAD+SmartTurn: Turn complete confirmed for {} audio samples",
                            sample_count
                        );
                        "vad_smart_turn_confirmed"
                    }
                    Ok(Ok(false)) => {
                        info!("VAD: Smart-turn says incomplete - waiting for more input");
                        // Reset VAD state so next TurnEnd can trigger again
                        {
                            let state = speech_final_state.write();
                            state.vad_turn_end_detected.store(false, Ordering::Release);
                        }
                        // Reset silence tracker for next detection attempt
                        silence_tracker.reset();
                        return; // Don't fire
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("VAD smart-turn detection error: {:?} - firing anyway", e);
                        "vad_smart_turn_error_fallback"
                    }
                    Err(_) => {
                        tracing::warn!(
                            "VAD smart-turn detection timeout after {}ms - firing anyway",
                            inference_timeout_ms
                        );
                        "vad_inference_timeout_fallback"
                    }
                }
            } else {
                // No turn detector - fire based on VAD silence alone
                info!("VAD: No turn detector - firing based on silence alone");
                "vad_silence_only"
            };

            // Fire speech_final
            let result = STTResult {
                transcript: String::new(),
                is_final: true,
                is_speech_final: true,
                confidence: 1.0,
            };

            Self::fire_speech_final(result, buffered_text, speech_final_state, detection_method)
                .await;

            // Reset silence tracker and clear VAD audio buffer for next utterance
            silence_tracker.reset();
            {
                let mut buffer = vad_audio_buffer.write();
                buffer.clear();
            }
            info!("VAD: Reset silence tracker and cleared audio buffer after speech_final");
        })
    }

    /// Check if VAD-based silence detection is enabled.
    ///
    /// # Feature-dependent behavior
    ///
    /// - **When `stt-vad` is compiled**: Always returns `true`. VAD is always
    ///   active and cannot be disabled at runtime.
    ///
    /// - **When `stt-vad` is NOT compiled**: Always returns `false` since VAD
    ///   functionality is unavailable.
    pub fn is_vad_enabled(&self) -> bool {
        #[cfg(feature = "stt-vad")]
        {
            true // VAD is always active when stt-vad feature is compiled
        }
        #[cfg(not(feature = "stt-vad"))]
        {
            false // VAD unavailable without stt-vad feature
        }
    }

    /// Get the configured VAD silence duration threshold in milliseconds.
    pub fn vad_silence_duration_ms(&self) -> u64 {
        self.config.vad_silence_duration_ms
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Core STT processing methods
    // ─────────────────────────────────────────────────────────────────────────────

    /// Fast synchronous check if result should be delivered
    /// Returns true if result should be processed and delivered to callback
    fn should_deliver_result(&self, result: &STTResult) -> bool {
        // Skip empty final results that aren't speech_final
        !(result.transcript.trim().is_empty() && result.is_final && !result.is_speech_final)
    }

    /// Handle turn detection logic asynchronously (non-blocking)
    /// This method spawns background tasks and doesn't block result delivery
    fn handle_turn_detection(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) {
        // Update text buffer and cancel any existing task (person still talking)
        let (buffered_text, is_new_segment) = {
            let mut state = speech_final_state.write();

            // CRITICAL: Cancel old task when new is_final arrives (person still talking)
            if let Some(old_handle) = state.turn_detection_handle.take() {
                debug!(
                    "New is_final arrived - cancelling previous turn detection (person still talking)"
                );
                old_handle.abort();
            }

            state.text_buffer = format!("{}{}", state.text_buffer, result.transcript);

            // Check if this is the first is_final for a new segment
            let is_new = state.segment_start_ms.load(Ordering::Acquire) == 0;

            (state.text_buffer.clone(), is_new)
        };

        // Create and store NEW detection task handle
        let detection_handle = self.create_detection_task(
            result,
            buffered_text,
            speech_final_state.clone(),
            turn_detector,
        );

        let mut state = speech_final_state.write();
        state.turn_detection_handle = Some(detection_handle);
        state
            .waiting_for_speech_final
            .store(true, Ordering::Release);

        // Schedule hard-timeout task if this is a new segment
        if is_new_segment {
            let now_ms = self.get_current_time_ms();
            state.segment_start_ms.store(now_ms, Ordering::Release);

            let deadline_ms = now_ms + self.config.speech_final_hard_timeout_ms as usize;
            state
                .hard_timeout_deadline_ms
                .store(deadline_ms, Ordering::Release);

            debug!(
                "Starting new speech segment - hard timeout will fire in {}ms at {}",
                self.config.speech_final_hard_timeout_ms, deadline_ms
            );

            // Cancel any existing hard timeout task
            if let Some(old_handle) = state.hard_timeout_handle.take() {
                debug!("Cancelling previous hard timeout task");
                old_handle.abort();
            }

            // Spawn hard timeout task
            let hard_timeout_handle =
                self.create_hard_timeout_task(speech_final_state.clone(), now_ms);
            state.hard_timeout_handle = Some(hard_timeout_handle);
        }
    }

    /// Handle a real speech_final result
    fn handle_real_speech_final(
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

        // Cancel any pending detection tasks
        self.cancel_detection_task(&mut state);

        // Reset state for next speech segment
        self.reset_speech_state(&mut state);

        Some(result)
    }

    /// Create a hard timeout task that enforces the maximum wait time
    ///
    /// This task ensures that every speech segment gets a speech_final within
    /// speech_final_hard_timeout_ms (default 5 seconds), regardless of whether
    /// the STT provider sends speech_final or the turn detector confirms.
    ///
    /// # Arguments
    /// * `speech_final_state` - Shared state for speech final tracking
    /// * `segment_start_ms` - When the current speech segment started
    fn create_hard_timeout_task(
        &self,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        segment_start_ms: usize,
    ) -> JoinHandle<()> {
        let hard_timeout_ms = self.config.speech_final_hard_timeout_ms;

        tokio::spawn(async move {
            // Calculate remaining time until hard timeout
            let now_ms = Self::get_current_time_ms_static();
            let elapsed_ms = now_ms.saturating_sub(segment_start_ms);
            let remaining_ms = hard_timeout_ms.saturating_sub(elapsed_ms as u64);

            debug!(
                "Hard timeout scheduled: will fire in {}ms (total timeout: {}ms, elapsed: {}ms)",
                remaining_ms, hard_timeout_ms, elapsed_ms
            );

            // Sleep for the remaining time
            tokio::time::sleep(Duration::from_millis(remaining_ms)).await;

            // Check if we should still fire (not cancelled by real speech_final)
            let should_fire = {
                let state = speech_final_state.read();
                state.waiting_for_speech_final.load(Ordering::Acquire)
            };

            if !should_fire {
                debug!("Hard timeout cancelled - speech_final already fired");
                return;
            }

            // Hard timeout has fired - force speech_final
            let total_wait_ms = Self::get_current_time_ms_static().saturating_sub(segment_start_ms);

            // Get the buffered text before firing
            let buffered_text = {
                let state = speech_final_state.read();
                state.text_buffer.clone()
            };

            tracing::warn!(
                "Hard timeout fired after {}ms - forcing speech_final (no real speech_final or turn detection confirmation received)",
                total_wait_ms
            );

            // Fire speech_final with empty result (we'll use buffered text)
            let forced_result = STTResult {
                transcript: String::new(),
                is_final: true,
                is_speech_final: false,
                confidence: 1.0,
            };

            Self::fire_speech_final(
                forced_result,
                buffered_text,
                speech_final_state,
                "hard_timeout_fallback",
            )
            .await;
        })
    }

    /// Create a detection task that waits for STT provider, then fires as timeout fallback.
    ///
    /// Voice AI Best Practice Logic:
    /// 1. Wait for STT provider to send real speech_final (they see the audio stream)
    /// 2. If STT is silent and text hasn't changed, fire artificial speech_final
    ///
    /// Note: This path does NOT use the TurnDetector because it doesn't have access
    /// to the audio buffer. Turn detection with audio is handled by the VAD path
    /// (`spawn_vad_turn_detection`). This timeout-based path serves as a fallback
    /// when VAD is not available or hasn't fired.
    fn create_detection_task(
        &self,
        result: STTResult,
        buffered_text: String,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        _turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) -> JoinHandle<()> {
        let stt_wait_ms = self.config.stt_speech_final_wait_ms;

        tokio::spawn(async move {
            // PHASE 1: Wait for STT provider to send real speech_final
            // This is the primary path - we trust the STT provider first
            debug!(
                "Waiting {}ms for real speech_final from STT provider",
                stt_wait_ms
            );
            tokio::time::sleep(Duration::from_millis(stt_wait_ms)).await;

            // Check if we should still fire (not cancelled by real speech_final or new is_final)
            let should_continue = {
                let state = speech_final_state.read();
                state.waiting_for_speech_final.load(Ordering::Acquire)
            };

            if !should_continue {
                debug!("Turn detection cancelled - real speech_final arrived or new is_final");
                return;
            }

            // PHASE 2: STT didn't send speech_final - check if text changed
            // Check if text buffer has changed (new transcripts arrived)
            let current_text = {
                let state = speech_final_state.read();
                state.text_buffer.clone()
            };

            // If text changed, someone is still talking - don't fire
            if current_text != buffered_text {
                info!(
                    "Text buffer changed during wait (old: '{}', new: '{}') - person still talking, not firing",
                    buffered_text, current_text
                );
                return;
            }

            // Text hasn't changed - fire based on timeout
            // Note: Turn detection with audio is handled by VAD path, not here
            let detection_method = "timeout_fallback";
            info!(
                "STT silent for {}ms, text unchanged - firing artificial speech_final",
                stt_wait_ms
            );

            // PHASE 3: Fire artificial speech_final
            Self::fire_speech_final(result, buffered_text, speech_final_state, detection_method)
                .await;
        })
    }

    /// Fire a forced speech_final event
    async fn fire_speech_final(
        _result: STTResult,
        buffered_text: String,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        detection_method: &str,
    ) {
        let callback_opt = {
            let state = speech_final_state.read();
            if state.waiting_for_speech_final.load(Ordering::Acquire) {
                state.user_callback.clone()
            } else {
                None
            }
        };

        if let Some(callback) = callback_opt {
            let fire_time_ms = Self::get_current_time_ms_static();

            // Update state before firing callback
            {
                let mut state = speech_final_state.write();
                state
                    .turn_detection_last_fired_ms
                    .store(fire_time_ms, Ordering::Release);
                state.last_forced_text = buffered_text.clone();
                state
                    .waiting_for_speech_final
                    .store(false, Ordering::Release);
                state.turn_detection_handle = None;
                state.text_buffer.clear();

                // Cancel and clear hard timeout handle
                if let Some(handle) = state.hard_timeout_handle.take() {
                    handle.abort();
                }

                // Clear segment timing for next utterance
                state.segment_start_ms.store(0, Ordering::Release);
                state.hard_timeout_deadline_ms.store(0, Ordering::Release);

                // Reset VAD state for next utterance
                state.reset_vad_state();
            }

            let forced_result = STTResult {
                transcript: String::new(),
                is_final: true,
                is_speech_final: true,
                confidence: 1.0,
            };

            info!("Forcing speech_final via {}", detection_method);
            callback(forced_result).await;
        }
    }

    /// Check if this is a duplicate speech_final event
    fn is_duplicate_speech_final(
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

    /// Cancel any existing detection task
    fn cancel_detection_task(&self, state: &mut SpeechFinalState) {
        if let Some(handle) = state.turn_detection_handle.take() {
            debug!("Cancelling pending turn detection task");
            handle.abort();
            state
                .waiting_for_speech_final
                .store(false, Ordering::Release);
        }

        // Also cancel hard timeout handle if present
        if let Some(handle) = state.hard_timeout_handle.take() {
            debug!("Cancelling pending hard timeout task");
            handle.abort();
        }

        // Cancel VAD turn detection handle if present
        if let Some(handle) = state.vad_turn_detection_handle.take() {
            debug!("Cancelling pending VAD turn detection task");
            handle.abort();
        }

        // Reset VAD state
        state.vad_turn_end_detected.store(false, Ordering::Release);
    }

    /// Reset speech state for next segment
    fn reset_speech_state(&self, state: &mut SpeechFinalState) {
        state.text_buffer.clear();
        state.last_forced_text.clear();
        state
            .waiting_for_speech_final
            .store(false, Ordering::Release);
        state
            .turn_detection_last_fired_ms
            .store(0, Ordering::Release);
        state.segment_start_ms.store(0, Ordering::Release);
        state.hard_timeout_deadline_ms.store(0, Ordering::Release);

        // Cancel and clear hard timeout handle if present
        if let Some(handle) = state.hard_timeout_handle.take() {
            handle.abort();
        }

        // Reset VAD state
        state.reset_vad_state();
    }

    /// Get current time in milliseconds
    fn get_current_time_ms(&self) -> usize {
        Self::get_current_time_ms_static()
    }

    /// Static helper to get current time in milliseconds
    fn get_current_time_ms_static() -> usize {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as usize
    }
}

/// Default processor instance with standard configuration
impl Default for STTResultProcessor {
    fn default() -> Self {
        Self::new(STTProcessingConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::voice_manager::callbacks::STTCallback;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn test_hard_timeout_fires_when_no_speech_final() {
        // Test that hard timeout fires after configured duration when no speech_final arrives
        let config = STTProcessingConfig::new(
            50,  // stt_speech_final_wait_ms
            50,  // turn_detection_inference_timeout_ms
            200, // speech_final_hard_timeout_ms - 200ms hard timeout
            100, // duplicate_window_ms
        );

        let processor = STTResultProcessor::new(config);

        // Track callback invocations
        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_speech_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        // Send an is_final result (no speech_final)
        let result = STTResult {
            transcript: "Hello world".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        // Process the result - should trigger turn detection and hard timeout
        let processed = processor.process_result(result, state.clone(), None).await;
        assert!(processed.is_some());

        // Wait for hard timeout to fire (200ms + buffer)
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Hard timeout should have fired the callback
        assert_eq!(
            callback_count.load(Ordering::SeqCst),
            1,
            "Hard timeout should have fired speech_final callback"
        );

        // State should be reset
        let final_state = state.read();
        assert!(!final_state.waiting_for_speech_final.load(Ordering::Acquire));
        assert_eq!(final_state.segment_start_ms.load(Ordering::Acquire), 0);
        assert_eq!(
            final_state.hard_timeout_deadline_ms.load(Ordering::Acquire),
            0
        );
    }

    #[tokio::test]
    async fn test_hard_timeout_cancelled_by_real_speech_final() {
        // Test that hard timeout is cancelled when real speech_final arrives
        let config = STTProcessingConfig::new(
            50,  // stt_speech_final_wait_ms
            50,  // turn_detection_inference_timeout_ms
            500, // speech_final_hard_timeout_ms - Long timeout
            100, // duplicate_window_ms
        );

        let processor = STTResultProcessor::new(config);

        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_speech_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        // Send an is_final result
        let is_final_result = STTResult {
            transcript: "Hello".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        processor
            .process_result(is_final_result, state.clone(), None)
            .await;

        // Wait a bit
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Send real speech_final before hard timeout fires
        let speech_final_result = STTResult {
            transcript: "Hello world".to_string(),
            is_final: true,
            is_speech_final: true,
            confidence: 0.95,
        };

        processor
            .process_result(speech_final_result, state.clone(), None)
            .await;

        // Wait to ensure hard timeout doesn't fire
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Should only have 1 callback (from real speech_final, not hard timeout)
        assert_eq!(
            callback_count.load(Ordering::SeqCst),
            1,
            "Only real speech_final should fire, not hard timeout"
        );
    }

    #[tokio::test]
    async fn test_hard_timeout_not_restarted_by_new_is_final() {
        // Test that hard timeout continues from first is_final when new is_final arrives
        let config = STTProcessingConfig::new(
            300, // stt_speech_final_wait_ms - Long turn detection wait
            50,  // turn_detection_inference_timeout_ms
            200, // speech_final_hard_timeout_ms - Hard timeout fires first
            100, // duplicate_window_ms
        );

        let processor = STTResultProcessor::new(config);

        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_speech_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        // Send first is_final at t=0
        let result1 = STTResult {
            transcript: "Hello".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        processor.process_result(result1, state.clone(), None).await;

        // Wait a bit (but less than hard timeout)
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Send another is_final at t=100ms (person still talking)
        let result2 = STTResult {
            transcript: " world".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        processor.process_result(result2, state.clone(), None).await;

        // Wait for hard timeout (should fire at t=200ms from first is_final)
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Hard timeout should fire once at t=200ms (not restarted by second is_final)
        assert_eq!(
            callback_count.load(Ordering::SeqCst),
            1,
            "Hard timeout should fire once based on first is_final timestamp"
        );
    }

    #[tokio::test]
    async fn test_segment_timing_reset_after_speech_final() {
        // Test that segment timing is properly reset after speech_final
        let config = STTProcessingConfig::default();
        let processor = STTResultProcessor::new(config);

        let callback_count = Arc::new(AtomicUsize::new(0));
        let callback_count_clone = callback_count.clone();

        let callback: STTCallback = Arc::new(move |result: STTResult| {
            let count = callback_count_clone.clone();
            Box::pin(async move {
                if result.is_speech_final {
                    count.fetch_add(1, Ordering::SeqCst);
                }
            }) as Pin<Box<dyn Future<Output = ()> + Send>>
        });

        let state = Arc::new(SyncRwLock::new(SpeechFinalState::with_callback(callback)));

        // Send is_final
        let result = STTResult {
            transcript: "First utterance".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        processor.process_result(result, state.clone(), None).await;

        // Verify segment timing was set
        {
            let s = state.read();
            assert_ne!(s.segment_start_ms.load(Ordering::Acquire), 0);
            assert_ne!(s.hard_timeout_deadline_ms.load(Ordering::Acquire), 0);
        }

        // Send real speech_final
        let speech_final = STTResult {
            transcript: "First utterance complete".to_string(),
            is_final: true,
            is_speech_final: true,
            confidence: 0.95,
        };

        processor
            .process_result(speech_final, state.clone(), None)
            .await;

        // Verify segment timing was reset
        {
            let s = state.read();
            assert_eq!(s.segment_start_ms.load(Ordering::Acquire), 0);
            assert_eq!(s.hard_timeout_deadline_ms.load(Ordering::Acquire), 0);
        }

        // Send new is_final for next utterance
        let result2 = STTResult {
            transcript: "Second utterance".to_string(),
            is_final: true,
            is_speech_final: false,
            confidence: 0.95,
        };

        processor.process_result(result2, state.clone(), None).await;

        // Verify segment timing was set again
        {
            let s = state.read();
            assert_ne!(s.segment_start_ms.load(Ordering::Acquire), 0);
            assert_ne!(s.hard_timeout_deadline_ms.load(Ordering::Acquire), 0);
        }
    }
}
