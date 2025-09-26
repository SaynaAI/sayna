//! Command handler for WebSocket messages
//!
//! This module handles LiveKit-specific commands such as sending messages
//! through the LiveKit data channel.

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};
use tracing::{debug, error};

use crate::livekit::LiveKitOperation;

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

    // Non-blocking path: use operation queue if available
    let operation_queue = {
        let state_guard = state.read().await;
        state_guard.livekit_operation_queue.clone()
    };

    if let Some(queue) = operation_queue {
        // Queue the operation non-blocking
        let (response_tx, response_rx) = oneshot::channel();

        if let Err(e) = queue
            .queue(LiveKitOperation::SendMessage {
                message: message.clone(),
                role: role.clone(),
                topic: topic.clone(),
                debug,
                response_tx,
                retry_count: 0,
            })
            .await
        {
            error!("Failed to queue send message operation: {:?}", e);
            let _ = message_tx
                .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                    message: format!("Failed to queue send message operation: {e:?}"),
                }))
                .await;
            return true;
        }

        // Wait for response asynchronously
        match response_rx.await {
            Ok(Ok(())) => {
                debug!(
                    "Message sent via LiveKit: {} chars, role: {}, topic: {:?}",
                    message.len(),
                    role,
                    topic
                );
            }
            Ok(Err(e)) => {
                error!("Failed to send message via LiveKit: {:?}", e);
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: format!("Failed to send message via LiveKit: {e:?}"),
                    }))
                    .await;
            }
            Err(_) => {
                error!("Operation worker disconnected");
                let _ = message_tx
                    .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                        message: "Operation worker disconnected".to_string(),
                    }))
                    .await;
            }
        }
    } else {
        // Fallback: no queue available
        let _ = message_tx
            .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                message: "LiveKit client not configured. Send config message with livekit configuration first."
                    .to_string(),
            }))
            .await;
    }

    true
}
