//! Command handler for WebSocket messages
//!
//! This module handles LiveKit-specific commands such as sending messages
//! through the LiveKit data channel.

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, error};

use super::{
    messages::{MessageRoute, OutgoingMessage},
    state::ConnectionState,
};

/// Handle send_message command for LiveKit data channel
///
/// Sends custom messages through the LiveKit data channel to other participants
/// in the room. Messages can be directed to specific topics and include debug data.
///
/// # Arguments
/// * `message` - The text message to send
/// * `role` - The sender's role identifier
/// * `topic` - Optional topic/channel for the message
/// * `debug` - Optional debug data to include
/// * `state` - Connection state containing LiveKit client
/// * `message_tx` - Channel for sending response messages
///
/// # Returns
/// * `bool` - true to continue processing, false to terminate connection
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