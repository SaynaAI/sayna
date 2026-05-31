//! Publishing the single agent audio track and feeding it from TTS.
//!
//! This module publishes one audio track for the session and routes synthesized
//! TTS PCM into the [`AudioPumpShared`] buffer that the [audio pump](super::audio_pump)
//! drains, mixes with the loading overlay, and captures. It also converts
//! inbound LiveKit audio frames back to PCM bytes for the audio callback.
//!
//! The audio pump is the *sole* writer to the published source; nothing here
//! calls `capture_frame` directly.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use livekit::options::TrackPublishOptions;
use livekit::prelude::{DataPacketKind, LocalParticipant, LocalTrackPublication, Room, RoomEvent};
use livekit::track::{LocalAudioTrack, LocalTrack, TrackSource};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::prelude::{AudioFrame, RtcAudioSource};
use tokio::sync::{Mutex, mpsc, oneshot};
use tracing::{debug, error, info, warn};

use super::audio_pump::{AudioPumpShared, spawn_audio_pump, stop_audio_pump};
use super::{
    LiveKitClient, LiveKitConfig, LiveKitOperation, operation_worker::OperationContext,
    sayna_audio_source_options,
};
use crate::AppError;

/// Name of the single published LiveKit audio track.
///
/// It carries the agent's full output — TTS plus any mixed-in loading overlay.
/// The wire name `"tts-audio"` is retained for compatibility with existing
/// subscribers and tooling even though the track is no longer TTS-only.
pub(super) const PUBLISHED_AUDIO_TRACK_NAME: &str = "tts-audio";

/// Internal queue depth, in milliseconds, for the published [`NativeAudioSource`].
///
/// libwebrtc requires this to be a **non-zero multiple of 10 ms** and asserts
/// otherwise. A queue of roughly `sample_rate / 100` ms (~240 ms at 24 kHz)
/// cushions TTS jitter and is shared by the loading overlay. Computing it as
/// `(sample_rate / 1000) * 10` keeps the result a multiple of 10 for *any*
/// client-supplied TTS sample rate — including ones such as 44_100 Hz, where the
/// older `(sample_rate * 10) / 1000` formula produced 441 and tripped the
/// assertion (a process crash). `.max(10)` keeps an absurdly low rate from
/// yielding a zero (invalid) depth.
pub(super) fn audio_source_queue_size_ms(sample_rate: u32) -> u32 {
    ((sample_rate / 1000) * 10).max(10)
}

/// Creates a `NativeAudioSource`, wraps it in the published audio track, and
/// publishes it on `participant`.
///
/// Returns the source (fed by the audio pump), the track handle (which must be
/// kept alive, or kept published via its publication, to stay published), and
/// the resulting publication. Shared by initial setup and reconnect so the
/// track is created identically in both paths.
pub(super) async fn create_and_publish_audio_track(
    participant: &LocalParticipant,
    config: &LiveKitConfig,
) -> Result<
    (
        Arc<NativeAudioSource>,
        LocalAudioTrack,
        LocalTrackPublication,
    ),
    AppError,
> {
    let source = Arc::new(NativeAudioSource::new(
        sayna_audio_source_options(),
        config.sample_rate,
        config.channels as u32,
        audio_source_queue_size_ms(config.sample_rate),
    ));

    let rtc_audio_source = RtcAudioSource::Native((*source).clone());
    let track = LocalAudioTrack::create_audio_track(PUBLISHED_AUDIO_TRACK_NAME, rtc_audio_source);

    let publish_options = TrackPublishOptions {
        source: TrackSource::Microphone,
        ..Default::default()
    };

    match participant
        .publish_track(LocalTrack::Audio(track.clone()), publish_options)
        .await
    {
        Ok(publication) => Ok((source, track, publication)),
        Err(e) => Err(AppError::InternalServerError(format!(
            "Failed to publish audio track: {e:?}"
        ))),
    }
}

impl LiveKitClient {
    pub(super) async fn setup_audio_publishing(&mut self) -> Result<(), AppError> {
        info!(
            "Setting up audio publishing: {}Hz, {} channels",
            self.config.sample_rate, self.config.channels
        );

        // Extract local participant while holding the room lock briefly.
        let local_participant = {
            let room_guard = self.room.lock().await;
            match &*room_guard {
                Some(room) => room.local_participant().clone(),
                None => {
                    return Err(AppError::InternalServerError(
                        "Room not available for track publishing".to_string(),
                    ));
                }
            }
        };

        let (source, track, publication) =
            create_and_publish_audio_track(&local_participant, &self.config).await?;
        info!("Successfully published audio track: {}", publication.sid());

        *self.audio_source.lock().await = Some(Arc::clone(&source));
        self.local_audio_track = Some(track);
        *self.local_track_publication.lock().await = Some(publication);
        self.has_audio_source_atomic.store(true, Ordering::Release);

        // Spawn the single writer. It drains any TTS already buffered in
        // `audio_pump` and mixes in the loading overlay when active.
        spawn_audio_pump(
            &self.pump,
            Arc::clone(&self.audio_pump),
            source,
            self.config.sample_rate,
            self.config.channels,
        )
        .await;

        info!("Audio publishing setup completed successfully");
        Ok(())
    }

    /// Send TTS audio data to the published LiveKit audio track.
    pub async fn send_tts_audio(&self, audio_data: Vec<u8>) -> Result<(), AppError> {
        debug!("send_tts_audio called with {} bytes", audio_data.len());

        if let Some(queue) = &self.operation_queue {
            let (tx, rx) = oneshot::channel();
            queue.queue_audio(audio_data, tx).await?;
            rx.await.map_err(|_| {
                AppError::InternalServerError("Operation worker disconnected".to_string())
            })?
        } else {
            Self::process_send_audio(
                audio_data,
                &self.audio_pump,
                &self.is_connected,
                &self.config,
            )
            .await
        }
    }

    /// Clear any buffered audio data so new audio plays without interference
    /// from previously queued TTS.
    ///
    /// # What Gets Cleared
    ///
    /// 1. **NativeAudioSource Buffer** (`audio_source.clear_buffer()`)
    ///    - Discards audio frames already captured to the source but not yet
    ///      sent to WebRTC.
    ///
    /// 2. **Buffered TTS** (`audio_pump.clear_tts()`)
    ///    - Discards TTS samples queued for the pump but not yet captured.
    ///
    /// An active loading overlay keeps playing: `clear` only drops buffered and
    /// in-flight audio, it does not stop the loading indicator.
    ///
    /// # Limitations
    ///
    /// - **WebRTC Transport Layer**: Audio frames already handed to WebRTC
    ///   cannot be cleared. Due to WebRTC buffering and network latency, a small
    ///   amount of audio (~10-100ms) may still reach remote participants after
    ///   this method returns. This is a fundamental WebRTC limitation, not an
    ///   implementation choice.
    ///
    /// # Thread Safety
    ///
    /// Safe to call concurrently with `send_tts_audio()`. When using the
    /// operation queue (default), the clear is serialized with other audio
    /// operations.
    ///
    /// # Errors
    ///
    /// Returns an error if not connected to a LiveKit room.
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
            Self::process_clear_audio(&self.audio_pump, &self.audio_source, &self.is_connected)
                .await
        }
    }

    pub(super) async fn process_clear_audio(
        shared: &Arc<AudioPumpShared>,
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
            debug!("Cleared published audio source buffer");
        } else {
            warn!("No audio source available - nothing to clear");
        }

        shared.clear_tts().await;
        debug!("Cleared buffered TTS audio");

        Ok(())
    }

    pub(super) async fn process_send_audio(
        audio_data: Vec<u8>,
        shared: &Arc<AudioPumpShared>,
        is_connected: &Arc<Mutex<bool>>,
        config: &LiveKitConfig,
    ) -> Result<(), AppError> {
        if !*is_connected.lock().await {
            return Err(AppError::InternalServerError(
                "Not connected to LiveKit room".to_string(),
            ));
        }

        // Convert and buffer; the pump drains and captures. It is fine to buffer
        // before the source is published — the pump is spawned with the source
        // and the operation worker only runs once the room is bootstrapped.
        let samples = Self::pcm_to_samples(&audio_data, config.channels)?;
        shared.push_tts(samples).await;
        Ok(())
    }

    /// Converts little-endian 16-bit PCM bytes into interleaved `i16` samples.
    ///
    /// Validates that the byte length is even and forms a whole number of frames
    /// for `channels`.
    pub(super) fn pcm_to_samples(audio_data: &[u8], channels: u16) -> Result<Vec<i16>, AppError> {
        if audio_data.len() & 1 != 0 {
            return Err(AppError::InternalServerError(
                "Invalid audio data length (must be even for 16-bit samples)".to_string(),
            ));
        }

        let num_samples = audio_data.len() / 2;
        if num_samples == 0 || !num_samples.is_multiple_of(channels as usize) {
            return Err(AppError::InternalServerError(format!(
                "Invalid audio data: {num_samples} samples doesn't divide evenly by {channels} channels"
            )));
        }

        // Zero-copy reinterpretation for aligned data; manual decode otherwise.
        let samples: Vec<i16> = if (audio_data.as_ptr() as usize).is_multiple_of(2) {
            // SAFETY: length is even and the pointer is 2-byte aligned, so the
            // bytes form `num_samples` valid `i16` values.
            unsafe {
                let ptr = audio_data.as_ptr() as *const i16;
                let slice = std::slice::from_raw_parts(ptr, num_samples);
                slice.to_vec()
            }
        } else {
            audio_data
                .chunks_exact(2)
                .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                .collect()
        };

        Ok(samples)
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

        match Room::connect(
            &ctx.config.url,
            &ctx.config.token,
            ctx.config.room_options(),
        )
        .await
        {
            Ok((new_room, room_events)) => {
                // The old pump's source died with the previous room. Stop it and
                // clear the stale audio resources before rebuilding.
                stop_audio_pump(&ctx.pump).await;
                *ctx.audio_source.lock().await = None;
                *ctx.local_track_publication.lock().await = None;
                ctx.has_audio_source_atomic.store(false, Ordering::Release);

                // The loading overlay does not auto-resume across a reconnect;
                // the client re-sends `loading_start`. Buffered TTS is retained
                // and drained by the new pump (mirroring the old drain-on-
                // reconnect behaviour).
                ctx.audio_pump.deactivate_loading();

                // Configure the data channel threshold on the new room.
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

                // Store the new room before any media-specific setup.
                *ctx.room.lock().await = Some(new_room);

                if !ctx.config.publish_audio {
                    info!("Successfully reconnected in strict no-media mode");
                    return Ok((Some(room_events), true));
                }

                // Extract local participant while holding the lock briefly.
                let local_participant = {
                    let room_guard = ctx.room.lock().await;
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

                match create_and_publish_audio_track(&local_participant, &ctx.config).await {
                    // The track handle is intentionally dropped: `OperationContext`
                    // has no field to hold it, and the publication keeps the track
                    // alive (as the previous reconnect path also relied on).
                    Ok((source, _track, publication)) => {
                        *ctx.audio_source.lock().await = Some(Arc::clone(&source));
                        *ctx.local_track_publication.lock().await = Some(publication);
                        ctx.has_audio_source_atomic.store(true, Ordering::Release);

                        // Respawn the single writer bound to the new source; it
                        // drains any TTS buffered during the outage.
                        spawn_audio_pump(
                            &ctx.pump,
                            Arc::clone(&ctx.audio_pump),
                            source,
                            ctx.config.sample_rate,
                            ctx.config.channels,
                        )
                        .await;

                        info!("Successfully reconnected and re-published audio track");
                        Ok((Some(room_events), true))
                    }
                    Err(e) => {
                        error!("Failed to re-publish audio track after reconnect: {:?}", e);
                        *ctx.is_connected.lock().await = false;
                        Err(e)
                    }
                }
            }
            Err(e) => {
                error!("Failed to reconnect to LiveKit room: {:?}", e);
                *ctx.is_connected.lock().await = false;
                ctx.has_audio_source_atomic.store(false, Ordering::Release);
                Err(AppError::InternalServerError(format!(
                    "Failed to reconnect: {e:?}"
                )))
            }
        }
    }
}
