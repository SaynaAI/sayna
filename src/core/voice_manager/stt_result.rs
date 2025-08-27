//! STT result processing with timing control

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::{debug, info};

use crate::core::{stt::STTResult, turn_detect::TurnDetector};

use super::state::SpeechFinalState;

/// Configuration for STT result processing
#[derive(Clone, Copy)]
pub struct STTProcessingConfig {
    /// Maximum time to wait for turn detection before falling back to timer (ms)
    pub turn_detection_timeout_ms: u64,
    /// Window to prevent duplicate speech_final events (ms)
    pub duplicate_window_ms: usize,
}

impl Default for STTProcessingConfig {
    fn default() -> Self {
        Self {
            turn_detection_timeout_ms: 2100, // 2.1 seconds max wait for turn detection
            duplicate_window_ms: 500,        // 500ms duplicate prevention window
        }
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
    /// - Turn detection ML model with timer fallback
    /// - Automatic timer after 5 seconds if turn detection doesn't trigger
    /// - Prevention of duplicate speech_final events
    pub async fn process_result(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) -> Option<STTResult> {
        // Skip empty final results that aren't speech_final
        if result.transcript.trim().is_empty() && result.is_final && !result.is_speech_final {
            return None;
        }

        let now_ms = self.get_current_time_ms();

        // Handle real speech_final
        if result.is_speech_final {
            return self.handle_real_speech_final(result, speech_final_state, now_ms);
        }

        // Update text buffer for non-empty transcripts
        if !result.transcript.trim().is_empty() {
            let mut state = speech_final_state.write();

            // Cancel existing detection task when we get something from the STT
            // This means the user probably did not finish their speech or is stopped speaking
            self.cancel_detection_task(&mut state);
        }

        // Handle is_final (but not speech_final)
        if result.is_final {
            {
                let mut state = speech_final_state.write();
                state.text_buffer = format!("{}{}", state.text_buffer, result.transcript);
            }
            self.handle_is_final(result.clone(), speech_final_state, turn_detector)
                .await;
        }

        // Always return the original result immediately
        Some(result)
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

    /// Handle is_final result (non-speech_final)
    async fn handle_is_final(
        &self,
        result: STTResult,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) {
        let mut state = speech_final_state.write();

        let buffered_text = state.text_buffer.clone();

        // Create detection task with both turn detection and timer fallback
        let detection_handle = self.create_detection_task(
            result,
            buffered_text,
            speech_final_state.clone(),
            turn_detector,
        );

        state.turn_detection_handle = Some(detection_handle);
        state
            .waiting_for_speech_final
            .store(true, Ordering::Release);
    }

    /// Create a detection task that runs both turn detection and timer simultaneously
    /// The timer acts as a guaranteed maximum timeout (5 seconds) regardless of turn detection
    fn create_detection_task(
        &self,
        result: STTResult,
        buffered_text: String,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) -> JoinHandle<()> {
        let max_timeout_ms = self.config.turn_detection_timeout_ms;

        tokio::spawn(async move {
            let detection_method;

            if let Some(detector) = turn_detector {
                // Run turn detection
                let turn_detection_future = async {
                    let detector_guard = detector.read().await;
                    match detector_guard.is_turn_complete(&buffered_text).await {
                        Ok(true) => {
                            info!("Turn detection triggered speech_final");
                            Some("turn_detection")
                        }
                        Ok(false) => {
                            debug!("Turn detection returned false");
                            None
                        }
                        Err(e) => {
                            debug!("Turn detection failed: {:?}", e);
                            None
                        }
                    }
                };

                // Timer future - always runs for max_timeout_ms
                let timer_future = async {
                    tokio::time::sleep(Duration::from_millis(max_timeout_ms)).await;
                    info!(
                        "Max timeout after {}ms, forcing speech_final",
                        max_timeout_ms
                    );
                    "max_timeout"
                };

                // Race both futures - first one to complete wins
                tokio::select! {
                    // If turn detection completes first and returns Some (positive result)
                    Some(method) = turn_detection_future => {
                        detection_method = method;
                    }
                    // If timer completes first
                    method = timer_future => {
                        detection_method = method;
                    }
                }
            } else {
                // No turn detector available, use max timeout only
                tokio::time::sleep(Duration::from_millis(max_timeout_ms)).await;
                detection_method = "timer_only";
                info!("No turn detector, using timer only");
            }

            // Always fire the speech_final after one of the conditions above
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
            handle.abort();
        }
    }

    /// Reset speech state for next segment
    fn reset_speech_state(&self, state: &mut SpeechFinalState) {
        state.text_buffer.clear();
        state.last_text.clear();
        state.last_forced_text.clear();
        state
            .waiting_for_speech_final
            .store(false, Ordering::Release);
        state
            .turn_detection_last_fired_ms
            .store(0, Ordering::Release);
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
