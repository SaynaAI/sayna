//! Dedicated "loading-audio" track and the loading-audio playback loop.
//!
//! This submodule owns a *second*, parallel published LiveKit audio track —
//! named [`LOADING_AUDIO_TRACK_NAME`] — that is fully independent of the TTS
//! track (`"tts-audio"`). It publishes that track, and runs a background loop
//! that feeds the configured [`LoadingClip`] into it on a seamless cursor-based
//! loop until cancelled, finishing with a short linear fade-out.
//!
//! ## Isolation invariant
//!
//! The loading-audio loop NEVER touches the TTS path's `operation_queue`,
//! `audio_queue`, `audio_generation`, or `has_audio_source_atomic`. It captures
//! frames directly on its own dedicated `loading_audio_source`. The loading
//! track is a second, parallel published track, independent of the TTS track.

use std::sync::Arc;

use livekit::options::TrackPublishOptions;
use livekit::prelude::LocalTrackPublication;
use livekit::track::{LocalAudioTrack, LocalTrack, TrackSource};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::LiveKitClient;
use crate::AppError;
use crate::livekit::loading_clip::LoadingClip;

/// Name of the published LiveKit audio track that carries the loading clip.
pub(super) const LOADING_AUDIO_TRACK_NAME: &str = "loading-audio";

/// Length of the linear fade-out applied when the loop stops, in milliseconds.
///
/// Roughly three 10 ms frames; long enough to avoid an audible click without a
/// perceptible tail.
const LOADING_AUDIO_FADE_OUT_MS: u32 = 30;

/// Depth of the loading audio source's internal buffering queue, in milliseconds.
///
/// `NativeAudioSource::new` interprets its fourth argument as a queue size in
/// milliseconds and asserts that it is a non-zero multiple of 10. A fixed value
/// — independent of the clip's (client-supplied) sample rate — guarantees that
/// an arbitrary sample rate can never produce an invalid queue size. 100 ms
/// mirrors LiveKit's `BackgroundAudioPlayer` reference design: once the queue
/// fills, `capture_frame().await` applies real-time back-pressure, pacing the
/// loop at one 10 ms frame per 10 ms of wall-clock with no manual timer.
pub(super) const LOADING_AUDIO_QUEUE_SIZE_MS: u32 = 100;

/// Handle to a running loading-audio loop: its task handle and cancellation token.
///
/// Visibility is `pub(crate)` so it can name the `pub(super)` `loading_loop`
/// field on the `pub` [`LiveKitClient`] struct without a private-interface
/// warning; it is only ever constructed and consumed within the `client`
/// submodule.
pub(crate) struct LoadingLoopHandle {
    /// Join handle of the spawned loop task.
    pub(super) handle: tokio::task::JoinHandle<()>,
    /// Token used to request that the loop fade out and exit.
    pub(super) token: CancellationToken,
}

/// Returns the interleaved `i16` sample count contained in one 10 ms frame.
///
/// For mono this equals `sample_rate / 100`; for stereo it is twice that.
pub(super) fn loading_frame_len(sample_rate: u32, channels: u16) -> usize {
    (sample_rate as usize / 100) * channels as usize
}

/// Fills `out` with the next `out.len()` interleaved samples taken from
/// `clip_samples`, wrapping seamlessly at the end of the clip.
///
/// `cursor` is the current read position into `clip_samples`. The new cursor
/// (modulo the clip length) is returned. An empty `clip_samples` is handled
/// safely: `out` is zero-filled and `0` is returned, with no panic.
pub(super) fn fill_loading_frame(clip_samples: &[i16], cursor: usize, out: &mut [i16]) -> usize {
    if clip_samples.is_empty() {
        out.fill(0);
        return 0;
    }

    let len = clip_samples.len();
    let mut pos = cursor % len;
    for slot in out.iter_mut() {
        *slot = clip_samples[pos];
        pos += 1;
        if pos == len {
            pos = 0;
        }
    }
    pos
}

/// Returns the linear fade-out factor for fade sample `index` of `total` total
/// fade samples, ramping from near `1.0` down to exactly `0.0`.
///
/// `fade_factor(total - 1, total)` is `0.0`, the value is monotonically
/// non-increasing in `index`, and `total == 0` yields `0.0` safely.
pub(super) fn fade_factor(index: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        (1.0 - (index + 1) as f32 / total as f32).max(0.0)
    }
}

/// Applies the linear fade-out ramp to `frame` in place, beginning at fade
/// sample `start_index` of `total` total fade samples. Returns the fade index
/// one past the last sample scaled, so a following frame continues the ramp.
///
/// Each sample is scaled by [`fade_factor`] and rounded to the nearest `i16`.
/// Because the factor is within `0.0..=1.0`, the scaled magnitude never exceeds
/// the input magnitude, so the `as i16` cast cannot overflow or wrap.
pub(super) fn apply_fade(frame: &mut [i16], start_index: usize, total: usize) -> usize {
    let mut index = start_index;
    for slot in frame.iter_mut() {
        let factor = fade_factor(index, total);
        *slot = (f32::from(*slot) * factor).round() as i16;
        index += 1;
    }
    index
}

/// Creates the dedicated loading audio source plus the "loading-audio" track
/// and publishes it on the given participant.
///
/// On success returns the audio source, the track handle (which must be kept
/// alive to keep the track published), and the resulting publication.
pub(super) async fn publish_loading_audio_track(
    local_participant: &livekit::prelude::LocalParticipant,
    sample_rate: u32,
    channels: u16,
) -> Result<
    (
        Arc<NativeAudioSource>,
        LocalAudioTrack,
        LocalTrackPublication,
    ),
    AppError,
> {
    // `auto_gain_control: false` is load-bearing for the `volume` feature:
    // WebRTC AGC would re-normalize loudness and silently undo the configured
    // volume attenuation already applied to the clip's samples.
    let audio_source_options = AudioSourceOptions {
        echo_cancellation: false,
        noise_suppression: false,
        auto_gain_control: false,
    };

    let source = Arc::new(NativeAudioSource::new(
        audio_source_options,
        sample_rate,
        channels as u32,
        LOADING_AUDIO_QUEUE_SIZE_MS,
    ));

    let rtc_audio_source = RtcAudioSource::Native((*source).clone());
    let track = LocalAudioTrack::create_audio_track(LOADING_AUDIO_TRACK_NAME, rtc_audio_source);

    let publish_options = TrackPublishOptions {
        source: TrackSource::Microphone,
        ..Default::default()
    };

    match local_participant
        .publish_track(LocalTrack::Audio(track.clone()), publish_options)
        .await
    {
        Ok(publication) => Ok((source, track, publication)),
        Err(e) => Err(AppError::InternalServerError(format!(
            "Failed to publish loading audio track: {e:?}"
        ))),
    }
}

impl LiveKitClient {
    /// Stores the loading audio clip to play while the agent is busy.
    ///
    /// Must be called before [`LiveKitClient::connect`]; the clip is published
    /// as a dedicated track during connection bootstrap.
    pub fn set_loading_audio_clip(&mut self, clip: LoadingClip) {
        self.loading_clip = Some(Arc::new(clip));
    }

    /// Publishes the dedicated "loading-audio" track if a loading clip is set.
    ///
    /// When no loading clip is configured this is a clean no-op (the feature is
    /// simply unused for this session). On a publish failure the error is
    /// propagated; the caller treats loading-track setup as non-fatal.
    pub(super) async fn setup_loading_audio_track(&mut self) -> Result<(), AppError> {
        let clip = match &self.loading_clip {
            Some(clip) => Arc::clone(clip),
            None => return Ok(()),
        };

        // Extract local participant while holding the room lock briefly.
        let local_participant = {
            let room_guard = self.room.lock().await;
            if let Some(room) = &*room_guard {
                room.local_participant().clone()
            } else {
                return Err(AppError::InternalServerError(
                    "Room not available for loading audio track publishing".to_string(),
                ));
            }
        };

        let (source, track, publication) =
            publish_loading_audio_track(&local_participant, clip.sample_rate(), clip.channels())
                .await?;

        *self.loading_audio_source.lock().await = Some(source);
        self.loading_audio_track = Some(track);
        *self.loading_track_publication.lock().await = Some(publication);

        info!(
            "Successfully published loading audio track: {}Hz, {} channels",
            clip.sample_rate(),
            clip.channels()
        );
        Ok(())
    }

    /// Starts looping the configured loading clip into the "loading-audio" track.
    ///
    /// Requires an active LiveKit connection and a configured, published loading
    /// clip. Calling this while a loop is already running is an idempotent
    /// no-op — including during the brief fade-out after a `stop_loading_audio`,
    /// which still counts as running. The loop runs until
    /// [`LiveKitClient::stop_loading_audio`] (or disconnect) cancels it.
    ///
    /// # Errors
    ///
    /// Returns an error if the client is not connected, or if no loading audio
    /// clip / source is available for this session.
    pub async fn start_loading_audio(&self) -> Result<(), AppError> {
        if !self.is_connected() {
            return Err(AppError::InternalServerError(
                "Not connected to LiveKit room".to_string(),
            ));
        }

        let source = {
            let guard = self.loading_audio_source.lock().await;
            guard.as_ref().map(Arc::clone)
        };
        let source = match source {
            Some(source) => source,
            None => {
                return Err(AppError::BadRequest(
                    "loading audio is not available for this session".to_string(),
                ));
            }
        };

        let clip = match &self.loading_clip {
            Some(clip) => Arc::clone(clip),
            None => {
                return Err(AppError::BadRequest(
                    "loading audio is not available for this session".to_string(),
                ));
            }
        };

        let mut loop_guard = self.loading_loop.lock().await;
        // A finished handle is stale and is replaced; a live one means the loop
        // is already running, so starting again is an idempotent no-op.
        if let Some(existing) = loop_guard.as_ref()
            && !existing.handle.is_finished()
        {
            debug!("Loading audio loop already running; start request ignored");
            return Ok(());
        }

        let token = CancellationToken::new();
        // Isolation invariant: the spawned loop owns only its dedicated audio
        // source, the loading clip, and its cancellation token. It never
        // touches the TTS operation_queue, audio_queue, audio_generation, or
        // has_audio_source_atomic.
        let handle = tokio::spawn(run_loading_loop(source, clip, token.clone()));
        *loop_guard = Some(LoadingLoopHandle { handle, token });

        info!("Started loading audio loop");
        Ok(())
    }

    /// Stops a running loading-audio loop, triggering its short fade-out.
    ///
    /// This is infallible, silent, and idempotent: with no loop running it does
    /// nothing. The loop's handle is intentionally left in place so that
    /// disconnect (or the next start) can await or replace it.
    pub async fn stop_loading_audio(&self) {
        let loop_guard = self.loading_loop.lock().await;
        if let Some(existing) = loop_guard.as_ref() {
            existing.token.cancel();
            debug!("Requested loading audio loop stop");
        }
    }

    /// Test-only: clones the running loading loop's cancellation token, if any,
    /// so a test can observe whether teardown (or `Drop`) cancelled it.
    #[cfg(test)]
    pub(crate) async fn loading_loop_token_for_test(&self) -> Option<CancellationToken> {
        self.loading_loop
            .lock()
            .await
            .as_ref()
            .map(|existing| existing.token.clone())
    }
}

/// Runs the loading-audio playback loop until cancelled, then fades out.
///
/// The task holds only its `Arc<NativeAudioSource>`, `Arc<LoadingClip>`, and
/// `CancellationToken` — no locks are held during the loop, and no TTS state is
/// ever touched (the isolation invariant from the module docs).
async fn run_loading_loop(
    source: Arc<NativeAudioSource>,
    clip: Arc<LoadingClip>,
    token: CancellationToken,
) {
    let sample_rate = clip.sample_rate();
    let channels = clip.channels();
    let samples_per_channel = (sample_rate / 100) as usize;
    let frame_len = loading_frame_len(sample_rate, channels);

    if frame_len == 0 || clip.samples().is_empty() {
        warn!("Loading audio clip has no playable frames; loop not started");
        return;
    }

    let mut frame_buf: Vec<i16> = vec![0; frame_len];
    let mut cursor: usize = 0;

    // Main loop: feed 10 ms frames until the cancellation token fires. Once the
    // source's internal queue fills, `capture_frame().await` applies real-time
    // back-pressure, pacing the loop at one 10 ms frame per 10 ms of wall-clock
    // — so no manual timer is needed.
    loop {
        cursor = fill_loading_frame(clip.samples(), cursor, &mut frame_buf);
        let frame = AudioFrame {
            data: std::borrow::Cow::Borrowed(&frame_buf),
            sample_rate,
            num_channels: channels as u32,
            samples_per_channel: samples_per_channel as u32,
        };

        tokio::select! {
            biased;
            _ = token.cancelled() => break,
            res = source.capture_frame(&frame) => {
                if res.is_err() {
                    debug!("Loading audio source closed; loop exiting without fade-out");
                    return;
                }
            }
        }
    }

    // Fade-out phase: scale subsequent frames by a linear 1.0 -> 0.0 ramp so the
    // loop stops without an audible click. The fade composes with the
    // already-volume-scaled clip samples.
    let fade_frames = (LOADING_AUDIO_FADE_OUT_MS / 10).max(1) as usize;
    let total_fade_samples = fade_frames * frame_len;
    let mut fade_index: usize = 0;

    for _ in 0..fade_frames {
        cursor = fill_loading_frame(clip.samples(), cursor, &mut frame_buf);
        fade_index = apply_fade(&mut frame_buf, fade_index, total_fade_samples);
        let frame = AudioFrame {
            data: std::borrow::Cow::Borrowed(&frame_buf),
            sample_rate,
            num_channels: channels as u32,
            samples_per_channel: samples_per_channel as u32,
        };
        if source.capture_frame(&frame).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loading_frame_len_mono_and_stereo() {
        for rate in [8_000u32, 16_000, 24_000, 44_100, 48_000] {
            let expected_mono = rate as usize / 100;
            assert_eq!(
                loading_frame_len(rate, 1),
                expected_mono,
                "mono frame length wrong at {rate} Hz"
            );
            assert_eq!(
                loading_frame_len(rate, 2),
                expected_mono * 2,
                "stereo frame length wrong at {rate} Hz"
            );
        }
    }

    #[test]
    fn test_fill_loading_frame_cursor_wraps_to_expected_offset() {
        // Clip of 10 samples; frames of 4 advance the cursor 4 each time.
        let clip: Vec<i16> = (0..10).collect();
        let mut out = [0i16; 4];
        let mut cursor = 0usize;
        for _ in 0..5 {
            cursor = fill_loading_frame(&clip, cursor, &mut out);
        }
        // After 5 frames of 4 samples, 20 samples consumed; 20 % 10 == 0.
        assert_eq!(cursor, 0);
    }

    #[test]
    fn test_fill_loading_frame_straddles_boundary_tail_then_head() {
        // Clip [10, 20, 30, 40] (len 4); a frame of 3 starting at cursor 2 must
        // yield the tail [30, 40] followed by the head [10]. The cursor advances
        // 3 from 2, wrapping to (2 + 3) % 4 == 1.
        let clip: Vec<i16> = vec![10, 20, 30, 40];
        let mut out = [0i16; 3];
        let cursor = fill_loading_frame(&clip, 2, &mut out);
        assert_eq!(out, [30, 40, 10]);
        assert_eq!(cursor, 1);
    }

    #[test]
    fn test_fill_loading_frame_reproduces_clip_contiguously() {
        // Reading the clip in frames must reproduce its contents in order.
        let clip: Vec<i16> = vec![1, 2, 3, 4, 5, 6];
        let mut reconstructed: Vec<i16> = Vec::new();
        let mut out = [0i16; 2];
        let mut cursor = 0usize;
        for _ in 0..3 {
            cursor = fill_loading_frame(&clip, cursor, &mut out);
            reconstructed.extend_from_slice(&out);
        }
        assert_eq!(reconstructed, clip);
        assert_eq!(cursor, 0);

        // Continuing wraps and reproduces the clip again from the start.
        let mut second_pass: Vec<i16> = Vec::new();
        for _ in 0..3 {
            cursor = fill_loading_frame(&clip, cursor, &mut out);
            second_pass.extend_from_slice(&out);
        }
        assert_eq!(second_pass, clip);
    }

    #[test]
    fn test_fill_loading_frame_empty_clip_is_safe() {
        let clip: Vec<i16> = Vec::new();
        let mut out = [7i16; 4];
        let cursor = fill_loading_frame(&clip, 3, &mut out);
        assert_eq!(cursor, 0);
        assert_eq!(out, [0, 0, 0, 0]);
    }

    #[test]
    fn test_fade_factor_monotonic_non_increasing_and_ends_at_zero() {
        let total = 9usize;
        let mut previous = f32::MAX;
        for index in 0..total {
            let value = fade_factor(index, total);
            assert!((0.0..=1.0).contains(&value), "fade factor out of range");
            assert!(value <= previous, "fade factor not non-increasing");
            previous = value;
        }
        assert_eq!(fade_factor(total - 1, total), 0.0);
    }

    #[test]
    fn test_fade_factor_zero_total_is_safe() {
        assert_eq!(fade_factor(0, 0), 0.0);
        assert_eq!(fade_factor(5, 0), 0.0);
    }

    #[test]
    fn test_apply_fade_produces_decreasing_amplitude_ending_at_zero() {
        // A constant-amplitude frame faded across the whole window must come out
        // monotonically non-increasing in magnitude and end at exactly zero.
        let total = 16usize;
        let mut frame = vec![1000i16; total];
        let next = apply_fade(&mut frame, 0, total);

        assert_eq!(next, total, "fade index must advance by the frame length");
        assert_eq!(frame[total - 1], 0, "the last faded sample must be zero");
        assert!(
            frame[0] > 0 && frame[0] <= 1000,
            "the first faded sample is attenuated but still non-zero"
        );
        for pair in frame.windows(2) {
            assert!(
                pair[1].abs() <= pair[0].abs(),
                "fade amplitude must be monotonically non-increasing"
            );
        }
    }

    #[test]
    fn test_apply_fade_continues_ramp_across_frames() {
        // Splitting the fade across two frames must continue the ramp seamlessly:
        // the second frame resumes where the first stopped and still ends at zero.
        let total = 8usize;
        let mut first = vec![2000i16; 4];
        let mut second = vec![2000i16; 4];

        let next = apply_fade(&mut first, 0, total);
        assert_eq!(next, 4);
        let end = apply_fade(&mut second, next, total);
        assert_eq!(end, total);
        assert_eq!(second[3], 0, "the final faded sample must be zero");

        let combined: Vec<i16> = first.iter().chain(second.iter()).copied().collect();
        for pair in combined.windows(2) {
            assert!(
                pair[1].abs() <= pair[0].abs(),
                "fade amplitude must be non-increasing across the frame boundary"
            );
        }
    }

    #[tokio::test]
    async fn test_run_loading_loop_exits_cleanly_when_precancelled() {
        // A 500 ms 16 kHz mono clip, well above the minimum duration.
        let samples: Vec<i16> = (0..8_000).map(|i| (i % 1000) as i16).collect();
        let data = crate::livekit::loading_clip::raw_pcm_to_base64(&samples);
        let clip = Arc::new(
            crate::livekit::decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), None)
                .expect("raw PCM clip should decode"),
        );

        let source = Arc::new(NativeAudioSource::new(
            AudioSourceOptions {
                echo_cancellation: false,
                noise_suppression: false,
                auto_gain_control: false,
            },
            clip.sample_rate(),
            clip.channels() as u32,
            LOADING_AUDIO_QUEUE_SIZE_MS,
        ));

        // A token cancelled before the loop runs: the loop must skip the main
        // phase, emit only the fade-out, and exit cleanly without an abort.
        let token = CancellationToken::new();
        token.cancel();

        let handle = tokio::spawn(run_loading_loop(source, clip, token));
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;

        assert!(
            outcome.is_ok(),
            "run_loading_loop must exit promptly when the token is pre-cancelled"
        );
        assert!(
            outcome.expect("loop completed within timeout").is_ok(),
            "run_loading_loop task must not panic"
        );
    }
}
