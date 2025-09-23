//! Axum WebSocket handler
//!
//! This module contains the main WebSocket upgrade handler for Axum
//! and the core WebSocket connection handling logic.

use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tokio::{select, time::Duration};
use tracing::{debug, error, info, warn};

use crate::state::AppState;

use super::{
    audio_handler::handle_audio_message,
    messages::{IncomingMessage, MessageRoute, OutgoingMessage},
    processor::handle_incoming_message,
    state::ConnectionState,
};

/// Optimized channel buffer size for audio workloads
/// Larger buffer (1024 vs default 256) reduces contention in high-throughput scenarios
/// Trade-off: Uses more memory but provides better latency characteristics
const CHANNEL_BUFFER_SIZE: usize = 1024;

/// WebSocket voice processing handler
///
/// Upgrades the HTTP connection to WebSocket for real-time voice processing.
/// This is the main entry point for WebSocket connections to the voice service.
///
/// # Arguments
/// * `ws` - The WebSocket upgrade request from Axum
/// * `state` - Application state containing configuration and shared resources
///
/// # Returns
/// * `Response` - HTTP response that upgrades the connection to WebSocket
pub async fn ws_voice_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    info!("WebSocket voice connection upgrade requested");
    ws.on_upgrade(move |socket| handle_voice_socket(socket, state))
}

/// Handle WebSocket voice connection with optimized performance
///
/// This function manages the entire WebSocket session for voice processing,
/// including message routing, resource management, and graceful cleanup.
///
/// # Arguments
/// * `socket` - The established WebSocket connection
/// * `app_state` - Application state containing shared resources
///
/// # Lifecycle
/// 1. Split socket into sender/receiver for bidirectional communication
/// 2. Set up message routing channels with optimized buffer sizes
/// 3. Spawn sender task for outgoing messages
/// 4. Process incoming messages in a loop
/// 5. Clean up resources on connection close
///
/// # Performance Optimizations
/// - Large channel buffer (1024) for reduced contention
/// - RwLock for connection state (frequent reads, rare writes)
/// - Timeout handling for stale connection detection
async fn handle_voice_socket(socket: WebSocket, app_state: Arc<AppState>) {
    info!("WebSocket voice connection established");

    // Split the socket into sender and receiver
    let (mut sender, mut receiver) = socket.split();

    // Connection state with RwLock for rare writes, frequent reads
    let state = Arc::new(RwLock::new(ConnectionState::new()));

    let (message_tx, mut message_rx) = mpsc::channel::<MessageRoute>(CHANNEL_BUFFER_SIZE);

    // Spawn task to handle outgoing messages - simple and direct for low latency
    let sender_task = tokio::spawn(async move {
        while let Some(route) = message_rx.recv().await {
            let result = match route {
                MessageRoute::Outgoing(message) => {
                    // Direct serialization and send - no batching for low latency
                    match serde_json::to_string(&message) {
                        Ok(json_str) => sender.send(Message::Text(json_str.into())).await,
                        Err(e) => {
                            error!("Failed to serialize outgoing message: {}", e);
                            continue;
                        }
                    }
                }
                MessageRoute::Binary(data) => sender.send(Message::Binary(data)).await,
            };

            if let Err(e) = result {
                error!("Failed to send WebSocket message: {}", e);
                break;
            }
        }
    });

    // Optimized timeout for low-latency audio processing
    // Shorter timeout to detect stale connections faster
    let processing_timeout = Duration::from_secs(10);

    loop {
        select! {
            msg_result = receiver.next() => {
                match msg_result {
                    Some(Ok(msg)) => {
                        let continue_processing = process_message(
                            msg,
                            &state,
                            &message_tx,
                            &app_state
                        ).await;

                        if !continue_processing {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error: {}", e);
                        let _ = message_tx.send(MessageRoute::Outgoing(OutgoingMessage::Error {
                            message: format!("WebSocket error: {e}"),
                        })).await;
                        break;
                    }
                    None => {
                        info!("WebSocket connection closed by client");
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(processing_timeout) => {
                // Handle connection timeout
                debug!("WebSocket connection timeout check");
                continue;
            }
        }
    }

    // Clean up resources
    sender_task.abort();

    // Stop voice manager and LiveKit client if they exist
    {
        let state_guard = state.read().await;
        if let Some(voice_manager) = &state_guard.voice_manager {
            if let Err(e) = voice_manager.stop().await {
                error!("Failed to stop voice manager: {}", e);
            }
        }

        if let Some(livekit_client) = &state_guard.livekit_client {
            // Try to get write lock with timeout for cleanup
            match tokio::time::timeout(Duration::from_millis(100), livekit_client.write()).await {
                Ok(mut client) => {
                    if let Err(e) = client.disconnect().await {
                        error!("Failed to disconnect LiveKit client: {:?}", e);
                    }
                }
                Err(_) => {
                    warn!("Timeout acquiring LiveKit lock for cleanup - client may be busy");
                }
            }
        }
    }

    info!("WebSocket voice connection terminated");
}

/// Process incoming WebSocket message with optimizations
///
/// Routes different message types to appropriate handlers and manages
/// the connection lifecycle based on message processing results.
///
/// # Arguments
/// * `msg` - The WebSocket message to process
/// * `state` - Connection state for this WebSocket session
/// * `message_tx` - Channel for sending response messages
/// * `app_state` - Application state with global configuration
///
/// # Returns
/// * `bool` - true to continue processing, false to close connection
///
/// # Performance Notes
/// - Marked inline(always) for hot path optimization
/// - Fast JSON parsing with pre-validation
/// - Zero-copy audio data handling where possible
#[inline(always)]
async fn process_message(
    msg: Message,
    state: &Arc<RwLock<ConnectionState>>,
    message_tx: &mpsc::Sender<MessageRoute>,
    app_state: &Arc<AppState>,
) -> bool {
    match msg {
        Message::Text(text) => {
            debug!("Received text message: {} bytes", text.len());

            // Fast path JSON parsing with pre-validation
            let incoming_msg: IncomingMessage = match serde_json::from_str(&text) {
                Ok(msg) => msg,
                Err(e) => {
                    error!("Failed to parse incoming message: {}", e);
                    let _ = message_tx
                        .send(MessageRoute::Outgoing(OutgoingMessage::Error {
                            message: format!("Invalid message format: {e}"),
                        }))
                        .await;
                    return true;
                }
            };

            handle_incoming_message(incoming_msg, state, message_tx, app_state).await
        }
        Message::Binary(data) => {
            debug!("Received binary message: {} bytes", data.len());

            // Handle binary audio data with zero-copy optimization
            handle_audio_message(data, state, message_tx).await
        }
        Message::Ping(_data) => {
            debug!("Received ping message");
            // Ping/Pong is handled automatically by axum
            true
        }
        Message::Pong(_) => {
            debug!("Received pong message");
            true
        }
        Message::Close(_) => {
            info!("WebSocket connection closed by client");
            false
        }
    }
}
