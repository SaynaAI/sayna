//! Mel spectrogram computation for Whisper input preprocessing
//!
//! This module handles conversion of raw PCM audio to mel spectrograms,
//! which are the required input format for Whisper models.
//!
//! ## Whisper Audio Parameters
//!
//! - Sample rate: 16,000 Hz
//! - Mel bins: 80 (large-v3 uses 128)
//! - FFT size (N_FFT): 400
//! - Hop length: 160
//! - Window: Hann
//! - Mel scale: Slaney normalization
//!
//! ## Processing Pipeline
//!
//! 1. Apply Hann window to audio frames
//! 2. Compute STFT (Short-Time Fourier Transform)
//! 3. Convert power spectrum to mel scale
//! 4. Apply log scaling
//! 5. Normalize to Whisper's expected range
//!
//! ## Reference
//!
//! Based on the original Whisper implementation:
//! https://github.com/openai/whisper/blob/main/whisper/audio.py

#![cfg(feature = "whisper-stt")]

use mel_spec::mel::{interleave_frames, log_mel_spectrogram, mel, norm_mel};
use mel_spec::stft::Spectrogram;
use ndarray::Array2;

/// Audio sample rate expected by Whisper (16kHz)
pub const SAMPLE_RATE: u32 = 16000;

/// Default number of mel bins used by most Whisper models
pub const DEFAULT_N_MELS: usize = 80;

/// Number of mel bins used by whisper-large-v3
pub const LARGE_V3_N_MELS: usize = 128;

/// FFT window size (25ms at 16kHz)
pub const N_FFT: usize = 400;

/// Number of samples between STFT frames (10ms at 16kHz)
pub const HOP_LENGTH: usize = 160;

/// Maximum audio length in seconds
pub const CHUNK_LENGTH_SECONDS: u32 = 30;

/// Maximum audio length in samples (30 seconds)
pub const CHUNK_LENGTH: usize = CHUNK_LENGTH_SECONDS as usize * SAMPLE_RATE as usize;

/// Number of mel spectrogram frames for 30 seconds
/// (480000 - 400) / 160 + 1 = 2999.something, but Whisper uses 3000 frames
pub const N_FRAMES: usize = CHUNK_LENGTH / HOP_LENGTH;

/// Mel spectrogram processor for Whisper
///
/// Provides Whisper-compatible mel spectrogram computation using the `mel_spec` crate.
/// The output matches reference implementations (whisper.cpp, whisper.py, librosa).
#[derive(Debug)]
pub struct MelProcessor {
    /// Precomputed mel filterbank matrix (N_MELS x (N_FFT/2 + 1))
    mel_filters: Array2<f64>,
    /// Number of mel bins configured for the current model
    n_mels: usize,
}

impl MelProcessor {
    /// Create a new mel spectrogram processor
    ///
    /// Initializes the mel filterbank matrix with Whisper-compatible parameters.
    pub fn new() -> Self {
        Self::with_n_mels(DEFAULT_N_MELS)
    }

    /// Create a mel processor with a custom mel-bin count
    pub fn with_n_mels(n_mels: usize) -> Self {
        // Create mel filterbank using Whisper parameters
        // - sr: sample rate (16000 Hz)
        // - n_fft: FFT size (400)
        // - n_mels: number of mel bins (80)
        // - f_min: minimum frequency (None = 0)
        // - f_max: maximum frequency (None = sr/2 = 8000 Hz)
        // - htk: use HTK formula (false for Slaney)
        // - norm: apply Slaney normalization (true)
        let mel_filters = mel(
            SAMPLE_RATE as f64,
            N_FFT,
            n_mels,
            None,  // f_min
            None,  // f_max
            false, // htk
            true,  // norm (Slaney normalization)
        );

        Self {
            mel_filters,
            n_mels,
        }
    }

    /// Compute mel spectrogram from audio samples
    ///
    /// # Arguments
    /// * `audio` - Raw PCM f32 samples at 16kHz, mono
    ///
    /// # Returns
    /// * Mel spectrogram as flattened array [N_MELS * N_FRAMES]
    ///   The data is arranged in row-major order: all mel bins for frame 0,
    ///   then all mel bins for frame 1, etc.
    pub fn compute(&self, audio: &[f32]) -> Vec<f32> {
        // Pad or trim to exactly 30 seconds
        let audio = self.pad_or_trim(audio);

        // Create STFT processor
        let mut spectrogram = Spectrogram::new(N_FFT, HOP_LENGTH);

        // Process audio through STFT and collect mel frames
        let mut mel_frames = Vec::new();

        // Feed audio in chunks matching the hop length for efficiency
        // The Spectrogram::add method uses overlap-and-save
        for chunk in audio.chunks(HOP_LENGTH) {
            if let Some(fft_result) = spectrogram.add(chunk) {
                // Compute log mel spectrogram for this frame
                let log_mel = log_mel_spectrogram(&fft_result, &self.mel_filters);

                // Normalize the mel frame
                let normalized = norm_mel(&log_mel);

                mel_frames.push(normalized);
            }
        }

        // If we don't have enough frames, pad with zeros
        while mel_frames.len() < N_FRAMES {
            mel_frames.push(Array2::zeros((self.n_mels, 1)));
        }

        // Interleave frames for Whisper consumption
        // major_column_order=false means row-major (each row = frequency band)
        // min_width ensures consistent output dimensions
        interleave_frames(&mel_frames, false, N_FRAMES)
    }

    /// Compute mel spectrogram from raw PCM16 bytes
    ///
    /// # Arguments
    /// * `audio_bytes` - Raw PCM16 bytes (little-endian signed 16-bit)
    ///
    /// # Returns
    /// * Mel spectrogram as flattened array [N_MELS * N_FRAMES]
    pub fn compute_from_bytes(&self, audio_bytes: &[u8]) -> Vec<f32> {
        let audio = pcm16_bytes_to_f32(audio_bytes);
        self.compute(&audio)
    }

    /// Pad or trim audio to exactly 30 seconds
    ///
    /// # Arguments
    /// * `audio` - Input audio samples
    ///
    /// # Returns
    /// * Audio samples padded with zeros or trimmed to CHUNK_LENGTH samples
    pub fn pad_or_trim(&self, audio: &[f32]) -> Vec<f32> {
        if audio.len() > CHUNK_LENGTH {
            audio[..CHUNK_LENGTH].to_vec()
        } else {
            let mut padded = audio.to_vec();
            padded.resize(CHUNK_LENGTH, 0.0);
            padded
        }
    }

    /// Get the expected output shape
    ///
    /// # Returns
    /// * Tuple of (n_mels, n_frames)
    pub fn output_shape(&self) -> (usize, usize) {
        (self.n_mels, N_FRAMES)
    }

    /// Get the configured number of mel bins
    pub fn n_mels(&self) -> usize {
        self.n_mels
    }
}

impl Default for MelProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for MelProcessor {
    fn clone(&self) -> Self {
        Self {
            mel_filters: self.mel_filters.clone(),
            n_mels: self.n_mels,
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert PCM16 bytes to f32 samples normalized to [-1.0, 1.0]
///
/// # Arguments
/// * `bytes` - Raw PCM16 bytes (little-endian signed 16-bit)
///
/// # Returns
/// * f32 samples in range [-1.0, 1.0]
pub fn pcm16_bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|chunk| {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            sample as f32 / 32768.0
        })
        .collect()
}

/// Convert PCM16 i16 samples to f32 samples normalized to [-1.0, 1.0]
///
/// # Arguments
/// * `samples` - i16 PCM samples
///
/// # Returns
/// * f32 samples in range [-1.0, 1.0]
pub fn pcm16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&s| s as f32 / 32768.0).collect()
}

/// Convert sample count to duration in milliseconds
///
/// # Arguments
/// * `num_samples` - Number of audio samples
///
/// # Returns
/// * Duration in milliseconds
pub fn samples_to_duration_ms(num_samples: usize) -> u64 {
    ((num_samples as u64) * 1000) / (SAMPLE_RATE as u64)
}

/// Convert duration in milliseconds to sample count
///
/// # Arguments
/// * `duration_ms` - Duration in milliseconds
///
/// # Returns
/// * Number of audio samples
pub fn duration_ms_to_samples(duration_ms: u64) -> usize {
    ((duration_ms * SAMPLE_RATE as u64) / 1000) as usize
}

/// Validate audio format requirements
///
/// # Arguments
/// * `sample_rate` - Audio sample rate in Hz
/// * `channels` - Number of audio channels
///
/// # Returns
/// * Ok(()) if format is valid (16kHz mono)
/// * Err with descriptive message if format is invalid
pub fn validate_audio_format(sample_rate: u32, channels: u16) -> Result<(), String> {
    if sample_rate != SAMPLE_RATE {
        return Err(format!(
            "Invalid sample rate: expected {} Hz, got {} Hz. \
            Audio must be resampled to 16kHz for Whisper.",
            SAMPLE_RATE, sample_rate
        ));
    }

    if channels != 1 {
        return Err(format!(
            "Invalid channel count: expected 1 (mono), got {}. \
            Audio must be converted to mono for Whisper.",
            channels
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(SAMPLE_RATE, 16000);
        assert_eq!(DEFAULT_N_MELS, 80);
        assert_eq!(N_FFT, 400);
        assert_eq!(HOP_LENGTH, 160);
        assert_eq!(CHUNK_LENGTH, 480000); // 30 * 16000
        assert_eq!(N_FRAMES, 3000); // 480000 / 160
    }

    #[test]
    fn test_mel_processor_creation() {
        let processor = MelProcessor::new();
        // Verify mel filterbank dimensions
        // Shape should be (N_MELS, N_FFT/2 + 1) = (80, 201)
        assert_eq!(processor.mel_filters.shape()[0], processor.n_mels());
        assert_eq!(processor.mel_filters.shape()[1], N_FFT / 2 + 1);
        assert_eq!(processor.n_mels(), DEFAULT_N_MELS);
    }

    #[test]
    fn test_output_shape() {
        let processor = MelProcessor::new();
        let (n_mels, n_frames) = processor.output_shape();
        assert_eq!(n_mels, processor.n_mels());
        assert_eq!(n_frames, 3000);
    }

    #[test]
    fn test_custom_n_mels() {
        let processor = MelProcessor::with_n_mels(LARGE_V3_N_MELS);
        let (n_mels, n_frames) = processor.output_shape();
        assert_eq!(n_mels, LARGE_V3_N_MELS);
        assert_eq!(n_frames, N_FRAMES);
    }

    #[test]
    fn test_pad_or_trim_short() {
        let processor = MelProcessor::new();
        let short_audio = vec![0.5f32; 1000];
        let padded = processor.pad_or_trim(&short_audio);
        assert_eq!(padded.len(), CHUNK_LENGTH);
        assert_eq!(padded[0], 0.5);
        assert_eq!(padded[CHUNK_LENGTH - 1], 0.0);
    }

    #[test]
    fn test_pad_or_trim_long() {
        let processor = MelProcessor::new();
        let long_audio = vec![0.5f32; CHUNK_LENGTH + 1000];
        let trimmed = processor.pad_or_trim(&long_audio);
        assert_eq!(trimmed.len(), CHUNK_LENGTH);
    }

    #[test]
    fn test_pad_or_trim_exact() {
        let processor = MelProcessor::new();
        let exact_audio = vec![0.5f32; CHUNK_LENGTH];
        let result = processor.pad_or_trim(&exact_audio);
        assert_eq!(result.len(), CHUNK_LENGTH);
        assert_eq!(result[0], 0.5);
        assert_eq!(result[CHUNK_LENGTH - 1], 0.5);
    }

    #[test]
    fn test_pcm16_bytes_to_f32() {
        // Test with known values
        // i16::MAX (32767) -> ~1.0
        let max_bytes: [u8; 2] = i16::MAX.to_le_bytes();
        let result = pcm16_bytes_to_f32(&max_bytes);
        assert_eq!(result.len(), 1);
        assert!((result[0] - (32767.0 / 32768.0)).abs() < 0.0001);

        // i16::MIN (-32768) -> -1.0
        let min_bytes: [u8; 2] = i16::MIN.to_le_bytes();
        let result = pcm16_bytes_to_f32(&min_bytes);
        assert_eq!(result.len(), 1);
        assert!((result[0] - (-1.0)).abs() < 0.0001);

        // 0 -> 0.0
        let zero_bytes: [u8; 2] = 0i16.to_le_bytes();
        let result = pcm16_bytes_to_f32(&zero_bytes);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], 0.0);
    }

    #[test]
    fn test_pcm16_to_f32() {
        let samples = vec![i16::MAX, i16::MIN, 0, 16384, -16384];
        let result = pcm16_to_f32(&samples);

        assert_eq!(result.len(), 5);
        assert!((result[0] - (32767.0 / 32768.0)).abs() < 0.0001);
        assert!((result[1] - (-1.0)).abs() < 0.0001);
        assert_eq!(result[2], 0.0);
        assert!((result[3] - 0.5).abs() < 0.0001);
        assert!((result[4] - (-0.5)).abs() < 0.0001);
    }

    #[test]
    fn test_samples_to_duration_ms() {
        // 16000 samples = 1000 ms
        assert_eq!(samples_to_duration_ms(16000), 1000);

        // 1600 samples = 100 ms
        assert_eq!(samples_to_duration_ms(1600), 100);

        // 160 samples = 10 ms
        assert_eq!(samples_to_duration_ms(160), 10);

        // 0 samples = 0 ms
        assert_eq!(samples_to_duration_ms(0), 0);
    }

    #[test]
    fn test_duration_ms_to_samples() {
        // 1000 ms = 16000 samples
        assert_eq!(duration_ms_to_samples(1000), 16000);

        // 100 ms = 1600 samples
        assert_eq!(duration_ms_to_samples(100), 1600);

        // 10 ms = 160 samples
        assert_eq!(duration_ms_to_samples(10), 160);

        // 0 ms = 0 samples
        assert_eq!(duration_ms_to_samples(0), 0);
    }

    #[test]
    fn test_validate_audio_format_valid() {
        let result = validate_audio_format(16000, 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_audio_format_invalid_sample_rate() {
        let result = validate_audio_format(44100, 1);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Invalid sample rate"));
        assert!(err.contains("44100"));
    }

    #[test]
    fn test_validate_audio_format_invalid_channels() {
        let result = validate_audio_format(16000, 2);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Invalid channel count"));
        assert!(err.contains("mono"));
    }

    #[test]
    fn test_compute_silent_audio() {
        let processor = MelProcessor::new();
        let silent_audio = vec![0.0f32; CHUNK_LENGTH];
        let mel = processor.compute(&silent_audio);
        let expected_len = processor.n_mels() * N_FRAMES;

        // Should return flattened mel spectrogram
        assert_eq!(mel.len(), expected_len);

        // Silent audio should produce low mel values
        // (after log, they should be negative or very small)
        let max_val = mel.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(max_val.is_finite());
    }

    #[test]
    fn test_compute_short_audio() {
        let processor = MelProcessor::new();
        // 1 second of audio at 16kHz
        let short_audio = vec![0.1f32; 16000];
        let mel = processor.compute(&short_audio);
        let expected_len = processor.n_mels() * N_FRAMES;

        // Should still return full 30 seconds worth of frames
        assert_eq!(mel.len(), expected_len);
    }

    #[test]
    fn test_compute_from_bytes() {
        let processor = MelProcessor::new();

        // Create PCM16 bytes for silent audio (1 second)
        let mut bytes = Vec::with_capacity(16000 * 2);
        for _ in 0..16000 {
            bytes.extend_from_slice(&0i16.to_le_bytes());
        }

        let mel = processor.compute_from_bytes(&bytes);
        let expected_len = processor.n_mels() * N_FRAMES;
        assert_eq!(mel.len(), expected_len);
    }

    #[test]
    fn test_mel_processor_clone() {
        let processor = MelProcessor::new();
        let cloned = processor.clone();

        // Both should produce same output for same input
        let audio = vec![0.5f32; 16000];
        let mel1 = processor.compute(&audio);
        let mel2 = cloned.compute(&audio);

        assert_eq!(mel1.len(), mel2.len());
        for (a, b) in mel1.iter().zip(mel2.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn test_mel_processor_default() {
        let processor = MelProcessor::default();
        assert_eq!(processor.mel_filters.shape()[0], DEFAULT_N_MELS);
    }
}
