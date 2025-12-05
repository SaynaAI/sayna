//! Command handler for WebSocket messages
//!
//! This module handles LiveKit-specific commands such as sending messages
//! through the LiveKit data channel and SIP call transfers.

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, oneshot};
use tracing::{debug, error, info, warn};

use crate::livekit::LiveKitOperation;
use crate::state::AppState;
use crate::utils::validate_phone_number;

use super::{
    error::WebSocketError,
    messages::{MessageRoute, OutgoingMessage},
    state::ConnectionState,
};

// Note: handle_sip_transfer calls the SIP handler directly rather than using
// the operation queue, because:
// 1. SIP transfers are not latency-critical like audio operations
// 2. The SIP handler is available in AppState, not in the operation worker context
// 3. This avoids needing to pass LiveKit credentials through the operation queue

/// Helper function to send SIP transfer error messages
async fn send_sip_transfer_error(error: &WebSocketError, message_tx: &mpsc::Sender<MessageRoute>) {
    let _ = message_tx
        .send(MessageRoute::Outgoing(OutgoingMessage::SIPTransferError {
            message: error.to_message(),
        }))
        .await;
}

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

/// Handle SIP transfer command
///
/// Initiates a SIP REFER transfer for the current participant in the LiveKit room.
/// The participant identity is fetched dynamically from the room via LiveKit API,
/// and the room name is derived from the WebSocket connection state.
///
/// # Arguments
/// * `transfer_to` - The destination phone number to transfer the call to
/// * `state` - Connection state containing LiveKit client and room info
/// * `message_tx` - Channel for sending response messages
/// * `app_state` - Application state containing global configuration
///
/// # Returns
/// * `bool` - true to continue processing, false to terminate connection
///
/// # Flow
/// 1. Validate the phone number format
/// 2. Retrieve room_name from connection state
/// 3. Fetch participants from room via LiveKit API and use the first one
/// 4. Call SIP handler directly to perform the transfer
pub async fn handle_sip_transfer(
    transfer_to: String,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    debug!(
        "Processing SIP transfer command: transfer_to={}",
        transfer_to
    );

    // Step 1: Validate phone number format
    let validated_phone = match validate_phone_number(&transfer_to) {
        Ok(phone) => phone,
        Err(validation_error) => {
            let error = WebSocketError::SIPTransferInvalidPhoneNumber(validation_error);
            warn!("SIP transfer validation failed: {}", error);
            send_sip_transfer_error(&error, message_tx).await;
            return true;
        }
    };

    // Step 2: Get room_name from state
    let room_name = {
        let state_guard = state.read().await;
        state_guard.livekit_room_name.clone()
    };

    // Check room_name exists
    let room_name = match room_name {
        Some(name) => name,
        None => {
            let error = WebSocketError::SIPTransferNoRoomName;
            warn!("SIP transfer failed: {}", error);
            send_sip_transfer_error(&error, message_tx).await;
            return true;
        }
    };

    // Step 3: Check SIP handler is available
    let sip_handler = match &app_state.livekit_sip_handler {
        Some(handler) => handler.clone(),
        None => {
            let error = WebSocketError::LiveKitNotConfigured;
            warn!("SIP transfer failed: SIP handler not configured");
            send_sip_transfer_error(&error, message_tx).await;
            return true;
        }
    };

    // Step 4: Check room handler is available for listing participants
    let room_handler = match &app_state.livekit_room_handler {
        Some(handler) => handler.clone(),
        None => {
            let error = WebSocketError::LiveKitNotConfigured;
            warn!("SIP transfer failed: LiveKit room handler not configured");
            send_sip_transfer_error(&error, message_tx).await;
            return true;
        }
    };

    // Step 5: Fetch participants from the room and use the first one
    let participant_identity = match room_handler.list_participants(&room_name).await {
        Ok(participants) if !participants.is_empty() => participants[0].identity.clone(),
        Ok(_) => {
            let error = WebSocketError::SIPTransferNoParticipant;
            warn!("SIP transfer failed: no participants in room {}", room_name);
            send_sip_transfer_error(&error, message_tx).await;
            return true;
        }
        Err(e) => {
            let error =
                WebSocketError::SIPTransferFailed(format!("Failed to list participants: {}", e));
            error!("SIP transfer failed: {}", error);
            send_sip_transfer_error(&error, message_tx).await;
            return true;
        }
    };

    // Step 6: Execute the SIP transfer
    debug!(
        "Initiating SIP transfer: room={}, participant={}, transfer_to={}",
        room_name, participant_identity, validated_phone
    );

    match sip_handler
        .transfer_call(&participant_identity, &room_name, &validated_phone)
        .await
    {
        Ok(()) => {
            info!(
                "SIP transfer successful: room={}, participant={}, transfer_to={}",
                room_name, participant_identity, validated_phone
            );
        }
        Err(e) => {
            let error = WebSocketError::SIPTransferFailed(e.to_string());
            error!("SIP transfer failed: {}", error);
            send_sip_transfer_error(&error, message_tx).await;
        }
    }

    true
}
