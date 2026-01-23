//! Digital Signal Processing utilities for DeepFilterNet.
//!
//! This module provides STFT/ISTFT operations, ERB filter bank, and feature extraction
//! for DeepFilterNet audio processing pipeline.

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::Arc;

use ndarray::{Array1, Array2, ArrayView1, ArrayViewMut1};
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, FftError, RealFftPlanner, RealToComplex};

use super::config::DfParams;

/// Constants for ERB scale conversion (Glasberg-Moore).
const ERB_A: f32 = 24.7;
const ERB_Q: f32 = 9.265;

/// Constants for feature normalization initialization.
const MEAN_NORM_INIT: [f32; 2] = [-60.0, -90.0];
const UNIT_NORM_INIT: [f32; 2] = [0.001, 0.0001];

/// Smoothing factor for exponential moving average normalization.
/// Used by `compute_feat_erb` and `compute_feat_spec` for exponential smoothing.
pub const ALPHA: f32 = 0.99;

/// Convert frequency in Hz to ERB scale.
#[inline]
fn freq2erb(freq_hz: f32) -> f32 {
    ERB_Q * (1.0 + freq_hz / (ERB_A * ERB_Q)).ln()
}

/// Convert ERB scale to frequency in Hz.
#[inline]
fn erb2freq(n_erb: f32) -> f32 {
    ERB_A * ERB_Q * ((n_erb / ERB_Q).exp() - 1.0)
}

/// Generate ERB filter bank frequency bin distribution.
///
/// Returns a vector where each element represents the number of frequency bins
/// in the corresponding ERB band.
pub fn erb_fb(sr: u32, fft_size: usize, nb_bands: usize, min_nb_freqs: usize) -> Vec<usize> {
    let nyquist = sr as f32 / 2.0;
    let freq_width = sr as f32 / fft_size as f32;
    let n_freqs = fft_size / 2 + 1;

    let erb_low = freq2erb(0.0);
    let erb_high = freq2erb(nyquist);
    let step = (erb_high - erb_low) / nb_bands as f32;

    let mut erb = vec![0usize; nb_bands];
    let mut prev_freq_bin = 0i32;
    let mut freq_over = 0i32;

    for i in 1..=nb_bands {
        let f = erb2freq(erb_low + i as f32 * step);
        let fb = (f / freq_width).round() as i32;

        let mut nb_freqs_curr = fb - prev_freq_bin - freq_over;

        if nb_freqs_curr < min_nb_freqs as i32 {
            freq_over = min_nb_freqs as i32 - nb_freqs_curr;
            nb_freqs_curr = min_nb_freqs as i32;
        } else {
            freq_over = 0;
        }

        erb[i - 1] = nb_freqs_curr as usize;
        prev_freq_bin = fb;
    }

    // Adjust last band to account for DC bin
    erb[nb_bands - 1] += 1;

    // Trim excess if total exceeds expected
    let total: usize = erb.iter().sum();
    if total > n_freqs {
        let excess = total - n_freqs;
        erb[nb_bands - 1] = erb[nb_bands - 1].saturating_sub(excess);
    }

    // Add any missing bins to last band
    let total: usize = erb.iter().sum();
    if total < n_freqs {
        erb[nb_bands - 1] += n_freqs - total;
    }

    erb
}

/// Compute the Vorbis window: sin(π/2 * sin²(π*n/N)).
fn compute_vorbis_window(window_size: usize) -> Vec<f32> {
    let window_size_h = window_size as f32 / 2.0;
    (0..window_size)
        .map(|i| {
            let sin_val = (0.5 * PI * (i as f32 + 0.5) / window_size_h).sin();
            (0.5 * PI * sin_val * sin_val).sin()
        })
        .collect()
}

/// DSP state for DeepFilterNet processing.
///
/// Holds FFT plans, window functions, buffers for overlap-add, and normalization states.
pub struct DfState {
    /// Sample rate in Hz.
    pub sr: u32,
    /// FFT size (window size).
    pub fft_size: usize,
    /// Hop size between frames.
    pub hop_size: usize,
    /// Number of frequency bins (fft_size / 2 + 1).
    pub freq_size: usize,
    /// Number of ERB bands.
    pub nb_erb: usize,
    /// Number of DF bins for deep filtering.
    pub nb_df: usize,
    /// DF order (number of frames for filtering).
    pub df_order: usize,
    /// DF lookahead frames.
    pub df_lookahead: usize,

    /// Forward FFT plan (real to complex).
    fft_forward: Arc<dyn RealToComplex<f32>>,
    /// Inverse FFT plan (complex to real).
    fft_inverse: Arc<dyn ComplexToReal<f32>>,

    /// Vorbis analysis/synthesis window.
    window: Vec<f32>,

    /// ERB filter bank (frequency bins per band).
    erb_fb: Vec<usize>,

    /// Analysis memory buffer for overlap-add.
    analysis_mem: Vec<f32>,
    /// Synthesis memory buffer for overlap-add.
    synthesis_mem: Vec<f32>,

    /// Input frame buffer for FFT.
    input_frame_buf: Vec<f32>,
    /// Output frame buffer from IFFT.
    output_frame_buf: Vec<f32>,

    /// Scratch buffer for FFT operations.
    fft_scratch: Vec<Complex32>,

    /// Rolling buffer for input spectra (for lookahead/DF).
    rolling_spec_buf_x: VecDeque<Array1<Complex32>>,
    /// Rolling buffer for output spectra.
    rolling_spec_buf_y: VecDeque<Array1<Complex32>>,

    /// Mean normalization state for ERB features.
    mean_norm_state: Array1<f32>,
    /// Unit normalization state for complex features.
    unit_norm_state: Array1<f32>,
}

impl DfState {
    /// Create a new DSP state from DfParams.
    pub fn new(params: &DfParams) -> Self {
        let fft_size = params.fft_size;
        let hop_size = params.hop_size;
        let freq_size = fft_size / 2 + 1;
        let nb_erb = params.nb_erb;
        let nb_df = params.nb_df;
        let df_order = params.df_order;
        let df_lookahead = params.df_lookahead;

        // Create FFT plans
        let mut planner = RealFftPlanner::<f32>::new();
        let fft_forward = planner.plan_fft_forward(fft_size);
        let fft_inverse = planner.plan_fft_inverse(fft_size);

        // Compute window
        let window = compute_vorbis_window(fft_size);

        // Generate ERB filter bank
        let erb_fb = erb_fb(params.sr, fft_size, nb_erb, params.min_nb_erb_freqs);

        // Allocate buffers
        let mem_size = fft_size - hop_size;
        let analysis_mem = vec![0.0; mem_size];
        let synthesis_mem = vec![0.0; mem_size];

        let input_frame_buf = vec![0.0; fft_size];
        let output_frame_buf = vec![0.0; fft_size];

        // Scratch buffer for FFT (size from realfft)
        let scratch_len = fft_forward
            .get_scratch_len()
            .max(fft_inverse.get_scratch_len());
        let fft_scratch = vec![Complex32::new(0.0, 0.0); scratch_len];

        // Rolling buffers for DF (need df_order + df_lookahead frames)
        let buf_size = df_order + df_lookahead;
        let rolling_spec_buf_x = VecDeque::with_capacity(buf_size);
        let rolling_spec_buf_y = VecDeque::with_capacity(buf_size);

        // Initialize normalization states
        let mean_norm_state = Array1::from_elem(nb_erb, Self::interp_norm_init(&MEAN_NORM_INIT));
        let unit_norm_state = Array1::from_elem(freq_size, Self::interp_norm_init(&UNIT_NORM_INIT));

        Self {
            sr: params.sr,
            fft_size,
            hop_size,
            freq_size,
            nb_erb,
            nb_df,
            df_order,
            df_lookahead,
            fft_forward,
            fft_inverse,
            window,
            erb_fb,
            analysis_mem,
            synthesis_mem,
            input_frame_buf,
            output_frame_buf,
            fft_scratch,
            rolling_spec_buf_x,
            rolling_spec_buf_y,
            mean_norm_state,
            unit_norm_state,
        }
    }

    /// Interpolate normalization initialization values.
    fn interp_norm_init(init: &[f32; 2]) -> f32 {
        (init[0] + init[1]) / 2.0
    }

    /// Reset all state buffers.
    pub fn reset(&mut self) {
        self.analysis_mem.fill(0.0);
        self.synthesis_mem.fill(0.0);
        self.input_frame_buf.fill(0.0);
        self.output_frame_buf.fill(0.0);
        self.rolling_spec_buf_x.clear();
        self.rolling_spec_buf_y.clear();
        self.mean_norm_state
            .fill(Self::interp_norm_init(&MEAN_NORM_INIT));
        self.unit_norm_state
            .fill(Self::interp_norm_init(&UNIT_NORM_INIT));
    }

    /// Get the ERB filter bank.
    pub fn erb_fb(&self) -> &[usize] {
        &self.erb_fb
    }

    /// Get mutable reference to mean normalization state.
    pub fn mean_norm_state_mut(&mut self) -> &mut Array1<f32> {
        &mut self.mean_norm_state
    }

    /// Get mutable reference to unit normalization state.
    pub fn unit_norm_state_mut(&mut self) -> &mut Array1<f32> {
        &mut self.unit_norm_state
    }

    /// Perform forward FFT (real to complex).
    ///
    /// Input: real-valued time-domain signal of length fft_size.
    /// Output: complex spectrum of length freq_size.
    pub fn rfft(&mut self, input: &[f32], output: &mut [Complex32]) -> Result<(), FftError> {
        debug_assert_eq!(input.len(), self.fft_size);
        debug_assert_eq!(output.len(), self.freq_size);

        self.input_frame_buf.copy_from_slice(input);
        self.fft_forward.process_with_scratch(
            &mut self.input_frame_buf,
            output,
            &mut self.fft_scratch,
        )
    }

    /// Perform inverse FFT (complex to real).
    ///
    /// Input: complex spectrum of length freq_size (will be modified).
    /// Output: real-valued time-domain signal of length fft_size.
    pub fn irfft(&mut self, input: &mut [Complex32], output: &mut [f32]) -> Result<(), FftError> {
        debug_assert_eq!(input.len(), self.freq_size);
        debug_assert_eq!(output.len(), self.fft_size);

        self.fft_inverse
            .process_with_scratch(input, output, &mut self.fft_scratch)
    }

    /// Perform STFT frame analysis.
    ///
    /// Applies windowing and overlap-add, then FFT.
    /// Returns the complex spectrum for the current frame.
    pub fn frame_analysis(&mut self, input_frame: &[f32]) -> Result<Array1<Complex32>, FftError> {
        debug_assert_eq!(input_frame.len(), self.hop_size);

        let overlap_size = self.fft_size - self.hop_size;

        // Part 1: Window previous frame data from analysis memory (overlap portion)
        for i in 0..overlap_size {
            self.input_frame_buf[i] = self.analysis_mem[i] * self.window[i];
        }

        // Part 2: Window current input frame
        for (i, &sample) in input_frame.iter().enumerate().take(self.hop_size) {
            let window_idx = overlap_size + i;
            self.input_frame_buf[window_idx] = sample * self.window[window_idx];
        }

        // Update analysis memory for next frame:
        // Shift old data left and append the portion of new input that will overlap
        // The new overlap region is the last (overlap_size) samples of the combined buffer
        // which is: analysis_mem[hop_size..] + input_frame[..]
        if overlap_size >= self.hop_size {
            // Shift the tail of analysis_mem to the front
            self.analysis_mem.copy_within(self.hop_size.., 0);
            // Copy the new input into the end (last hop_size positions)
            let dest_start = overlap_size - self.hop_size;
            self.analysis_mem[dest_start..].copy_from_slice(input_frame);
        } else {
            // overlap_size < hop_size: just keep the last overlap_size samples from input
            self.analysis_mem
                .copy_from_slice(&input_frame[self.hop_size - overlap_size..]);
        }

        // Apply FFT (no normalization - realfft is unnormalized)
        let mut spectrum = vec![Complex32::new(0.0, 0.0); self.freq_size];
        self.fft_forward.process_with_scratch(
            &mut self.input_frame_buf,
            &mut spectrum,
            &mut self.fft_scratch,
        )?;

        Ok(Array1::from_vec(spectrum))
    }

    /// Perform ISTFT frame synthesis.
    ///
    /// Applies IFFT, windowing, and overlap-add.
    /// Returns the time-domain output frame.
    ///
    /// # Errors
    /// Returns `FftError::InputValues` if DC or Nyquist bins have non-zero imaginary parts.
    /// This allows callers to handle IFFT failures via passthrough with warning logs.
    pub fn frame_synthesis(&mut self, spectrum: &mut [Complex32]) -> Result<Vec<f32>, FftError> {
        debug_assert_eq!(spectrum.len(), self.freq_size);

        // For realfft complex-to-real transform, DC (bin 0) and Nyquist (last bin)
        // must have zero imaginary parts. Return an error if they don't, so callers
        // can handle the failure (e.g., passthrough with warning logs).
        const EPSILON: f32 = 1e-10;
        let dc_invalid = spectrum[0].im.abs() > EPSILON;
        let nyquist_invalid = self.freq_size > 1 && spectrum[self.freq_size - 1].im.abs() > EPSILON;

        if dc_invalid || nyquist_invalid {
            return Err(FftError::InputValues(dc_invalid, nyquist_invalid));
        }

        // Apply IFFT
        // All FFT/IFFT errors are propagated to callers so they can handle failures
        // via passthrough with warning logs.
        self.fft_inverse.process_with_scratch(
            spectrum,
            &mut self.output_frame_buf,
            &mut self.fft_scratch,
        )?;

        // Normalize IFFT output (realfft doesn't normalize)
        // For perfect STFT/ISTFT reconstruction with Vorbis window and 50% overlap:
        // - FFT is unnormalized
        // - IFFT needs 1/N normalization
        // - Window² overlap-add sums to 1 (COLA property)
        // The normalization factor is 2 * hop_size / fft_size² to compensate for
        // the unnormalized FFT/IFFT pair and the window energy.
        let ifft_norm = 2.0 * self.hop_size as f32 / (self.fft_size as f32 * self.fft_size as f32);

        // Apply normalization and window
        for i in 0..self.fft_size {
            self.output_frame_buf[i] *= ifft_norm * self.window[i];
        }

        // Overlap-add: combine first part with synthesis memory
        let output: Vec<f32> = (0..self.hop_size)
            .map(|i| self.output_frame_buf[i] + self.synthesis_mem[i])
            .collect();

        // Update synthesis memory for next frame
        // For 50% overlap: synthesis_mem stores the second half of the current frame's
        // windowed IFFT output, which will overlap with the first half of the next frame.
        // Simply copy the second half of output_frame_buf to synthesis_mem.
        self.synthesis_mem
            .copy_from_slice(&self.output_frame_buf[self.hop_size..]);

        Ok(output)
    }

    /// Add spectrum to rolling input buffer.
    pub fn push_rolling_spec_x(&mut self, spec: Array1<Complex32>) {
        let max_size = self.df_order + self.df_lookahead;
        if self.rolling_spec_buf_x.len() >= max_size {
            self.rolling_spec_buf_x.pop_front();
        }
        self.rolling_spec_buf_x.push_back(spec);
    }

    /// Add spectrum to rolling output buffer.
    pub fn push_rolling_spec_y(&mut self, spec: Array1<Complex32>) {
        let max_size = self.df_order + self.df_lookahead;
        if self.rolling_spec_buf_y.len() >= max_size {
            self.rolling_spec_buf_y.pop_front();
        }
        self.rolling_spec_buf_y.push_back(spec);
    }

    /// Get the rolling input spectra buffer.
    pub fn rolling_spec_x(&self) -> &VecDeque<Array1<Complex32>> {
        &self.rolling_spec_buf_x
    }

    /// Get the rolling output spectra buffer.
    pub fn rolling_spec_y(&self) -> &VecDeque<Array1<Complex32>> {
        &self.rolling_spec_buf_y
    }
}

/// Compute ERB band energies from complex spectrum.
///
/// Converts spectrum to log-scale band energies and applies mean normalization.
pub fn compute_feat_erb(
    spectrum: ArrayView1<Complex32>,
    erb_fb: &[usize],
    mean_state: &mut ArrayViewMut1<f32>,
    alpha: f32,
) -> Array1<f32> {
    let nb_erb = erb_fb.len();
    let mut band_energies = Array1::zeros(nb_erb);

    let mut freq_idx = 0;
    for (band_idx, &nb_freqs) in erb_fb.iter().enumerate() {
        let mut energy = 0.0f32;
        for _ in 0..nb_freqs {
            if freq_idx < spectrum.len() {
                let c = spectrum[freq_idx];
                energy += c.re * c.re + c.im * c.im;
                freq_idx += 1;
            }
        }
        // Convert to log scale (dB)
        let log_energy = (energy + 1e-10).log10() * 10.0;
        band_energies[band_idx] = log_energy;
    }

    // Apply exponential mean normalization
    for i in 0..nb_erb {
        mean_state[i] = alpha * mean_state[i] + (1.0 - alpha) * band_energies[i];
        band_energies[i] -= mean_state[i];
    }

    band_energies
}

/// Compute complex spectral features with unit normalization.
///
/// Returns flattened real/imaginary pairs for the first nb_df bins.
pub fn compute_feat_spec(
    spectrum: ArrayView1<Complex32>,
    nb_df: usize,
    unit_state: &mut ArrayViewMut1<f32>,
    alpha: f32,
) -> Array1<f32> {
    let len = nb_df.min(spectrum.len());
    let mut features = Array1::zeros(len * 2);

    for i in 0..len {
        let c = spectrum[i];
        let mag = (c.re * c.re + c.im * c.im).sqrt();

        // Update unit normalization state
        unit_state[i] = alpha * unit_state[i] + (1.0 - alpha) * mag;

        // Normalize by magnitude (avoid division by zero)
        let norm = unit_state[i].max(1e-10);
        features[i * 2] = c.re / norm;
        features[i * 2 + 1] = c.im / norm;
    }

    features
}

/// Apply interpolated ERB band gains to spectrum.
///
/// Multiplies each frequency bin by its corresponding band gain.
pub fn apply_interp_band_gain(spectrum: &mut [Complex32], band_gains: &[f32], erb_fb: &[usize]) {
    let mut freq_idx = 0;
    for (band_idx, &nb_freqs) in erb_fb.iter().enumerate() {
        let gain = band_gains.get(band_idx).copied().unwrap_or(1.0);
        for _ in 0..nb_freqs {
            if freq_idx < spectrum.len() {
                spectrum[freq_idx] *= gain;
                freq_idx += 1;
            }
        }
    }
}

/// Apply deep filtering coefficients to spectrum.
///
/// Applies temporal filtering using DF coefficients across multiple frames.
/// The coefficients are complex-valued and convolved with the rolling spectrum buffer.
///
/// # Arguments
/// * `rolling_spec` - VecDeque of past spectral frames (oldest first)
/// * `coefs` - DF coefficients, shape [nb_df, df_order] as complex values (flattened re/im)
/// * `output_spec` - Output spectrum slice to write filtered bins
/// * `nb_df` - Number of frequency bins to filter
/// * `df_order` - Number of frames for convolution
pub fn apply_df(
    rolling_spec: &VecDeque<Array1<Complex32>>,
    coefs: &Array2<f32>,
    output_spec: &mut [Complex32],
    nb_df: usize,
    df_order: usize,
) {
    let n_frames = rolling_spec.len();
    if n_frames < df_order {
        return;
    }

    // Determine Nyquist bin index (last bin in spectrum)
    let nyquist_idx = output_spec.len().saturating_sub(1);

    // Zero the DF bins first, but preserve DC (bin 0) and Nyquist bins.
    // DC and Nyquist must remain real-valued for valid real IFFT (Hermitian symmetry).
    for i in 1..nb_df.min(output_spec.len()) {
        if i != nyquist_idx {
            output_spec[i] = Complex32::new(0.0, 0.0);
        }
    }

    // Apply DF convolution, skipping DC (bin 0) and Nyquist bins to preserve
    // their real-valued property required for real IFFT.
    // coefs shape: [nb_df, df_order * 2] (real/imag pairs)
    for (freq_idx, row) in coefs.outer_iter().enumerate().take(nb_df) {
        if freq_idx >= output_spec.len() {
            break;
        }
        // Skip DC and Nyquist bins to preserve existing values
        if freq_idx == 0 || freq_idx == nyquist_idx {
            continue;
        }

        let mut acc = Complex32::new(0.0, 0.0);

        for t in 0..df_order {
            let frame_idx = n_frames - df_order + t;
            if let Some(spec_frame) = rolling_spec.get(frame_idx) {
                let spec_len = spec_frame.len();
                if freq_idx < spec_len {
                    let s = spec_frame[freq_idx];
                    // Get coefficient (complex: re, im)
                    let c_re = row.get(t * 2).copied().unwrap_or(0.0);
                    let c_im = row.get(t * 2 + 1).copied().unwrap_or(0.0);
                    let c = Complex32::new(c_re, c_im);
                    acc += s * c;
                }
            }
        }

        output_spec[freq_idx] = acc;
    }
}

/// Helper to convert i16 PCM samples to f32 normalized [-1, 1].
#[inline]
pub fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples.iter().map(|&s| s as f32 / 32768.0).collect()
}

/// Helper to convert f32 normalized samples to i16 PCM.
#[inline]
pub fn f32_to_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|&s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_erb_scale_conversion() {
        // Test roundtrip conversion
        let freqs = [100.0, 500.0, 1000.0, 4000.0, 8000.0];
        for &f in &freqs {
            let erb = freq2erb(f);
            let f_back = erb2freq(erb);
            assert!(
                (f - f_back).abs() < 0.01,
                "Roundtrip failed for {} Hz: got {} Hz",
                f,
                f_back
            );
        }
    }

    #[test]
    fn test_erb_fb_generation() {
        let sr = 48000;
        let fft_size = 960;
        let nb_bands = 32;
        let min_nb_freqs = 2;

        let erb = erb_fb(sr, fft_size, nb_bands, min_nb_freqs);

        // Should have exactly nb_bands entries
        assert_eq!(erb.len(), nb_bands);

        // Total should equal freq_size (fft_size / 2 + 1)
        let total: usize = erb.iter().sum();
        let expected = fft_size / 2 + 1;
        assert_eq!(
            total, expected,
            "ERB total {} != expected {}",
            total, expected
        );

        // Each band should have at least min_nb_freqs (except possibly last due to adjustment)
        for (i, &count) in erb.iter().enumerate().take(nb_bands - 1) {
            assert!(
                count >= min_nb_freqs,
                "Band {} has {} bins, expected at least {}",
                i,
                count,
                min_nb_freqs
            );
        }
    }

    #[test]
    fn test_vorbis_window() {
        let window_size = 960;
        let hop_size = 480;
        let window = compute_vorbis_window(window_size);

        assert_eq!(window.len(), window_size);

        // Window should be symmetric
        for i in 0..window_size / 2 {
            let diff = (window[i] - window[window_size - 1 - i]).abs();
            assert!(diff < 1e-6, "Window not symmetric at index {}", i);
        }

        // Values should be in [0, 1]
        for &v in &window {
            assert!((0.0..=1.0).contains(&v), "Window value {} out of range", v);
        }

        // For 50% overlap, window²[i] + window²[i + hop_size] should sum to 1
        // This is the COLA (Constant Overlap-Add) property
        for i in 0..hop_size {
            let sum = window[i] * window[i] + window[i + hop_size] * window[i + hop_size];
            assert!(
                (sum - 1.0).abs() < 1e-5,
                "COLA property violated at index {}: sum = {}",
                i,
                sum
            );
        }

        // Verify window energy equals hop_size (for perfect COLA)
        let window_energy: f32 = window.iter().map(|w| w * w).sum();
        assert!(
            (window_energy - hop_size as f32).abs() < 1.0,
            "Window energy {} != expected {}",
            window_energy,
            hop_size
        );
    }

    #[test]
    fn test_stft_istft_roundtrip() {
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        // Create a simple test signal (sine wave)
        let freq = 440.0;
        let sr = params.sr as f32;
        let hop_size = params.hop_size;

        let signal: Vec<f32> = (0..hop_size)
            .map(|i| (2.0 * PI * freq * i as f32 / sr).sin() * 0.5)
            .collect();

        // Process through STFT
        let spectrum = state
            .frame_analysis(&signal)
            .expect("frame_analysis failed");

        // Process through ISTFT
        let mut spec_vec: Vec<Complex32> = spectrum.to_vec();
        let reconstructed = state
            .frame_synthesis(&mut spec_vec)
            .expect("frame_synthesis failed");

        assert_eq!(reconstructed.len(), hop_size);

        // The first frame won't match well due to overlap-add initialization,
        // so we just check that we get valid output
        for &s in &reconstructed {
            assert!(s.is_finite(), "Output contains non-finite value");
        }
    }

    #[test]
    fn test_stft_istft_multiple_frames() {
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        let freq = 440.0;
        let sr = params.sr as f32;
        let hop_size = params.hop_size;
        let n_frames = 15;

        // Create test signal (sine wave)
        let signal: Vec<f32> = (0..hop_size * n_frames)
            .map(|i| (2.0 * PI * freq * i as f32 / sr).sin() * 0.5)
            .collect();

        let mut output = Vec::new();

        // Process frame by frame
        for frame_idx in 0..n_frames {
            let start = frame_idx * hop_size;
            let end = start + hop_size;
            let frame = &signal[start..end];

            let spectrum = state.frame_analysis(frame).expect("frame_analysis failed");
            let mut spec_vec: Vec<Complex32> = spectrum.to_vec();
            let out_frame = state
                .frame_synthesis(&mut spec_vec)
                .expect("frame_synthesis failed");
            output.extend(out_frame);
        }

        assert_eq!(output.len(), signal.len());

        // STFT/ISTFT has inherent latency of fft_size - hop_size samples
        let latency = params.fft_size - hop_size;

        // Skip first 4 frames for overlap-add to stabilize fully
        let skip_samples = hop_size * 4;
        let check_samples = hop_size * (n_frames - 8);

        if check_samples > 0 {
            let mut max_error = 0.0f32;

            for i in skip_samples..(skip_samples + check_samples) {
                if i >= latency {
                    let expected = signal[i - latency];
                    let actual = output[i];
                    let error = (expected - actual).abs();
                    max_error = max_error.max(error);
                }
            }

            assert!(
                max_error < 0.001,
                "STFT/ISTFT roundtrip error too large: {}",
                max_error
            );
        }
    }

    #[test]
    fn test_apply_interp_band_gain() {
        let params = DfParams::default();
        let state = DfState::new(&params);

        let freq_size = state.freq_size;
        let erb_fb = state.erb_fb();

        // Create test spectrum
        let mut spectrum: Vec<Complex32> = (0..freq_size)
            .map(|i| Complex32::new(i as f32, 0.0))
            .collect();

        // Create band gains (all 0.5)
        let band_gains: Vec<f32> = vec![0.5; state.nb_erb];

        apply_interp_band_gain(&mut spectrum, &band_gains, erb_fb);

        // All values should be halved
        for (i, &s) in spectrum.iter().enumerate() {
            let expected = i as f32 * 0.5;
            assert!(
                (s.re - expected).abs() < 1e-6,
                "Gain application failed at bin {}",
                i
            );
        }
    }

    #[test]
    fn test_compute_feat_erb() {
        let params = DfParams::default();
        let state = DfState::new(&params);

        let freq_size = state.freq_size;
        let erb_fb = state.erb_fb();

        // Create test spectrum with known energy
        let spectrum: Array1<Complex32> = Array1::from_elem(freq_size, Complex32::new(1.0, 0.0));

        let mut mean_state = Array1::from_elem(state.nb_erb, -75.0);
        let features = compute_feat_erb(spectrum.view(), erb_fb, &mut mean_state.view_mut(), ALPHA);

        assert_eq!(features.len(), state.nb_erb);

        // Features should be finite
        for &f in features.iter() {
            assert!(f.is_finite(), "Feature contains non-finite value");
        }
    }

    #[test]
    fn test_compute_feat_spec() {
        let params = DfParams::default();
        let state = DfState::new(&params);

        let freq_size = state.freq_size;
        let nb_df = state.nb_df;

        // Create test spectrum
        let spectrum: Array1<Complex32> = Array1::from_elem(freq_size, Complex32::new(1.0, 1.0));

        let mut unit_state = Array1::from_elem(freq_size, 0.001);
        let features = compute_feat_spec(spectrum.view(), nb_df, &mut unit_state.view_mut(), ALPHA);

        // Should have 2 * nb_df features (real + imag)
        assert_eq!(features.len(), nb_df * 2);

        // Features should be finite
        for &f in features.iter() {
            assert!(f.is_finite(), "Feature contains non-finite value");
        }
    }

    #[test]
    fn test_i16_f32_conversion() {
        let i16_samples = vec![0i16, 16384, -16384, 32767, -32768];
        let f32_samples = i16_to_f32(&i16_samples);

        assert!((f32_samples[0] - 0.0).abs() < 1e-5);
        assert!((f32_samples[1] - 0.5).abs() < 0.001);
        assert!((f32_samples[2] + 0.5).abs() < 0.001);
        assert!((f32_samples[3] - 1.0).abs() < 0.001);
        assert!((f32_samples[4] + 1.0).abs() < 0.001);

        let back = f32_to_i16(&f32_samples);
        assert_eq!(back[0], 0);
        assert!((back[3] - 32767).abs() <= 1);
        assert!((back[4] + 32767).abs() <= 1);
    }

    #[test]
    fn test_df_state_reset() {
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        // Add some data
        let spectrum = Array1::from_elem(state.freq_size, Complex32::new(1.0, 0.0));
        state.push_rolling_spec_x(spectrum.clone());
        state.push_rolling_spec_y(spectrum);

        assert!(!state.rolling_spec_buf_x.is_empty());
        assert!(!state.rolling_spec_buf_y.is_empty());

        // Reset
        state.reset();

        assert!(state.rolling_spec_buf_x.is_empty());
        assert!(state.rolling_spec_buf_y.is_empty());
    }

    #[test]
    fn test_apply_df_preserves_dc_and_nyquist_bins() {
        // Test that apply_df preserves DC (bin 0) and Nyquist bins, which must remain
        // real-valued for valid ISTFT (Hermitian symmetry requirement).
        let params = DfParams::default();
        let state = DfState::new(&params);

        let freq_size = state.freq_size;
        let nb_df = state.nb_df;
        let df_order = state.df_order;
        let nyquist_idx = freq_size - 1;

        // Create synthetic rolling spectra buffer with enough frames for DF
        let mut rolling_spec: VecDeque<Array1<Complex32>> = VecDeque::new();
        for _ in 0..df_order {
            // Each frame has complex values (including imaginary parts)
            let frame: Array1<Complex32> = Array1::from_iter(
                (0..freq_size).map(|i| Complex32::new(1.0 + i as f32 * 0.01, 0.5)),
            );
            rolling_spec.push_back(frame);
        }

        // Create output spectrum with known DC and Nyquist values (real-valued)
        let mut output_spec: Vec<Complex32> = (0..freq_size)
            .map(|i| Complex32::new(1.0 + i as f32 * 0.1, 0.0))
            .collect();

        // Set DC and Nyquist bins to specific real values that we want to preserve
        let dc_original = Complex32::new(10.0, 0.0);
        let nyquist_original = Complex32::new(20.0, 0.0);
        output_spec[0] = dc_original;
        output_spec[nyquist_idx] = nyquist_original;

        // Create synthetic DF coefficients that would modify bins if applied
        // Shape: [nb_df, df_order * 2] (real/imag pairs)
        let coefs = Array2::from_elem((nb_df, df_order * 2), 0.5);

        // Apply deep filtering
        apply_df(&rolling_spec, &coefs, &mut output_spec, nb_df, df_order);

        // Verify DC bin (bin 0) is preserved
        assert_eq!(
            output_spec[0], dc_original,
            "DC bin should be preserved by apply_df"
        );

        // Verify Nyquist bin is preserved (if within nb_df range)
        if nyquist_idx < nb_df {
            assert_eq!(
                output_spec[nyquist_idx], nyquist_original,
                "Nyquist bin should be preserved by apply_df"
            );
        }

        // Verify DC and Nyquist bins remain real-valued (zero imaginary)
        assert!(
            output_spec[0].im.abs() < 1e-10,
            "DC bin must have zero imaginary part for valid ISTFT"
        );
        assert!(
            output_spec[nyquist_idx].im.abs() < 1e-10,
            "Nyquist bin must have zero imaginary part for valid ISTFT"
        );
    }

    #[test]
    fn test_apply_df_modifies_middle_bins() {
        // Test that apply_df does modify the middle bins (not DC or Nyquist),
        // confirming the skip logic only applies to boundary bins.
        let params = DfParams::default();
        let state = DfState::new(&params);

        let freq_size = state.freq_size;
        let nb_df = state.nb_df;
        let df_order = state.df_order;
        let nyquist_idx = freq_size - 1;

        // Create rolling spectra with non-trivial values
        let mut rolling_spec: VecDeque<Array1<Complex32>> = VecDeque::new();
        for _ in 0..df_order {
            let frame: Array1<Complex32> = Array1::from_elem(freq_size, Complex32::new(2.0, 1.0));
            rolling_spec.push_back(frame);
        }

        // Create output spectrum initialized to zero
        let mut output_spec: Vec<Complex32> = vec![Complex32::new(0.0, 0.0); freq_size];

        // Set DC and Nyquist to known values
        output_spec[0] = Complex32::new(10.0, 0.0);
        output_spec[nyquist_idx] = Complex32::new(20.0, 0.0);

        // Create non-zero coefficients
        let coefs = Array2::from_elem((nb_df, df_order * 2), 1.0);

        // Apply deep filtering
        apply_df(&rolling_spec, &coefs, &mut output_spec, nb_df, df_order);

        // Middle bins (1 to nb_df-1, excluding Nyquist if in range) should be modified
        // They were initialized to zero and should now have non-zero values from convolution
        let middle_bin = 1.min(nb_df - 1);
        if middle_bin > 0 && middle_bin != nyquist_idx && middle_bin < nb_df {
            // The bin should have been modified by apply_df (no longer zero)
            let is_modified = output_spec[middle_bin].re.abs() > 1e-10
                || output_spec[middle_bin].im.abs() > 1e-10;
            assert!(
                is_modified,
                "Middle bins should be modified by apply_df convolution"
            );
        }
    }

    #[test]
    fn test_frame_synthesis_after_apply_df_succeeds() {
        // Integration test: verify that a spectrum processed through apply_df
        // can successfully pass through frame_synthesis (DC/Nyquist remain valid).
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        let freq_size = state.freq_size;
        let nb_df = state.nb_df;
        let df_order = state.df_order;
        let nyquist_idx = freq_size - 1;

        // Create rolling spectra buffer
        let mut rolling_spec: VecDeque<Array1<Complex32>> = VecDeque::new();
        for _ in 0..df_order {
            let frame: Array1<Complex32> = Array1::from_elem(freq_size, Complex32::new(1.0, 0.5));
            rolling_spec.push_back(frame);
        }

        // Start with a valid spectrum (DC and Nyquist have zero imaginary)
        let mut spectrum: Vec<Complex32> = (0..freq_size)
            .map(|i| Complex32::new(0.5, if i == 0 || i == nyquist_idx { 0.0 } else { 0.1 }))
            .collect();

        // Apply DF (which should preserve DC and Nyquist)
        let coefs = Array2::from_elem((nb_df, df_order * 2), 0.5);
        apply_df(&rolling_spec, &coefs, &mut spectrum, nb_df, df_order);

        // Verify DC and Nyquist are still valid for ISTFT
        assert!(
            spectrum[0].im.abs() < 1e-10,
            "DC bin imaginary should be zero after apply_df"
        );
        assert!(
            spectrum[nyquist_idx].im.abs() < 1e-10,
            "Nyquist bin imaginary should be zero after apply_df"
        );

        // frame_synthesis should succeed with this spectrum
        let result = state.frame_synthesis(&mut spectrum);
        assert!(
            result.is_ok(),
            "frame_synthesis should succeed after apply_df preserves DC/Nyquist"
        );

        // Output should be finite
        let output = result.unwrap();
        for &s in &output {
            assert!(s.is_finite(), "Output should be finite");
        }
    }

    #[test]
    fn test_compute_feat_erb_with_zeros() {
        let params = DfParams::default();
        let state = DfState::new(&params);

        let freq_size = state.freq_size;
        let erb_fb = state.erb_fb();

        // Create spectrum with all zeros
        let spectrum: Array1<Complex32> = Array1::from_elem(freq_size, Complex32::new(0.0, 0.0));

        let mut mean_state = Array1::from_elem(state.nb_erb, -75.0);
        let features = compute_feat_erb(spectrum.view(), erb_fb, &mut mean_state.view_mut(), ALPHA);

        // Features should be finite even with zero input (epsilon guards log)
        for &f in features.iter() {
            assert!(
                f.is_finite(),
                "Feature contains non-finite value with zero input"
            );
        }
    }

    #[test]
    fn test_compute_feat_erb_with_tiny_values() {
        let params = DfParams::default();
        let state = DfState::new(&params);

        let freq_size = state.freq_size;
        let erb_fb = state.erb_fb();

        // Create spectrum with very small values (near epsilon)
        let tiny = 1e-20f32;
        let spectrum: Array1<Complex32> = Array1::from_elem(freq_size, Complex32::new(tiny, tiny));

        let mut mean_state = Array1::from_elem(state.nb_erb, -75.0);
        let features = compute_feat_erb(spectrum.view(), erb_fb, &mut mean_state.view_mut(), ALPHA);

        // Features should be finite even with tiny input
        for &f in features.iter() {
            assert!(
                f.is_finite(),
                "Feature contains non-finite value with tiny input"
            );
        }
    }

    #[test]
    fn test_compute_feat_erb_with_large_values() {
        let params = DfParams::default();
        let state = DfState::new(&params);

        let freq_size = state.freq_size;
        let erb_fb = state.erb_fb();

        // Create spectrum with large values (but not f32::MAX to avoid overflow in squaring)
        let large = 1e10f32;
        let spectrum: Array1<Complex32> =
            Array1::from_elem(freq_size, Complex32::new(large, large));

        let mut mean_state = Array1::from_elem(state.nb_erb, -75.0);
        let features = compute_feat_erb(spectrum.view(), erb_fb, &mut mean_state.view_mut(), ALPHA);

        // Features should be finite even with large input
        for &f in features.iter() {
            assert!(
                f.is_finite(),
                "Feature contains non-finite value with large input"
            );
        }
    }

    #[test]
    fn test_compute_feat_spec_with_zeros() {
        let params = DfParams::default();
        let state = DfState::new(&params);

        let freq_size = state.freq_size;
        let nb_df = state.nb_df;

        // Create spectrum with all zeros
        let spectrum: Array1<Complex32> = Array1::from_elem(freq_size, Complex32::new(0.0, 0.0));

        let mut unit_state = Array1::from_elem(freq_size, 0.001);
        let features = compute_feat_spec(spectrum.view(), nb_df, &mut unit_state.view_mut(), ALPHA);

        // Features should be finite even with zero input (division guarded)
        for &f in features.iter() {
            assert!(
                f.is_finite(),
                "Feature contains non-finite value with zero input"
            );
        }
    }

    #[test]
    fn test_compute_feat_spec_with_tiny_values() {
        let params = DfParams::default();
        let state = DfState::new(&params);

        let freq_size = state.freq_size;
        let nb_df = state.nb_df;

        // Create spectrum with very small values
        let tiny = 1e-20f32;
        let spectrum: Array1<Complex32> = Array1::from_elem(freq_size, Complex32::new(tiny, tiny));

        let mut unit_state = Array1::from_elem(freq_size, 0.001);
        let features = compute_feat_spec(spectrum.view(), nb_df, &mut unit_state.view_mut(), ALPHA);

        // Features should be finite even with tiny input
        for &f in features.iter() {
            assert!(
                f.is_finite(),
                "Feature contains non-finite value with tiny input"
            );
        }
    }

    #[test]
    fn test_stft_istft_with_zeros() {
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        // Create all-zero input
        let zero_frame = vec![0.0f32; params.hop_size];

        let spectrum = state
            .frame_analysis(&zero_frame)
            .expect("frame_analysis failed");
        let mut spec_vec: Vec<Complex32> = spectrum.to_vec();
        let output = state
            .frame_synthesis(&mut spec_vec)
            .expect("frame_synthesis failed");

        // Output should be finite
        for &s in &output {
            assert!(
                s.is_finite(),
                "Output contains non-finite value with zero input"
            );
        }
    }

    #[test]
    fn test_stft_istft_with_tiny_values() {
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        // Create very small input
        let tiny_frame = vec![1e-20f32; params.hop_size];

        let spectrum = state
            .frame_analysis(&tiny_frame)
            .expect("frame_analysis failed");
        let mut spec_vec: Vec<Complex32> = spectrum.to_vec();
        let output = state
            .frame_synthesis(&mut spec_vec)
            .expect("frame_synthesis failed");

        // Output should be finite
        for &s in &output {
            assert!(
                s.is_finite(),
                "Output contains non-finite value with tiny input"
            );
        }
    }

    #[test]
    fn test_stft_istft_with_large_values() {
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        // Create large input (but valid f32 range)
        let large_frame = vec![0.999f32; params.hop_size];

        let spectrum = state
            .frame_analysis(&large_frame)
            .expect("frame_analysis failed");
        let mut spec_vec: Vec<Complex32> = spectrum.to_vec();
        let output = state
            .frame_synthesis(&mut spec_vec)
            .expect("frame_synthesis failed");

        // Output should be finite
        for &s in &output {
            assert!(
                s.is_finite(),
                "Output contains non-finite value with large input"
            );
        }
    }

    #[test]
    fn test_irfft_returns_error_for_invalid_dc_bin() {
        // Test that irfft propagates FftError::InputValues when DC bin has non-zero imaginary part.
        // This verifies error propagation for IFFT failures per Task 017 requirements.
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        // Create a spectrum with invalid DC bin (non-zero imaginary part)
        let mut invalid_spectrum = vec![Complex32::new(0.0, 0.0); state.freq_size];
        invalid_spectrum[0] = Complex32::new(1.0, 1.0); // DC bin with non-zero imaginary - invalid for real FFT

        let mut output = vec![0.0f32; state.fft_size];

        // irfft should return an error for invalid input
        let result = state.irfft(&mut invalid_spectrum, &mut output);
        assert!(
            result.is_err(),
            "irfft should return error for invalid DC bin with non-zero imaginary part"
        );
    }

    #[test]
    fn test_irfft_returns_error_for_invalid_nyquist_bin() {
        // Test that irfft propagates FftError::InputValues when Nyquist bin has non-zero imaginary part.
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        // Create a spectrum with invalid Nyquist bin (non-zero imaginary part)
        let mut invalid_spectrum = vec![Complex32::new(0.0, 0.0); state.freq_size];
        let nyquist_idx = state.freq_size - 1;
        invalid_spectrum[nyquist_idx] = Complex32::new(1.0, 1.0); // Nyquist bin with non-zero imaginary - invalid

        let mut output = vec![0.0f32; state.fft_size];

        // irfft should return an error for invalid input
        let result = state.irfft(&mut invalid_spectrum, &mut output);
        assert!(
            result.is_err(),
            "irfft should return error for invalid Nyquist bin with non-zero imaginary part"
        );
    }

    #[test]
    fn test_frame_synthesis_returns_error_for_invalid_dc_bin() {
        // Test that frame_synthesis returns FftError::InputValues when DC bin has
        // non-zero imaginary part. This allows callers to handle IFFT failures via
        // passthrough with warning logs (Task 023/025 requirement).
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        // Create a spectrum with invalid DC bin (non-zero imaginary part)
        let mut spectrum = vec![Complex32::new(0.0, 0.0); state.freq_size];
        spectrum[0] = Complex32::new(1.0, 0.5); // DC with non-zero imaginary - invalid

        let result = state.frame_synthesis(&mut spectrum);
        assert!(
            result.is_err(),
            "frame_synthesis should return error for invalid DC bin with non-zero imaginary part"
        );
    }

    #[test]
    fn test_frame_synthesis_returns_error_for_invalid_nyquist_bin() {
        // Test that frame_synthesis returns FftError::InputValues when Nyquist bin has
        // non-zero imaginary part.
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        // Create a spectrum with invalid Nyquist bin (non-zero imaginary part)
        let mut spectrum = vec![Complex32::new(0.0, 0.0); state.freq_size];
        let nyquist_idx = state.freq_size - 1;
        spectrum[nyquist_idx] = Complex32::new(1.0, 0.5); // Nyquist with non-zero imaginary - invalid

        let result = state.frame_synthesis(&mut spectrum);
        assert!(
            result.is_err(),
            "frame_synthesis should return error for invalid Nyquist bin with non-zero imaginary part"
        );
    }

    #[test]
    fn test_frame_synthesis_succeeds_with_valid_spectrum() {
        // Test that frame_synthesis succeeds when DC and Nyquist bins have zero imaginary parts.
        let params = DfParams::default();
        let mut state = DfState::new(&params);

        // Create a valid spectrum (DC and Nyquist with zero imaginary parts)
        let mut spectrum = vec![Complex32::new(1.0, 0.0); state.freq_size];
        spectrum[0] = Complex32::new(1.0, 0.0); // DC with zero imaginary - valid
        let nyquist_idx = state.freq_size - 1;
        spectrum[nyquist_idx] = Complex32::new(1.0, 0.0); // Nyquist with zero imaginary - valid

        let result = state.frame_synthesis(&mut spectrum);
        assert!(
            result.is_ok(),
            "frame_synthesis should succeed with valid DC/Nyquist bins"
        );

        // Output should be finite
        let output = result.unwrap();
        for &s in &output {
            assert!(s.is_finite(), "Output should contain finite values");
        }
    }
}
