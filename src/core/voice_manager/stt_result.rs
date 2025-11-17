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
    /// Time to wait for STT provider to send real speech_final (ms)
    /// This is the primary window - we trust STT provider during this time
    pub stt_speech_final_wait_ms: u64,
    /// Maximum time to wait for turn detection inference to complete (ms)
    pub turn_detection_inference_timeout_ms: u64,
    /// Hard upper bound timeout for any user utterance (ms)
    /// This guarantees that no utterance will wait longer than this value
    /// even if neither the STT provider nor turn detector fire
    pub speech_final_hard_timeout_ms: u64,
    /// Window to prevent duplicate speech_final events (ms)
    pub duplicate_window_ms: usize,
}

impl Default for STTProcessingConfig {
    fn default() -> Self {
        Self {
            stt_speech_final_wait_ms: 2000, // Wait 2s for real speech_final from STT
            turn_detection_inference_timeout_ms: 100, // 100ms max for model inference
            speech_final_hard_timeout_ms: 5000, // 5s hard upper bound for any utterance
            duplicate_window_ms: 500,       // 500ms duplicate prevention window
        }
    }
}

impl STTProcessingConfig {
    /// Create a new STTProcessingConfig with explicit timeout values
    pub fn new(
        stt_speech_final_wait_ms: u64,
        turn_detection_inference_timeout_ms: u64,
        speech_final_hard_timeout_ms: u64,
        duplicate_window_ms: usize,
    ) -> Self {
        Self {
            stt_speech_final_wait_ms,
            turn_detection_inference_timeout_ms,
            speech_final_hard_timeout_ms,
            duplicate_window_ms,
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

    /// Create a detection task that waits for STT provider, then uses turn detection as fallback
    ///
    /// Voice AI Best Practice Logic:
    /// 1. Wait for STT provider to send real speech_final (they see the audio stream)
    /// 2. If STT is silent and text hasn't changed, run turn detection to confirm
    /// 3. Only fire artificial speech_final if turn detection confirms turn is complete
    fn create_detection_task(
        &self,
        result: STTResult,
        buffered_text: String,
        speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
        turn_detector: Option<Arc<RwLock<TurnDetector>>>,
    ) -> JoinHandle<()> {
        let stt_wait_ms = self.config.stt_speech_final_wait_ms;
        let inference_timeout_ms = self.config.turn_detection_inference_timeout_ms;

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

            // PHASE 2: STT didn't send speech_final - verify with turn detection
            let detection_method = if let Some(detector) = turn_detector {
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

                // Text hasn't changed - run turn detection to confirm turn is complete
                debug!(
                    "STT silent for {}ms, running turn detection to confirm",
                    stt_wait_ms
                );
                let turn_result =
                    tokio::time::timeout(Duration::from_millis(inference_timeout_ms), async {
                        let detector_guard = detector.read().await;
                        detector_guard.is_turn_complete(&current_text).await
                    })
                    .await;

                match turn_result {
                    Ok(Ok(true)) => {
                        info!(
                            "Turn detection confirms turn complete - firing artificial speech_final"
                        );
                        "turn_detection_confirmed"
                    }
                    Ok(Ok(false)) => {
                        info!(
                            "Turn detection says turn incomplete - not firing (person may still be thinking)"
                        );
                        return; // Don't fire - person may continue speaking
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(
                            "Turn detection error: {:?} - firing as fallback ({}ms silence)",
                            e,
                            stt_wait_ms
                        );
                        "turn_detection_error_fallback"
                    }
                    Err(_) => {
                        tracing::warn!(
                            "Turn detection inference timeout after {}ms - firing as fallback",
                            inference_timeout_ms
                        );
                        "inference_timeout_fallback"
                    }
                }
            } else {
                // No turn detector - fire based on silence duration alone
                info!("No turn detector - firing after {}ms silence", stt_wait_ms);
                "no_detector_timeout"
            };

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
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    #[tokio::test]
    async fn test_hard_timeout_fires_when_no_speech_final() {
        // Test that hard timeout fires after configured duration when no speech_final arrives
        let config = STTProcessingConfig {
            stt_speech_final_wait_ms: 50,
            turn_detection_inference_timeout_ms: 50,
            speech_final_hard_timeout_ms: 200, // 200ms hard timeout
            duplicate_window_ms: 100,
        };

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

        let state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::new(),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: Some(callback),
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::new(),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

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
        let config = STTProcessingConfig {
            stt_speech_final_wait_ms: 50,
            turn_detection_inference_timeout_ms: 50,
            speech_final_hard_timeout_ms: 500, // Long timeout
            duplicate_window_ms: 100,
        };

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

        let state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::new(),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: Some(callback),
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::new(),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

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
        let config = STTProcessingConfig {
            stt_speech_final_wait_ms: 300, // Long turn detection wait
            turn_detection_inference_timeout_ms: 50,
            speech_final_hard_timeout_ms: 200, // Hard timeout fires first
            duplicate_window_ms: 100,
        };

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

        let state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::new(),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: Some(callback),
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::new(),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

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

        let state = Arc::new(SyncRwLock::new(SpeechFinalState {
            text_buffer: String::new(),
            turn_detection_handle: None,
            hard_timeout_handle: None,
            waiting_for_speech_final: AtomicBool::new(false),
            user_callback: Some(callback),
            turn_detection_last_fired_ms: AtomicUsize::new(0),
            last_forced_text: String::new(),
            segment_start_ms: AtomicUsize::new(0),
            hard_timeout_deadline_ms: AtomicUsize::new(0),
        }));

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
