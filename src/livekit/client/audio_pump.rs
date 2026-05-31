//! The single audio writer: one task that owns the published audio source and
//! mixes the loading-indicator overlay into the TTS stream.
//!
//! ## Why a single writer
//!
//! LiveKit is an SFU: it forwards each published track as an independent stream
//! and never mixes audio server-side. Many subscribers — custom browser clients
//! that attach one `<audio>` element, and some SIP bridges — only ever play a
//! single audio track. Publishing the loading indicator on a *second* track
//! therefore fails to reach them. Instead, Sayna publishes exactly one audio
//! track and mixes the loading clip into it here, so any single-track subscriber
//! hears it.
//!
//! A [`NativeAudioSource`] has single-writer semantics: two tasks both calling
//! [`NativeAudioSource::capture_frame`] would *interleave* frames in time, not
//! sum them. So all audio funnels through one pump task — the sole caller of
//! `capture_frame` — which sums the two streams before handing each frame to the
//! source.
//!
//! ## How the pump runs
//!
//! The pump alternates between two modes, driven by [`AudioPumpShared`]:
//!
//! * **Pass-through** (overlay inactive): TTS PCM is captured exactly as it
//!   arrives — no silence padding — so output is bit-identical to feeding the
//!   source directly. When no TTS is buffered the pump parks on a [`Notify`]
//!   until woken by new audio or an overlay start.
//! * **Mixing** (overlay active): the pump emits continuous 10 ms frames built
//!   from the looping clip, summing in any buffered TTS (silence-filled when
//!   speech is absent) with saturating `i16` addition — the same algorithm as
//!   LiveKit's own `AudioMixer`. A stop request triggers a short linear
//!   fade-out, after which the pump returns to pass-through.
//!
//! Because both modes live in one task, mode changes never race a concurrent
//! writer. Real-time pacing comes for free from `capture_frame().await`, which
//! back-pressures once the source's internal queue fills — no manual timer.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arc_swap::ArcSwapOption;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use livekit::webrtc::prelude::AudioFrame;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::LiveKitClient;
use crate::AppError;
use crate::livekit::loading_clip::LoadingClip;
use crate::livekit::resample::resample_loading_samples;

/// Length of the linear fade-out applied when the loading overlay stops, in ms.
///
/// Roughly three 10 ms frames; long enough to avoid an audible click without a
/// perceptible tail.
const LOADING_FADE_OUT_MS: u32 = 30;

/// How long [`stop_audio_pump`] waits for the pump to exit before aborting it.
const PUMP_STOP_TIMEOUT_MS: u64 = 500;

/// Safety bound on how long [`AudioPumpShared::push_tts`] will wait for the pump
/// to drain a full buffer before giving up and buffering anyway.
///
/// Normal back-pressure unblocks within one frame (~10 ms) because the pump
/// drains continuously, so this only ever trips if the pump is stalled or
/// absent — in which case proceeding (buffering, never dropping) is safer than
/// blocking the producer forever.
const TTS_BACKPRESSURE_TIMEOUT_MS: u64 = 5_000;

/// State shared between the single audio pump and the rest of the client.
///
/// Created once in [`LiveKitClient::new`] and reused across reconnects, so the
/// resampled overlay and any buffered TTS survive a transient room rebuild. The
/// pump reads this state; `process_send_audio`, `clear_audio`, the overlay
/// start/stop API, and reconnect write it.
pub(crate) struct AudioPumpShared {
    /// TTS PCM awaiting playback: interleaved `i16` at the track's rate/channels.
    tts: Mutex<VecDeque<i16>>,
    /// Back-pressure threshold (~2 s of audio). Once buffered TTS reaches this,
    /// `push_tts` waits for the pump to drain rather than dropping audio, so a
    /// faster-than-real-time producer is throttled to the pump's real-time rate
    /// without losing any speech.
    max_tts_samples: usize,
    /// Wakes a parked pump when TTS is enqueued or the overlay is started.
    wake: Notify,
    /// Wakes a back-pressured `push_tts` when the pump frees buffer space.
    drained: Notify,
    /// Whether the loading overlay is currently mixed into the output.
    loading_active: AtomicBool,
    /// Set to request that the active overlay fade out and stop.
    loading_stopping: AtomicBool,
    /// The loading clip resampled to the track's rate/channels, ready to mix.
    /// `None` when no loading audio is configured. Lock-free so it can be
    /// updated (even mid-session) without blocking the pump.
    loading_samples: ArcSwapOption<Vec<i16>>,
}

impl AudioPumpShared {
    /// Creates empty shared state sized for the given track format.
    pub(super) fn new(sample_rate: u32, channels: u16) -> Self {
        // ~2 seconds of interleaved audio, matching the previous bound of
        // "200 frames at 100 frames/sec".
        let max_tts_samples = (sample_rate as usize) * (channels as usize) * 2;
        Self {
            tts: Mutex::new(VecDeque::new()),
            max_tts_samples,
            wake: Notify::new(),
            drained: Notify::new(),
            loading_active: AtomicBool::new(false),
            loading_stopping: AtomicBool::new(false),
            loading_samples: ArcSwapOption::from(None),
        }
    }

    /// Buffers TTS samples for the pump, applying back-pressure when the buffer
    /// is full so no audio is ever dropped.
    ///
    /// Samples are interleaved `i16` already at the track's rate/channels. When
    /// the buffer has reached [`Self::max_tts_samples`], this waits for the pump
    /// to drain it — throttling a faster-than-real-time producer (the awaited
    /// TTS callback) to the pump's real-time rate, exactly as the previous direct
    /// `capture_frame().await` back-pressured it. A safety timeout
    /// ([`TTS_BACKPRESSURE_TIMEOUT_MS`]) keeps a stalled or absent pump from
    /// blocking the producer forever; on timeout it buffers anyway rather than
    /// dropping speech.
    pub(super) async fn push_tts(&self, samples: Vec<i16>) {
        let wait_for_room = async {
            loop {
                // Register interest before checking, so a drain that happens
                // between the check and the await is never missed.
                let drained = self.drained.notified();
                if self.tts.lock().await.len() < self.max_tts_samples {
                    break;
                }
                drained.await;
            }
        };
        if tokio::time::timeout(
            std::time::Duration::from_millis(TTS_BACKPRESSURE_TIMEOUT_MS),
            wait_for_room,
        )
        .await
        .is_err()
        {
            warn!("TTS back-pressure wait timed out; the audio pump may be stalled");
        }

        self.tts.lock().await.extend(samples);
        self.wake.notify_one();
    }

    /// Clears buffered TTS samples. Overlay playback is intentionally untouched,
    /// preserving the independence of `clear` from the loading indicator.
    pub(super) async fn clear_tts(&self) {
        self.tts.lock().await.clear();
    }

    /// Stores the resampled overlay clip (already at the track's format), or
    /// clears it with `None`.
    pub(super) fn set_loading_samples(&self, samples: Option<Arc<Vec<i16>>>) {
        self.loading_samples.store(samples);
    }

    /// Returns whether a non-empty overlay clip is configured.
    pub(super) fn has_loading_samples(&self) -> bool {
        self.loading_samples
            .load()
            .as_ref()
            .is_some_and(|clip| !clip.is_empty())
    }

    /// Forces the overlay inactive (used on reconnect/disconnect). The pump
    /// picks this up on its next iteration and returns to pass-through.
    pub(super) fn deactivate_loading(&self) {
        self.loading_active.store(false, Ordering::Release);
        self.loading_stopping.store(false, Ordering::Release);
    }
}

/// Handle to the running audio pump: its task and cancellation token.
pub(crate) struct AudioPumpHandle {
    /// Join handle of the spawned pump task.
    pub(super) handle: tokio::task::JoinHandle<()>,
    /// Token used to request that the pump exit.
    pub(super) token: CancellationToken,
}

/// Returns the interleaved `i16` sample count contained in one 10 ms frame.
///
/// For mono this equals `sample_rate / 100`; for stereo it is twice that.
pub(super) fn frame_len_10ms(sample_rate: u32, channels: u16) -> usize {
    (sample_rate as usize / 100) * channels as usize
}

/// Fills `out` with the next `out.len()` interleaved samples taken from `clip`,
/// wrapping seamlessly at the end of the clip.
///
/// `cursor` is the current read position into `clip`. The new cursor (modulo the
/// clip length) is returned. An empty `clip` is handled safely: `out` is
/// zero-filled and `0` is returned, with no panic.
pub(super) fn fill_loading_frame(clip: &[i16], cursor: usize, out: &mut [i16]) -> usize {
    if clip.is_empty() {
        out.fill(0);
        return 0;
    }

    let len = clip.len();
    let mut pos = cursor % len;
    for slot in out.iter_mut() {
        *slot = clip[pos];
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

/// Builds one `AudioFrame` borrowing `buf`, tagged with the track's fixed
/// `sample_rate`, channel count, and per-channel sample count.
fn build_audio_frame(
    buf: &[i16],
    sample_rate: u32,
    channels: u16,
    samples_per_channel: usize,
) -> AudioFrame<'_> {
    AudioFrame {
        data: Cow::Borrowed(buf),
        sample_rate,
        num_channels: channels as u32,
        samples_per_channel: samples_per_channel as u32,
    }
}

/// Removes up to `max` interleaved samples from the front of the TTS buffer.
///
/// Returns fewer than `max` only when the buffer holds fewer; the count is
/// always a whole number of frames because only whole frames are ever enqueued.
async fn take_tts(shared: &AudioPumpShared, max: usize) -> Vec<i16> {
    let taken: Vec<i16> = {
        let mut queue = shared.tts.lock().await;
        let n = max.min(queue.len());
        queue.drain(..n).collect()
    };
    if !taken.is_empty() {
        // Freed buffer space; wake any back-pressured producer.
        shared.drained.notify_one();
    }
    taken
}

/// Sums buffered TTS into `frame` with saturating `i16` addition, consuming up
/// to `frame.len()` samples. Positions past the available TTS keep the overlay
/// value unchanged (silence-filled speech), so the overlay plays alone when no
/// speech is buffered.
async fn mix_tts_into(shared: &AudioPumpShared, frame: &mut [i16]) {
    let n = {
        let mut queue = shared.tts.lock().await;
        let n = frame.len().min(queue.len());
        for (slot, tts) in frame.iter_mut().zip(queue.drain(..n)) {
            *slot = slot.saturating_add(tts);
        }
        n
    };
    if n > 0 {
        // Freed buffer space; wake any back-pressured producer.
        shared.drained.notify_one();
    }
}

/// Runs the single audio pump until cancelled or its source closes.
///
/// The task owns its `Arc<NativeAudioSource>` clone and a clone of the shared
/// state; it holds no lock across `capture_frame().await`.
async fn run_audio_pump(
    source: Arc<NativeAudioSource>,
    shared: Arc<AudioPumpShared>,
    sample_rate: u32,
    channels: u16,
    token: CancellationToken,
) {
    let frame_len = frame_len_10ms(sample_rate, channels);
    if frame_len == 0 {
        warn!("Audio pump: zero-length frame ({sample_rate} Hz, {channels} ch); exiting");
        return;
    }
    // Per-channel sample count of a full 10 ms frame. `channels >= 1` here
    // because `frame_len != 0`.
    let full_spc = frame_len / channels as usize;
    let fade_total = (LOADING_FADE_OUT_MS / 10).max(1) as usize * frame_len;

    let mut frame_buf: Vec<i16> = vec![0; frame_len];
    // Overlay playback state, owned solely by this task.
    let mut overlay: Option<Arc<Vec<i16>>> = None;
    let mut cursor = 0usize;
    let mut fade_index = 0usize;
    let mut fading = false;

    info!("Audio pump started ({sample_rate} Hz, {channels} ch)");

    loop {
        if token.is_cancelled() {
            break;
        }

        if shared.loading_active.load(Ordering::Acquire) {
            // --- Mixing mode: continuous 10 ms overlay frames, TTS summed in ---

            // On the inactive -> active transition, bind the current clip and
            // restart it from the beginning.
            if overlay.is_none() {
                overlay = shared.loading_samples.load_full();
                cursor = 0;
                fade_index = 0;
                fading = false;
                if overlay.as_ref().is_none_or(|clip| clip.is_empty()) {
                    // Activated without a usable clip; revert to pass-through.
                    shared.deactivate_loading();
                    overlay = None;
                    continue;
                }
                // Invariant: the resampled overlay holds whole interleaved frames,
                // so cursor wrapping in `fill_loading_frame` never drifts channel
                // phase (e.g. swaps L/R on a stereo clip). Guaranteed by the
                // resampler's `out_frames * channels` output and the whole-frames
                // guard in `resample_loading_samples`.
                debug_assert!(
                    overlay
                        .as_ref()
                        .is_some_and(|clip| clip.len() % channels as usize == 0),
                    "overlay clip length must be a whole number of frames"
                );
            }
            let clip = match overlay.as_ref() {
                Some(clip) => clip,
                // Unreachable: set non-empty just above. Revert defensively.
                None => {
                    shared.deactivate_loading();
                    continue;
                }
            };

            if !fading && shared.loading_stopping.load(Ordering::Acquire) {
                fading = true;
            }

            // Fill the next overlay frame, but commit the cursor advance only
            // after the frame is captured, so a cancel mid-frame never skips
            // clip audio.
            let next_cursor = fill_loading_frame(clip, cursor, &mut frame_buf);
            if fading {
                fade_index = apply_fade(&mut frame_buf, fade_index, fade_total);
            }
            mix_tts_into(&shared, &mut frame_buf).await;

            let frame = build_audio_frame(&frame_buf, sample_rate, channels, full_spc);
            tokio::select! {
                biased;
                _ = token.cancelled() => break,
                res = source.capture_frame(&frame) => {
                    if res.is_err() {
                        debug!("Audio pump: source closed during overlay; exiting");
                        break;
                    }
                    cursor = next_cursor;
                    if fading && fade_index >= fade_total {
                        // Fade complete: overlay finished, back to pass-through.
                        shared.deactivate_loading();
                        overlay = None;
                    }
                }
            }
        } else {
            // --- Pass-through mode: play TTS exactly; park when idle ---
            overlay = None;
            let taken = take_tts(&shared, frame_len).await;
            if taken.is_empty() {
                tokio::select! {
                    biased;
                    _ = token.cancelled() => break,
                    _ = shared.wake.notified() => {}
                }
                continue;
            }
            let spc = taken.len() / channels as usize;
            let frame = build_audio_frame(&taken, sample_rate, channels, spc);
            tokio::select! {
                biased;
                _ = token.cancelled() => break,
                res = source.capture_frame(&frame) => {
                    if res.is_err() {
                        debug!("Audio pump: source closed; exiting");
                        break;
                    }
                }
            }
        }
    }

    debug!("Audio pump exited");
}

/// Spawns the audio pump for `source`, storing its handle in `pump_slot` and
/// cancelling/aborting any pump already there first.
///
/// Holding `pump_slot` across the swap means a concurrent reconnect or disconnect
/// (which also locks the slot) can never leave two pumps writing to one source.
pub(super) async fn spawn_audio_pump(
    pump_slot: &Arc<Mutex<Option<AudioPumpHandle>>>,
    shared: Arc<AudioPumpShared>,
    source: Arc<NativeAudioSource>,
    sample_rate: u32,
    channels: u16,
) {
    let token = CancellationToken::new();
    let handle = tokio::spawn(run_audio_pump(
        source,
        shared,
        sample_rate,
        channels,
        token.clone(),
    ));

    let mut slot = pump_slot.lock().await;
    if let Some(previous) = slot.take() {
        previous.token.cancel();
        previous.handle.abort();
    }
    *slot = Some(AudioPumpHandle { handle, token });
}

/// Cancels the running pump (if any), awaiting it briefly then aborting as a
/// backstop. Infallible and idempotent.
pub(super) async fn stop_audio_pump(pump_slot: &Arc<Mutex<Option<AudioPumpHandle>>>) {
    let pump = pump_slot.lock().await.take();
    if let Some(pump) = pump {
        pump.token.cancel();
        let abort = pump.handle.abort_handle();
        if tokio::time::timeout(
            std::time::Duration::from_millis(PUMP_STOP_TIMEOUT_MS),
            pump.handle,
        )
        .await
        .is_err()
        {
            abort.abort();
            warn!("Audio pump did not stop within timeout; aborted");
        }
    }
}

impl LiveKitClient {
    /// Stores the loading clip to play while the agent is busy, resampling it to
    /// the published track's rate and channel count so it can be mixed in.
    ///
    /// May be called before [`LiveKitClient::connect`] or to update the clip
    /// mid-session; the pump picks up the new clip the next time the overlay
    /// starts.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message if the clip cannot be resampled to the
    /// track format (e.g. an unsupported channel count).
    pub fn set_loading_audio_clip(&mut self, clip: LoadingClip) -> Result<(), String> {
        let resampled = resample_loading_samples(
            clip.samples(),
            clip.sample_rate(),
            clip.channels(),
            self.config.sample_rate,
            self.config.channels,
        )?;
        self.audio_pump
            .set_loading_samples(Some(Arc::new(resampled)));
        Ok(())
    }

    /// Starts mixing the configured loading clip into the published audio track.
    ///
    /// Requires an active LiveKit connection and a configured loading clip.
    /// Calling this while the overlay is already active — including during the
    /// brief fade-out after a [`LiveKitClient::stop_loading_audio`] — is an
    /// idempotent no-op.
    ///
    /// # Errors
    ///
    /// Returns an error if the client is not connected, or if no loading clip is
    /// configured for this session.
    pub async fn start_loading_audio(&self) -> Result<(), AppError> {
        if !self.is_connected() {
            return Err(AppError::BadRequest(
                "loading indicator requires an active LiveKit connection; connect to a room first"
                    .to_string(),
            ));
        }

        if !self.audio_pump.has_loading_samples() {
            return Err(AppError::BadRequest(
                "no loading audio configured".to_string(),
            ));
        }

        // Idempotent: a `true` previous value means the overlay is already
        // active (or fading out), so this start is a no-op.
        if self.audio_pump.loading_active.swap(true, Ordering::AcqRel) {
            debug!("Loading audio overlay already active; start request ignored");
            return Ok(());
        }

        self.audio_pump
            .loading_stopping
            .store(false, Ordering::Release);
        self.audio_pump.wake.notify_one();
        info!("Started loading audio overlay");
        Ok(())
    }

    /// Requests that an active loading overlay fade out and stop.
    ///
    /// Infallible, silent, and idempotent: with no overlay active it does
    /// nothing.
    pub async fn stop_loading_audio(&self) {
        if self.audio_pump.loading_active.load(Ordering::Acquire) {
            self.audio_pump
                .loading_stopping
                .store(true, Ordering::Release);
            self.audio_pump.wake.notify_one();
            debug!("Requested loading audio overlay stop");
        }
    }

    // ---- Test-only accessors (avoid direct field access in tests) ----

    #[cfg(test)]
    pub(crate) fn has_loading_samples(&self) -> bool {
        self.audio_pump.has_loading_samples()
    }

    #[cfg(test)]
    pub(crate) fn loading_overlay_active(&self) -> bool {
        self.audio_pump.loading_active.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) async fn tts_buffer_len(&self) -> usize {
        self.audio_pump.tts.lock().await.len()
    }

    #[cfg(test)]
    pub(crate) async fn pump_running(&self) -> bool {
        self.pump
            .lock()
            .await
            .as_ref()
            .is_some_and(|pump| !pump.handle.is_finished())
    }

    /// Clones the running pump's cancellation token, if any, so a test can
    /// observe whether teardown (or `Drop`) cancelled it.
    #[cfg(test)]
    pub(crate) async fn pump_token_for_test(&self) -> Option<CancellationToken> {
        self.pump
            .lock()
            .await
            .as_ref()
            .map(|pump| pump.token.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_len_10ms_mono_and_stereo() {
        for rate in [8_000u32, 16_000, 24_000, 44_100, 48_000] {
            let expected_mono = rate as usize / 100;
            assert_eq!(
                frame_len_10ms(rate, 1),
                expected_mono,
                "mono frame length wrong at {rate} Hz"
            );
            assert_eq!(
                frame_len_10ms(rate, 2),
                expected_mono * 2,
                "stereo frame length wrong at {rate} Hz"
            );
        }
    }

    #[test]
    fn test_fill_loading_frame_cursor_wraps_to_expected_offset() {
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
        let clip: Vec<i16> = vec![10, 20, 30, 40];
        let mut out = [0i16; 3];
        let cursor = fill_loading_frame(&clip, 2, &mut out);
        assert_eq!(out, [30, 40, 10]);
        assert_eq!(cursor, 1);
    }

    #[test]
    fn test_fill_loading_frame_reproduces_clip_contiguously() {
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
        let total = 8usize;
        let mut first = vec![2000i16; 4];
        let mut second = vec![2000i16; 4];

        let next = apply_fade(&mut first, 0, total);
        assert_eq!(next, 4);
        let end = apply_fade(&mut second, next, total);
        assert_eq!(end, total);
        assert_eq!(second[3], 0, "the final faded sample must be zero");
    }

    #[tokio::test]
    async fn test_push_then_take_is_fifo() {
        let shared = AudioPumpShared::new(16_000, 1);
        shared.push_tts(vec![1, 2, 3, 4, 5]).await;
        let first = take_tts(&shared, 3).await;
        assert_eq!(first, vec![1, 2, 3]);
        let rest = take_tts(&shared, 10).await;
        assert_eq!(rest, vec![4, 5]);
        assert!(take_tts(&shared, 10).await.is_empty());
    }

    #[tokio::test]
    async fn test_push_tts_backpressures_until_drained() {
        // Mono 8 kHz -> back-pressure threshold is 8000 * 1 * 2 = 16000 samples.
        let shared = Arc::new(AudioPumpShared::new(8_000, 1));
        let bound = 16_000usize;

        // Fill exactly to the threshold; this push does not block.
        shared.push_tts(vec![1i16; bound]).await;
        assert_eq!(shared.tts.lock().await.len(), bound);

        // A further push must block (back-pressure) until the buffer is drained,
        // never dropping audio.
        let pusher_shared = Arc::clone(&shared);
        let pusher = tokio::spawn(async move {
            pusher_shared.push_tts((0..100i16).collect()).await;
        });

        // Give the spawned push a moment; it must still be blocked.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !pusher.is_finished(),
            "push_tts must block while the buffer is at the threshold"
        );

        // Draining frees space and signals the back-pressured producer.
        let drained = take_tts(&shared, 8_000).await;
        assert_eq!(drained.len(), 8_000);

        // The blocked push now completes; nothing was dropped.
        tokio::time::timeout(std::time::Duration::from_secs(1), pusher)
            .await
            .expect("the back-pressured push must complete after draining")
            .expect("pusher task must not panic");
        assert_eq!(shared.tts.lock().await.len(), bound - 8_000 + 100);
    }

    #[tokio::test]
    async fn test_mix_tts_into_sums_with_saturation() {
        let shared = AudioPumpShared::new(16_000, 1);
        shared.push_tts(vec![100, -100, i16::MAX, i16::MIN]).await;
        // Overlay values chosen to exercise normal add and both clip directions.
        let mut frame = vec![50i16, -50, 10, -10];
        mix_tts_into(&shared, &mut frame).await;
        assert_eq!(
            frame,
            vec![150, -150, i16::MAX, i16::MIN],
            "mix must be a saturating sum"
        );
    }

    #[tokio::test]
    async fn test_mix_tts_into_silence_fills_remaining_overlay() {
        let shared = AudioPumpShared::new(16_000, 1);
        shared.push_tts(vec![1000, 2000]).await; // only two TTS samples
        let mut frame = vec![5i16, 5, 5, 5];
        mix_tts_into(&shared, &mut frame).await;
        // First two summed; the rest keep the overlay value (no TTS = silence).
        assert_eq!(frame, vec![1005, 2005, 5, 5]);
    }

    #[tokio::test]
    async fn test_deactivate_loading_resets_flags() {
        let shared = AudioPumpShared::new(16_000, 1);
        shared.loading_active.store(true, Ordering::Release);
        shared.loading_stopping.store(true, Ordering::Release);
        shared.deactivate_loading();
        assert!(!shared.loading_active.load(Ordering::Acquire));
        assert!(!shared.loading_stopping.load(Ordering::Acquire));
    }

    #[test]
    fn test_has_loading_samples_reflects_clip() {
        let shared = AudioPumpShared::new(16_000, 1);
        assert!(!shared.has_loading_samples());
        shared.set_loading_samples(Some(Arc::new(vec![1, 2, 3])));
        assert!(shared.has_loading_samples());
        shared.set_loading_samples(Some(Arc::new(Vec::new())));
        assert!(!shared.has_loading_samples(), "empty clip is not usable");
        shared.set_loading_samples(None);
        assert!(!shared.has_loading_samples());
    }

    // Constructs a real libwebrtc `NativeAudioSource`. libwebrtc's global
    // runtime intermittently segfaults under the full unit-test binary's
    // concurrency, so this test is `#[ignore]`d from the default run and
    // executed isolated by a dedicated CI step (see `.github/workflows/ci.yml`).
    // Run locally with:
    //   cargo test --all-features --lib -- --ignored --test-threads=1 livekit_native_
    #[tokio::test]
    #[ignore = "creates native libwebrtc objects; run isolated via the dedicated CI step (see ci.yml)"]
    async fn livekit_native_pump_exits_on_cancel() {
        let source = Arc::new(NativeAudioSource::new(
            super::super::sayna_audio_source_options(),
            16_000,
            1,
            100,
        ));
        let shared = Arc::new(AudioPumpShared::new(16_000, 1));
        let token = CancellationToken::new();

        let handle = tokio::spawn(run_audio_pump(source, shared, 16_000, 1, token.clone()));

        // The pump starts idle (no TTS, no overlay) and parks. Cancelling must
        // make it exit promptly.
        token.cancel();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        assert!(outcome.is_ok(), "pump must exit promptly on cancel");
        assert!(
            outcome.expect("pump joined within timeout").is_ok(),
            "pump task must not panic"
        );
    }
}
