//! Audio processing handler for WebSocket connections
//!
//! This module handles all audio-related operations including:
//! - Processing incoming audio data from clients
//! - Routing audio through STT (Speech-to-Text) providers
//! - Managing TTS (Text-to-Speech) synthesis requests
//! - Handling audio clear/interruption commands

use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error, info};

use crate::core::voice_manager::VoiceManager;

use super::{
    messages::{MessageRoute, OutgoingMessage},
    state::ConnectionState,
};

/// Handle incoming audio data with zero-copy optimizations
///
/// Processes raw audio data received from WebSocket clients and forwards it
/// to the configured STT provider for transcription.
///
/// # Arguments
/// * `audio_data` - Raw audio bytes received from the client
/// * `state` - Connection state containing voice manager and configuration
/// * `message_tx` - Channel for sending response messages back to the client
///
/// # Returns
/// * `bool` - true to continue processing, false to terminate connection
///
/// # Performance Notes
/// - Uses read-only lock for fast state access
/// - Zero-copy data passing where possible
/// - Marked inline for hot path optimization
#[inline(always)]
pub async fn handle_audio_message(
    audio_data: Bytes,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    debug!("Processing audio data: {} bytes", audio_data.len());

    // Fast path: read lock to check state and get voice manager
    let voice_manager = {
        let state_guard = state.read().await;

        // Check if audio processing is enabled (atomic read, no lock overhead)
        if !state_guard.is_audio_enabled() {
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message:
                        "Audio processing is disabled. Send config message with audio=true first."
                            .to_string(),
                }))
                .await;
            return true;
        }

        match &state_guard.voice_manager {
            Some(vm) => vm.clone(),
            None => {
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: "Voice manager not configured. Send config message with audio=true first."
                            .to_string(),
                    }))
                    .await;
                return true;
            }
        }
    };

    // Direct pass-through without unnecessary allocation
    // The Bytes type already provides efficient cloning and slicing

    // Send audio to STT provider with zero-copy optimization
    // Converting to Vec only when needed by the provider
    if let Err(e) = voice_manager.receive_audio(audio_data.to_vec()).await {
        error!("Failed to process audio: {}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to process audio: {e}"),
            }))
            .await;
    }

    true
}

/// Handle text-to-speech synthesis request
///
/// Processes speak commands to synthesize text into audio using the configured
/// TTS provider. Supports queuing, flushing, and interruption control.
///
/// # Arguments
/// * `text` - Text to synthesize into speech
/// * `flush` - Whether to clear the TTS queue before speaking (default: true)
/// * `allow_interruption` - Whether this audio can be interrupted (default: true)
/// * `state` - Connection state containing voice manager
/// * `message_tx` - Channel for sending response messages
///
/// # Returns
/// * `bool` - true to continue processing, false to terminate connection
pub async fn handle_speak_message(
    text: String,
    flush: Option<bool>,
    allow_interruption: Option<bool>,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    // Default flush to true for backward compatibility
    let should_flush = flush.unwrap_or(true);
    // Default allow_interruption to true for backward compatibility
    let allow_interruption = allow_interruption.unwrap_or(true);

    debug!(
        "Processing speak command: {} chars (flush: {}, allow_interruption: {})",
        text.len(),
        should_flush,
        allow_interruption
    );

    // Fast path: read lock to check state and get voice manager
    let voice_manager = match get_voice_manager_if_audio_enabled(state, message_tx).await {
        Some(vm) => vm,
        None => return true,
    };

    info!(
        "Speaking text (flush: {}, allow_interruption: {}): {}",
        should_flush, allow_interruption, text
    );

    // Send text to TTS provider with flush and allow_interruption parameters
    if let Err(e) = voice_manager
        .speak_with_interruption(&text, should_flush, allow_interruption)
        .await
    {
        error!("Failed to synthesize speech: {}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to synthesize speech: {e}"),
            }))
            .await;
    } else {
        debug!(
            "Speech synthesis started for: {} chars (flush: {}, allow_interruption: {})",
            text.len(),
            should_flush,
            allow_interruption
        );
    }

    true
}

/// Handle audio clear/interruption command
///
/// Clears the TTS queue and any pending audio. Respects non-interruptible
/// audio playback settings.
///
/// # Arguments
/// * `state` - Connection state containing voice and LiveKit managers
/// * `message_tx` - Channel for sending response messages
///
/// # Returns
/// * `bool` - true to continue processing, false to terminate connection
pub async fn handle_clear_message(
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    debug!("Processing clear command");

    // Fast path: read lock to get both managers
    let (voice_manager, livekit_client) = {
        let state_guard = state.read().await;

        // Check if audio processing is enabled for voice manager operations
        let vm = if state_guard.is_audio_enabled() {
            match &state_guard.voice_manager {
                Some(vm) => Some(vm.clone()),
                None => {
                    let _ = message_tx
                        .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                            message: "Voice manager not configured. Send config message with audio=true first."
                                .to_string(),
                        }))
                        .await;
                    return true;
                }
            }
        } else {
            // Audio is disabled, so voice manager operations are not available
            None
        };

        let lk = state_guard.livekit_client.clone();
        (vm, lk)
    };

    // Check if we're in a non-interruptible state
    let is_blocked = if let Some(ref vm) = voice_manager {
        vm.is_interruption_blocked().await
    } else {
        false
    };

    if is_blocked {
        debug!("Clear command ignored - currently in non-interruptible audio playback");
        return true;
    }

    // Clear TTS provider and audio buffers (only if audio is enabled)
    // Note: The VoiceManager's clear_tts() will automatically call the audio_clear_callback
    // which clears the LiveKit audio buffer, so we don't need to do it separately
    if let Some(vm) = voice_manager {
        if let Err(e) = vm.clear_tts().await {
            error!("Failed to clear TTS provider: {}", e);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: format!("Failed to clear TTS provider: {e}"),
                }))
                .await;
        } else {
            debug!("Successfully cleared TTS and audio buffers");
        }
    } else {
        debug!("Audio processing disabled - skipping TTS provider clear");

        // If audio is disabled but LiveKit is configured, still clear LiveKit audio
        if let Some(livekit_manager) = livekit_client {
            // Use write() to wait for the lock - clear operation is important
            let mut client = livekit_manager.write().await;
            match client.clear_audio().await {
                Ok(()) => {
                    debug!("Successfully cleared LiveKit audio buffer (audio disabled mode)");
                }
                Err(e) => {
                    error!("Failed to clear LiveKit audio buffer: {}", e);
                }
            }
        }
    }

    debug!("Clear command completed");
    true
}

/// Helper function to get voice manager if audio is enabled
///
/// Checks if audio processing is enabled and returns the voice manager if available.
/// Sends appropriate error messages if audio is disabled or voice manager is not configured.
///
/// # Arguments
/// * `state` - Connection state to check
/// * `message_tx` - Channel for sending error messages
///
/// # Returns
/// * `Option<Arc<VoiceManager>>` - Voice manager if available, None otherwise
async fn get_voice_manager_if_audio_enabled(
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> Option<Arc<VoiceManager>> {
    let state_guard = state.read().await;

    // Check if audio processing is enabled (atomic read, no lock overhead)
    if !state_guard.is_audio_enabled() {
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message:
                    "Audio processing is disabled. Send config message with audio=true first."
                        .to_string(),
            }))
            .await;
        return None;
    }

    match &state_guard.voice_manager {
        Some(vm) => Some(vm.clone()),
        None => {
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: "Voice manager not configured. Send config message with audio=true first."
                        .to_string(),
                }))
                .await;
            None
        }
    }
}