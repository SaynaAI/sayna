//! WebSocket message processing orchestrator
//!
//! This module serves as the main entry point for processing incoming WebSocket
//! messages, delegating to specialized handlers based on message type.

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

use crate::state::AppState;

use super::{
    audio_handler::{handle_clear_message, handle_speak_message},
    command_handler::{handle_send_message, handle_sip_transfer},
    config_handler::handle_config_message,
    messages::{IncomingMessage, MessageRoute},
    state::ConnectionState,
};

/// Process incoming WebSocket message based on its type
///
/// This is the main message router that delegates to specialized handlers
/// based on the message type. It maintains the separation of concerns by
/// routing audio, configuration, and command messages to their respective handlers.
///
/// # Arguments
/// * `msg` - The parsed incoming message from the WebSocket client
/// * `state` - Connection state shared across handlers
/// * `message_tx` - Channel for sending response messages back to the client
/// * `app_state` - Application state containing global configuration
///
/// # Returns
/// * `bool` - true to continue processing, false to terminate the connection
///
/// # Performance Notes
/// - Marked inline to reduce function call overhead in the hot path
/// - Delegates to specialized handlers for better code organization
#[inline]
pub async fn handle_incoming_message(
    msg: IncomingMessage,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    match msg {
        IncomingMessage::Config {
            stream_id,
            audio,
            stt_config,
            tts_config,
            livekit,
        } => {
            handle_config_message(
                stream_id, audio, stt_config, tts_config, livekit, state, message_tx, app_state,
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
        IncomingMessage::SIPTransfer { transfer_to } => {
            handle_sip_transfer(transfer_to, state, message_tx, app_state).await
        }
    }
}
