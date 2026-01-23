//! Configuration for DeepFilterNet noise filter.

use anyhow::{Context, Result};
use ort::session::builder::GraphOptimizationLevel as OrtGraphOptLevel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Default URL for downloading DeepFilterNet3 low-latency ONNX models
const DEFAULT_MODEL_URL: &str =
    "https://github.com/Rikorose/DeepFilterNet/raw/main/models/DeepFilterNet3_ll_onnx.tar.gz";

/// Subdirectory name for noise filter cache
pub const CACHE_SUBDIR: &str = "noise_filter";

/// Model filenames in the tarball
pub const ENC_MODEL_FILENAME: &str = "enc.onnx";
pub const ERB_DEC_MODEL_FILENAME: &str = "erb_dec.onnx";
pub const DF_DEC_MODEL_FILENAME: &str = "df_dec.onnx";
pub const CONFIG_FILENAME: &str = "config.ini";

/// Parameters from the [df] section of config.ini.
/// These control the DSP pipeline configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DfParams {
    /// Sample rate in Hz (typically 48000).
    pub sr: u32,
    /// FFT size in samples (typically 960).
    pub fft_size: usize,
    /// STFT hop size in samples (typically 480).
    pub hop_size: usize,
    /// Number of ERB bands (typically 32).
    pub nb_erb: usize,
    /// Number of deep filtering bins (typically 96).
    pub nb_df: usize,
    /// Minimum frequency bins per ERB band (typically 2).
    pub min_nb_erb_freqs: usize,
    /// Deep filtering order (typically 5).
    pub df_order: usize,
    /// Deep filtering look-ahead frames (typically 0).
    pub df_lookahead: usize,
}

impl Default for DfParams {
    fn default() -> Self {
        Self {
            sr: 48000,
            fft_size: 960,
            hop_size: 480,
            nb_erb: 32,
            nb_df: 96,
            min_nb_erb_freqs: 2,
            df_order: 5,
            df_lookahead: 0,
        }
    }
}

/// Parameters from the [deepfilternet] section of config.ini.
/// These control the neural network architecture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepFilterNetParams {
    /// Convolution look-ahead frames (typically 0 for low-latency).
    pub conv_lookahead: usize,
    /// Convolution channel width (layer_width, typically 64).
    pub conv_ch: usize,
}

impl Default for DeepFilterNetParams {
    fn default() -> Self {
        Self {
            conv_lookahead: 0,
            conv_ch: 64, // Default from upstream DeepFilterNet
        }
    }
}

/// Configuration for the noise filter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoiseFilterConfig {
    /// Path to the DeepFilterNet model directory.
    pub model_path: Option<PathBuf>,

    /// URL to download the model from (if not cached).
    pub model_url: Option<String>,

    /// Cache directory for downloaded models.
    pub cache_path: Option<PathBuf>,

    /// Input sample rate (will be resampled to 48kHz internally).
    pub sample_rate: u32,

    /// Attenuation limit in dB (higher = more aggressive filtering).
    /// Default: 100.0 dB (essentially unlimited attenuation).
    /// Note: The runtime worker pool in `src/core/noise_filter/pool.rs` overrides this
    /// to 40.0 dB for better speech naturalness in real-time voice processing.
    /// A 40dB limit ensures ~1% of the original signal remains, preventing artifacts.
    pub atten_lim_db: f32,

    /// Post-filter beta (controls noise floor).
    pub post_filter_beta: f32,

    /// Number of threads for ONNX inference.
    pub num_threads: Option<usize>,

    /// Minimum dB threshold for LSNR gating.
    /// Below this, the frame is considered mostly noise and a zero mask is applied.
    /// Matches upstream DeepFilterNet RuntimeParams default: -10.0
    pub min_db_thresh: f32,

    /// Maximum dB threshold (LSNR) for applying ERB gains.
    /// Above this, the signal is considered clean speech and gains are skipped.
    /// Matches upstream DeepFilterNet RuntimeParams default: 30.0
    pub max_db_erb_thresh: f32,

    /// Maximum dB threshold (LSNR) for applying deep filtering.
    /// Above this but below max_db_erb_thresh, only ERB gains are applied.
    /// Matches upstream DeepFilterNet RuntimeParams default: 20.0
    pub max_db_df_thresh: f32,

    /// ONNX graph optimization level.
    pub graph_optimization_level: GraphOptimizationLevel,

    /// Parsed DSP parameters from config.ini.
    #[serde(skip)]
    pub df_params: Option<DfParams>,

    /// Parsed neural network parameters from config.ini.
    #[serde(skip)]
    pub dfnet_params: Option<DeepFilterNetParams>,
}

impl Default for NoiseFilterConfig {
    fn default() -> Self {
        Self {
            model_path: None,
            model_url: Some(DEFAULT_MODEL_URL.to_string()),
            cache_path: None,
            sample_rate: 16000,
            atten_lim_db: 100.0,
            post_filter_beta: 0.02,
            num_threads: Some(4),
            min_db_thresh: -10.0,
            max_db_erb_thresh: 30.0,
            max_db_df_thresh: 20.0,
            graph_optimization_level: GraphOptimizationLevel::Level3,
            df_params: None,
            dfnet_params: None,
        }
    }
}

impl NoiseFilterConfig {
    /// Get the cache directory for noise filter models.
    ///
    /// Returns an error if no cache path is configured.
    pub fn get_cache_dir(&self) -> Result<PathBuf> {
        let cache_dir = if let Some(cache_path) = &self.cache_path {
            cache_path.join(CACHE_SUBDIR)
        } else {
            anyhow::bail!("No cache directory specified for noise filter");
        };
        Ok(cache_dir)
    }

    /// Parse parameters from a config.ini file.
    ///
    /// The config.ini file is expected to have [df] and optionally [deepfilternet] sections.
    pub fn from_ini(path: &Path) -> Result<(DfParams, DeepFilterNetParams)> {
        let path_str = path
            .to_str()
            .context("Invalid path: contains non-UTF8 characters")?;

        // Use the ini! macro with safe variant to handle errors gracefully
        let ini: HashMap<String, HashMap<String, Option<String>>> = ini::ini!(safe path_str)
            .map_err(|e| anyhow::anyhow!("Failed to load config.ini from {:?}: {}", path, e))?;

        let df_params = Self::parse_df_section(&ini)?;
        let dfnet_params = Self::parse_deepfilternet_section(&ini)?;

        Ok((df_params, dfnet_params))
    }

    /// Parse the [df] section from config.ini.
    fn parse_df_section(
        ini: &HashMap<String, HashMap<String, Option<String>>>,
    ) -> Result<DfParams> {
        let section = ini
            .get("df")
            .context("Missing [df] section in config.ini")?;

        let get_value = |key: &str| -> Result<String> {
            section
                .get(key)
                .context(format!("Missing '{}' in [df] section", key))?
                .clone()
                .context(format!("Empty value for '{}' in [df] section", key))
        };

        let sr = get_value("sr")?
            .parse::<u32>()
            .context("Invalid 'sr' value")?;

        let fft_size = get_value("fft_size")?
            .parse::<usize>()
            .context("Invalid 'fft_size' value")?;

        let hop_size = get_value("hop_size")?
            .parse::<usize>()
            .context("Invalid 'hop_size' value")?;

        let nb_erb = get_value("nb_erb")?
            .parse::<usize>()
            .context("Invalid 'nb_erb' value")?;

        let nb_df = get_value("nb_df")?
            .parse::<usize>()
            .context("Invalid 'nb_df' value")?;

        let min_nb_erb_freqs = get_value("min_nb_erb_freqs")?
            .parse::<usize>()
            .context("Invalid 'min_nb_erb_freqs' value")?;

        let df_order = get_value("df_order")?
            .parse::<usize>()
            .context("Invalid 'df_order' value")?;

        let df_lookahead = get_value("df_lookahead")?
            .parse::<usize>()
            .context("Invalid 'df_lookahead' value")?;

        Ok(DfParams {
            sr,
            fft_size,
            hop_size,
            nb_erb,
            nb_df,
            min_nb_erb_freqs,
            df_order,
            df_lookahead,
        })
    }

    /// Parse the [deepfilternet] section from config.ini.
    fn parse_deepfilternet_section(
        ini: &HashMap<String, HashMap<String, Option<String>>>,
    ) -> Result<DeepFilterNetParams> {
        let section = match ini.get("deepfilternet") {
            Some(s) => s,
            None => return Ok(DeepFilterNetParams::default()),
        };

        let conv_lookahead = section
            .get("conv_lookahead")
            .and_then(|v| v.clone())
            .map(|v| v.parse::<usize>())
            .transpose()
            .context("Invalid 'conv_lookahead' value")?
            .unwrap_or(0);

        let conv_ch = section
            .get("conv_ch")
            .and_then(|v| v.clone())
            .map(|v| v.parse::<usize>())
            .transpose()
            .context("Invalid 'conv_ch' value")?
            .unwrap_or(64); // Default from upstream DeepFilterNet

        Ok(DeepFilterNetParams {
            conv_lookahead,
            conv_ch,
        })
    }
}

/// ONNX graph optimization level.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum GraphOptimizationLevel {
    /// No optimization.
    Disabled,
    /// Basic optimizations.
    Basic,
    /// Extended optimizations (includes basic).
    Extended,
    /// All optimizations (includes basic and extended).
    #[default]
    Level3,
}

impl GraphOptimizationLevel {
    /// Convert to ORT graph optimization level.
    pub fn to_ort_level(&self) -> OrtGraphOptLevel {
        match self {
            Self::Disabled => OrtGraphOptLevel::Disable,
            Self::Basic => OrtGraphOptLevel::Level1,
            Self::Extended => OrtGraphOptLevel::Level2,
            Self::Level3 => OrtGraphOptLevel::Level3,
        }
    }
}
