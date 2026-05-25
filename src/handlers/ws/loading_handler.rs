//! Loading-indicator audio handler for WebSocket connections
//!
//! This module handles the `loading_start` and `loading_stop` control messages.
//! The loading-indicator audio loop is controlled exclusively by these two
//! commands — the `speak` and `clear` commands never start, stop, or otherwise
//! affect it.

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error};

use crate::AppError;

use super::{
    messages::{MessageRoute, OutgoingMessage},
    state::ConnectionState,
};

/// Send an error message back to the WebSocket client.
///
/// Send failures are ignored because they only mean the client has already
/// disconnected.
async fn send_error(message_tx: &mpsc::Sender<MessageRoute>, message: impl Into<String>) {
    let _ = message_tx
        .send(MessageRoute::Outgoing(OutgoingMessage::Error {
            message: message.into(),
        }))
        .await;
}

/// Handle the `loading_start` command.
///
/// Begins looping the configured loading-indicator audio into the LiveKit room.
/// Requires that audio is enabled and a LiveKit room is configured; otherwise an
/// error message is sent back to the client.
///
/// # Arguments
/// * `state` - Connection state containing the LiveKit client
/// * `message_tx` - Channel for sending response messages back to the client
///
/// # Returns
/// * `bool` - always `true`; this handler never terminates the connection
pub async fn handle_loading_start_message(
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
) -> bool {
    debug!("Processing loading_start command");

    // Snapshot the state we need, then drop the read lock immediately.
    let (stream_id, audio_enabled, livekit_client, loading_audio_error) = {
        let state_guard = state.read().await;
        (
            state_guard.stream_id.clone(),
            state_guard.is_audio_enabled(),
            state_guard.livekit_client.clone(),
            state_guard.loading_audio_error.clone(),
        )
    };

    if stream_id.is_none() {
        send_error(
            message_tx,
            "Send a config message first before using loading_start.",
        )
        .await;
        return true;
    }

    if !audio_enabled {
        send_error(
            message_tx,
            "Loading indicator requires audio to be enabled. Send a config message with audio=true.",
        )
        .await;
        return true;
    }

    // The `loading_audio` supplied in the config message failed to decode.
    // The reason was already reported once at config time; report it again
    // clearly here rather than a generic "not available".
    if let Some(reason) = loading_audio_error {
        send_error(message_tx, reason).await;
        return true;
    }

    // A LiveKit room is required to play the loading audio.
    let livekit_client = match livekit_client {
        Some(client) => client,
        None => {
            send_error(
                message_tx,
                "Loading indicator requires a LiveKit room. Include a livekit configuration in your config message.",
            )
            .await;
            return true;
        }
    };

    // `start_loading_audio` takes `&self`, so a read lock is sufficient.
    let client_guard = livekit_client.read().await;
    if let Err(e) = client_guard.start_loading_audio().await {
        // Surface the inner message verbatim: `start_loading_audio` already
        // produces end-user-facing strings, so the `AppError` `Display` prefix
        // ("Bad request: ", etc.) would only add noise for the client.
        //
        // A `BadRequest` (no clip configured, or the track failed to publish)
        // is a normal client-side condition, not a server fault — log it
        // quietly; only genuine server faults warrant `error!`.
        let (message, is_client_error) = match e {
            AppError::BadRequest(m) => (m, true),
            AppError::InternalServerError(m)
            | AppError::NotFound(m)
            | AppError::Unauthorized(m) => (m, false),
        };
        if is_client_error {
            debug!("loading_start rejected: {message}");
        } else {
            error!("Failed to start loading-indicator audio: {message}");
        }
        send_error(message_tx, message).await;
    } else {
        debug!("Loading-indicator audio started");
    }

    true
}

/// Handle the `loading_stop` command.
///
/// Stops the loading-indicator audio loop (with a short fade-out). When there is
/// no LiveKit client this is a silent no-op: no message and no error is sent.
///
/// # Arguments
/// * `state` - Connection state containing the LiveKit client
///
/// # Returns
/// * `bool` - always `true`; this handler never terminates the connection
pub async fn handle_loading_stop_message(state: &Arc<RwLock<ConnectionState>>) -> bool {
    debug!("Processing loading_stop command");

    // Snapshot the LiveKit client, then drop the read lock immediately.
    let livekit_client = {
        let state_guard = state.read().await;
        state_guard.livekit_client.clone()
    };

    // Silent no-op when there is no LiveKit client.
    if let Some(client) = livekit_client {
        // `stop_loading_audio` takes `&self`, so a read lock is sufficient.
        let client_guard = client.read().await;
        client_guard.stop_loading_audio().await;
        debug!("Loading-indicator audio stopped");
    } else {
        debug!("No LiveKit client - loading_stop is a no-op");
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_handle_loading_start_message_rejects_when_audio_disabled() {
        let mut state_inner = ConnectionState::new();
        state_inner.stream_id = Some("test-stream".to_string());
        let state = Arc::new(RwLock::new(state_inner));
        let (message_tx, mut message_rx) = mpsc::channel(4);

        let continue_processing = handle_loading_start_message(&state, &message_tx).await;

        assert!(continue_processing);
        match message_rx.recv().await {
            Some(MessageRoute::Outgoing(OutgoingMessage::Error { message })) => {
                assert!(message.contains("audio to be enabled"));
            }
            Some(_) => panic!("expected audio-disabled error"),
            None => panic!("expected audio-disabled error"),
        }
    }

    #[tokio::test]
    async fn test_handle_loading_start_message_reports_decode_error() {
        // loading_audio was supplied in the config message but failed to
        // decode. loading_start must replay the original decode reason, not a
        // generic "not available" message.
        let mut state_inner = ConnectionState::new();
        state_inner.stream_id = Some("test-stream".to_string());
        state_inner.set_audio_enabled(true);
        state_inner.loading_audio_error =
            Some("loading_audio.data is not valid base64".to_string());
        let state = Arc::new(RwLock::new(state_inner));
        let (message_tx, mut message_rx) = mpsc::channel(4);

        let continue_processing = handle_loading_start_message(&state, &message_tx).await;

        assert!(continue_processing);
        match message_rx.recv().await {
            Some(MessageRoute::Outgoing(OutgoingMessage::Error { message })) => {
                assert_eq!(message, "loading_audio.data is not valid base64");
            }
            Some(_) => panic!("expected the original decode-failure error"),
            None => panic!("expected the original decode-failure error"),
        }
    }

    #[tokio::test]
    async fn test_handle_loading_start_message_rejects_when_no_livekit() {
        let state = Arc::new(RwLock::new(ConnectionState::new()));
        {
            let mut state_guard = state.write().await;
            state_guard.stream_id = Some("test-stream".to_string());
            state_guard.set_audio_enabled(true);
        }
        let (message_tx, mut message_rx) = mpsc::channel(4);

        let continue_processing = handle_loading_start_message(&state, &message_tx).await;

        assert!(continue_processing);
        match message_rx.recv().await {
            Some(MessageRoute::Outgoing(OutgoingMessage::Error { message })) => {
                assert!(message.contains("LiveKit room"));
            }
            Some(_) => panic!("expected missing-LiveKit error"),
            None => panic!("expected missing-LiveKit error"),
        }
    }

    #[tokio::test]
    async fn test_handle_loading_start_message_forwards_missing_clip_error() {
        // Handler-level missing-clip case: audio is enabled and a connected
        // LiveKit client is present, but no loading clip was configured. The
        // handler must forward `start_loading_audio`'s specific BadRequest
        // message verbatim rather than swallow it or send a generic error.
        use crate::livekit::{LiveKitClient, LiveKitConfig};

        let client = LiveKitClient::new(LiveKitConfig {
            url: "wss://test-server.com".to_string(),
            token: "mock-jwt-token".to_string(),
            room_name: "test-room".to_string(),
            publish_audio: true,
            subscribe_audio: true,
            sample_rate: 24_000,
            channels: 1,
            enable_noise_filter: false,
            listen_participants: vec![],
        });
        client.set_connected(true).await;

        let mut state_inner = ConnectionState::new();
        state_inner.stream_id = Some("test-stream".to_string());
        state_inner.set_audio_enabled(true);
        state_inner.livekit_client = Some(Arc::new(RwLock::new(client)));
        let state = Arc::new(RwLock::new(state_inner));
        let (message_tx, mut message_rx) = mpsc::channel(4);

        let continue_processing = handle_loading_start_message(&state, &message_tx).await;

        assert!(continue_processing);
        match message_rx.recv().await {
            Some(MessageRoute::Outgoing(OutgoingMessage::Error { message })) => {
                assert!(
                    message.contains("no loading audio configured"),
                    "unexpected error message: {message}"
                );
            }
            Some(_) => panic!("expected the missing-clip BadRequest forwarded verbatim"),
            None => panic!("expected the missing-clip BadRequest forwarded verbatim"),
        }
    }

    #[tokio::test]
    async fn test_handle_loading_start_message_rejects_before_config() {
        let state = Arc::new(RwLock::new(ConnectionState::new()));
        let (message_tx, mut message_rx) = mpsc::channel(4);

        let continue_processing = handle_loading_start_message(&state, &message_tx).await;

        assert!(continue_processing);
        match message_rx.recv().await {
            Some(MessageRoute::Outgoing(OutgoingMessage::Error { message })) => {
                assert!(
                    message.contains("config message first"),
                    "unexpected error: {message}"
                );
            }
            Some(_) => panic!("expected config-first error"),
            None => panic!("expected config-first error"),
        }
    }

    #[tokio::test]
    async fn test_handle_loading_stop_message_silent_with_client_no_loop() {
        use crate::livekit::{LiveKitClient, LiveKitConfig};

        let client = LiveKitClient::new(LiveKitConfig {
            url: "wss://test-server.com".to_string(),
            token: "mock-jwt-token".to_string(),
            room_name: "test-room".to_string(),
            publish_audio: true,
            subscribe_audio: true,
            sample_rate: 24_000,
            channels: 1,
            enable_noise_filter: false,
            listen_participants: vec![],
        });
        client.set_connected(true).await;

        let mut state_inner = ConnectionState::new();
        state_inner.set_audio_enabled(true);
        state_inner.stream_id = Some("test-stream".to_string());
        state_inner.livekit_client = Some(Arc::new(RwLock::new(client)));
        let state = Arc::new(RwLock::new(state_inner));
        let (_tx, mut message_rx) = mpsc::channel::<MessageRoute>(4);

        let continue_processing = handle_loading_stop_message(&state).await;

        assert!(continue_processing);
        assert!(
            message_rx.try_recv().is_err(),
            "loading_stop must be silent when a LiveKit client exists but no loop is running"
        );
    }

    // This test constructs a real libwebrtc `NativeAudioSource` and so wears
    // the `livekit_native_` quarantine prefix described in
    // `src/livekit/client/tests.rs:499-506`: it is `#[ignore]`d out of the
    // default `cargo test` run and executed isolated by the dedicated CI step
    // in `.github/workflows/ci.yml`.
    #[tokio::test]
    #[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
    async fn livekit_native_handle_loading_start_message_success_is_silent() {
        use crate::livekit::loading_clip::make_test_loading_clip;
        use crate::livekit::{LiveKitClient, LiveKitConfig, sayna_audio_source_options};
        use livekit::webrtc::audio_source::native::NativeAudioSource;

        let clip = make_test_loading_clip();
        let mut client = LiveKitClient::new(LiveKitConfig {
            url: "wss://test-server.com".to_string(),
            token: "mock-jwt-token".to_string(),
            room_name: "test-room".to_string(),
            publish_audio: true,
            subscribe_audio: true,
            sample_rate: 24_000,
            channels: 1,
            enable_noise_filter: false,
            listen_participants: vec![],
        });
        client.set_connected(true).await;
        client.set_loading_audio_clip(clip);

        let source = Arc::new(NativeAudioSource::new(
            sayna_audio_source_options(),
            16_000,
            1,
            100, // NativeAudioSource queue depth in ms (matches LOADING_AUDIO_QUEUE_SIZE_MS)
        ));
        *client.loading_audio_source.lock().await = Some(source);

        let mut state_inner = ConnectionState::new();
        state_inner.set_audio_enabled(true);
        state_inner.stream_id = Some("test-stream".to_string());
        state_inner.livekit_client = Some(Arc::new(RwLock::new(client)));
        let state = Arc::new(RwLock::new(state_inner));
        let (message_tx, mut message_rx) = mpsc::channel(4);

        let continue_processing = handle_loading_start_message(&state, &message_tx).await;
        drop(message_tx);

        assert!(continue_processing);
        assert!(
            message_rx.try_recv().is_err(),
            "successful loading_start must not emit any WebSocket message"
        );

        // Tear down the spawned loop so the test does not leak a background task.
        {
            let guard = state.read().await;
            if let Some(lk) = &guard.livekit_client {
                lk.read().await.stop_loading_audio().await;
            }
        }
    }

    #[tokio::test]
    async fn test_handle_loading_stop_message_is_silent_noop() {
        let state = Arc::new(RwLock::new(ConnectionState::new()));
        let (_tx, mut message_rx) = mpsc::channel::<MessageRoute>(4);

        let continue_processing = handle_loading_stop_message(&state).await;

        assert!(continue_processing);
        assert!(
            message_rx.try_recv().is_err(),
            "loading_stop must not emit any message when there is no LiveKit client"
        );
    }
}
