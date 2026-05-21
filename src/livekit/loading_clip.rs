//! # Loading Clip Decode Layer
//!
//! Pure decoding and validation for the "loading indicator" audio clip — a short
//! audio sample played back to a caller while the system is busy (for example,
//! while a slow tool or model call is in flight).
//!
//! This module is intentionally a self-contained decode/validation layer. It
//! depends only on `std`, [`hound`], and [`base64`]; it knows nothing about
//! WebSocket messages or LiveKit-client types. Callers pass a base64-encoded
//! audio payload (WAV or raw PCM) and receive a fully decoded, validated, and
//! volume-scaled [`LoadingClip`] ready for playback.

use base64::{Engine as _, engine::general_purpose};

/// Minimum accepted duration of a loading clip, in milliseconds.
///
/// Clips shorter than this are rejected because they are too brief to be a
/// meaningful audible indicator.
pub const MIN_LOADING_AUDIO_MS: u32 = 250;

/// Maximum accepted duration of a loading clip, in milliseconds.
///
/// Clips longer than this are rejected to bound memory use and avoid playing an
/// excessively long sample.
pub const MAX_LOADING_AUDIO_MS: u32 = 10_000;

/// Lowest accepted clip sample rate, in Hz.
pub const LOADING_AUDIO_SAMPLE_RATE_MIN: u32 = 8_000;

/// Highest accepted clip sample rate, in Hz.
pub const LOADING_AUDIO_SAMPLE_RATE_MAX: u32 = 48_000;

/// Maximum accepted size of the decoded audio payload, in bytes.
///
/// Derived as the worst-case size of a clip at the maximum sample rate, with
/// stereo channels, 16-bit (2 bytes per sample) PCM, lasting the maximum
/// allowed duration. This evaluates to `1_920_000` bytes and is used as an
/// early guard to reject oversized payloads before fully decoding them.
pub const MAX_LOADING_AUDIO_DECODED_BYTES: usize = (LOADING_AUDIO_SAMPLE_RATE_MAX as usize)
    * 2 /* max channels */
    * 2 /* bytes per i16 */
    * (MAX_LOADING_AUDIO_MS as usize)
    / 1000;

/// A decoded, validated, ready-to-play loading audio sample.
///
/// The PCM data in [`LoadingClip::samples`] is flat and interleaved when
/// stereo, and already has the requested volume scaling applied. Construct one
/// via [`decode_loading_clip`].
#[derive(Clone)]
pub struct LoadingClip {
    /// Decoded PCM, flat and interleaved when stereo, with `volume` applied.
    samples: Vec<i16>,
    /// The clip's native sample rate, in Hz.
    sample_rate: u32,
    /// Number of channels: `1` (mono) or `2` (stereo).
    channels: u16,
    /// The volume (in `0.0..=1.0`) that was applied to `samples`.
    volume: f32,
}

impl std::fmt::Debug for LoadingClip {
    /// Formats the clip without dumping the (potentially ~1 MB) sample buffer;
    /// only the sample count and metadata are shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadingClip")
            .field("sample_count", &self.samples.len())
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("volume", &self.volume)
            .finish()
    }
}

impl LoadingClip {
    /// Returns the decoded PCM samples, flat and interleaved when stereo, with
    /// volume scaling already applied.
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }

    /// Returns the clip's native sample rate, in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Returns the number of channels: `1` (mono) or `2` (stereo).
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Returns the volume (in `0.0..=1.0`) that was applied to the samples.
    pub fn volume(&self) -> f32 {
        self.volume
    }
}

/// Applies linear volume scaling to PCM samples without overflow.
///
/// Each sample is widened to `f32`, multiplied by `volume`, rounded, and
/// clamped back into the `i16` range so that no value can wrap or overflow.
fn apply_volume(samples: &mut [i16], volume: f32) {
    for sample in samples.iter_mut() {
        let scaled = (f32::from(*sample) * volume).round();
        *sample = scaled.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
    }
}

/// Decodes and validates a base64-encoded loading audio clip.
///
/// `data` is the base64-encoded payload. `format` is an optional explicit
/// container hint (`"wav"` or `"pcm"`); when `None` the format is auto-detected
/// from the decoded bytes. `sample_rate` and `channels` describe raw PCM input
/// and are ignored for WAV input (the WAV header is authoritative). `volume` is
/// an optional playback gain in `0.0..=1.0`; out-of-range values are clamped
/// rather than rejected, and the applied value is recorded on the returned clip.
///
/// On success returns a fully decoded, validated, volume-scaled [`LoadingClip`].
/// On failure returns an `Err` with a human-readable, end-user-facing message.
pub fn decode_loading_clip(
    data: &str,
    format: Option<&str>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
    volume: Option<f32>,
) -> Result<LoadingClip, String> {
    // 1. Early size guard based on the base64 length (~3 decoded bytes per 4
    //    encoded characters), so an obviously oversized payload is rejected
    //    before allocating the decoded buffer.
    let estimated_decoded_bytes = data.len() / 4 * 3;
    if estimated_decoded_bytes > MAX_LOADING_AUDIO_DECODED_BYTES {
        return Err(format!(
            "loading_audio.data decoded audio would exceed the maximum size of \
             {MAX_LOADING_AUDIO_DECODED_BYTES} bytes"
        ));
    }

    // 2. Base64 decode.
    let bytes = general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "loading_audio.data is not valid base64".to_string())?;

    // 3. Post-decode size guard against the exact decoded length.
    if bytes.len() > MAX_LOADING_AUDIO_DECODED_BYTES {
        return Err(format!(
            "loading_audio.data decoded audio exceeds the maximum size of \
             {MAX_LOADING_AUDIO_DECODED_BYTES} bytes"
        ));
    }

    // 4. Format detection: explicit hint wins, otherwise auto-detect.
    let is_wav = match format {
        Some(fmt) => match fmt.trim().to_lowercase().as_str() {
            "wav" => true,
            "pcm" => false,
            _ => {
                return Err("loading_audio.format must be \"wav\" or \"pcm\"".to_string());
            }
        },
        None => bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE",
    };

    // 5/6. Decode into PCM samples plus the authoritative sample rate and
    //      channel count for the chosen container.
    let (mut samples, sample_rate, channels) = if is_wav {
        decode_wav(&bytes)?
    } else {
        decode_raw_pcm(&bytes, sample_rate, channels)?
    };

    // 7. Validate channel count.
    if channels != 1 && channels != 2 {
        return Err("loading_audio supports mono or stereo only".to_string());
    }

    // 8. Validate sample rate.
    if !(LOADING_AUDIO_SAMPLE_RATE_MIN..=LOADING_AUDIO_SAMPLE_RATE_MAX).contains(&sample_rate) {
        return Err(format!(
            "loading_audio sample rate must be between {LOADING_AUDIO_SAMPLE_RATE_MIN} Hz \
             and {LOADING_AUDIO_SAMPLE_RATE_MAX} Hz"
        ));
    }

    // 9. Validate duration.
    let frames = samples.len() / channels as usize;
    if samples.is_empty() || frames == 0 {
        return Err(format!(
            "loading_audio is too short; the minimum duration is {MIN_LOADING_AUDIO_MS} ms"
        ));
    }
    let duration_ms = frames as u64 * 1000 / sample_rate as u64;
    if duration_ms < MIN_LOADING_AUDIO_MS as u64 {
        return Err(format!(
            "loading_audio is too short; the minimum duration is {MIN_LOADING_AUDIO_MS} ms"
        ));
    }
    if duration_ms > MAX_LOADING_AUDIO_MS as u64 {
        return Err(format!(
            "loading_audio is too long; the maximum duration is {MAX_LOADING_AUDIO_MS} ms"
        ));
    }

    // 10. Apply volume scaling. Out-of-range values are clamped, not rejected.
    let volume = volume.unwrap_or(1.0).clamp(0.0, 1.0);
    if (volume - 1.0).abs() > f32::EPSILON {
        apply_volume(&mut samples, volume);
    }

    // 11. Done.
    Ok(LoadingClip {
        samples,
        sample_rate,
        channels,
        volume,
    })
}

/// Decodes a 16-bit PCM WAV payload, returning its samples, sample rate, and
/// channel count taken from the (authoritative) WAV header.
fn decode_wav(bytes: &[u8]) -> Result<(Vec<i16>, u32, u16), String> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("loading_audio is not a valid WAV file: {e}"))?;

    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(
            "loading_audio: only 16-bit PCM is supported; please provide 16-bit PCM audio"
                .to_string(),
        );
    }

    // Guard against a forged WAV `data`-chunk length. `hound`'s sample
    // iterator pre-allocates its `Vec` from the header's declared sample
    // count, so a tiny file claiming an enormous `data` chunk would force a
    // multi-gigabyte allocation before a single sample byte is read. The
    // base64-length and decoded-byte guards in `decode_loading_clip` cannot
    // catch this: the payload is small on the wire but its header claims to
    // decode huge. Reject using the declared count, before `collect()`.
    let declared_bytes = (reader.len() as usize).saturating_mul(2);
    if declared_bytes > MAX_LOADING_AUDIO_DECODED_BYTES {
        return Err(format!(
            "loading_audio.data decoded audio exceeds the maximum size of \
             {MAX_LOADING_AUDIO_DECODED_BYTES} bytes"
        ));
    }

    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<i16>, _>>()
        .map_err(|e| format!("loading_audio could not be read as 16-bit PCM: {e}"))?;

    // Reject a malformed WAV whose sample count is not a whole number of
    // interleaved frames, mirroring the raw-PCM frame-alignment check.
    if spec.channels == 0 || !samples.len().is_multiple_of(spec.channels as usize) {
        return Err("loading_audio WAV data is not a whole number of audio frames".to_string());
    }

    Ok((samples, spec.sample_rate, spec.channels))
}

/// Decodes a little-endian raw 16-bit PCM payload using the caller-supplied
/// `sample_rate` and `channels`, validating that the byte length is
/// frame-aligned.
fn decode_raw_pcm(
    bytes: &[u8],
    sample_rate: Option<u32>,
    channels: Option<u16>,
) -> Result<(Vec<i16>, u32, u16), String> {
    let sample_rate = sample_rate
        .ok_or_else(|| "sample_rate is required for raw PCM loading_audio".to_string())?;
    let channels = channels.unwrap_or(1);

    // `channels` defaults to 1 and is validated against {1, 2} by the caller,
    // so `frame_bytes` is always non-zero (>= 2) here.
    let frame_bytes = 2 * channels as usize;
    if !bytes.len().is_multiple_of(2) || !bytes.len().is_multiple_of(frame_bytes) {
        return Err("loading_audio raw PCM length is not frame-aligned".to_string());
    }

    let samples = bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    Ok((samples, sample_rate, channels))
}

/// Encodes interleaved 16-bit samples as little-endian raw PCM, then base64.
///
/// Test-only helper shared by the loading-audio test suites in this crate.
#[cfg(test)]
pub(crate) fn raw_pcm_to_base64(samples: &[i16]) -> String {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an in-memory 16-bit-PCM WAV with the given sample rate, channel
    /// count, and interleaved samples, returning its base64 encoding.
    fn make_wav_base64(sample_rate: u32, channels: u16, samples: &[i16]) -> String {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        {
            let mut writer =
                hound::WavWriter::new(&mut cursor, spec).expect("WAV writer should be created");
            for sample in samples {
                writer
                    .write_sample(*sample)
                    .expect("sample should be written");
            }
            writer.finalize().expect("WAV writer should finalize");
        }
        general_purpose::STANDARD.encode(cursor.into_inner())
    }

    /// Builds an in-memory WAV with an arbitrary bit depth and sample format so
    /// that 24-bit and 32-bit-float rejection paths can be exercised. Samples
    /// are written as `i32` values; for the float format they are written via
    /// `write_sample` with an `f32` cast.
    fn make_wav_base64_with_format(
        sample_rate: u32,
        channels: u16,
        bits_per_sample: u16,
        sample_format: hound::SampleFormat,
        frame_count: usize,
    ) -> String {
        let spec = hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample,
            sample_format,
        };
        let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
        {
            let mut writer =
                hound::WavWriter::new(&mut cursor, spec).expect("WAV writer should be created");
            for i in 0..(frame_count * channels as usize) {
                match sample_format {
                    hound::SampleFormat::Float => {
                        let value = ((i % 100) as f32 / 100.0) - 0.5;
                        writer
                            .write_sample(value)
                            .expect("float sample should be written");
                    }
                    hound::SampleFormat::Int => {
                        writer
                            .write_sample((i % 1000) as i32)
                            .expect("int sample should be written");
                    }
                }
            }
            writer.finalize().expect("WAV writer should finalize");
        }
        general_purpose::STANDARD.encode(cursor.into_inner())
    }

    #[test]
    fn test_valid_mono_wav_decodes() {
        // 8000 samples at 16 kHz mono == 500 ms.
        let samples: Vec<i16> = (0..8000).map(|i| (i % 1000) as i16).collect();
        let data = make_wav_base64(16_000, 1, &samples);

        let clip = decode_loading_clip(&data, None, None, None, None)
            .expect("valid mono WAV should decode");

        assert_eq!(clip.samples().len(), 8000);
        assert_eq!(clip.sample_rate(), 16_000);
        assert_eq!(clip.channels(), 1);
    }

    #[test]
    fn test_valid_stereo_wav_decodes() {
        // 8000 frames * 2 channels == 16000 interleaved samples, 500 ms at 16 kHz.
        let samples: Vec<i16> = (0..16_000).map(|i| (i % 1000) as i16).collect();
        let data = make_wav_base64(16_000, 2, &samples);

        let clip = decode_loading_clip(&data, None, None, None, None)
            .expect("valid stereo WAV should decode");

        assert_eq!(clip.channels(), 2);
        assert_eq!(clip.samples().len(), 16_000);
        assert_eq!(clip.sample_rate(), 16_000);
    }

    #[test]
    fn test_valid_raw_pcm_decodes() {
        let samples: Vec<i16> = (0..8000).map(|i| (i % 500) as i16).collect();
        let data = raw_pcm_to_base64(&samples);

        let clip = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), None)
            .expect("valid raw PCM should decode");

        assert_eq!(clip.samples().len(), 8000);
        assert_eq!(clip.sample_rate(), 16_000);
        assert_eq!(clip.channels(), 1);
    }

    #[test]
    fn test_raw_pcm_without_sample_rate_rejected() {
        let samples: Vec<i16> = (0..8000).map(|i| (i % 500) as i16).collect();
        let data = raw_pcm_to_base64(&samples);

        let err = decode_loading_clip(&data, Some("pcm"), None, Some(1), None)
            .expect_err("raw PCM without sample_rate should be rejected");

        assert_eq!(err, "sample_rate is required for raw PCM loading_audio");
    }

    #[test]
    fn test_auto_detection_wav_and_raw_pcm() {
        // WAV bytes without a format hint are detected as WAV.
        let wav_samples: Vec<i16> = (0..8000).map(|i| (i % 1000) as i16).collect();
        let wav_data = make_wav_base64(16_000, 1, &wav_samples);
        let wav_clip = decode_loading_clip(&wav_data, None, None, None, None)
            .expect("WAV bytes should auto-detect as WAV");
        assert_eq!(wav_clip.sample_rate(), 16_000);

        // Non-WAV bytes without a format hint are treated as raw PCM.
        let pcm_samples: Vec<i16> = (0..8000).map(|i| (i % 500) as i16).collect();
        let pcm_data = raw_pcm_to_base64(&pcm_samples);
        let pcm_clip = decode_loading_clip(&pcm_data, None, Some(16_000), Some(1), None)
            .expect("non-WAV bytes should auto-detect as raw PCM");
        assert_eq!(pcm_clip.samples().len(), 8000);
    }

    #[test]
    fn test_explicit_format_overrides_detection() {
        // Raw PCM samples whose bytes do not form a RIFF/WAVE header, but the
        // caller forces the "pcm" format anyway.
        let pcm_samples: Vec<i16> = (0..8000).map(|i| (i % 500) as i16).collect();
        let pcm_data = raw_pcm_to_base64(&pcm_samples);
        let clip = decode_loading_clip(&pcm_data, Some("pcm"), Some(16_000), Some(1), None)
            .expect("explicit pcm format should decode raw PCM");
        assert_eq!(clip.samples().len(), 8000);

        // Forcing "wav" on raw PCM bytes fails WAV parsing.
        let err = decode_loading_clip(&pcm_data, Some("wav"), Some(16_000), Some(1), None)
            .expect_err("explicit wav format on raw PCM should fail");
        assert!(
            err.contains("not a valid WAV file"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_invalid_base64_rejected() {
        let err = decode_loading_clip("not valid base64 @@@", None, None, None, None)
            .expect_err("invalid base64 should be rejected");
        assert_eq!(err, "loading_audio.data is not valid base64");
    }

    #[test]
    fn test_24bit_wav_rejected() {
        // 8000 frames at 16 kHz keeps the duration valid so the 16-bit check
        // is what triggers the rejection.
        let data = make_wav_base64_with_format(16_000, 1, 24, hound::SampleFormat::Int, 8000);
        let err = decode_loading_clip(&data, None, None, None, None)
            .expect_err("24-bit WAV should be rejected");
        assert!(err.contains("16-bit"), "unexpected error: {err}");
    }

    #[test]
    fn test_32bit_float_wav_rejected() {
        let data = make_wav_base64_with_format(16_000, 1, 32, hound::SampleFormat::Float, 8000);
        let err = decode_loading_clip(&data, None, None, None, None)
            .expect_err("32-bit float WAV should be rejected");
        assert!(err.contains("16-bit"), "unexpected error: {err}");
    }

    #[test]
    fn test_oversized_input_rejected() {
        // Raw PCM larger than MAX_LOADING_AUDIO_DECODED_BYTES.
        let oversized_samples = MAX_LOADING_AUDIO_DECODED_BYTES; // each i16 == 2 bytes
        let samples: Vec<i16> = vec![0; oversized_samples];
        let data = raw_pcm_to_base64(&samples);

        let err = decode_loading_clip(&data, Some("pcm"), Some(48_000), Some(1), None)
            .expect_err("oversized input should be rejected");
        assert!(err.contains("maximum size"), "unexpected error: {err}");
    }

    #[test]
    fn test_too_short_clip_rejected() {
        // 1000 samples at 16 kHz mono == ~62 ms, below MIN_LOADING_AUDIO_MS.
        let samples: Vec<i16> = vec![100; 1000];
        let data = raw_pcm_to_base64(&samples);

        let err = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), None)
            .expect_err("too-short clip should be rejected");
        assert!(err.contains("too short"), "unexpected error: {err}");
    }

    #[test]
    fn test_too_long_clip_rejected() {
        // 11 seconds at 16 kHz mono exceeds MAX_LOADING_AUDIO_MS (10 s) while
        // staying within the decoded byte limit (352000 bytes < 1_920_000).
        let samples: Vec<i16> = vec![100; 16_000 * 11];
        let data = raw_pcm_to_base64(&samples);

        let err = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), None)
            .expect_err("too-long clip should be rejected");
        assert!(err.contains("too long"), "unexpected error: {err}");
    }

    #[test]
    fn test_odd_length_raw_pcm_rejected() {
        // An odd byte count cannot form whole 16-bit samples.
        let bytes = vec![1u8, 2, 3];
        let data = general_purpose::STANDARD.encode(bytes);

        let err = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), None)
            .expect_err("odd-length raw PCM should be rejected");
        assert_eq!(err, "loading_audio raw PCM length is not frame-aligned");
    }

    #[test]
    fn test_non_frame_aligned_stereo_raw_pcm_rejected() {
        // 8001 samples is even in bytes but not divisible by 2 channels.
        let samples: Vec<i16> = vec![100; 8001];
        let data = raw_pcm_to_base64(&samples);

        let err = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(2), None)
            .expect_err("non-frame-aligned stereo raw PCM should be rejected");
        assert_eq!(err, "loading_audio raw PCM length is not frame-aligned");
    }

    #[test]
    fn test_sample_rate_too_low_rejected() {
        // 4 kHz is below LOADING_AUDIO_SAMPLE_RATE_MIN.
        let samples: Vec<i16> = vec![100; 8000];
        let data = raw_pcm_to_base64(&samples);

        let err = decode_loading_clip(&data, Some("pcm"), Some(4_000), Some(1), None)
            .expect_err("too-low sample rate should be rejected");
        assert!(err.contains("sample rate"), "unexpected error: {err}");
    }

    #[test]
    fn test_sample_rate_too_high_rejected() {
        // 96 kHz is above LOADING_AUDIO_SAMPLE_RATE_MAX. 8000 samples keeps the
        // duration valid so the sample-rate check is what triggers rejection.
        let samples: Vec<i16> = vec![100; 8000];
        let data = raw_pcm_to_base64(&samples);

        let err = decode_loading_clip(&data, Some("pcm"), Some(96_000), Some(1), None)
            .expect_err("too-high sample rate should be rejected");
        assert!(err.contains("sample rate"), "unexpected error: {err}");
    }

    #[test]
    fn test_volume_half_scales_samples() {
        let samples: Vec<i16> = vec![1000; 8000];
        let data = raw_pcm_to_base64(&samples);

        let clip = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), Some(0.5))
            .expect("clip with volume 0.5 should decode");

        for sample in clip.samples() {
            // 1000 * 0.5 == 500, within rounding tolerance.
            assert!((*sample - 500).abs() <= 1, "sample not halved: {sample}");
        }
        assert!((clip.volume() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_volume_one_and_none_leave_samples_unchanged() {
        let samples: Vec<i16> = (0..8000).map(|i| (i % 1000) as i16).collect();
        let data = raw_pcm_to_base64(&samples);

        let clip_one = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), Some(1.0))
            .expect("clip with volume 1.0 should decode");
        assert_eq!(clip_one.samples(), samples.as_slice());

        let clip_none = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), None)
            .expect("clip with no volume should decode");
        assert_eq!(clip_none.samples(), samples.as_slice());
    }

    #[test]
    fn test_out_of_range_volume_is_clamped() {
        let samples: Vec<i16> = (0..8000).map(|i| (i % 1000) as i16).collect();
        let data = raw_pcm_to_base64(&samples);

        // Above 1.0 is clamped to 1.0 (samples unchanged), not an error.
        let clip_high = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), Some(2.0))
            .expect("volume 2.0 should be clamped, not rejected");
        assert!((clip_high.volume() - 1.0).abs() < f32::EPSILON);
        assert_eq!(clip_high.samples(), samples.as_slice());

        // Below 0.0 is clamped to 0.0 (samples silenced), not an error.
        let clip_low = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), Some(-1.0))
            .expect("volume -1.0 should be clamped, not rejected");
        assert!(clip_low.volume().abs() < f32::EPSILON);
        assert!(clip_low.samples().iter().all(|s| *s == 0));
    }

    #[test]
    fn test_volume_scaling_never_overflows() {
        // Extreme input samples with volume near 1.0 must not wrap/overflow.
        let mut samples: Vec<i16> = Vec::with_capacity(8000);
        for i in 0..8000 {
            samples.push(if i % 2 == 0 { i16::MAX } else { i16::MIN });
        }
        let data = raw_pcm_to_base64(&samples);

        let clip = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), Some(0.999_999))
            .expect("extreme samples should decode");

        for (i, sample) in clip.samples().iter().enumerate() {
            // Compare magnitudes via i32 so i16::MIN's absolute value is
            // representable. Attenuation below 1.0 must never flip the sign or
            // grow the magnitude beyond the original (which would mean a wrap).
            let scaled_abs = i32::from(*sample).abs();
            if i % 2 == 0 {
                assert!(*sample >= 0, "positive sample flipped sign: {sample}");
                assert!(
                    scaled_abs <= i32::from(i16::MAX),
                    "i16::MAX scaled value wrapped: {sample}"
                );
            } else {
                assert!(*sample <= 0, "negative sample flipped sign: {sample}");
                assert!(
                    scaled_abs <= -i32::from(i16::MIN),
                    "i16::MIN scaled value wrapped: {sample}"
                );
            }
        }
    }

    #[test]
    fn test_volume_accessor_returns_clamped_value() {
        let samples: Vec<i16> = vec![100; 8000];
        let data = raw_pcm_to_base64(&samples);

        let clip = decode_loading_clip(&data, Some("pcm"), Some(16_000), Some(1), Some(5.0))
            .expect("clip should decode with clamped volume");
        assert!((clip.volume() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_invalid_format_string_rejected() {
        let samples: Vec<i16> = vec![100; 8000];
        let data = raw_pcm_to_base64(&samples);

        let err = decode_loading_clip(&data, Some("mp3"), Some(16_000), Some(1), None)
            .expect_err("unknown format should be rejected");
        assert_eq!(err, "loading_audio.format must be \"wav\" or \"pcm\"");
    }

    #[test]
    fn test_forged_wav_data_chunk_length_rejected() {
        // Build a valid small WAV, then overwrite its `data` chunk size field
        // (the u32 at the canonical offset 40) with a huge value. `hound`
        // parses the header without reading the samples, so without the
        // declared-size guard in `decode_wav` this would drive a multi-
        // gigabyte `Vec` pre-allocation. The guard must reject it instead.
        let samples: Vec<i16> = (0..8_000).map(|i| (i % 1000) as i16).collect();
        let mut wav_bytes = {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
            {
                let mut writer =
                    hound::WavWriter::new(&mut cursor, spec).expect("WAV writer should be created");
                for sample in &samples {
                    writer
                        .write_sample(*sample)
                        .expect("sample should be written");
                }
                writer.finalize().expect("WAV writer should finalize");
            }
            cursor.into_inner()
        };
        assert!(
            wav_bytes.len() > 44,
            "a canonical 16-bit PCM WAV has a 44-byte header"
        );
        // Forge both the RIFF chunk size (offset 4) and the `data` chunk size
        // (offset 40). 0x7FFF_FFF0 bytes of "data" is ~2.1 GB — far past the
        // size cap, and even (so `hound` accepts it as whole 2-byte frames).
        wav_bytes[4..8].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        wav_bytes[40..44].copy_from_slice(&0x7FFF_FFF0u32.to_le_bytes());

        let data = general_purpose::STANDARD.encode(&wav_bytes);
        let err = decode_loading_clip(&data, Some("wav"), None, None, None)
            .expect_err("a WAV with a forged data-chunk length must be rejected");
        assert!(err.contains("maximum size"), "unexpected error: {err}");
    }
}
