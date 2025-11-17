//! Audio publishing and frame conversion helpers for `LiveKitClient`.
//!
//! This module encapsulates track setup for TTS output as well as utilities
//! for streaming synthesized audio into LiveKit and translating audio frames
//! between byte and `AudioFrame` representations.

use std::collections::VecDeque;
use std::sync::Arc;

use livekit::options::TrackPublishOptions;
use livekit::prelude::{DataPacketKind, Room, RoomEvent, RoomOptions};
use livekit::track::{LocalAudioTrack, LocalTrack, TrackSource};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource};
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{debug, error, info, warn};

use super::{LiveKitClient, LiveKitConfig, LiveKitOperation, operation_worker::OperationContext};
use crate::AppError;

impl LiveKitClient {
    pub(super) async fn setup_audio_publishing(&mut self) -> Result<(), AppError> {
        info!(
            "Setting up audio publishing: {}Hz, {} channels",
            self.config.sample_rate, self.config.channels
        );

        let audio_source_options = AudioSourceOptions {
            echo_cancellation: false,
            noise_suppression: false,
            auto_gain_control: false,
        };

        let samples_per_frame = (self.config.sample_rate * 10) / 1000;

        let audio_source = Arc::new(NativeAudioSource::new(
            audio_source_options,
            self.config.sample_rate,
            self.config.channels as u32,
            samples_per_frame,
        ));

        let rtc_audio_source = RtcAudioSource::Native((*audio_source).clone());
        let local_audio_track = LocalAudioTrack::create_audio_track("tts-audio", rtc_audio_source);

        // Extract local participant while holding the lock briefly
        let local_participant = {
            let room_guard = self.room.lock().await;
            if let Some(room) = &*room_guard {
                room.local_participant().clone()
            } else {
                return Err(AppError::InternalServerError(
                    "Room not available for track publishing".to_string(),
                ));
            }
        };
        // Room lock is now released

        let publish_options = TrackPublishOptions {
            source: TrackSource::Microphone,
            ..Default::default()
        };

        // Publish track without holding the room lock
        match local_participant
            .publish_track(
                LocalTrack::Audio(local_audio_track.clone()),
                publish_options,
            )
            .await
        {
            Ok(publication) => {
                info!(
                    "Successfully published TTS audio track: {}",
                    publication.sid()
                );

                *self.audio_source.lock().await = Some(audio_source.clone());
                self.local_audio_track = Some(local_audio_track);
                *self.local_track_publication.lock().await = Some(publication);
                self.has_audio_source_atomic
                    .store(true, std::sync::atomic::Ordering::Release);

                // Spawn a lightweight task to drain queued audio without blocking
                let audio_queue_clone = Arc::clone(&self.audio_queue);
                let audio_source_clone = Arc::clone(&audio_source);
                let sample_rate = self.config.sample_rate;
                let channels = self.config.channels;

                let handle = tokio::spawn(async move {
                    // Extract all queued data to avoid holding the lock during async operations
                    let queued_data: Vec<Vec<u8>> = {
                        let mut queue = audio_queue_clone.lock().await;
                        if !queue.is_empty() {
                            info!(
                                "Draining {} queued audio messages after track publish",
                                queue.len()
                            );
                            queue.drain(..).collect()
                        } else {
                            Vec::new()
                        }
                    };

                    // Process the drained audio without holding any locks
                    for audio_data in queued_data {
                        // Use convert_audio_to_frame_ref to avoid allocations
                        match LiveKitClient::convert_audio_to_frame_ref(
                            &audio_data,
                            sample_rate,
                            channels,
                        ) {
                            Ok(audio_frame) => {
                                if let Err(e) = audio_source_clone.capture_frame(&audio_frame).await
                                {
                                    error!("Failed to send queued audio frame: {:?}", e);
                                    // Don't re-queue on error to avoid infinite loop
                                }
                            }
                            Err(e) => {
                                error!("Failed to convert queued audio frame: {:?}", e);
                            }
                        }
                    }
                });

                // Track the spawn handle for lifecycle management
                self.active_streams.lock().await.push(handle);

                info!("Audio source, track, and publication are now set");
            }
            Err(e) => {
                error!("Failed to publish audio track: {:?}", e);
                return Err(AppError::InternalServerError(format!(
                    "Failed to publish audio track: {e:?}"
                )));
            }
        }

        info!("Audio publishing setup completed successfully");
        Ok(())
    }

    /// Send TTS audio data to the published LiveKit audio track.
    pub async fn send_tts_audio(&self, audio_data: Vec<u8>) -> Result<(), AppError> {
        debug!("send_tts_audio called with {} bytes", audio_data.len());

        if let Some(queue) = &self.operation_queue {
            let (tx, rx) = oneshot::channel();
            queue
                .queue(LiveKitOperation::SendAudio {
                    audio_data,
                    response_tx: tx,
                })
                .await?;
            rx.await.map_err(|_| {
                AppError::InternalServerError("Operation worker disconnected".to_string())
            })?
        } else {
            // Use the shared helper for consistency
            Self::process_send_audio(
                audio_data,
                &self.audio_queue,
                &self.audio_source,
                &self.is_connected,
                &self.config,
            )
            .await
        }
    }

    /// Clear any buffered audio data from the local source and queue.
    pub async fn clear_audio(&self) -> Result<(), AppError> {
        debug!("clear_audio requested");

        if let Some(queue) = &self.operation_queue {
            let (tx, rx) = oneshot::channel();
            queue
                .queue(LiveKitOperation::ClearAudio { response_tx: tx })
                .await?;
            rx.await.map_err(|_| {
                AppError::InternalServerError("Operation worker disconnected".to_string())
            })?
        } else {
            if !*self.is_connected.lock().await {
                return Err(AppError::InternalServerError(
                    "Not connected to LiveKit room".to_string(),
                ));
            }

            if let Some(audio_source) = self.audio_source.lock().await.as_ref() {
                audio_source.clear_buffer();
                debug!("Cleared local audio buffer");
            } else {
                warn!("No audio source available - nothing to clear");
            }

            self.audio_queue.lock().await.clear();
            debug!("Cleared audio queue");
            Ok(())
        }
    }

    pub(super) async fn process_clear_audio(
        audio_queue: &Arc<Mutex<VecDeque<Vec<u8>>>>,
        audio_source: &Arc<Mutex<Option<Arc<NativeAudioSource>>>>,
        is_connected: &Arc<Mutex<bool>>,
    ) -> Result<(), AppError> {
        if !*is_connected.lock().await {
            return Err(AppError::InternalServerError(
                "Not connected to LiveKit room".to_string(),
            ));
        }

        if let Some(source) = audio_source.lock().await.as_ref() {
            source.clear_buffer();
            debug!("Cleared local audio buffer");
        } else {
            warn!("No audio source available - nothing to clear");
        }

        audio_queue.lock().await.clear();
        debug!("Cleared audio queue");

        Ok(())
    }

    pub(super) async fn process_send_audio(
        audio_data: Vec<u8>,
        audio_queue: &Arc<Mutex<VecDeque<Vec<u8>>>>,
        audio_source: &Arc<Mutex<Option<Arc<NativeAudioSource>>>>,
        is_connected: &Arc<Mutex<bool>>,
        config: &LiveKitConfig,
    ) -> Result<(), AppError> {
        if !*is_connected.lock().await {
            return Err(AppError::InternalServerError(
                "Not connected to LiveKit room".to_string(),
            ));
        }

        // Clone the Arc<NativeAudioSource> out of the mutex to avoid holding lock during await
        let source = {
            let guard = audio_source.lock().await;
            guard.as_ref().cloned()
        };

        if let Some(source) = source {
            // Try to convert and send without holding any locks
            match Self::convert_audio_to_frame_ref(&audio_data, config.sample_rate, config.channels)
            {
                Ok(audio_frame) => {
                    // Try up to 3 times with the current frame before queuing
                    let mut retry_count = 0;
                    const MAX_RETRIES: usize = 3;

                    loop {
                        match source.capture_frame(&audio_frame).await {
                            Ok(()) => {
                                debug!("Successfully sent audio frame directly");

                                // Try to drain any queued audio now that we have capacity
                                let queued_data: Vec<Vec<u8>> = {
                                    let mut queue = audio_queue.lock().await;
                                    if !queue.is_empty() && queue.len() <= 5 {
                                        // Drain a few items to reduce latency
                                        let drain_count = queue.len().min(5);
                                        queue.drain(..drain_count).collect()
                                    } else {
                                        Vec::new()
                                    }
                                };

                                // Process drained items without holding locks
                                for data in queued_data {
                                    if let Ok(frame) = Self::convert_audio_to_frame_ref(
                                        &data,
                                        config.sample_rate,
                                        config.channels,
                                    ) {
                                        let _ = source.capture_frame(&frame).await;
                                    }
                                }

                                return Ok(());
                            }
                            Err(e) if retry_count < MAX_RETRIES => {
                                retry_count += 1;
                                debug!(
                                    "Retry {}/{} for frame capture after error: {:?}",
                                    retry_count, MAX_RETRIES, e
                                );
                                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                            }
                            Err(e) => {
                                error!(
                                    "Failed to capture frame after {} retries: {:?}, queuing",
                                    MAX_RETRIES, e
                                );
                                // Apply bounded queue with backpressure - drop oldest if queue is too large
                                let mut queue = audio_queue.lock().await;
                                if queue.len() >= super::MAX_AUDIO_QUEUE_SIZE {
                                    // Drop the oldest audio frame to prevent unbounded latency
                                    match queue.pop_front() {
                                        Some(_dropped) => {
                                            warn!(
                                                "Audio queue full, dropping oldest frame to prevent latency spike"
                                            );
                                        }
                                        None => warn!(
                                            "Audio queue full but nothing to drop; queue state may be inconsistent"
                                        ),
                                    }
                                }
                                queue.push_back(audio_data);
                                debug!(
                                    "Queued audio data for later processing, queue size: {}",
                                    queue.len()
                                );
                                return Ok(());
                            }
                        }
                    }
                }
                Err(e) => {
                    // Conversion failure is likely a format issue, not transient
                    error!("Failed to convert audio frame: {:?}", e);
                    return Err(AppError::InternalServerError(format!(
                        "Audio format error: {e:?}"
                    )));
                }
            }
        }

        // No audio source yet, queue for when track is ready with bounded queue
        let mut queue = audio_queue.lock().await;
        if queue.len() >= super::MAX_AUDIO_QUEUE_SIZE {
            match queue.pop_front() {
                Some(_dropped) => {
                    warn!(
                        "Audio queue full (no source), dropping oldest frame to prevent latency spike"
                    );
                }
                None => warn!(
                    "Audio queue full (no source) but nothing to drop; queue state may be inconsistent"
                ),
            }
        }
        queue.push_back(audio_data);
        debug!(
            "Queued audio data (no source yet), queue size: {}",
            queue.len()
        );
        Ok(())
    }

    pub(super) fn convert_audio_to_frame_ref(
        audio_data: &[u8],
        sample_rate: u32,
        channels: u16,
    ) -> Result<AudioFrame<'static>, AppError> {
        if audio_data.len() & 1 != 0 {
            return Err(AppError::InternalServerError(
                "Invalid audio data length (must be even for 16-bit samples)".to_string(),
            ));
        }

        let num_samples = audio_data.len() / 2;
        let samples_per_channel = num_samples / channels as usize;

        if samples_per_channel == 0 || !num_samples.is_multiple_of(channels as usize) {
            return Err(AppError::InternalServerError(format!(
                "Invalid audio data: {num_samples} samples doesn't divide evenly by {channels} channels"
            )));
        }

        // Zero-copy conversion using bytemuck for aligned data
        // Fall back to manual conversion for unaligned data
        let samples: Vec<i16> = if (audio_data.as_ptr() as usize).is_multiple_of(2) {
            // Data is aligned, use zero-copy reinterpretation
            unsafe {
                // SAFETY: We've verified the length is even and the data is aligned
                let ptr = audio_data.as_ptr() as *const i16;
                let slice = std::slice::from_raw_parts(ptr, num_samples);
                slice.to_vec()
            }
        } else {
            // Data is unaligned, fall back to manual conversion
            let mut samples = Vec::with_capacity(num_samples);
            for chunk in audio_data.chunks_exact(2) {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                samples.push(sample);
            }
            samples
        };

        Ok(AudioFrame {
            data: samples.into(),
            sample_rate,
            num_channels: channels as u32,
            samples_per_channel: samples_per_channel as u32,
        })
    }

    pub(super) fn convert_frame_to_audio(
        audio_frame: &AudioFrame,
        buffer_pool: Option<&Arc<Mutex<Vec<Vec<u8>>>>>,
    ) -> Vec<u8> {
        let capacity = audio_frame.data.len() * 2;
        let mut audio_bytes = if let Some(pool) = buffer_pool {
            Self::get_buffer_from_pool(pool, capacity)
        } else {
            Vec::with_capacity(capacity)
        };

        for sample in audio_frame.data.iter() {
            let bytes = sample.to_le_bytes();
            audio_bytes.push(bytes[0]);
            audio_bytes.push(bytes[1]);
        }

        audio_bytes
    }

    pub(super) async fn process_reconnect(
        ctx: &OperationContext,
    ) -> Result<(Option<mpsc::UnboundedReceiver<RoomEvent>>, bool), AppError> {
        warn!("Processing reconnect operation due to publisher timeout");

        if !*ctx.is_connected.lock().await {
            info!("Already disconnected, skipping reconnect");
            return Ok((None, false));
        }

        match Room::connect(&ctx.config.url, &ctx.config.token, RoomOptions::default()).await {
            Ok((new_room, room_events)) => {
                // Clear old audio resources
                *ctx.audio_source.lock().await = None;
                *ctx.local_track_publication.lock().await = None;

                // Configure data channel threshold on the new room
                {
                    let participant = new_room.local_participant();
                    if let Err(e) = participant.set_data_channel_buffered_amount_low_threshold(
                        super::RELIABLE_BUFFER_THRESHOLD_BYTES,
                        DataPacketKind::Reliable,
                    ) {
                        warn!(
                            "Failed to set data channel buffered amount threshold: {:?}",
                            e
                        );
                    } else {
                        debug!(
                            "Set reliable data channel buffered amount threshold to {} bytes",
                            super::RELIABLE_BUFFER_THRESHOLD_BYTES
                        );
                    }
                }

                // Extract local participant while holding the lock briefly, then store the new room
                let local_participant = {
                    let mut room_guard = ctx.room.lock().await;
                    *room_guard = Some(new_room);

                    if let Some(room_ref) = &*room_guard {
                        room_ref.local_participant().clone()
                    } else {
                        error!("Room not available after reconnect");
                        *ctx.is_connected.lock().await = false;
                        return Err(AppError::InternalServerError(
                            "Room not available after reconnect".to_string(),
                        ));
                    }
                };
                // Room lock is now released

                let audio_source_options = AudioSourceOptions {
                    echo_cancellation: false,
                    noise_suppression: false,
                    auto_gain_control: false,
                };

                let samples_per_frame = (ctx.config.sample_rate * 10) / 1000;
                let new_audio_source = Arc::new(NativeAudioSource::new(
                    audio_source_options,
                    ctx.config.sample_rate,
                    ctx.config.channels as u32,
                    samples_per_frame,
                ));

                let rtc_audio_source = RtcAudioSource::Native((*new_audio_source).clone());
                let local_audio_track =
                    LocalAudioTrack::create_audio_track("tts-audio", rtc_audio_source);

                let publish_options = TrackPublishOptions {
                    source: TrackSource::Microphone,
                    ..Default::default()
                };

                // Publish track without holding the room lock
                match local_participant
                    .publish_track(
                        LocalTrack::Audio(local_audio_track.clone()),
                        publish_options,
                    )
                    .await
                {
                    Ok(publication) => {
                        *ctx.audio_source.lock().await = Some(new_audio_source.clone());
                        *ctx.local_track_publication.lock().await = Some(publication);

                        // Spawn a task to drain queued audio just like in initial setup
                        let audio_queue_clone = Arc::clone(&ctx.audio_queue);
                        let audio_source_clone = Arc::clone(&new_audio_source);
                        let sample_rate = ctx.config.sample_rate;
                        let channels = ctx.config.channels;
                        let active_streams_clone = Arc::clone(&ctx.active_streams);

                        let handle = tokio::spawn(async move {
                            // Extract all queued data to avoid holding the lock during async operations
                            let queued_data: Vec<Vec<u8>> = {
                                let mut queue = audio_queue_clone.lock().await;
                                if !queue.is_empty() {
                                    info!(
                                        "Draining {} queued audio messages after reconnect",
                                        queue.len()
                                    );
                                    queue.drain(..).collect()
                                } else {
                                    Vec::new()
                                }
                            };

                            // Process the drained audio without holding any locks
                            for audio_data in queued_data {
                                match Self::convert_audio_to_frame_ref(
                                    &audio_data,
                                    sample_rate,
                                    channels,
                                ) {
                                    Ok(audio_frame) => {
                                        if let Err(e) =
                                            audio_source_clone.capture_frame(&audio_frame).await
                                        {
                                            error!("Failed to send queued audio frame: {:?}", e);
                                        }
                                    }
                                    Err(e) => {
                                        error!("Failed to convert queued audio to frame: {:?}", e);
                                    }
                                }
                            }
                        });

                        // Track the spawn handle for lifecycle management
                        active_streams_clone.lock().await.push(handle);

                        info!("Successfully reconnected and re-published audio track");

                        // Return room_events for event handler restart and success flag
                        Ok((Some(room_events), true))
                    }
                    Err(e) => {
                        error!("Failed to re-publish audio track after reconnect: {:?}", e);
                        *ctx.is_connected.lock().await = false;
                        Err(AppError::InternalServerError(format!(
                            "Failed to re-publish audio track: {e:?}"
                        )))
                    }
                }
            }
            Err(e) => {
                error!("Failed to reconnect to LiveKit room: {:?}", e);
                *ctx.is_connected.lock().await = false;
                Err(AppError::InternalServerError(format!(
                    "Failed to reconnect: {e:?}"
                )))
            }
        }
    }
}
