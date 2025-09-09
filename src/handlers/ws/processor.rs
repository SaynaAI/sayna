//! WebSocket message processing logic
//!
//! This module contains all the message processing functions for handling
//! WebSocket messages, including configuration, audio, speak, clear, and send_message commands.

use base64::{Engine as _, engine::general_purpose};
use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio::time::{Duration, timeout};
use tracing::{debug, error, info, warn};

use crate::{
    core::{
        stt::STTResult,
        tts::AudioData,
        voice_manager::{VoiceManager, VoiceManagerConfig},
    },
    livekit::LiveKitClient,
    state::AppState,
};

use super::{
    config::{
        LiveKitWebSocketConfig, STTWebSocketConfig, TTSWebSocketConfig, compute_tts_config_hash,
    },
    messages::{
        IncomingMessage, MessageRoute, OutgoingMessage, ParticipantDisconnectedInfo, UnifiedMessage,
    },
    state::ConnectionState,
};

/// Handle configuration message with optimizations
pub async fn handle_config_message(
    audio: Option<bool>,
    stt_ws_config: Option<STTWebSocketConfig>,
    tts_ws_config: Option<TTSWebSocketConfig>,
    livekit_ws_config: Option<LiveKitWebSocketConfig>,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    // Determine if audio processing is enabled (default to true)
    let audio_enabled = audio.unwrap_or(true);

    info!(
        "Configuring connection with audio_enabled: {}, LiveKit: {}",
        audio_enabled,
        livekit_ws_config.is_some()
    );

    let livekit_url = app_state.config.livekit_url.clone();

    // Validate that required configs are provided when audio is enabled
    if audio_enabled {
        if stt_ws_config.is_none() {
            let error_msg = "STT configuration is required when audio=true".to_string();
            error!("{}", error_msg);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: error_msg,
                }))
                .await;
            return true;
        }

        if tts_ws_config.is_none() {
            let error_msg = "TTS configuration is required when audio=true".to_string();
            error!("{}", error_msg);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: error_msg,
                }))
                .await;
            return true;
        }
    }

    // Store audio_enabled flag in connection state first
    {
        let state_guard = state.read().await;
        state_guard.set_audio_enabled(audio_enabled);
    }

    // Initialize voice manager only if audio is enabled
    let voice_manager = if audio_enabled {
        let stt_ws_config_ref = stt_ws_config.as_ref().unwrap(); // Safe to unwrap after validation
        let tts_ws_config_ref = tts_ws_config.as_ref().unwrap(); // Safe to unwrap after validation

        info!(
            "Initializing voice manager with STT provider: {} and TTS provider: {}",
            stt_ws_config_ref.provider, tts_ws_config_ref.provider
        );

        // Get API keys from server config using utility function
        let stt_api_key = match app_state.config.get_api_key(&stt_ws_config_ref.provider) {
            Ok(key) => key,
            Err(error_msg) => {
                error!("{}", error_msg);
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: error_msg,
                    }))
                    .await;
                return true;
            }
        };

        let tts_api_key = match app_state.config.get_api_key(&tts_ws_config_ref.provider) {
            Ok(key) => key,
            Err(error_msg) => {
                error!("{}", error_msg);
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: error_msg,
                    }))
                    .await;
                return true;
            }
        };

        // Create full configs with API keys
        let stt_config = stt_ws_config_ref.to_stt_config(stt_api_key);
        let tts_config = tts_ws_config_ref.to_tts_config(tts_api_key);

        // Create voice manager configuration
        let voice_config = VoiceManagerConfig {
            stt_config,
            tts_config,
        };

        let turn_detector = app_state.core_state.get_turn_detector();

        // Create voice manager
        let voice_manager = match VoiceManager::new(voice_config, turn_detector) {
            Ok(vm) => Arc::new(vm),
            Err(e) => {
                error!("Failed to create voice manager: {}", e);
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: format!("Failed to create voice manager: {e}"),
                    }))
                    .await;
                return true;
            }
        };

        // Inject cache and precomputed TTS config hash for caching
        {
            let tts_cfg = voice_manager.get_config().tts_config.clone();
            let cfg_hash = compute_tts_config_hash(&tts_cfg);

            // Provide cache from AppState
            if let Err(e) = voice_manager
                .set_tts_cache(app_state.cache(), Some(cfg_hash))
                .await
            {
                error!("Failed to set TTS cache: {}", e);
            }
        }

        // Start voice manager
        if let Err(e) = voice_manager.start().await {
            error!("Failed to start voice manager: {}", e);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: format!("Failed to start voice manager: {e}"),
                }))
                .await;
            return true;
        }

        // Set up STT callback with optimized message routing
        let message_tx_clone = message_tx.clone();
        if let Err(e) = voice_manager
            .on_stt_result(move |result: STTResult| {
                let message_tx = message_tx_clone.clone();
                Box::pin(async move {
                    let msg = OutgoingMessage::STTResult {
                        transcript: result.transcript,
                        is_final: result.is_final,
                        is_speech_final: result.is_speech_final,
                        confidence: result.confidence,
                    };
                    let _ = message_tx.send(MessageRoute::Outgoing(msg)).await;
                })
            })
            .await
        {
            error!("Failed to set up STT callback: {}", e);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: format!("Failed to set up STT callback: {e}"),
                }))
                .await;
            return true;
        }

        // Set up TTS error callback
        let message_tx_clone = message_tx.clone();
        if let Err(e) = voice_manager
            .on_tts_error(move |error| {
                let message_tx = message_tx_clone.clone();
                Box::pin(async move {
                    let msg = OutgoingMessage::Error {
                        message: format!("TTS error: {error}"),
                    };
                    let _ = message_tx.send(MessageRoute::Outgoing(msg)).await;
                })
            })
            .await
        {
            error!("Failed to set up TTS error callback: {}", e);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: format!("Failed to set up TTS error callback: {e}"),
                }))
                .await;
            return true;
        }

        // Wait for providers to be ready with timeout
        let ready_timeout = Duration::from_secs(30);
        let ready_check = async {
            while !voice_manager.is_ready().await {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };

        if timeout(ready_timeout, ready_check).await.is_err() {
            error!("Timeout waiting for voice providers to be ready");
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: "Timeout waiting for voice providers to be ready".to_string(),
                }))
                .await;
            return true;
        }

        // Store voice manager in state
        {
            let mut state_guard = state.write().await;
            state_guard.voice_manager = Some(voice_manager.clone());
        }

        Some(voice_manager)
    } else {
        info!("Audio processing disabled - skipping voice manager initialization");
        None
    };

    // Set up an early TTS audio callback to catch any audio that arrives
    // before LiveKit is fully ready. This is critical for cached audio that returns immediately.
    if let Some(voice_manager_ref) = &voice_manager {
        let message_tx_for_early_tts = message_tx.clone();

        if let Err(e) = voice_manager_ref
            .on_tts_audio(move |audio_data: AudioData| {
                let message_tx = message_tx_for_early_tts.clone();

                Box::pin(async move {
                    debug!(
                        "Early TTS audio callback triggered: {} bytes",
                        audio_data.data.len()
                    );

                    // Send audio as binary data to WebSocket
                    let audio_bytes = Bytes::from(audio_data.data);
                    let _ = message_tx.send(MessageRoute::Binary(audio_bytes)).await;
                })
            })
            .await
        {
            warn!("Failed to register early TTS audio callback: {:?}", e);
        }
    }

    // Set up LiveKit client if configuration is provided
    let livekit_client_arc: Option<Arc<RwLock<LiveKitClient>>> =
        if let Some(livekit_ws_config) = livekit_ws_config {
            info!("Setting up LiveKit client with URL: {}", livekit_url);

            // Use default TTS config for LiveKit audio parameters when TTS is not configured
            let default_tts_config = TTSWebSocketConfig {
                provider: "deepgram".to_string(),
                voice_id: None,
                speaking_rate: None,
                audio_format: None,
                sample_rate: Some(24000), // Default sample rate for LiveKit
                connection_timeout: None,
                request_timeout: None,
                model: "".to_string(),
            };

            let tts_config_for_livekit = tts_ws_config.as_ref().unwrap_or(&default_tts_config);
            let livekit_config =
                livekit_ws_config.to_livekit_config(tts_config_for_livekit, &livekit_url);
            let mut livekit_client = LiveKitClient::new(livekit_config);

            // Set up LiveKit audio callback to forward audio to STT processing (only if audio is enabled)
            if let Some(voice_manager_ref) = &voice_manager {
                let voice_manager_clone = voice_manager_ref.clone();
                let message_tx_clone = message_tx.clone();

                livekit_client.set_audio_callback(move |audio_data: Vec<u8>| {
                    let voice_manager = voice_manager_clone.clone();
                    let message_tx = message_tx_clone.clone();

                    // Direct processing - spawn lightweight task for async processing
                    tokio::spawn(async move {
                        debug!("Received LiveKit audio: {} bytes", audio_data.len());

                        // Forward LiveKit audio to the same STT processing pipeline
                        if let Err(e) = voice_manager.receive_audio(audio_data).await {
                            error!("Failed to process LiveKit audio: {:?}", e);
                            let _ = message_tx
                                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                                    message: format!("Failed to process LiveKit audio: {e:?}"),
                                }))
                                .await;
                        }
                    });
                });
            } else {
                info!("Audio processing disabled - LiveKit audio callback not set");
            }

            // Set up LiveKit data callback to forward data messages to WebSocket client
            let message_tx_clone = message_tx.clone();
            livekit_client.set_data_callback(move |data_message| {
                let message_tx = message_tx_clone.clone();

                // Spawn task for async send to ensure delivery
                tokio::spawn(async move {
                    debug!(
                        "Received LiveKit data from {}: {} bytes",
                        data_message.participant_identity,
                        data_message.data.len()
                    );

                    // Create unified message structure for LiveKit data
                    let unified_message =
                        if let Ok(text_message) = String::from_utf8(data_message.data.clone()) {
                            // Data can be decoded as UTF-8 text
                            UnifiedMessage {
                                message: Some(text_message),
                                data: None,
                                identity: data_message.participant_identity,
                                topic: data_message.topic.unwrap_or_else(|| "default".to_string()),
                                room: "livekit".to_string(), // TODO: Get actual room name from LiveKit client
                                timestamp: data_message.timestamp,
                            }
                        } else {
                            // Binary data - encode as base64
                            UnifiedMessage {
                                message: None,
                                data: Some(general_purpose::STANDARD.encode(&data_message.data)),
                                identity: data_message.participant_identity,
                                topic: data_message.topic.unwrap_or_else(|| "default".to_string()),
                                room: "livekit".to_string(), // TODO: Get actual room name from LiveKit client
                                timestamp: data_message.timestamp,
                            }
                        };

                    let outgoing_msg = OutgoingMessage::Message {
                        message: unified_message,
                    };

                    // Use send for guaranteed delivery
                    let _ = message_tx.send(MessageRoute::Outgoing(outgoing_msg)).await;
                });
            });

            // Set up LiveKit participant disconnect callback to forward events to WebSocket client
            let message_tx_clone = message_tx.clone();
            livekit_client.set_participant_disconnect_callback(move |disconnect_event| {
                let message_tx = message_tx_clone.clone();

                // Spawn task for async send to ensure delivery
                tokio::spawn(async move {
                    debug!(
                        "Participant {} disconnected from LiveKit room {}",
                        disconnect_event.participant_identity, disconnect_event.room_name
                    );

                    // Create participant disconnected info for WebSocket client
                    let participant_info = ParticipantDisconnectedInfo {
                        identity: disconnect_event.participant_identity,
                        name: disconnect_event.participant_name,
                        room: disconnect_event.room_name,
                        timestamp: disconnect_event.timestamp,
                    };

                    let outgoing_msg = OutgoingMessage::ParticipantDisconnected {
                        participant: participant_info,
                    };

                    // Use send for guaranteed delivery
                    let _ = message_tx.send(MessageRoute::Outgoing(outgoing_msg)).await;
                });
            });

            // Connect to LiveKit room
            if let Err(e) = livekit_client.connect().await {
                error!("Failed to connect to LiveKit room: {:?}", e);
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: format!("Failed to connect to LiveKit room: {e:?}"),
                    }))
                    .await;
                return true;
            }

            // Wait for audio source to be available (max 10 seconds)
            let mut wait_count = 0;
            const MAX_WAIT_MS: u64 = 10000;
            const POLL_INTERVAL_MS: u64 = 100;

            while wait_count * POLL_INTERVAL_MS < MAX_WAIT_MS {
                // Check if connected and audio source is available
                if livekit_client.is_connected().await && livekit_client.has_audio_source().await {
                    info!(
                        "LiveKit audio source is ready after {}ms",
                        wait_count * POLL_INTERVAL_MS
                    );
                    break;
                }

                tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
                wait_count += 1;
            }

            // Final check
            if !livekit_client.is_connected().await || !livekit_client.has_audio_source().await {
                warn!(
                    "LiveKit connected but audio source not available after {}ms wait",
                    MAX_WAIT_MS
                );
                // Continue anyway - audio will be routed through WebSocket as fallback
            }

            // Store LiveKit client in state
            let livekit_client_arc = Arc::new(RwLock::new(livekit_client));
            {
                let mut state_guard = state.write().await;
                state_guard.livekit_client = Some(livekit_client_arc.clone());
            }

            // Register audio clear callback with VoiceManager if both are available
            if let Some(voice_manager_ref) = &voice_manager {
                let livekit_client_clone = livekit_client_arc.clone();

                if let Err(e) = voice_manager_ref
                    .on_audio_clear(move || {
                        let livekit = livekit_client_clone.clone();
                        Box::pin(async move {
                            // Use try_write to avoid blocking in callback
                            // If we can't get the lock, it's okay - audio will be cleared eventually
                            match livekit.try_write() {
                                Ok(mut client) => {
                                    if let Err(e) = client.clear_audio().await {
                                        warn!("Failed to clear LiveKit audio buffer: {:?}", e);
                                    } else {
                                        debug!("Cleared LiveKit audio buffer during interruption");
                                    }
                                }
                                Err(_) => {
                                    debug!("LiveKit client busy during audio clear - skipping");
                                }
                            }
                        })
                    })
                    .await
                {
                    warn!("Failed to register audio clear callback: {:?}", e);
                }
            }

            info!("LiveKit client connected and ready");
            Some(livekit_client_arc)
        } else {
            None
        };

    // Replace the early TTS callback with the final one that includes LiveKit routing
    // This ensures that cached audio that arrives very quickly gets sent to LiveKit
    if let Some(voice_manager_ref) = &voice_manager {
        let message_tx_for_tts = message_tx.clone();
        let livekit_client_for_tts = livekit_client_arc.clone();

        if let Err(e) = voice_manager_ref
            .on_tts_audio(move |audio_data: AudioData| {
                let message_tx = message_tx_for_tts.clone();
                let livekit_client = livekit_client_for_tts.clone();

                Box::pin(async move {
                    let mut sent_to_livekit = false;

                    // Try to send to LiveKit first if available
                    if let Some(livekit_client_arc) = &livekit_client {
                        // Use Tokio-native async lock with timeout
                        // This will wait up to 50ms for the lock to become available
                        const LOCK_TIMEOUT_MS: u64 = 100;

                        match tokio::time::timeout(
                            tokio::time::Duration::from_millis(LOCK_TIMEOUT_MS),
                            livekit_client_arc.write()
                        ).await {
                            Ok(mut client) => {
                                // Check if LiveKit is connected before attempting to send
                                if client.is_connected().await {
                                    match client.send_tts_audio(audio_data.data.clone()).await {
                                        Ok(()) => {
                                            debug!(
                                                "TTS audio successfully sent to LiveKit: {} bytes",
                                                audio_data.data.len()
                                            );
                                            sent_to_livekit = true;
                                        }
                                        Err(e) => {
                                            error!("Failed to send TTS audio to LiveKit: {:?}", e);
                                            // Will fall back to WebSocket below
                                        }
                                    }
                                } else {
                                    debug!(
                                        "LiveKit client not connected, falling back to WebSocket"
                                    );
                                }
                            }
                            Err(_) => {
                                error!(
                                    "Failed to acquire LiveKit lock within {}ms timeout, falling back to WebSocket",
                                    LOCK_TIMEOUT_MS
                                );
                                // Will fall back to WebSocket below
                            }
                        }
                    }

                    // Fall back to WebSocket if LiveKit is not available or failed
                    if !sent_to_livekit {
                        debug!(
                            "Sending TTS audio to WebSocket client: {} bytes",
                            audio_data.data.len()
                        );
                        let audio_bytes = Bytes::from(audio_data.data);
                        if let Err(e) = message_tx.send(MessageRoute::Binary(audio_bytes)).await {
                            error!("Failed to send TTS audio to WebSocket: {:?}", e);
                        }
                    }
                })
            })
            .await
        {
            error!("Failed to set up TTS audio callback: {}", e);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: format!("Failed to set up TTS audio callback: {e}"),
                }))
                .await;
            return true;
        }
    } else {
        info!("Audio processing disabled - TTS audio callback not set");
    }

    // Send ready message
    let _ = message_tx
        .send(MessageRoute::Outgoing(OutgoingMessage::Ready))
        .await;
    info!("Voice manager ready and configured");

    true
}

/// Handle audio input message with optimizations
/// This is a hot path for real-time audio processing
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

/// Handle speak command message with optimizations
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

/// Handle clear command message with optimizations
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
                    warn!("Failed to clear LiveKit audio buffer: {}", e);
                }
            }
        }
    }

    debug!("Clear command completed");
    true
}

/// Handle send_message command with optimizations
pub async fn handle_send_message(
    message: String,
    role: String,
    topic: Option<String>,
    debug: Option<serde_json::Value>,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    debug!(
        "Processing send_message command: {} chars, role: {}, topic: {:?}",
        message.len(),
        role,
        topic
    );

    // Fast path: read lock to get LiveKit client
    let livekit_client = {
        let state_guard = state.read().await;
        match &state_guard.livekit_client {
            Some(client) => client.clone(),
            None => {
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: "LiveKit client not configured. Send config message with livekit configuration first."
                            .to_string(),
                    }))
                    .await;
                return true;
            }
        }
    };

    // Use write() instead of try_write() to wait for the lock
    // This is better than complex retry logic since the LiveKit operations are fast
    let mut client = livekit_client.write().await;

    if let Err(e) = client
        .send_message(&message, &role, topic.as_deref(), debug)
        .await
    {
        error!("Failed to send message via LiveKit: {:?}", e);
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: format!("Failed to send message via LiveKit: {e:?}"),
            }))
            .await;
    } else {
        debug!(
            "Message sent via LiveKit: {} chars, role: {}, topic: {:?}",
            message.len(),
            role,
            topic
        );
    }

    true
}

/// Handle parsed incoming message with optimizations
#[inline]
pub async fn handle_incoming_message(
    msg: IncomingMessage,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    match msg {
        IncomingMessage::Config {
            audio,
            stt_config,
            tts_config,
            livekit,
        } => {
            println!("Processing config message");
            handle_config_message(
                audio, stt_config, tts_config, livekit, state, message_tx, app_state,
            )
            .await
        }
        IncomingMessage::Speak {
            text,
            flush,
            allow_interruption,
        } => handle_speak_message(text, flush, allow_interruption, state, message_tx).await,
        IncomingMessage::Clear => handle_clear_message(state, message_tx).await,
        IncomingMessage::SendMessage {
            message,
            role,
            topic,
            debug,
        } => handle_send_message(message, role, topic, debug, state, message_tx).await,
    }
}
