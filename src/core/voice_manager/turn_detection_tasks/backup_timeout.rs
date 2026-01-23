//! Backup timeout task for forcing speech_final after prolonged silence.
//!
//! When Smart-Turn returns Incomplete, this backup timeout ensures that
//! a speech_final is eventually emitted if the user remains silent for
//! an extended period.

use parking_lot::RwLock as SyncRwLock;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::debug;

use crate::core::vad::SilenceTracker;

use super::super::state::SpeechFinalState;
use super::super::vad_processor::VADState;
use super::speech_final::fire_speech_final;

/// Spawn a backup timeout task that forces speech_final after prolonged silence.
///
/// This task sleeps for `timeout_ms`, then checks if we're still waiting for
/// speech_final. If so, it fires a forced speech_final, resets the silence
/// tracker, and clears the audio buffer.
///
/// # Arguments
/// * `timeout_ms` - Duration to wait before forcing speech_final (in milliseconds)
/// * `buffered_text` - The accumulated text to include in the speech_final event
/// * `speech_final_state` - Shared state for tracking speech_final status
/// * `silence_tracker` - The silence tracker to reset after firing
/// * `vad_state` - The VAD state containing the audio buffer to clear
///
/// # Returns
/// A `JoinHandle` that can be used to abort the task if speech resumes.
pub fn spawn_backup_timeout_task(
    timeout_ms: u64,
    buffered_text: String,
    speech_final_state: Arc<SyncRwLock<SpeechFinalState>>,
    silence_tracker: Arc<SilenceTracker>,
    vad_state: Arc<VADState>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        debug!(
            "Backup timeout: Sleeping for {}ms before forcing speech_final",
            timeout_ms
        );

        tokio::time::sleep(Duration::from_millis(timeout_ms)).await;

        // Check if we're still waiting for speech_final
        let should_fire = {
            let state = speech_final_state.read();
            state.waiting_for_speech_final.load(Ordering::Acquire)
        };

        if !should_fire {
            debug!("Backup timeout: No longer waiting for speech_final - cancelled");
            return;
        }

        debug!(
            "Backup timeout: Forcing speech_final after {}ms silence",
            timeout_ms
        );

        fire_speech_final(buffered_text, speech_final_state, "backup_silence_timeout").await;

        // Reset silence tracker and clear audio buffer
        silence_tracker.reset();
        vad_state.clear_audio_buffer();
        debug!("Backup timeout: Reset silence tracker and cleared audio buffer");
    })
}
