//! DeepFilterNet noise filter module.
//!
//! This module provides real-time noise suppression using DeepFilterNet with ORT (ONNX Runtime).
//! It replaces the legacy tract-based implementation with a more performant and maintainable
//! ORT-based approach, aligning with other ONNX modules in the codebase (turn_detect, vad).
//!
//! # Feature Flags
//!
//! This module is gated behind the `noise-filter` feature flag.
//!
//! # Example
//!
//! ```ignore
//! use sayna::core::noise_filter::{NoiseFilter, NoiseFilterBuilder, NoiseFilterConfig};
//!
//! // Create filter with default config
//! let filter = NoiseFilterBuilder::new()
//!     .sample_rate(16000)
//!     .build()
//!     .await?;
//!
//! // Process audio samples
//! let filtered = filter.process(&audio_samples)?;
//! ```

#[cfg(feature = "noise-filter")]
pub mod assets;
#[cfg(feature = "noise-filter")]
pub mod config;
#[cfg(feature = "noise-filter")]
pub mod dsp;
#[cfg(feature = "noise-filter")]
pub mod model_manager;
#[cfg(feature = "noise-filter")]
pub mod pool;

#[cfg(feature = "noise-filter")]
pub use config::NoiseFilterConfig;

// Re-export pool types for convenient access
#[cfg(feature = "noise-filter")]
pub use pool::{StreamNoiseProcessor, reduce_noise_async};

#[cfg(feature = "noise-filter")]
pub use assets::download_assets;

#[cfg(feature = "noise-filter")]
use anyhow::Result;
#[cfg(feature = "noise-filter")]
use audioadapter_buffers::direct::SequentialSliceOfVecs;
#[cfg(feature = "noise-filter")]
use rubato::{Async, FixedAsync, PolynomialDegree, Resampler};
#[cfg(feature = "noise-filter")]
use std::path::PathBuf;
#[cfg(feature = "noise-filter")]
use tracing::debug;

#[cfg(feature = "noise-filter")]
use self::dsp::DfState;
#[cfg(feature = "noise-filter")]
use self::model_manager::ModelManager;

/// Real-time noise filter using DeepFilterNet.
#[cfg(feature = "noise-filter")]
pub struct NoiseFilter {
    config: NoiseFilterConfig,
    model_manager: ModelManager,
    dsp_state: DfState,
    /// Model sample rate (typically 48000).
    model_sr: u32,
    /// Resampler for upsampling input to model SR (None if same rate).
    resampler_up: Option<Async<f32>>,
    /// Resampler for downsampling model output to input SR (None if same rate).
    resampler_down: Option<Async<f32>>,
    /// Input buffer for accumulating samples until we have enough for one hop.
    input_buffer: Vec<f32>,
    /// Output buffer for resampled output samples.
    output_buffer: Vec<f32>,
    /// Number of samples needed at model SR for one hop.
    model_hop_size: usize,
    /// Number of samples needed at input SR that produce one hop at model SR.
    input_hop_size: usize,
    /// Counter for silent frames (used for optimization).
    skip_counter: usize,
}

#[cfg(feature = "noise-filter")]
impl NoiseFilter {
    /// Create a new noise filter with the given configuration.
    pub async fn new(config: NoiseFilterConfig) -> Result<Self> {
        // Ensure assets are downloaded
        assets::ensure_assets(&config).await?;

        // Create model manager (loads ONNX models)
        let model_manager = ModelManager::new(config.clone()).await?;

        // Create DSP state from model params
        let df_params = model_manager.df_params();
        let dsp_state = DfState::new(df_params);

        let model_sr = df_params.sr;
        let input_sr = config.sample_rate;
        let model_hop_size = df_params.hop_size;

        // Calculate input hop size based on resampling ratio
        let ratio = model_sr as f64 / input_sr as f64;
        let input_hop_size = (model_hop_size as f64 / ratio).round() as usize;

        // Create resamplers if input SR differs from model SR
        let (resampler_up, resampler_down) = if input_sr != model_sr {
            // Upsampler: input_sr -> model_sr
            let up = Async::<f32>::new_poly(
                ratio,
                1.0, // Max relative ratio deviation
                PolynomialDegree::Septic,
                input_hop_size,
                1, // Single channel
                FixedAsync::Input,
            )?;

            // Downsampler: model_sr -> input_sr
            let down = Async::<f32>::new_poly(
                1.0 / ratio,
                1.0,
                PolynomialDegree::Septic,
                model_hop_size,
                1,
                FixedAsync::Input,
            )?;

            (Some(up), Some(down))
        } else {
            (None, None)
        };

        debug!(
            "NoiseFilter initialized: input_sr={}, model_sr={}, input_hop={}, model_hop={}",
            input_sr, model_sr, input_hop_size, model_hop_size
        );

        Ok(Self {
            config,
            model_manager,
            dsp_state,
            model_sr,
            resampler_up,
            resampler_down,
            input_buffer: Vec::with_capacity(input_hop_size * 2),
            output_buffer: Vec::new(),
            model_hop_size,
            input_hop_size,
            skip_counter: 0,
        })
    }

    /// Process audio samples through the noise filter.
    ///
    /// Input is i16 PCM samples at the configured sample rate.
    /// Output is filtered i16 PCM samples at the same rate.
    pub fn process(&mut self, samples: &[i16]) -> Result<Vec<i16>> {
        // Convert i16 to f32
        let f32_samples: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();

        // Process and get f32 output
        let filtered = self.process_f32(&f32_samples)?;

        // Convert back to i16
        Ok(filtered
            .iter()
            .map(|&s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16)
            .collect())
    }

    /// Process f32 audio samples through the noise filter.
    ///
    /// Input should be normalized to [-1.0, 1.0] range.
    pub fn process_f32(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        // Add samples to input buffer
        self.input_buffer.extend_from_slice(samples);

        let mut output = Vec::new();

        // Process complete hops
        while self.input_buffer.len() >= self.input_hop_size {
            // Extract one hop worth of input samples
            let input_chunk: Vec<f32> = self.input_buffer.drain(..self.input_hop_size).collect();

            // Process this hop
            let processed = self.process_hop(&input_chunk)?;

            output.extend(processed);
        }

        Ok(output)
    }

    /// Process a single hop of audio.
    fn process_hop(&mut self, input: &[f32]) -> Result<Vec<f32>> {
        // Check for silence (optimization)
        const SILENCE_THRESHOLD: f32 = 1e-7;
        const MAX_SKIP_FRAMES: usize = 5;

        if input.iter().all(|s| s.abs() < SILENCE_THRESHOLD) {
            self.skip_counter += 1;
            if self.skip_counter > MAX_SKIP_FRAMES {
                // Return silence
                return Ok(vec![0.0; input.len()]);
            }
            // Pass through silent frame
            return Ok(input.to_vec());
        } else {
            self.skip_counter = 0;
        }

        // Resample to model SR if needed
        let model_input = if let Some(ref mut resampler) = self.resampler_up {
            let input_vec = vec![input.to_vec()];
            let input_frames = input.len();
            let input_adapter = SequentialSliceOfVecs::new(&input_vec, 1, input_frames).unwrap();
            let max_output = resampler.output_frames_max();
            let mut output_vec = vec![vec![0.0f32; max_output]; 1];
            let mut output_adapter =
                SequentialSliceOfVecs::new_mut(&mut output_vec, 1, max_output).unwrap();
            let (_, frames_written) =
                resampler.process_into_buffer(&input_adapter, &mut output_adapter, None)?;
            output_vec[0].truncate(frames_written);
            output_vec.into_iter().next().unwrap_or_default()
        } else {
            input.to_vec()
        };

        // Ensure we have exactly model_hop_size samples
        let model_frame = if model_input.len() >= self.model_hop_size {
            &model_input[..self.model_hop_size]
        } else {
            // Pad with zeros if not enough samples
            let mut padded = model_input;
            padded.resize(self.model_hop_size, 0.0);
            return self.process_padded_hop(padded, input.len());
        };

        // Process through DeepFilterNet
        let filtered = self
            .model_manager
            .process_frame(&mut self.dsp_state, model_frame)?;

        // Resample back to input SR if needed
        let output = if let Some(ref mut resampler) = self.resampler_down {
            let filtered_vec = vec![filtered];
            let input_frames = filtered_vec[0].len();
            let input_adapter = SequentialSliceOfVecs::new(&filtered_vec, 1, input_frames).unwrap();
            let max_output = resampler.output_frames_max();
            let mut output_vec = vec![vec![0.0f32; max_output]; 1];
            let mut output_adapter =
                SequentialSliceOfVecs::new_mut(&mut output_vec, 1, max_output).unwrap();
            let (_, frames_written) =
                resampler.process_into_buffer(&input_adapter, &mut output_adapter, None)?;
            output_vec[0].truncate(frames_written);
            output_vec.into_iter().next().unwrap_or_default()
        } else {
            filtered
        };

        // Ensure output length matches input length
        let mut result = output;
        result.resize(input.len(), 0.0);

        Ok(result)
    }

    /// Process a padded hop (when we have less than model_hop_size samples).
    fn process_padded_hop(
        &mut self,
        mut padded: Vec<f32>,
        original_len: usize,
    ) -> Result<Vec<f32>> {
        padded.resize(self.model_hop_size, 0.0);

        let filtered = self
            .model_manager
            .process_frame(&mut self.dsp_state, &padded)?;

        // Resample back if needed
        let output = if let Some(ref mut resampler) = self.resampler_down {
            let filtered_vec = vec![filtered];
            let input_frames = filtered_vec[0].len();
            let input_adapter = SequentialSliceOfVecs::new(&filtered_vec, 1, input_frames).unwrap();
            let max_output = resampler.output_frames_max();
            let mut output_vec = vec![vec![0.0f32; max_output]; 1];
            let mut output_adapter =
                SequentialSliceOfVecs::new_mut(&mut output_vec, 1, max_output).unwrap();
            let (_, frames_written) =
                resampler.process_into_buffer(&input_adapter, &mut output_adapter, None)?;
            output_vec[0].truncate(frames_written);
            output_vec.into_iter().next().unwrap_or_default()
        } else {
            filtered
        };

        // Truncate to original length
        let mut result = output;
        result.truncate(original_len);

        Ok(result)
    }

    /// Flush any remaining samples in the buffer.
    ///
    /// Call this when done processing to get any remaining output.
    pub fn flush(&mut self) -> Result<Vec<f32>> {
        if self.input_buffer.is_empty() {
            return Ok(Vec::new());
        }

        // Pad remaining buffer and process
        let remaining = std::mem::take(&mut self.input_buffer);
        let original_len = remaining.len();

        let mut padded = remaining;
        padded.resize(self.input_hop_size, 0.0);

        let processed = self.process_hop(&padded)?;

        // Truncate to original remaining length
        Ok(processed.into_iter().take(original_len).collect())
    }

    /// Reset all internal state (for new stream).
    pub fn reset(&mut self) {
        self.model_manager.reset();
        self.dsp_state.reset();
        self.input_buffer.clear();
        self.output_buffer.clear();
        self.skip_counter = 0;
    }

    /// Get the current configuration.
    pub fn config(&self) -> &NoiseFilterConfig {
        &self.config
    }

    /// Get the hop size at input sample rate.
    pub fn input_hop_size(&self) -> usize {
        self.input_hop_size
    }

    /// Get the model sample rate.
    pub fn model_sr(&self) -> u32 {
        self.model_sr
    }
}

/// Builder for creating NoiseFilter instances.
#[cfg(feature = "noise-filter")]
pub struct NoiseFilterBuilder {
    config: NoiseFilterConfig,
}

#[cfg(feature = "noise-filter")]
impl Default for NoiseFilterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "noise-filter")]
impl NoiseFilterBuilder {
    /// Create a new builder with default configuration.
    pub fn new() -> Self {
        Self {
            config: NoiseFilterConfig::default(),
        }
    }

    /// Set the input sample rate.
    pub fn sample_rate(mut self, rate: u32) -> Self {
        self.config.sample_rate = rate;
        self
    }

    /// Set the model path.
    pub fn model_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.model_path = Some(path.into());
        self
    }

    /// Set the cache path.
    pub fn cache_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.cache_path = Some(path.into());
        self
    }

    /// Set the attenuation limit in dB.
    pub fn atten_lim_db(mut self, db: f32) -> Self {
        self.config.atten_lim_db = db;
        self
    }

    /// Set the post-filter beta.
    pub fn post_filter_beta(mut self, beta: f32) -> Self {
        self.config.post_filter_beta = beta;
        self
    }

    /// Set the number of inference threads.
    pub fn num_threads(mut self, threads: usize) -> Self {
        self.config.num_threads = Some(threads);
        self
    }

    /// Build the noise filter.
    pub async fn build(self) -> Result<NoiseFilter> {
        NoiseFilter::new(self.config).await
    }
}
