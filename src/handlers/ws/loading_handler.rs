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
    let (audio_enabled, livekit_client) = {
        let state_guard = state.read().await;
        (
            state_guard.is_audio_enabled(),
            state_guard.livekit_client.clone(),
        )
    };

    // Audio must be enabled. This also covers the "no config received yet" case,
    // since audio defaults to disabled.
    if !audio_enabled {
        send_error(
            message_tx,
            "Loading indicator requires audio to be enabled. Send a config message with audio=true first.",
        )
        .await;
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
        let message = match e {
            AppError::BadRequest(m)
            | AppError::InternalServerError(m)
            | AppError::NotFound(m)
            | AppError::Unauthorized(m) => m,
        };
        error!("Failed to start loading-indicator audio: {}", message);
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
        let state = Arc::new(RwLock::new(ConnectionState::new()));
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
    async fn test_handle_loading_start_message_rejects_when_no_livekit() {
        let state = Arc::new(RwLock::new(ConnectionState::new()));
        {
            let state_guard = state.write().await;
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
    async fn test_handle_loading_stop_message_is_silent_noop() {
        let state = Arc::new(RwLock::new(ConnectionState::new()));
        let (message_tx, mut message_rx): (
            mpsc::Sender<MessageRoute>,
            mpsc::Receiver<MessageRoute>,
        ) = mpsc::channel(4);

        let continue_processing = handle_loading_stop_message(&state).await;
        drop(message_tx);

        assert!(continue_processing);
        assert!(
            message_rx.try_recv().is_err(),
            "loading_stop must not emit any message when there is no LiveKit client"
        );
    }
}
