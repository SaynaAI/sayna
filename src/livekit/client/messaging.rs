//! Data channel publishing helpers for `LiveKitClient`.
//!
//! Consolidates utilities for sending structured payloads over LiveKit's
//! reliable data channels.

use std::sync::Arc;

use livekit::prelude::{DataPacket, Room};
use tokio::sync::{Mutex, oneshot};
use tracing::debug;

use super::{LiveKitClient, LiveKitOperation};
use crate::AppError;

impl LiveKitClient {
    /// Send a data message to the LiveKit room.
    pub async fn send_data_message<T: serde::Serialize>(
        &self,
        topic: &str,
        data: T,
    ) -> Result<(), AppError> {
        debug!("Sending data message to topic: {}", topic);

        // Convert to Value for queuing
        let data_value = serde_json::to_value(data)
            .map_err(|e| AppError::InternalServerError(format!("Failed to serialize data: {e}")))?;

        if let Some(queue) = &self.operation_queue {
            let (tx, rx) = oneshot::channel();
            queue
                .queue(LiveKitOperation::SendDataMessage {
                    topic: topic.to_string(),
                    data: data_value,
                    response_tx: tx,
                    retry_count: 0,
                })
                .await?;
            rx.await.map_err(|_| {
                AppError::InternalServerError("Operation worker disconnected".to_string())
            })?
        } else {
            // Use the shared helper for consistency, propagate errors in direct path
            Self::publish_data_packet_internal(&self.room, &self.is_connected, topic, data_value)
                .await
        }
    }

    /// Internal helper for publishing data packets - shared between direct and queued paths
    pub(super) async fn publish_data_packet_internal(
        room: &Arc<Mutex<Option<Room>>>,
        is_connected: &Arc<Mutex<bool>>,
        topic: &str,
        data: serde_json::Value,
    ) -> Result<(), AppError> {
        if !*is_connected.lock().await {
            return Err(AppError::InternalServerError(
                "Not connected to LiveKit room".to_string(),
            ));
        }

        let serialized_data = serde_json::to_vec(&data).map_err(|e| {
            AppError::InternalServerError(format!("Failed to serialize JSON data: {e}"))
        })?;

        // Build the data packet before locking
        let data_packet = DataPacket {
            payload: serialized_data,
            topic: Some(topic.to_string()),
            ..Default::default()
        };

        // Clone the participant handle to release the mutex before awaiting
        let participant = {
            let room_guard = room.lock().await;
            room_guard.as_ref().map(|r| r.local_participant().clone())
        };

        if let Some(participant_ref) = participant {
            match participant_ref.publish_data(data_packet).await {
                Ok(_) => {
                    debug!("Successfully sent data message to topic: {}", topic);
                    Ok(())
                }
                Err(e) => {
                    // Return error for direct path, but log warning for queued path
                    // The operation_worker will handle this appropriately
                    Err(AppError::InternalServerError(format!(
                        "Failed to send data message to topic {topic}: {e:?}"
                    )))
                }
            }
        } else {
            Err(AppError::InternalServerError(
                "Room not available for data message publishing".to_string(),
            ))
        }
    }

    /// Send a text message to the LiveKit room in the specified JSON format.
    pub async fn send_message(
        &self,
        message: &str,
        role: &str,
        topic: Option<&str>,
        debug: bool,
    ) -> Result<(), AppError> {
        let topic = topic.unwrap_or("messages");
        let data = serde_json::json!({
            "message": message,
            "role": role,
        });

        if debug {
            debug!("Preparing to send message: {:?}", data);
        }

        self.send_data_message(topic, data).await
    }
}
