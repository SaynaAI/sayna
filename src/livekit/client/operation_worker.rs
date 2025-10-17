//! Background operation queue worker for `LiveKitClient`.
//!
//! Handles serialized execution of LiveKit operations to avoid lock
//! contention and to centralize error handling.

use std::collections::VecDeque;
use std::sync::Arc;

use livekit::prelude::{LocalTrackPublication, Room};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::{
    AudioCallback, DataCallback, LiveKitClient, LiveKitConfig, LiveKitOperation, OperationQueue,
    ParticipantDisconnectCallback,
};
use crate::AppError;

/// Context struct for process_operation to avoid too many arguments
pub(super) struct OperationContext {
    pub(super) room: Arc<Mutex<Option<Room>>>,
    pub(super) audio_queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    pub(super) audio_source: Arc<Mutex<Option<Arc<NativeAudioSource>>>>,
    pub(super) is_connected: Arc<Mutex<bool>>,
    pub(super) config: LiveKitConfig,
    pub(super) local_track_publication: Arc<Mutex<Option<LocalTrackPublication>>>,
    pub(super) active_streams: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    pub(super) audio_callback: Option<AudioCallback>,
    pub(super) data_callback: Option<DataCallback>,
    pub(super) participant_disconnect_callback: Option<ParticipantDisconnectCallback>,
}

impl LiveKitClient {
    pub(super) async fn start_operation_worker(&mut self) {
        let (queue, mut receiver) = OperationQueue::new(1024);
        self.operation_queue = Some(queue);

        let context = OperationContext {
            room: Arc::clone(&self.room),
            audio_queue: Arc::clone(&self.audio_queue),
            audio_source: Arc::clone(&self.audio_source),
            is_connected: Arc::clone(&self.is_connected),
            config: self.config.clone(),
            local_track_publication: Arc::clone(&self.local_track_publication),
            active_streams: Arc::clone(&self.active_streams),
            audio_callback: self.audio_callback.clone(),
            data_callback: self.data_callback.clone(),
            participant_disconnect_callback: self.participant_disconnect_callback.clone(),
        };
        let stats = Arc::clone(&self.stats);

        let handle = tokio::spawn(async move {
            info!("Operation worker started");
            let mut operation_count = 0u64;

            while let Some(queued_op) = receiver.recv().await {
                let start_time = std::time::Instant::now();
                let is_shutdown = matches!(&queued_op.operation, LiveKitOperation::Shutdown { .. });

                // Track operation type before processing
                let is_audio_op = matches!(
                    &queued_op.operation,
                    LiveKitOperation::SendAudio { .. } | LiveKitOperation::ClearAudio { .. }
                );
                let is_message_op = matches!(
                    &queued_op.operation,
                    LiveKitOperation::SendMessage { .. } | LiveKitOperation::SendDataMessage { .. }
                );

                let result = LiveKitClient::process_operation(queued_op.operation, &context).await;

                // Use simplified stats recording
                if let Ok(mut stats_guard) = stats.try_lock() {
                    let latency = start_time.elapsed();
                    let latency_ms = latency.as_millis() as u64;

                    stats_guard.total_operations += 1;
                    if result.is_ok() {
                        stats_guard.successful_operations += 1;
                    } else {
                        stats_guard.failed_operations += 1;
                    }

                    if is_audio_op {
                        stats_guard.audio_operations += 1;
                    }
                    if is_message_op {
                        stats_guard.message_operations += 1;
                    }

                    stats_guard.max_latency_ms = stats_guard.max_latency_ms.max(latency_ms);
                    if stats_guard.total_operations == 1 {
                        stats_guard.average_latency_ms = latency_ms;
                    } else {
                        stats_guard.average_latency_ms = (stats_guard.average_latency_ms
                            * (stats_guard.total_operations - 1)
                            + latency_ms)
                            / stats_guard.total_operations;
                    }

                    operation_count += 1;
                    if operation_count.is_multiple_of(100) {
                        stats_guard.log_stats();
                    }
                }

                if is_shutdown {
                    info!("Operation worker shutting down");
                    break;
                }
            }

            info!("Operation worker finished");
        });

        self.operation_worker_handle = Some(handle);
    }

    async fn process_operation(
        operation: LiveKitOperation,
        ctx: &OperationContext,
    ) -> Result<(), AppError> {
        match operation {
            LiveKitOperation::SendAudio {
                audio_data,
                response_tx,
            } => {
                let result = LiveKitClient::process_send_audio(
                    audio_data,
                    &ctx.audio_queue,
                    &ctx.audio_source,
                    &ctx.is_connected,
                    &ctx.config,
                )
                .await;
                let _ = response_tx.send(result);
            }
            LiveKitOperation::SendMessage {
                message,
                role,
                topic,
                debug,
                response_tx,
                retry_count: _,
            } => {
                let result = LiveKitClient::process_send_message(
                    message,
                    role,
                    topic,
                    debug,
                    &ctx.room,
                    &ctx.is_connected,
                )
                .await;
                let _ = response_tx.send(result);
            }
            LiveKitOperation::SendDataMessage {
                topic,
                data,
                response_tx,
                retry_count: _,
            } => {
                let result = LiveKitClient::process_send_data_message(
                    topic,
                    data,
                    &ctx.room,
                    &ctx.is_connected,
                )
                .await;
                let _ = response_tx.send(result);
            }
            LiveKitOperation::ClearAudio { response_tx } => {
                let result = LiveKitClient::process_clear_audio(
                    &ctx.audio_queue,
                    &ctx.audio_source,
                    &ctx.is_connected,
                )
                .await;
                let _ = response_tx.send(result);
            }
            LiveKitOperation::IsConnected { response_tx } => {
                let connected = *ctx.is_connected.lock().await;
                let _ = response_tx.send(connected);
            }
            LiveKitOperation::HasAudioSource { response_tx } => {
                let has_source = ctx.audio_source.lock().await.is_some()
                    && ctx.local_track_publication.lock().await.is_some();
                let _ = response_tx.send(has_source);
            }
            LiveKitOperation::Reconnect { response_tx } => {
                warn!("Manual reconnect requested (should be rare with keep-alive)");
                let result = LiveKitClient::process_reconnect(ctx).await;

                // Handle event handler restart if reconnect succeeded
                let final_result = match result {
                    Ok((Some(room_events), true)) => {
                        // Start new event handler with the reconnected room events
                        LiveKitClient::restart_event_handler(
                            room_events,
                            &ctx.audio_callback,
                            &ctx.data_callback,
                            &ctx.participant_disconnect_callback,
                            &ctx.active_streams,
                            &ctx.is_connected,
                            &ctx.config,
                        )
                        .await;
                        Ok(())
                    }
                    Ok((_, _)) => Ok(()),
                    Err(e) => Err(e),
                };

                let _ = response_tx.send(final_result);
            }
            LiveKitOperation::Shutdown { ack_tx } => {
                // Send acknowledgement if channel provided
                if let Some(tx) = ack_tx {
                    let _ = tx.send(());
                }
            }
        }
        Ok(())
    }

    async fn process_send_message(
        message: String,
        role: String,
        topic: Option<String>,
        debug: Option<serde_json::Value>,
        room: &Arc<Mutex<Option<Room>>>,
        is_connected: &Arc<Mutex<bool>>,
    ) -> Result<(), AppError> {
        if !*is_connected.lock().await {
            return Err(AppError::InternalServerError(
                "Not connected to LiveKit room".to_string(),
            ));
        }

        let topic = topic.as_deref().unwrap_or("messages");
        let json_message = serde_json::json!({
            "message": message,
            "role": role,
            "debug": debug
        });

        LiveKitClient::process_send_data_message(
            topic.to_string(),
            json_message,
            room,
            is_connected,
        )
        .await
    }

    async fn process_send_data_message(
        topic: String,
        data: serde_json::Value,
        room: &Arc<Mutex<Option<Room>>>,
        is_connected: &Arc<Mutex<bool>>,
    ) -> Result<(), AppError> {
        // Use the shared helper from messaging module for consistency
        LiveKitClient::publish_data_packet_internal(room, is_connected, &topic, data).await
    }
}
