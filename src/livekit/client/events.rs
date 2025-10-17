//! Room event handling for `LiveKitClient`.
//!
//! Spawns background tasks that react to LiveKit room updates and forward
//! audio or data payloads through registered callbacks.

use std::sync::Arc;

use futures::StreamExt;
use livekit::prelude::{RemoteTrack, RoomEvent};
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use super::{
    AudioCallback, DataCallback, DataMessage, LiveKitClient, LiveKitConfig,
    ParticipantDisconnectCallback, ParticipantDisconnectEvent,
};
use crate::AppError;
#[cfg(feature = "noise-filter")]
use crate::utils::noise_filter::reduce_noise_async;

impl LiveKitClient {
    pub(super) async fn start_event_handler(&mut self) -> Result<(), AppError> {
        if let Some(mut room_events) = self.room_events.take() {
            let audio_callback = self.audio_callback.clone();
            let data_callback = self.data_callback.clone();
            let participant_disconnect_callback = self.participant_disconnect_callback.clone();
            let active_streams = Arc::clone(&self.active_streams);
            let is_connected = Arc::clone(&self.is_connected);
            let config = self.config.clone();

            tokio::spawn(async move {
                while let Some(event) = room_events.recv().await {
                    if let Err(e) = LiveKitClient::handle_room_event(
                        event,
                        &audio_callback,
                        &data_callback,
                        &participant_disconnect_callback,
                        &active_streams,
                        &is_connected,
                        &config,
                    )
                    .await
                    {
                        error!("Error handling room event: {:?}", e);
                    }
                }
                info!("Room event handler finished");
            });
        }

        Ok(())
    }

    pub(super) async fn restart_event_handler(
        mut room_events: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
        audio_callback: &Option<AudioCallback>,
        data_callback: &Option<DataCallback>,
        participant_disconnect_callback: &Option<ParticipantDisconnectCallback>,
        active_streams: &Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
        is_connected: &Arc<Mutex<bool>>,
        config: &LiveKitConfig,
    ) {
        let audio_callback = audio_callback.clone();
        let data_callback = data_callback.clone();
        let participant_disconnect_callback = participant_disconnect_callback.clone();
        let active_streams = Arc::clone(active_streams);
        let is_connected = Arc::clone(is_connected);
        let config = config.clone();

        tokio::spawn(async move {
            info!("Restarting room event handler after reconnect");
            while let Some(event) = room_events.recv().await {
                if let Err(e) = LiveKitClient::handle_room_event(
                    event,
                    &audio_callback,
                    &data_callback,
                    &participant_disconnect_callback,
                    &active_streams,
                    &is_connected,
                    &config,
                )
                .await
                {
                    error!("Error handling room event: {:?}", e);
                }
            }
            info!("Restarted room event handler finished");
        });
    }

    async fn handle_room_event(
        event: RoomEvent,
        audio_callback: &Option<AudioCallback>,
        data_callback: &Option<DataCallback>,
        participant_disconnect_callback: &Option<ParticipantDisconnectCallback>,
        active_streams: &Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
        is_connected: &Arc<Mutex<bool>>,
        config: &LiveKitConfig,
    ) -> Result<(), AppError> {
        match event {
            RoomEvent::TrackSubscribed {
                track,
                publication,
                participant,
            } => {
                info!(
                    "Track subscribed - Track: {:?}, Participant: {:?}",
                    publication.sid(),
                    participant.identity()
                );

                // Check if we should process this participant's audio
                if !config.listen_participants.is_empty()
                    && !config
                        .listen_participants
                        .contains(&participant.identity().to_string())
                {
                    debug!(
                        "Ignoring audio track from participant {} (not in listen_participants list)",
                        participant.identity()
                    );
                    return Ok(());
                }

                match track {
                    RemoteTrack::Audio(audio_track) => {
                        debug!(
                            "Received audio track from participant: {}",
                            participant.identity()
                        );

                        if let Some(callback) = audio_callback {
                            let callback_clone = Arc::clone(callback);
                            let rtc_track = audio_track.rtc_track();
                            let mut audio_stream = NativeAudioStream::new(
                                rtc_track,
                                config.sample_rate as i32,
                                config.channels as i32,
                            );

                            let enable_noise_filter = config.enable_noise_filter;
                            #[cfg(feature = "noise-filter")]
                            let sample_rate = config.sample_rate;
                            let participant_id = participant.identity().to_string();

                            // Create a local buffer pool for this audio stream
                            let stream_buffer_pool = Arc::new(Mutex::new(
                                (0..4).map(|_| Vec::with_capacity(4800)).collect::<Vec<_>>(),
                            ));

                            let handle = tokio::spawn(async move {
                                info!(
                                    "Starting audio stream processing for participant: {} (noise_filter: {})",
                                    participant_id, enable_noise_filter
                                );

                                #[cfg(not(feature = "noise-filter"))]
                                if enable_noise_filter {
                                    warn!(
                                        "Noise filter requested for participant {} but the 'noise-filter' feature is disabled; forwarding raw audio",
                                        participant_id
                                    );
                                }

                                while let Some(audio_frame) = audio_stream.next().await {
                                    debug!(
                                        "Received audio frame: {} samples",
                                        audio_frame.data.len()
                                    );

                                    let audio_buffer = LiveKitClient::convert_frame_to_audio(
                                        &audio_frame,
                                        Some(&stream_buffer_pool),
                                    );

                                    if enable_noise_filter {
                                        #[cfg(feature = "noise-filter")]
                                        {
                                            // For noise filtering, we need to clone since the filter may modify the buffer
                                            match reduce_noise_async(
                                                audio_buffer.clone().into(),
                                                sample_rate,
                                            )
                                            .await
                                            {
                                                Ok(filtered_audio) => {
                                                    LiveKitClient::return_buffer_to_pool(
                                                        &stream_buffer_pool,
                                                        audio_buffer,
                                                    );
                                                    callback_clone(filtered_audio);
                                                }
                                                Err(e) => {
                                                    error!("Error reducing noise: {:?}", e);
                                                    callback_clone(audio_buffer);
                                                }
                                            }
                                        }
                                        #[cfg(not(feature = "noise-filter"))]
                                        {
                                            callback_clone(audio_buffer);
                                        }
                                    } else {
                                        callback_clone(audio_buffer);
                                    }
                                }

                                info!("Audio stream ended for participant: {}", participant_id);
                            });

                            // Clean up completed handles before adding new one
                            let mut streams = active_streams.lock().await;
                            streams.retain(|h| !h.is_finished());
                            streams.push(handle);
                        } else {
                            warn!("No audio callback set, ignoring audio track");
                        }
                    }
                    RemoteTrack::Video(_) => {
                        debug!(
                            "Received video track (ignoring): {}",
                            participant.identity()
                        );
                    }
                }
            }
            RoomEvent::TrackUnsubscribed {
                publication,
                participant,
                ..
            } => {
                info!(
                    "Track unsubscribed - Track: {:?}, Participant: {:?}",
                    publication.sid(),
                    participant.identity()
                );
            }
            RoomEvent::ParticipantConnected(participant) => {
                info!("Participant connected: {}", participant.identity());
            }
            RoomEvent::ParticipantDisconnected(participant) => {
                info!("Participant disconnected: {}", participant.identity());

                if let Some(callback) = participant_disconnect_callback {
                    let event = ParticipantDisconnectEvent {
                        participant_identity: participant.identity().to_string(),
                        participant_name: Some(participant.name().to_string()),
                        room_name: config.room_name.clone(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                    };

                    callback(event);
                }
            }
            RoomEvent::Disconnected { reason } => {
                warn!("Room disconnected with reason: {:?}", reason);
                *is_connected.lock().await = false;
            }
            RoomEvent::DataReceived {
                payload,
                topic,
                participant,
                ..
            } => {
                let participant_identity = participant
                    .as_ref()
                    .map(|p| p.identity().to_string())
                    .unwrap_or_else(|| "server".to_string());

                debug!(
                    "Data received from participant: {}, {} bytes",
                    participant_identity,
                    payload.len()
                );

                // Check if we should process this participant's data messages
                if !config.listen_participants.is_empty()
                    && participant.is_some()
                    && !config.listen_participants.contains(&participant_identity)
                {
                    debug!(
                        "Ignoring data message from participant {} (not in listen_participants list)",
                        participant_identity
                    );
                    return Ok(());
                }

                if let Some(callback) = data_callback {
                    let data_message = DataMessage {
                        participant_identity,
                        data: payload.to_vec(),
                        topic,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                    };

                    callback(data_message);
                } else {
                    debug!("No data callback set, ignoring data message");
                }
            }
            _ => {
                debug!("Received other room event: {:?}", event);
            }
        }

        Ok(())
    }
}
