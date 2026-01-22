//! ONNX model management for DeepFilterNet.
//!
//! This module handles loading and running the DeepFilterNet ONNX models
//! through the ORT (ONNX Runtime) backend.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use ndarray::{Array1, Array2, ArrayView1};
use ort::session::Session;
use ort::session::builder::SessionBuilder;
use ort::value::Value;
use realfft::num_complex::Complex32;
use tracing::{debug, info, warn};

use super::assets;
use super::config::{
    DF_DEC_MODEL_FILENAME, DfParams, ENC_MODEL_FILENAME, ERB_DEC_MODEL_FILENAME, NoiseFilterConfig,
};
use super::dsp::{
    ALPHA, DfState, apply_df, apply_interp_band_gain, compute_feat_erb, compute_feat_spec,
};

/// Number of encoder hidden states passed as skip connections.
const NUM_ENC_HIDDEN: usize = 4;

/// Manages the DeepFilterNet ONNX models (encoder, ERB decoder, DF decoder).
pub struct ModelManager {
    /// Encoder session.
    enc_session: Arc<Mutex<Session>>,
    /// ERB decoder session.
    erb_dec_session: Arc<Mutex<Session>>,
    /// DF decoder session.
    df_dec_session: Arc<Mutex<Session>>,
    /// Configuration.
    config: NoiseFilterConfig,
    /// DSP parameters from model config.ini.
    df_params: DfParams,
    /// Encoder hidden states (e0, e1, e2, e3) from previous frame.
    enc_hidden: Vec<Vec<f32>>,
    /// Encoder embedding from previous frame.
    enc_emb: Vec<f32>,
    /// Encoder c0 state from previous frame.
    enc_c0: Vec<f32>,
}

impl ModelManager {
    /// Create a new model manager and load the ONNX models.
    pub async fn new(config: NoiseFilterConfig) -> Result<Self> {
        let model_dir = assets::model_path(&config)?;

        info!("Loading DeepFilterNet ONNX models from: {:?}", model_dir);

        // Load all three models
        let (enc_session, erb_dec_session, df_dec_session, df_params) =
            tokio::task::spawn_blocking({
                let model_dir = model_dir.clone();
                let config = config.clone();
                move || Self::load_models(&model_dir, &config)
            })
            .await
            .context("Failed to spawn blocking task for ONNX model loading")??;

        // Validate model I/O
        Self::validate_models(&enc_session, &erb_dec_session, &df_dec_session)?;

        // Initialize hidden states (will be properly sized on first inference)
        let enc_hidden = vec![Vec::new(); NUM_ENC_HIDDEN];
        let enc_emb = Vec::new();
        let enc_c0 = Vec::new();

        Ok(Self {
            enc_session: Arc::new(Mutex::new(enc_session)),
            erb_dec_session: Arc::new(Mutex::new(erb_dec_session)),
            df_dec_session: Arc::new(Mutex::new(df_dec_session)),
            config,
            df_params,
            enc_hidden,
            enc_emb,
            enc_c0,
        })
    }

    /// Load all three ONNX models.
    fn load_models(
        model_dir: &Path,
        config: &NoiseFilterConfig,
    ) -> Result<(Session, Session, Session, DfParams)> {
        // Parse config.ini for parameters
        let config_ini_path = model_dir.join("config.ini");
        let (df_params, dfnet_params) = NoiseFilterConfig::from_ini(&config_ini_path)?;

        // Log model parameters from config.ini
        info!(
            "DeepFilterNet model loaded: sr={}, fft_size={}, hop_size={}, nb_erb={}, nb_df={}, df_order={}, conv_ch={}",
            df_params.sr,
            df_params.fft_size,
            df_params.hop_size,
            df_params.nb_erb,
            df_params.nb_df,
            df_params.df_order,
            dfnet_params.conv_ch
        );

        // Log derived parameters at debug level
        let freq_size = df_params.fft_size / 2 + 1;
        debug!(
            "DeepFilterNet derived params: freq_size={}, df_lookahead={}",
            freq_size, df_params.df_lookahead
        );

        let enc_path = model_dir.join(ENC_MODEL_FILENAME);
        let erb_dec_path = model_dir.join(ERB_DEC_MODEL_FILENAME);
        let df_dec_path = model_dir.join(DF_DEC_MODEL_FILENAME);

        info!("Loading encoder from: {:?}", enc_path);
        let enc_session = Self::create_session(&enc_path, config)?;

        info!("Loading ERB decoder from: {:?}", erb_dec_path);
        let erb_dec_session = Self::create_session(&erb_dec_path, config)?;

        info!("Loading DF decoder from: {:?}", df_dec_path);
        let df_dec_session = Self::create_session(&df_dec_path, config)?;

        Ok((enc_session, erb_dec_session, df_dec_session, df_params))
    }

    /// Create an ONNX session with configuration.
    fn create_session(model_path: &Path, config: &NoiseFilterConfig) -> Result<Session> {
        let mut builder = SessionBuilder::new()?
            .with_optimization_level(config.graph_optimization_level.to_ort_level())?;

        if let Some(num_threads) = config.num_threads {
            builder = builder
                .with_intra_threads(num_threads)?
                .with_inter_threads(1)?;
        }

        let session = builder.commit_from_file(model_path)?;
        Ok(session)
    }

    /// Validate model inputs and outputs match upstream DeepFilterNet specification.
    ///
    /// Expected I/O names per libDF/src/tract.rs:
    /// - Encoder inputs: `feat_erb`, `feat_spec`
    /// - Encoder outputs: `e0`, `e1`, `e2`, `e3`, `emb`, `c0`, `lsnr`
    /// - ERB decoder inputs: `emb`, `e3`, `e2`, `e1`, `e0`
    /// - ERB decoder output: `m`
    /// - DF decoder inputs: `emb`, `c0`
    /// - DF decoder output: `coefs`
    fn validate_models(
        enc_session: &Session,
        erb_dec_session: &Session,
        df_dec_session: &Session,
    ) -> Result<()> {
        // Expected I/O names per upstream libDF/src/tract.rs
        const ENC_EXPECTED_INPUTS: &[&str] = &["feat_erb", "feat_spec"];
        const ENC_EXPECTED_OUTPUTS: &[&str] = &["e0", "e1", "e2", "e3", "emb", "c0", "lsnr"];
        const ERB_DEC_EXPECTED_INPUTS: &[&str] = &["emb", "e3", "e2", "e1", "e0"];
        const ERB_DEC_EXPECTED_OUTPUTS: &[&str] = &["m"];
        const DF_DEC_EXPECTED_INPUTS: &[&str] = &["emb", "c0"];
        const DF_DEC_EXPECTED_OUTPUTS: &[&str] = &["coefs"];

        // Encoder validation
        debug!("Encoder inputs: {:?}", enc_session.inputs().len());
        for (i, input) in enc_session.inputs().iter().enumerate() {
            debug!("  Input {}: name={}", i, input.name());
        }
        debug!("Encoder outputs: {:?}", enc_session.outputs().len());
        for (i, output) in enc_session.outputs().iter().enumerate() {
            debug!("  Output {}: name={}", i, output.name());
        }

        Self::validate_io_names(
            "Encoder",
            &enc_session
                .inputs()
                .iter()
                .map(|i| i.name())
                .collect::<Vec<_>>(),
            ENC_EXPECTED_INPUTS,
            "inputs",
        )?;
        Self::validate_io_names(
            "Encoder",
            &enc_session
                .outputs()
                .iter()
                .map(|o| o.name())
                .collect::<Vec<_>>(),
            ENC_EXPECTED_OUTPUTS,
            "outputs",
        )?;

        // ERB decoder validation
        debug!("ERB decoder inputs: {:?}", erb_dec_session.inputs().len());
        for (i, input) in erb_dec_session.inputs().iter().enumerate() {
            debug!("  Input {}: name={}", i, input.name());
        }
        debug!("ERB decoder outputs: {:?}", erb_dec_session.outputs().len());

        Self::validate_io_names(
            "ERB decoder",
            &erb_dec_session
                .inputs()
                .iter()
                .map(|i| i.name())
                .collect::<Vec<_>>(),
            ERB_DEC_EXPECTED_INPUTS,
            "inputs",
        )?;
        Self::validate_io_names(
            "ERB decoder",
            &erb_dec_session
                .outputs()
                .iter()
                .map(|o| o.name())
                .collect::<Vec<_>>(),
            ERB_DEC_EXPECTED_OUTPUTS,
            "outputs",
        )?;

        // DF decoder validation
        debug!("DF decoder inputs: {:?}", df_dec_session.inputs().len());
        for (i, input) in df_dec_session.inputs().iter().enumerate() {
            debug!("  Input {}: name={}", i, input.name());
        }
        debug!("DF decoder outputs: {:?}", df_dec_session.outputs().len());

        Self::validate_io_names(
            "DF decoder",
            &df_dec_session
                .inputs()
                .iter()
                .map(|i| i.name())
                .collect::<Vec<_>>(),
            DF_DEC_EXPECTED_INPUTS,
            "inputs",
        )?;
        Self::validate_io_names(
            "DF decoder",
            &df_dec_session
                .outputs()
                .iter()
                .map(|o| o.name())
                .collect::<Vec<_>>(),
            DF_DEC_EXPECTED_OUTPUTS,
            "outputs",
        )?;

        info!("All DeepFilterNet models validated successfully");
        Ok(())
    }

    /// Validate that model I/O names match expected names from upstream specification.
    fn validate_io_names(
        model_name: &str,
        actual: &[&str],
        expected: &[&str],
        io_type: &str,
    ) -> Result<()> {
        // Check that all expected names are present (order may vary in ONNX models)
        for &expected_name in expected {
            if !actual.contains(&expected_name) {
                anyhow::bail!(
                    "{} model missing expected {} '{}'. \
                     Expected: {:?}, Actual: {:?}",
                    model_name,
                    io_type,
                    expected_name,
                    expected,
                    actual
                );
            }
        }
        Ok(())
    }

    /// Process a single audio frame through the DeepFilterNet pipeline.
    ///
    /// # Arguments
    /// * `state` - DSP state containing FFT plans, buffers, and normalization states.
    /// * `input_frame` - Time-domain audio frame (length = hop_size).
    ///
    /// # Returns
    /// * Filtered time-domain audio frame (length = hop_size).
    pub fn process_frame(&mut self, state: &mut DfState, input_frame: &[f32]) -> Result<Vec<f32>> {
        debug_assert_eq!(input_frame.len(), state.hop_size);

        // Step 1: STFT Analysis
        let spectrum = state.frame_analysis(input_frame);
        state.push_rolling_spec_x(spectrum.clone());

        // Step 2: Compute features
        // Get references we need before mutable borrows
        let erb_fb_copy: Vec<usize> = state.erb_fb().to_vec();
        let nb_df = state.nb_df;

        let feat_erb = {
            let mut mean_state = state.mean_norm_state_mut().view_mut();
            compute_feat_erb(spectrum.view(), &erb_fb_copy, &mut mean_state, ALPHA)
        };

        let feat_spec = {
            let mut unit_state = state.unit_norm_state_mut().view_mut();
            compute_feat_spec(spectrum.view(), nb_df, &mut unit_state, ALPHA)
        };

        // Step 3: Run Encoder
        let (lsnr, enc_outputs) = self.run_encoder(&feat_erb, &feat_spec)?;

        // Update encoder hidden states for next frame
        self.update_encoder_states(&enc_outputs);

        // Step 4: LSNR Gating - determine which stages to apply
        let (apply_erb, apply_erb_zeros, apply_df) = self.compute_gating(lsnr);

        debug!(
            "LSNR: {:.2} dB, apply_erb: {}, apply_erb_zeros: {}, apply_df: {}",
            lsnr, apply_erb, apply_erb_zeros, apply_df
        );

        // Prepare output spectrum (clone input spectrum)
        let mut output_spectrum: Vec<Complex32> = spectrum.to_vec();

        // Step 5: Run ERB Decoder and apply mask, or apply zero mask for low LSNR
        if apply_erb {
            let erb_gains = self.run_erb_decoder(&enc_outputs)?;
            self.apply_erb_mask(&mut output_spectrum, &erb_gains, state);
        } else if apply_erb_zeros {
            // Low LSNR: apply zero mask (maximum suppression) - mostly noise
            self.apply_zero_mask(&mut output_spectrum, state);
        }

        // Step 6: Run DF Decoder and apply deep filtering
        if apply_df && state.rolling_spec_x().len() >= state.df_order {
            let df_coefs = self.run_df_decoder(&enc_outputs)?;
            self.apply_deep_filtering(&mut output_spectrum, &df_coefs, state);
        }

        // Push output spectrum to rolling buffer
        state.push_rolling_spec_y(Array1::from_vec(output_spectrum.clone()));

        // Step 7: Apply post-filter (noise floor subtraction) if enabled
        if self.config.post_filter_beta > 0.0 {
            self.apply_post_filter(&mut output_spectrum, &spectrum.view());
        }

        // Step 8: Apply attenuation limiting
        self.apply_atten_limit(&mut output_spectrum, &spectrum.view());

        // Step 9: ISTFT Synthesis
        let output_frame = state.frame_synthesis(&mut output_spectrum);

        // Step 10: Validate output for NaN/Inf and fallback to input if corrupted
        if Self::contains_non_finite(&output_frame) {
            warn!(
                "DeepFilterNet produced non-finite output (NaN/Inf), falling back to original audio"
            );
            return Ok(input_frame.to_vec());
        }

        Ok(output_frame)
    }

    /// Run the encoder model.
    ///
    /// Returns (lsnr, encoder_outputs) where encoder_outputs contains e0-e3, emb, c0.
    fn run_encoder(
        &self,
        feat_erb: &Array1<f32>,
        feat_spec: &Array1<f32>,
    ) -> Result<(f32, EncoderOutputs)> {
        let mut session = self.enc_session.lock().unwrap();

        // Collect output names first to avoid borrow issues
        let output_names: Vec<String> = session
            .outputs()
            .iter()
            .map(|o| o.name().to_string())
            .collect();

        // Prepare inputs with shape [1, 1, S, features] where S=1 for streaming
        // DeepFilterNet expects: feat_erb [1, 1, 1, nb_erb] and feat_spec [1, 2, 1, nb_df]
        let erb_data: Vec<f32> = feat_erb.iter().copied().collect();
        let spec_data: Vec<f32> = feat_spec.iter().copied().collect();

        // Build inputs: upstream encoder only takes feat_erb and feat_spec
        // Shape: feat_erb [1, 1, 1, nb_erb], feat_spec [1, 2, 1, nb_df]
        let inputs: Vec<(&str, Value)> = vec![
            (
                "feat_erb",
                Value::from_array(([1usize, 1, 1, self.df_params.nb_erb], erb_data))?.into(),
            ),
            (
                "feat_spec",
                Value::from_array(([1usize, 2, 1, self.df_params.nb_df], spec_data))?.into(),
            ),
        ];

        // Run encoder
        let outputs = session.run(inputs).context("Encoder inference failed")?;

        // Extract outputs, preserving original shapes
        let mut enc_outputs = EncoderOutputs::default();
        let mut lsnr = 0.0f32;

        for name in &output_names {
            if let Some(value) = outputs.get(name.as_str())
                && let Ok(tensor) = value.try_extract_tensor::<f32>()
            {
                let (shape, data) = tensor;
                let shape_vec: Vec<usize> = shape.iter().map(|&d| d as usize).collect();

                if name.contains("lsnr") {
                    lsnr = data.first().copied().unwrap_or(0.0);
                } else if name.starts_with("e") && name.len() == 2 {
                    let idx = name[1..].parse::<usize>().unwrap_or(0);
                    if idx < NUM_ENC_HIDDEN {
                        debug!(
                            "Encoder output '{}' shape: {:?}, len: {}",
                            name,
                            shape_vec,
                            data.len()
                        );
                        enc_outputs.e[idx] = TensorData::new(data.to_vec(), shape_vec);
                    }
                } else if name.contains("emb") {
                    debug!(
                        "Encoder output '{}' shape: {:?}, len: {}",
                        name,
                        shape_vec,
                        data.len()
                    );
                    enc_outputs.emb = TensorData::new(data.to_vec(), shape_vec);
                } else if name.contains("c0") {
                    debug!(
                        "Encoder output '{}' shape: {:?}, len: {}",
                        name,
                        shape_vec,
                        data.len()
                    );
                    enc_outputs.c0 = TensorData::new(data.to_vec(), shape_vec);
                }
            }
        }

        Ok((lsnr, enc_outputs))
    }

    /// Update encoder hidden states for next frame.
    fn update_encoder_states(&mut self, outputs: &EncoderOutputs) {
        for i in 0..NUM_ENC_HIDDEN {
            if !outputs.e[i].is_empty() {
                self.enc_hidden[i] = outputs.e[i].data.clone();
            }
        }
        if !outputs.emb.is_empty() {
            self.enc_emb = outputs.emb.data.clone();
        }
        if !outputs.c0.is_empty() {
            self.enc_c0 = outputs.c0.data.clone();
        }
    }

    /// Compute gating based on LSNR thresholds.
    ///
    /// Returns (apply_erb, apply_erb_zeros, apply_df) flags matching upstream `apply_stages`:
    /// - If lsnr < min_db_thresh: apply zero mask, skip DF (mostly noise)
    /// - If lsnr > max_db_erb_thresh: skip gains + DF (mostly speech, clean)
    /// - If lsnr > max_db_df_thresh: apply gains, skip DF
    /// - Else: apply gains + DF
    fn compute_gating(&self, lsnr: f32) -> (bool, bool, bool) {
        if lsnr < self.config.min_db_thresh {
            // Only noise detected, apply a zero mask (maximum suppression)
            (false, true, false)
        } else if lsnr > self.config.max_db_erb_thresh {
            // Clean speech signal detected, don't apply any processing
            (false, false, false)
        } else if lsnr > self.config.max_db_df_thresh {
            // Only slightly noisy signal, apply 1st stage (ERB gains), skip DF
            (true, false, false)
        } else {
            // Regular noisy signal, apply both 1st stage (ERB) and 2nd stage (DF)
            (true, false, true)
        }
    }

    /// Run the ERB decoder model.
    ///
    /// Uses the original tensor shapes from encoder outputs to ensure correct data layout.
    fn run_erb_decoder(&self, enc_outputs: &EncoderOutputs) -> Result<Vec<f32>> {
        let mut session = self.erb_dec_session.lock().unwrap();

        // Get output name before running
        let output_name = session.outputs()[0].name().to_string();

        // Build inputs using the ORIGINAL shapes from encoder outputs
        // ERB decoder takes: emb, e3, e2, e1, e0 (skip connections in reverse order)
        let mut inputs: Vec<(&str, Value)> =
            vec![("emb", Self::create_value_with_shape(&enc_outputs.emb)?)];

        // Add skip connections in reverse order: e3, e2, e1, e0 (per upstream spec)
        if !enc_outputs.e[3].is_empty() {
            inputs.push(("e3", Self::create_value_with_shape(&enc_outputs.e[3])?));
        }
        if !enc_outputs.e[2].is_empty() {
            inputs.push(("e2", Self::create_value_with_shape(&enc_outputs.e[2])?));
        }
        if !enc_outputs.e[1].is_empty() {
            inputs.push(("e1", Self::create_value_with_shape(&enc_outputs.e[1])?));
        }
        if !enc_outputs.e[0].is_empty() {
            inputs.push(("e0", Self::create_value_with_shape(&enc_outputs.e[0])?));
        }

        let outputs = session
            .run(inputs)
            .context("ERB decoder inference failed")?;

        // Extract ERB gains from first output
        let output_value = outputs
            .get(output_name.as_str())
            .context("No output from ERB decoder")?;
        let tensor = output_value
            .try_extract_tensor::<f32>()
            .context("Failed to extract ERB decoder output")?;

        let (_, data) = tensor;
        Ok(data.to_vec())
    }

    /// Create an ORT Value from TensorData, preserving the original shape.
    fn create_value_with_shape(tensor_data: &TensorData) -> Result<Value> {
        let shape = &tensor_data.shape;
        let data = &tensor_data.data;

        // Create value based on the number of dimensions
        match shape.len() {
            1 => Ok(Value::from_array(([shape[0]], data.clone()))?.into()),
            2 => Ok(Value::from_array(([shape[0], shape[1]], data.clone()))?.into()),
            3 => Ok(Value::from_array(([shape[0], shape[1], shape[2]], data.clone()))?.into()),
            4 => Ok(
                Value::from_array(([shape[0], shape[1], shape[2], shape[3]], data.clone()))?.into(),
            ),
            5 => Ok(Value::from_array((
                [shape[0], shape[1], shape[2], shape[3], shape[4]],
                data.clone(),
            ))?
            .into()),
            _ => anyhow::bail!(
                "Unsupported tensor shape with {} dimensions: {:?}",
                shape.len(),
                shape
            ),
        }
    }

    /// Run the DF decoder model.
    ///
    /// Uses the original tensor shapes from encoder outputs to ensure correct data layout.
    fn run_df_decoder(&self, enc_outputs: &EncoderOutputs) -> Result<Array2<f32>> {
        let mut session = self.df_dec_session.lock().unwrap();

        // Get output name before running
        let output_name = session.outputs()[0].name().to_string();

        let nb_df = self.df_params.nb_df;
        let df_order = self.df_params.df_order;

        // Build inputs using the ORIGINAL shapes from encoder outputs
        // DF decoder takes embedding and c0 from encoder
        let inputs: Vec<(&str, Value)> = vec![
            ("emb", Self::create_value_with_shape(&enc_outputs.emb)?),
            ("c0", Self::create_value_with_shape(&enc_outputs.c0)?),
        ];

        let outputs = session.run(inputs).context("DF decoder inference failed")?;

        // Extract DF coefficients from first output
        // Shape should be [1, nb_df, df_order, 2] (real/imag)
        let output_value = outputs
            .get(output_name.as_str())
            .context("No output from DF decoder")?;
        let tensor = output_value
            .try_extract_tensor::<f32>()
            .context("Failed to extract DF decoder output")?;

        let (shape, data) = tensor;

        debug!("DF decoder output shape: {:?}", shape);

        // Flatten the last two dimensions (df_order, 2) into df_order * 2
        let coefs = Array2::from_shape_vec(
            (nb_df, df_order * 2),
            data.iter().take(nb_df * df_order * 2).copied().collect(),
        )
        .unwrap_or_else(|_| Array2::zeros((nb_df, df_order * 2)));

        Ok(coefs)
    }

    /// Apply ERB mask to the spectrum.
    ///
    /// The ERB decoder outputs gain/mask values that are applied directly to the spectrum.
    /// This matches upstream DeepFilterNet behavior which does not apply any transformation.
    fn apply_erb_mask(&self, spectrum: &mut [Complex32], erb_gains: &[f32], state: &DfState) {
        // Apply gains directly without transformation, matching upstream behavior.
        // The neural network is trained to output mask values in the appropriate range.
        apply_interp_band_gain(spectrum, erb_gains, state.erb_fb());
    }

    /// Apply zero mask to the spectrum (for low LSNR, mostly noise).
    fn apply_zero_mask(&self, spectrum: &mut [Complex32], state: &DfState) {
        // Apply zero gains across all ERB bands (maximum suppression)
        let zero_gains = vec![0.0f32; state.nb_erb];
        apply_interp_band_gain(spectrum, &zero_gains, state.erb_fb());
    }

    /// Apply deep filtering to the spectrum.
    fn apply_deep_filtering(
        &self,
        output_spectrum: &mut [Complex32],
        df_coefs: &Array2<f32>,
        state: &DfState,
    ) {
        apply_df(
            state.rolling_spec_x(),
            df_coefs,
            output_spectrum,
            state.nb_df,
            state.df_order,
        );
    }

    /// Apply post-filter (noise floor subtraction).
    fn apply_post_filter(
        &self,
        output_spectrum: &mut [Complex32],
        input_spectrum: &ArrayView1<Complex32>,
    ) {
        let beta = self.config.post_filter_beta;
        for (i, spec) in output_spectrum.iter_mut().enumerate() {
            if i < input_spectrum.len() {
                let input = input_spectrum[i];
                // Compute magnitudes with epsilon guard to prevent NaN from sqrt of negative
                let input_mag_sq = input.re * input.re + input.im * input.im;
                let output_mag_sq = spec.re * spec.re + spec.im * spec.im;
                let input_mag = input_mag_sq.max(0.0).sqrt();
                let output_mag = output_mag_sq.max(0.0).sqrt();

                if output_mag > 1e-10 {
                    // Subtract noise floor estimate
                    let noise_est = input_mag * beta;
                    let new_mag = (output_mag - noise_est).max(0.0);
                    let scale = new_mag / output_mag;
                    // Guard against NaN/Inf scale values
                    if scale.is_finite() {
                        spec.re *= scale;
                        spec.im *= scale;
                    }
                }
            }
        }
    }

    /// Apply attenuation limiting to prevent over-suppression.
    ///
    /// Uses the upstream DeepFilterNet approach of blending enhanced and noisy spectra:
    /// `output = enhanced * (1 - lim) + noisy * lim`
    ///
    /// where `lim = 10^(-atten_lim_db/20)` is the linear gain floor.
    /// This ensures that even fully suppressed bins retain at least `lim` of the original signal.
    fn apply_atten_limit(
        &self,
        output_spectrum: &mut [Complex32],
        input_spectrum: &ArrayView1<Complex32>,
    ) {
        // Convert dB attenuation limit to linear gain floor
        // e.g., 40 dB limit -> min_gain = 0.01, 100 dB -> min_gain = 0.00001
        // Clamp atten_lim_db to prevent overflow (max ~700 dB before f32 underflow to 0)
        let clamped_db = self.config.atten_lim_db.clamp(0.0, 300.0);
        let lim = 10.0f32.powf(-clamped_db / 20.0);

        // Blend enhanced and noisy spectra: output = enhanced * (1 - lim) + noisy * lim
        // This matches upstream DeepFilterNet tract.rs implementation
        for (i, spec) in output_spectrum.iter_mut().enumerate() {
            if i < input_spectrum.len() {
                let noisy = input_spectrum[i];
                // Scale enhanced by (1 - lim) and add noisy * lim
                let new_re = spec.re * (1.0 - lim) + noisy.re * lim;
                let new_im = spec.im * (1.0 - lim) + noisy.im * lim;
                // Only apply if result is finite, otherwise preserve original
                if new_re.is_finite() && new_im.is_finite() {
                    spec.re = new_re;
                    spec.im = new_im;
                }
            }
        }
    }

    /// Get the configuration.
    pub fn config(&self) -> &NoiseFilterConfig {
        &self.config
    }

    /// Get the DSP parameters.
    pub fn df_params(&self) -> &DfParams {
        &self.df_params
    }

    /// Check if a slice contains any non-finite values (NaN or Inf).
    ///
    /// Returns `true` if any value is NaN or Inf.
    #[inline]
    fn contains_non_finite(samples: &[f32]) -> bool {
        samples.iter().any(|&s| !s.is_finite())
    }

    /// Reset all internal state (for new stream).
    pub fn reset(&mut self) {
        for hidden in &mut self.enc_hidden {
            hidden.clear();
        }
        self.enc_emb.clear();
        self.enc_c0.clear();
    }
}

/// Encoder output tensors with their original shapes preserved.
#[derive(Default)]
struct EncoderOutputs {
    /// Hidden states e0, e1, e2, e3 with their shapes.
    e: [TensorData; NUM_ENC_HIDDEN],
    /// Embedding tensor with shape.
    emb: TensorData,
    /// c0 state for DF decoder with shape.
    c0: TensorData,
}

/// Tensor data with its original shape preserved.
#[derive(Default, Clone)]
struct TensorData {
    /// Flattened data.
    data: Vec<f32>,
    /// Original shape from ONNX output.
    shape: Vec<usize>,
}

impl TensorData {
    fn new(data: Vec<f32>, shape: Vec<usize>) -> Self {
        Self { data, shape }
    }

    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_outputs_default() {
        let outputs = EncoderOutputs::default();
        assert!(outputs.emb.is_empty());
        assert!(outputs.c0.is_empty());
        for e in &outputs.e {
            assert!(e.is_empty());
        }
    }

    #[test]
    fn test_gating_logic() {
        let config = NoiseFilterConfig::default();
        let manager = ModelManagerGatingTest { config };

        // Test case 1: Very low LSNR (below min_db_thresh=-10) - mostly noise
        // Should apply zero mask (apply_erb_zeros=true), skip DF
        let (apply_erb, apply_erb_zeros, apply_df) = manager.compute_gating(-20.0);
        assert!(!apply_erb, "Low LSNR: should not apply ERB gains");
        assert!(apply_erb_zeros, "Low LSNR: should apply zero mask");
        assert!(!apply_df, "Low LSNR: should not apply DF");

        // Test case 2: Boundary at min_db_thresh (-10 dB) - upstream uses strict less-than
        // So at exactly -10.0, it falls through to apply both ERB and DF
        let (apply_erb, apply_erb_zeros, apply_df) = manager.compute_gating(-10.0);
        assert!(
            apply_erb,
            "At min_db_thresh: should apply ERB gains (boundary)"
        );
        assert!(
            !apply_erb_zeros,
            "At min_db_thresh: should not apply zero mask"
        );
        assert!(apply_df, "At min_db_thresh: should apply DF (boundary)");

        // Test case 3: High LSNR (above max_db_erb_thresh=30) - clean speech
        // Should skip all processing
        let (apply_erb, apply_erb_zeros, apply_df) = manager.compute_gating(35.0);
        assert!(!apply_erb, "High LSNR: should skip ERB gains");
        assert!(!apply_erb_zeros, "High LSNR: should not apply zero mask");
        assert!(!apply_df, "High LSNR: should skip DF");

        // Test case 4: Mid-high LSNR (above max_db_df_thresh=20, below max_db_erb_thresh=30)
        // Should apply ERB gains only, skip DF
        let (apply_erb, apply_erb_zeros, apply_df) = manager.compute_gating(25.0);
        assert!(apply_erb, "Mid-high LSNR: should apply ERB gains");
        assert!(
            !apply_erb_zeros,
            "Mid-high LSNR: should not apply zero mask"
        );
        assert!(!apply_df, "Mid-high LSNR: should skip DF");

        // Test case 5: Mid LSNR (above min_db_thresh=-10, below max_db_df_thresh=20)
        // Should apply both ERB gains and DF
        let (apply_erb, apply_erb_zeros, apply_df) = manager.compute_gating(10.0);
        assert!(apply_erb, "Mid LSNR: should apply ERB gains");
        assert!(!apply_erb_zeros, "Mid LSNR: should not apply zero mask");
        assert!(apply_df, "Mid LSNR: should apply DF");

        // Test case 6: Just above min_db_thresh - should apply both stages
        let (apply_erb, apply_erb_zeros, apply_df) = manager.compute_gating(-5.0);
        assert!(apply_erb, "Just above min: should apply ERB gains");
        assert!(
            !apply_erb_zeros,
            "Just above min: should not apply zero mask"
        );
        assert!(apply_df, "Just above min: should apply DF");

        // Test case 7: At max_db_df_thresh boundary (20 dB) - upstream uses strict greater-than
        // So at exactly 20.0, it falls through to apply both ERB and DF
        let (apply_erb, apply_erb_zeros, apply_df) = manager.compute_gating(20.0);
        assert!(apply_erb, "At max_db_df_thresh: should apply ERB gains");
        assert!(
            !apply_erb_zeros,
            "At max_db_df_thresh: should not apply zero mask"
        );
        assert!(apply_df, "At max_db_df_thresh: should apply DF (boundary)");

        // Test case 8: Just above max_db_df_thresh - should skip DF
        let (apply_erb, apply_erb_zeros, apply_df) = manager.compute_gating(21.0);
        assert!(
            apply_erb,
            "Just above max_db_df_thresh: should apply ERB gains"
        );
        assert!(
            !apply_erb_zeros,
            "Just above max_db_df_thresh: should not apply zero mask"
        );
        assert!(!apply_df, "Just above max_db_df_thresh: should skip DF");
    }

    /// Helper struct for testing gating logic without loading models.
    struct ModelManagerGatingTest {
        config: NoiseFilterConfig,
    }

    impl ModelManagerGatingTest {
        /// Compute gating based on LSNR thresholds, matching upstream apply_stages.
        fn compute_gating(&self, lsnr: f32) -> (bool, bool, bool) {
            if lsnr < self.config.min_db_thresh {
                // Only noise detected, apply zero mask
                (false, true, false)
            } else if lsnr > self.config.max_db_erb_thresh {
                // Clean speech, skip all processing
                (false, false, false)
            } else if lsnr > self.config.max_db_df_thresh {
                // Slightly noisy, apply ERB only
                (true, false, false)
            } else {
                // Regular noisy signal, apply both ERB and DF
                (true, false, true)
            }
        }
    }

    #[test]
    fn test_contains_non_finite_with_normal_values() {
        let normal_samples = vec![0.0f32, 0.5, -0.5, 1.0, -1.0, 0.001];
        assert!(
            !ModelManager::contains_non_finite(&normal_samples),
            "Normal values should not be detected as non-finite"
        );
    }

    #[test]
    fn test_contains_non_finite_with_nan() {
        let samples_with_nan = vec![0.0f32, 0.5, f32::NAN, 0.5];
        assert!(
            ModelManager::contains_non_finite(&samples_with_nan),
            "Should detect NaN in samples"
        );
    }

    #[test]
    fn test_contains_non_finite_with_inf() {
        let samples_with_inf = vec![0.0f32, 0.5, f32::INFINITY, 0.5];
        assert!(
            ModelManager::contains_non_finite(&samples_with_inf),
            "Should detect positive infinity in samples"
        );

        let samples_with_neg_inf = vec![0.0f32, 0.5, f32::NEG_INFINITY, 0.5];
        assert!(
            ModelManager::contains_non_finite(&samples_with_neg_inf),
            "Should detect negative infinity in samples"
        );
    }

    #[test]
    fn test_contains_non_finite_with_zeros() {
        let zero_samples = vec![0.0f32; 100];
        assert!(
            !ModelManager::contains_non_finite(&zero_samples),
            "All zeros should not be detected as non-finite"
        );
    }

    #[test]
    fn test_contains_non_finite_with_tiny_values() {
        let tiny_samples = vec![1e-38f32; 100]; // Near minimum positive f32
        assert!(
            !ModelManager::contains_non_finite(&tiny_samples),
            "Tiny values should not be detected as non-finite"
        );
    }

    #[test]
    fn test_contains_non_finite_with_large_values() {
        let large_samples = vec![1e38f32; 100]; // Near maximum positive f32
        assert!(
            !ModelManager::contains_non_finite(&large_samples),
            "Large but finite values should not be detected as non-finite"
        );
    }

    #[test]
    fn test_contains_non_finite_empty() {
        let empty: Vec<f32> = vec![];
        assert!(
            !ModelManager::contains_non_finite(&empty),
            "Empty slice should not contain non-finite values"
        );
    }
}
