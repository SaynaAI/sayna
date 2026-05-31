//! Offline sample-rate and channel conversion for the loading-indicator clip.
//!
//! The loading clip is mixed into the *single* published audio track (see
//! [`super::client`]'s audio pump), so its PCM must match that track's sample
//! rate and channel count before it can be summed with TTS audio frame-for-frame.
//! A client-supplied clip can be any supported rate (8–48 kHz) and either mono or
//! stereo, so this module reconciles it to the track format exactly once, at clip
//! load time — never on the audio hot path.
//!
//! Rate conversion uses [`rubato`]'s synchronous FFT resampler ([`rubato::Fft`]),
//! a high-quality band-limited resampler well suited to a fixed input→output
//! ratio. Channel conversion is a plain interleaved remix: mono is duplicated to
//! both stereo channels, and stereo is downmixed by averaging. Conversion runs in
//! `f32` and returns interleaved `i16` rounded and saturated back into range.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::audioadapter_buffers::owned::InterleavedOwned;
use rubato::{Fft, FixedSync, Resampler};

/// Desired processing chunk size handed to [`rubato::Fft`], in frames.
///
/// `rubato` only rejects a zero chunk size; any positive value is rounded up to
/// a valid FFT size derived from `gcd(src_rate, dst_rate)`, so this is purely a
/// throughput/latency hint for an offline one-shot conversion. 1024 frames keeps
/// the FFT work modest while comfortably covering a clip's worth of audio.
const RESAMPLE_CHUNK_FRAMES: usize = 1024;

/// Number of sub-chunks per processing chunk (see [`rubato::Fft::new`]).
///
/// One sub-chunk is sufficient for an offline conversion; `rubato` clamps `0` to
/// `1` regardless.
const RESAMPLE_SUB_CHUNKS: usize = 1;

/// Resamples and remixes an interleaved `i16` clip to a target track format.
///
/// `samples` is interleaved PCM with `src_channels` channels at `src_rate` Hz.
/// The result is interleaved PCM with `dst_channels` channels at `dst_rate` Hz,
/// suitable for summing into the published audio source frame-for-frame.
///
/// When the source already matches the target rate and channel count the input
/// is cloned verbatim (no resampler is constructed). When only the rate matches,
/// channel remix runs without resampling.
///
/// Only mono (`1`) and stereo (`2`) are supported on either side, matching the
/// loading-clip decoder's own validation; any other channel count, or a zero
/// sample rate, returns an error.
///
/// # Errors
///
/// Returns a human-readable error if the channel counts are unsupported, a rate
/// is zero, or the resampler reports a construction/processing failure.
pub(crate) fn resample_loading_samples(
    samples: &[i16],
    src_rate: u32,
    src_channels: u16,
    dst_rate: u32,
    dst_channels: u16,
) -> Result<Vec<i16>, String> {
    if !matches!(src_channels, 1 | 2) || !matches!(dst_channels, 1 | 2) {
        return Err(format!(
            "loading audio channel conversion supports mono or stereo only \
             (got {src_channels} -> {dst_channels})"
        ));
    }
    if src_rate == 0 || dst_rate == 0 {
        return Err("loading audio sample rate must be non-zero".to_string());
    }
    if !samples.len().is_multiple_of(src_channels as usize) {
        return Err(format!(
            "loading audio sample count {} is not a whole number of {src_channels}-channel frames",
            samples.len()
        ));
    }

    // Fast path: already in the track's format, nothing to convert.
    if src_rate == dst_rate && src_channels == dst_channels {
        return Ok(samples.to_vec());
    }

    // Remix to the target channel count up front so the (potentially expensive)
    // resampler runs over exactly the channels the track needs.
    let remixed = remix_to_f32(samples, src_channels, dst_channels)?;

    let converted = if src_rate == dst_rate {
        // Channel-only conversion; no resampling required.
        remixed
    } else {
        resample_f32(&remixed, dst_channels as usize, src_rate, dst_rate)?
    };

    Ok(f32_to_i16(&converted))
}

/// Remixes interleaved `i16` `samples` from `src_channels` to `dst_channels`,
/// widening to `f32`.
///
/// Mono → stereo duplicates each sample into both channels; stereo → mono
/// averages the two channels; equal channel counts are a straight widening copy.
fn remix_to_f32(samples: &[i16], src_channels: u16, dst_channels: u16) -> Result<Vec<f32>, String> {
    match (src_channels, dst_channels) {
        (s, d) if s == d => Ok(samples.iter().map(|&v| f32::from(v)).collect()),
        (1, 2) => {
            let mut out = Vec::with_capacity(samples.len() * 2);
            for &s in samples {
                let v = f32::from(s);
                out.push(v);
                out.push(v);
            }
            Ok(out)
        }
        (2, 1) => {
            let mut out = Vec::with_capacity(samples.len() / 2);
            for pair in samples.chunks_exact(2) {
                out.push((f32::from(pair[0]) + f32::from(pair[1])) * 0.5);
            }
            Ok(out)
        }
        _ => Err(format!(
            "unsupported loading audio channel conversion {src_channels} -> {dst_channels}"
        )),
    }
}

/// Resamples interleaved `f32` audio from `src_rate` to `dst_rate`.
///
/// `interleaved` holds `channels`-interleaved samples; the returned vector is
/// interleaved at the same channel count with the resampler's warm-up delay
/// trimmed off, so it represents the clip resampled one-for-one in time.
fn resample_f32(
    interleaved: &[f32],
    channels: usize,
    src_rate: u32,
    dst_rate: u32,
) -> Result<Vec<f32>, String> {
    let src_frames = interleaved.len() / channels;
    if src_frames == 0 {
        return Ok(Vec::new());
    }

    let mut resampler = Fft::<f32>::new(
        src_rate as usize,
        dst_rate as usize,
        RESAMPLE_CHUNK_FRAMES,
        RESAMPLE_SUB_CHUNKS,
        channels,
        FixedSync::Input,
    )
    .map_err(|e| format!("failed to construct loading audio resampler: {e}"))?;

    let input = InterleavedSlice::new(interleaved, channels, src_frames)
        .map_err(|e| format!("invalid loading audio resampler input: {e}"))?;

    // `process_all_into_buffer` consumes the whole clip — chunking, the final
    // partial chunk, and the silence flush — and writes the delay-trimmed result
    // to the front of the output buffer. The buffer must be sized for the worst
    // case it may touch (delay + max output chunk + expected frames).
    let out_capacity_frames = resampler.process_all_needed_output_len(src_frames);
    let mut output = InterleavedOwned::<f32>::new(0.0, channels, out_capacity_frames);

    let (_in_frames, out_frames) = resampler
        .process_all_into_buffer(&input, &mut output, src_frames, None)
        .map_err(|e| format!("failed to resample loading audio: {e}"))?;

    let mut data = output.take_data();
    data.truncate(out_frames * channels);
    Ok(data)
}

/// Converts `f32` PCM back to `i16`, rounding to nearest and saturating into the
/// `i16` range so resampler overshoot can never wrap.
fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&v| v.round().clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frame count rubato produces for a clip: `ceil(dst/src * input_frames)`.
    fn expected_out_frames(src_frames: usize, src_rate: u32, dst_rate: u32) -> usize {
        (f64::from(dst_rate) / f64::from(src_rate) * src_frames as f64).ceil() as usize
    }

    #[test]
    fn identical_format_is_returned_verbatim() {
        let samples: Vec<i16> = (0..480).map(|i| (i * 7 % 600 - 300) as i16).collect();
        let out = resample_loading_samples(&samples, 16_000, 1, 16_000, 1).unwrap();
        assert_eq!(out, samples, "matching rate+channels must clone the input");
    }

    #[test]
    fn stereo_format_unchanged_is_returned_verbatim() {
        // Interleaved stereo, equal rate+channels: an exact clone, no averaging.
        let samples: Vec<i16> = vec![10, -10, 20, -20, 30, -30, 40, -40];
        let out = resample_loading_samples(&samples, 24_000, 2, 24_000, 2).unwrap();
        assert_eq!(out, samples);
    }

    #[test]
    fn mono_to_stereo_duplicates_each_sample() {
        let samples: Vec<i16> = vec![100, 200, 300, -400];
        let out = resample_loading_samples(&samples, 16_000, 1, 16_000, 2).unwrap();
        assert_eq!(out, vec![100, 100, 200, 200, 300, 300, -400, -400]);
    }

    #[test]
    fn stereo_to_mono_averages_channels() {
        // (l + r) / 2, rounded to nearest i16.
        let samples: Vec<i16> = vec![100, 200, -50, 50, 1000, 1001];
        let out = resample_loading_samples(&samples, 16_000, 2, 16_000, 1).unwrap();
        // (100+200)/2=150, (-50+50)/2=0, (1000+1001)/2=1000.5 -> 1001 (round half up via f32)
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], 150);
        assert_eq!(out[1], 0);
        assert!(
            (out[2] - 1001).abs() <= 1,
            "average rounds to nearest, got {}",
            out[2]
        );
    }

    #[test]
    fn upsample_doubles_frame_count() {
        // 0.5 s of 8 kHz mono = 4000 frames -> ~8000 frames at 16 kHz.
        let src_frames = 4_000usize;
        let samples: Vec<i16> = vec![0; src_frames];
        let out = resample_loading_samples(&samples, 8_000, 1, 16_000, 1).unwrap();
        assert_eq!(out.len(), expected_out_frames(src_frames, 8_000, 16_000));
    }

    #[test]
    fn downsample_halves_frame_count() {
        // 48 kHz mono -> 24 kHz mono is a 2:1 decimation.
        let src_frames = 4_800usize;
        let samples: Vec<i16> = vec![0; src_frames];
        let out = resample_loading_samples(&samples, 48_000, 1, 24_000, 1).unwrap();
        assert_eq!(out.len(), expected_out_frames(src_frames, 48_000, 24_000));
    }

    #[test]
    fn combined_rate_and_channel_conversion_length_and_channels() {
        // Stereo 44.1 kHz -> mono 24 kHz: remix to mono, then resample.
        let src_frames = 4_410usize;
        let samples: Vec<i16> = vec![0; src_frames * 2];
        let out = resample_loading_samples(&samples, 44_100, 2, 24_000, 1).unwrap();
        assert_eq!(out.len(), expected_out_frames(src_frames, 44_100, 24_000));
    }

    #[test]
    fn dc_level_is_preserved_through_resampling() {
        // A constant (DC) signal must survive band-limited resampling unchanged
        // except for warm-up at the very edges, which the delay trim removes.
        const LEVEL: i16 = 1_000;
        let src_frames = 2_000usize;
        let samples: Vec<i16> = vec![LEVEL; src_frames];
        let out = resample_loading_samples(&samples, 8_000, 1, 16_000, 1).unwrap();

        // Inspect the interior, away from both ends, where any edge transient has
        // fully settled.
        let mid = &out[out.len() / 4..out.len() * 3 / 4];
        for &s in mid {
            assert!(
                (i32::from(s) - i32::from(LEVEL)).abs() <= 5,
                "DC level should be preserved, got {s}"
            );
        }
    }

    #[test]
    fn unsupported_channel_count_is_rejected() {
        let samples: Vec<i16> = vec![0; 100];
        assert!(resample_loading_samples(&samples, 16_000, 3, 16_000, 1).is_err());
        assert!(resample_loading_samples(&samples, 16_000, 1, 16_000, 0).is_err());
    }

    #[test]
    fn zero_sample_rate_is_rejected() {
        let samples: Vec<i16> = vec![0; 100];
        assert!(resample_loading_samples(&samples, 0, 1, 16_000, 1).is_err());
        assert!(resample_loading_samples(&samples, 16_000, 1, 0, 1).is_err());
    }

    #[test]
    fn f32_to_i16_saturates_without_wrapping() {
        let input = vec![40_000.0_f32, -40_000.0, 100.4, 100.6, -0.4, -0.6];
        let out = f32_to_i16(&input);
        assert_eq!(out, vec![i16::MAX, i16::MIN, 100, 101, 0, -1]);
    }
}
