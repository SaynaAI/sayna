use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::state::AppState;

/// WebSocket echo server handler
/// Upgrades the HTTP connection to WebSocket and echoes back all received messages
pub async fn ws_echo_handler(
    ws: WebSocketUpgrade,
    State(_state): State<Arc<AppState>>,
) -> Response {
    info!("WebSocket connection upgrade requested");
    ws.on_upgrade(handle_socket)
}

/// Handle WebSocket connection
/// This function is called after the WebSocket upgrade is successful
async fn handle_socket(socket: WebSocket) {
    info!("WebSocket connection established");

    // Split the socket into sender and receiver
    let (mut sender, mut receiver) = socket.split();

    // Process incoming messages
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(msg) => {
                match msg {
                    Message::Text(text) => {
                        info!("Received text message: {}", text);

                        // Echo the message back
                        if let Err(e) = sender.send(Message::Text(text)).await {
                            error!("Failed to send echo message: {}", e);
                            break;
                        }
                    }
                    Message::Binary(data) => {
                        info!("Received binary message: {} bytes", data.len());

                        // Echo the binary data back
                        if let Err(e) = sender.send(Message::Binary(data)).await {
                            error!("Failed to send echo binary message: {}", e);
                            break;
                        }
                    }
                    Message::Ping(data) => {
                        info!("Received ping message");

                        // Respond with pong
                        if let Err(e) = sender.send(Message::Pong(data)).await {
                            error!("Failed to send pong response: {}", e);
                            break;
                        }
                    }
                    Message::Pong(_) => {
                        info!("Received pong message");
                        // No action needed for pong messages
                    }
                    Message::Close(_) => {
                        info!("WebSocket connection closed by client");
                        break;
                    }
                }
            }
            Err(e) => {
                warn!("WebSocket error: {}", e);
                break;
            }
        }
    }

    info!("WebSocket connection terminated");
}
