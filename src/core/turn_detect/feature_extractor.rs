//! Whisper-style mel spectrogram feature extraction for smart-turn model.
//!
//! This module implements the audio preprocessing pipeline required by the
//! smart-turn model, converting raw PCM audio to mel spectrogram features.
//!
//! The smart-turn v3 model expects mel spectrogram input with shape (1, mel_bins, mel_frames)
//! where mel_bins and mel_frames are configured via TurnDetectorConfig:
//! - Default mel_bins: 80 (Whisper standard configuration)
//! - Default mel_frames: 800

use anyhow::{Context, Result};
use ndarray::{Array1, Array2, s};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::f32::consts::PI;
use std::sync::Arc;
use tracing::debug;

use super::config::TurnDetectorConfig;

/// Whisper feature extraction constants
const FFT_SIZE: usize = 400;
const HOP_LENGTH: usize = 160;

/// Feature extractor for smart-turn model.
///
/// Converts raw audio samples to mel spectrogram features in the format
/// expected by the smart-turn ONNX model. Uses Whisper-style preprocessing
/// with 128 mel bins and normalized output.
pub struct FeatureExtractor {
    /// Pre-computed mel filter bank (n_mels x n_freqs)
    mel_filters: Array2<f32>,
    /// Pre-computed Hann window for STFT
    hann_window: Array1<f32>,
    /// FFT instance for efficient O(n log n) Fourier transform
    fft: Arc<dyn Fft<f32>>,
    /// Sample rate (expected 16000 Hz)
    sample_rate: u32,
    /// Number of mel bins (from config)
    n_mels: usize,
    /// Number of output time frames (from config)
    model_frames: usize,
    /// Maximum audio duration in seconds (expected 8)
    max_duration_seconds: u8,
}

impl FeatureExtractor {
    /// Create a new feature extractor with the given configuration.
    ///
    /// # Arguments
    /// * `config` - Turn detector configuration containing mel spectrogram parameters
    ///
    /// # Returns
    /// * `Result<Self>` - Feature extractor or error if initialization fails
    pub fn new(config: &TurnDetectorConfig) -> Result<Self> {
        let n_mels = config.mel_bins;
        let mel_filters = Self::create_mel_filterbank(n_mels, config.sample_rate)
            .context("Failed to create mel filterbank")?;
        let hann_window = Self::create_hann_window();

        // Create FFT planner and plan for FFT_SIZE
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        debug!(
            "FeatureExtractor initialized: sample_rate={}, n_mels={}, model_frames={}",
            config.sample_rate, n_mels, config.mel_frames
        );

        Ok(Self {
            mel_filters,
            hann_window,
            fft,
            sample_rate: config.sample_rate,
            n_mels,
            model_frames: config.mel_frames,
            max_duration_seconds: config.max_audio_duration_seconds,
        })
    }

    /// Extract mel spectrogram features from audio samples.
    ///
    /// # Arguments
    /// * `audio` - Raw audio samples as i16 PCM (16kHz mono expected)
    ///
    /// # Returns
    /// * `Array2<f32>` - Mel spectrogram with shape `(n_mels, model_frames)`
    ///
    /// # Example
    /// ```ignore
    /// let extractor = FeatureExtractor::new(&config)?;
    /// let audio: Vec<i16> = load_audio(); // 16kHz mono PCM
    /// let mel_spec = extractor.extract(&audio)?;
    /// assert_eq!(mel_spec.dim(), (config.mel_bins, config.mel_frames));
    /// ```
    pub fn extract(&self, audio: &[i16]) -> Result<Array2<f32>> {
        // Convert i16 to f32 normalized to [-1.0, 1.0]
        let audio_f32: Vec<f32> = audio.iter().map(|&s| s as f32 / 32768.0).collect();

        self.extract_from_f32(&audio_f32)
    }

    /// Extract features from f32 audio samples (already normalized to [-1.0, 1.0]).
    ///
    /// # Arguments
    /// * `audio` - Normalized f32 audio samples in range [-1.0, 1.0]
    ///
    /// # Returns
    /// * `Array2<f32>` - Mel spectrogram with shape `(n_mels, model_frames)`
    pub fn extract_from_f32(&self, audio: &[f32]) -> Result<Array2<f32>> {
        let max_samples = (self.sample_rate as usize) * (self.max_duration_seconds as usize);

        // Prepare audio (truncate or pad to max duration)
        let audio = self.prepare_audio(audio, max_samples);

        // Compute STFT magnitudes
        let stft_magnitudes = self.compute_stft(&audio)?;

        // Apply mel filterbank
        let mel_spec = self.apply_mel_filters(&stft_magnitudes)?;

        // Convert to log scale (log10 with floor)
        let log_mel_spec = self.log_mel_spectrogram(&mel_spec);

        // Normalize and resize to model_frames
        let normalized = self.normalize_and_resize(log_mel_spec)?;

        debug!(
            "Extracted mel spectrogram: input_samples={}, output_shape={:?}",
            audio.len(),
            normalized.dim()
        );

        Ok(normalized)
    }

    /// Prepare audio by truncating or zero-padding.
    ///
    /// If audio is longer than max_samples, keeps the most recent audio.
    /// If audio is shorter, pads with zeros at the beginning.
    fn prepare_audio(&self, audio: &[f32], max_samples: usize) -> Vec<f32> {
        if audio.len() > max_samples {
            // Truncate to last max_samples (keep most recent audio)
            audio[audio.len() - max_samples..].to_vec()
        } else if audio.len() < max_samples {
            // Pad with zeros at the beginning
            let padding = max_samples - audio.len();
            let mut padded = vec![0.0f32; padding];
            padded.extend_from_slice(audio);
            padded
        } else {
            audio.to_vec()
        }
    }

    /// Compute Short-Time Fourier Transform magnitudes using FFT.
    ///
    /// Uses a Hann window of FFT_SIZE samples with HOP_LENGTH stride.
    /// Leverages rustfft for O(n log n) performance instead of naive O(n²) DFT.
    fn compute_stft(&self, audio: &[f32]) -> Result<Array2<f32>> {
        // Calculate number of frames we can compute
        let n_frames = if audio.len() >= FFT_SIZE {
            (audio.len() - FFT_SIZE) / HOP_LENGTH + 1
        } else {
            0
        };

        let n_freqs = FFT_SIZE / 2 + 1; // Only positive frequencies

        let mut magnitudes = Array2::<f32>::zeros((n_freqs, n_frames));

        // Pre-allocate scratch buffer for FFT (reused across frames)
        let mut scratch = vec![Complex::new(0.0f32, 0.0f32); self.fft.get_inplace_scratch_len()];

        for frame_idx in 0..n_frames {
            let start = frame_idx * HOP_LENGTH;
            let end = start + FFT_SIZE;

            if end > audio.len() {
                break;
            }

            // Apply Hann window and prepare complex input
            let mut buffer: Vec<Complex<f32>> = self
                .hann_window
                .iter()
                .zip(&audio[start..end])
                .map(|(&w, &s)| Complex::new(s * w, 0.0))
                .collect();

            // Run FFT in-place
            self.fft.process_with_scratch(&mut buffer, &mut scratch);

            // Store magnitude of positive frequencies
            for (i, c) in buffer.iter().take(n_freqs).enumerate() {
                magnitudes[[i, frame_idx]] = (c.re * c.re + c.im * c.im).sqrt();
            }
        }

        Ok(magnitudes)
    }

    /// Apply mel filterbank to STFT magnitudes.
    ///
    /// Multiplies the STFT magnitude spectrogram by the mel filterbank matrix
    /// to produce mel-frequency bins.
    fn apply_mel_filters(&self, stft: &Array2<f32>) -> Result<Array2<f32>> {
        // mel_filters: (n_mels, n_freqs)
        // stft: (n_freqs, n_frames)
        // result: (n_mels, n_frames)

        let n_frames = stft.dim().1;
        let mut mel_spec = Array2::<f32>::zeros((self.n_mels, n_frames));

        for frame_idx in 0..n_frames {
            let frame = stft.slice(s![.., frame_idx]);
            for mel_idx in 0..self.n_mels {
                let filter = self.mel_filters.slice(s![mel_idx, ..]);
                let value: f32 = filter.iter().zip(frame.iter()).map(|(&f, &s)| f * s).sum();
                mel_spec[[mel_idx, frame_idx]] = value;
            }
        }

        Ok(mel_spec)
    }

    /// Convert mel spectrogram to log scale.
    ///
    /// Applies log10 with a small floor value to avoid log(0).
    fn log_mel_spectrogram(&self, mel_spec: &Array2<f32>) -> Array2<f32> {
        let min_val = 1e-10f32;
        mel_spec.mapv(|x| x.max(min_val).log10())
    }

    /// Normalize and resize mel spectrogram to model_frames.
    ///
    /// Applies per-channel (Whisper-style) normalization and pads/truncates
    /// the time dimension to match the expected model input size.
    fn normalize_and_resize(&self, mut mel_spec: Array2<f32>) -> Result<Array2<f32>> {
        // Per-channel normalization (Whisper style)
        let mean = mel_spec.mean().unwrap_or(0.0);
        let std = mel_spec.std(0.0);
        let std = if std < 1e-6 { 1.0 } else { std };

        mel_spec = (mel_spec - mean) / std;

        // Resize to model_frames
        let current_frames = mel_spec.dim().1;

        if current_frames == self.model_frames {
            Ok(mel_spec)
        } else if current_frames > self.model_frames {
            // Truncate from the end (keep most recent frames)
            Ok(mel_spec
                .slice(s![.., current_frames - self.model_frames..])
                .to_owned())
        } else {
            // Pad at the beginning with zeros
            let mut padded = Array2::<f32>::zeros((self.n_mels, self.model_frames));
            let offset = self.model_frames - current_frames;
            padded.slice_mut(s![.., offset..]).assign(&mel_spec);
            Ok(padded)
        }
    }

    /// Create mel filterbank matrix.
    ///
    /// Creates triangular mel-frequency filterbank using the HTK mel scale formula.
    fn create_mel_filterbank(n_mels: usize, sample_rate: u32) -> Result<Array2<f32>> {
        let n_freqs = FFT_SIZE / 2 + 1;
        let mut filters = Array2::<f32>::zeros((n_mels, n_freqs));

        // Mel scale parameters
        let f_min = 0.0f32;
        let f_max = (sample_rate / 2) as f32;

        let mel_min = Self::hz_to_mel(f_min);
        let mel_max = Self::hz_to_mel(f_max);

        // Create mel points (n_mels + 2 points for triangular filters)
        let mel_points: Vec<f32> = (0..=n_mels + 1)
            .map(|i| mel_min + (i as f32) * (mel_max - mel_min) / ((n_mels + 1) as f32))
            .collect();

        // Convert back to Hz and then to FFT bin indices
        let hz_points: Vec<f32> = mel_points.iter().map(|&m| Self::mel_to_hz(m)).collect();

        let bin_points: Vec<usize> = hz_points
            .iter()
            .map(|&hz| ((FFT_SIZE as f32 + 1.0) * hz / (sample_rate as f32)).floor() as usize)
            .collect();

        // Create triangular filters
        for i in 0..n_mels {
            let start = bin_points[i];
            let center = bin_points[i + 1];
            let end = bin_points[i + 2];

            // Rising slope (from start to center)
            for k in start..center {
                if k < n_freqs && center > start {
                    filters[[i, k]] = (k - start) as f32 / (center - start) as f32;
                }
            }

            // Falling slope (from center to end)
            for k in center..end {
                if k < n_freqs && end > center {
                    filters[[i, k]] = (end - k) as f32 / (end - center) as f32;
                }
            }
        }

        Ok(filters)
    }

    /// Create Hann window of FFT_SIZE length.
    fn create_hann_window() -> Array1<f32> {
        Array1::from_iter(
            (0..FFT_SIZE)
                .map(|n| 0.5 * (1.0 - (2.0 * PI * n as f32 / (FFT_SIZE - 1) as f32).cos())),
        )
    }

    /// Convert frequency in Hz to mel scale (HTK formula).
    fn hz_to_mel(hz: f32) -> f32 {
        2595.0 * (1.0 + hz / 700.0).log10()
    }

    /// Convert mel scale to frequency in Hz (HTK formula).
    fn mel_to_hz(mel: f32) -> f32 {
        700.0 * (10.0f32.powf(mel / 2595.0) - 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> TurnDetectorConfig {
        TurnDetectorConfig::default()
    }

    #[test]
    fn test_hz_to_mel_conversion() {
        // Test known conversion values (HTK mel scale)
        assert!((FeatureExtractor::hz_to_mel(0.0) - 0.0).abs() < 0.01);

        // At 1000 Hz, mel value should be approximately 1000 (by design of HTK formula)
        let mel_1000 = FeatureExtractor::hz_to_mel(1000.0);
        assert!(
            (mel_1000 - 999.985).abs() < 1.0,
            "mel(1000Hz) = {}, expected ~1000",
            mel_1000
        );
    }

    #[test]
    fn test_mel_to_hz_roundtrip() {
        let test_frequencies = [0.0, 100.0, 500.0, 1000.0, 4000.0, 8000.0];
        for &hz in &test_frequencies {
            let mel = FeatureExtractor::hz_to_mel(hz);
            let hz_back = FeatureExtractor::mel_to_hz(mel);
            assert!(
                (hz - hz_back).abs() < 0.01,
                "Roundtrip failed for {} Hz: got {} Hz",
                hz,
                hz_back
            );
        }
    }

    #[test]
    fn test_prepare_audio_exact_length() {
        let config = create_test_config();
        let extractor = FeatureExtractor::new(&config).unwrap();

        let audio = vec![0.5f32; 2000];
        let prepared = extractor.prepare_audio(&audio, 2000);

        assert_eq!(prepared.len(), 2000);
        assert_eq!(prepared[0], 0.5);
        assert_eq!(prepared[1999], 0.5);
    }

    #[test]
    fn test_prepare_audio_padding() {
        let config = create_test_config();
        let extractor = FeatureExtractor::new(&config).unwrap();

        let short_audio = vec![0.5f32; 1000];
        let prepared = extractor.prepare_audio(&short_audio, 2000);

        assert_eq!(prepared.len(), 2000);
        // First 1000 samples should be zeros (padding at beginning)
        assert_eq!(prepared[0], 0.0);
        assert_eq!(prepared[999], 0.0);
        // Last 1000 samples should be the original audio
        assert_eq!(prepared[1000], 0.5);
        assert_eq!(prepared[1999], 0.5);
    }

    #[test]
    fn test_prepare_audio_truncation() {
        let config = create_test_config();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Create audio with distinguishable values
        let long_audio: Vec<f32> = (0..3000).map(|i| i as f32 / 3000.0).collect();
        let prepared = extractor.prepare_audio(&long_audio, 2000);

        assert_eq!(prepared.len(), 2000);
        // Should keep the last 2000 samples (most recent audio)
        assert_eq!(prepared[0], 1000.0 / 3000.0);
        assert_eq!(prepared[1999], 2999.0 / 3000.0);
    }

    #[test]
    fn test_hann_window_properties() {
        let window = FeatureExtractor::create_hann_window();

        assert_eq!(window.len(), FFT_SIZE);
        // Hann window should be 0 at endpoints
        assert!(window[0].abs() < 1e-6);
        assert!(window[FFT_SIZE - 1].abs() < 1e-6);
        // Hann window should be 1 at center
        let center = FFT_SIZE / 2;
        assert!((window[center] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_mel_filterbank_shape() {
        let config = create_test_config();
        let filters =
            FeatureExtractor::create_mel_filterbank(config.mel_bins, config.sample_rate).unwrap();

        let n_freqs = FFT_SIZE / 2 + 1;
        assert_eq!(filters.dim(), (config.mel_bins, n_freqs));
    }

    #[test]
    fn test_mel_filterbank_values() {
        let config = create_test_config();
        let filters =
            FeatureExtractor::create_mel_filterbank(config.mel_bins, config.sample_rate).unwrap();

        // All filter values should be non-negative
        for &val in filters.iter() {
            assert!(val >= 0.0, "Filter value should be non-negative: {}", val);
        }

        // Most filters should have positive response
        // Note: The first few filters at very low frequencies may have zero response
        // due to FFT frequency resolution (this is expected for mel filterbanks)
        let mut positive_count = 0;
        for mel_idx in 0..config.mel_bins {
            let filter = filters.slice(s![mel_idx, ..]);
            let sum: f32 = filter.iter().sum();
            if sum > 0.0 {
                positive_count += 1;
            }
        }
        // At least 80% of filters should have positive response
        // Note: With FFT_SIZE=400 at 16kHz, some low-frequency mel bins
        // will map to the same FFT bins, resulting in empty filters
        assert!(
            positive_count >= (config.mel_bins * 8 / 10),
            "Expected most filters to have positive response, got {}/{}",
            positive_count,
            config.mel_bins
        );
    }

    #[test]
    fn test_extract_output_shape() {
        let config = create_test_config();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Create 1 second of audio at 16kHz
        let audio: Vec<i16> = (0..16000)
            .map(|i| ((i as f32 * 0.1).sin() * 16000.0) as i16)
            .collect();

        let mel_spec = extractor.extract(&audio).unwrap();

        assert_eq!(
            mel_spec.dim(),
            (config.mel_bins, config.mel_frames),
            "Expected shape ({}, {}), got {:?}",
            config.mel_bins,
            config.mel_frames,
            mel_spec.dim()
        );
    }

    #[test]
    fn test_extract_from_f32() {
        let config = create_test_config();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Create normalized f32 audio
        let audio: Vec<f32> = (0..16000).map(|i| (i as f32 * 0.1).sin()).collect();

        let mel_spec = extractor.extract_from_f32(&audio).unwrap();

        assert_eq!(mel_spec.dim(), (config.mel_bins, config.mel_frames));
    }

    #[test]
    fn test_extract_short_audio() {
        let config = create_test_config();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Very short audio (less than FFT_SIZE samples)
        let audio: Vec<i16> = vec![0; 100];

        let mel_spec = extractor.extract(&audio).unwrap();

        // Should still produce correct output shape (padded)
        assert_eq!(mel_spec.dim(), (config.mel_bins, config.mel_frames));
    }

    #[test]
    fn test_extract_empty_audio() {
        let config = create_test_config();
        let extractor = FeatureExtractor::new(&config).unwrap();

        let audio: Vec<i16> = vec![];

        let mel_spec = extractor.extract(&audio).unwrap();

        // Should produce output shape with all-zero padding
        assert_eq!(mel_spec.dim(), (config.mel_bins, config.mel_frames));
    }

    #[test]
    fn test_extract_max_duration_audio() {
        let config = create_test_config();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Create exactly max duration of audio (8 seconds at 16kHz)
        let max_samples =
            (config.sample_rate as usize) * (config.max_audio_duration_seconds as usize);
        let audio: Vec<i16> = (0..max_samples)
            .map(|i| ((i as f32 * 0.05).sin() * 16000.0) as i16)
            .collect();

        let mel_spec = extractor.extract(&audio).unwrap();

        assert_eq!(mel_spec.dim(), (config.mel_bins, config.mel_frames));
    }

    #[test]
    fn test_normalization() {
        let config = create_test_config();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Create 8 seconds of audio (full duration) to avoid zero-padding
        // which would affect normalization stats
        let max_samples =
            (config.sample_rate as usize) * (config.max_audio_duration_seconds as usize);
        let audio: Vec<f32> = (0..max_samples)
            .map(|i| (i as f32 * 0.01 * 2.0 * PI / 16000.0).sin())
            .collect();

        let mel_spec = extractor.extract_from_f32(&audio).unwrap();

        // After normalization, mean should be close to 0 and std close to 1
        // Note: With resize/truncation the stats may shift slightly
        let mean = mel_spec.mean().unwrap();
        let std = mel_spec.std(0.0);

        // Allow more tolerance since resize may truncate some frames
        assert!(
            mean.abs() < 0.1,
            "Normalized mean should be ~0, got {}",
            mean
        );
        assert!(
            (std - 1.0).abs() < 0.1,
            "Normalized std should be ~1, got {}",
            std
        );
    }

    #[test]
    fn test_fft_performance() {
        use std::time::Instant;

        let config = create_test_config();
        let extractor = FeatureExtractor::new(&config).unwrap();

        // Generate 8 seconds of test audio (128000 samples at 16kHz)
        let audio: Vec<f32> = (0..128000).map(|i| (i as f32 * 0.01).sin()).collect();

        let start = Instant::now();
        let result = extractor.extract_from_f32(&audio).unwrap();
        let elapsed = start.elapsed();

        // Verify output shape is correct
        assert_eq!(result.dim(), (config.mel_bins, config.mel_frames));

        // Performance thresholds differ between debug and release builds.
        // Release: should complete under 100ms with optimized FFT (~20ms typical)
        // Debug: unoptimized code is significantly slower
        #[cfg(debug_assertions)]
        let max_ms = 2000; // 2 seconds for debug builds (unoptimized)
        #[cfg(not(debug_assertions))]
        let max_ms = 100; // 100ms for release builds

        println!("Feature extraction took: {:?}", elapsed);
        assert!(
            elapsed.as_millis() < max_ms,
            "Feature extraction too slow: {:?} (threshold: {}ms)",
            elapsed,
            max_ms
        );
    }
}
