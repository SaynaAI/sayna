use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Filename of the smart-turn ONNX model
pub const MODEL_FILENAME: &str = "smart-turn-v3.2-cpu.onnx";

/// Base URL for downloading smart-turn models from HuggingFace
const MODEL_BASE_URL: &str = "https://huggingface.co/pipecat-ai/smart-turn-v3/resolve/main";

/// Returns the default model download URL
fn default_model_url() -> String {
    format!("{}/{}", MODEL_BASE_URL, MODEL_FILENAME)
}

/// Default number of mel frequency bins (Whisper standard configuration)
pub const DEFAULT_MEL_BINS: usize = 80;

/// Default number of mel spectrogram time frames
pub const DEFAULT_MEL_FRAMES: usize = 800;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnDetectorConfig {
    pub threshold: f32,
    pub sample_rate: u32,
    pub max_audio_duration_seconds: u8,
    pub mel_bins: usize,
    pub mel_frames: usize,
    pub model_path: Option<PathBuf>,
    pub model_url: Option<String>,
    pub cache_path: Option<PathBuf>,
    pub use_quantized: bool,
    pub num_threads: Option<usize>,
    pub graph_optimization_level: GraphOptimizationLevel,
}

impl Default for TurnDetectorConfig {
    fn default() -> Self {
        Self {
            // Increased from 0.5 to 0.6 to reduce false positives on filler sounds
            // like "um", "mmm", "ehhh". The model explicitly trained on these sounds
            // and should recognize them as "user still thinking", but a higher threshold
            // provides additional safety margin.
            threshold: 0.6,
            sample_rate: 16000,
            max_audio_duration_seconds: 8,
            mel_bins: DEFAULT_MEL_BINS,
            mel_frames: DEFAULT_MEL_FRAMES,
            model_path: None,
            model_url: Some(default_model_url()),
            cache_path: None,
            use_quantized: true,
            num_threads: Some(4),
            graph_optimization_level: GraphOptimizationLevel::Level3,
        }
    }
}

impl TurnDetectorConfig {
    pub fn get_cache_dir(&self) -> Result<PathBuf> {
        let cache_dir = if let Some(cache_path) = &self.cache_path {
            // Use the cache_path from config and add turn_detect subdirectory
            cache_path.join("turn_detect")
        } else {
            anyhow::bail!("No cache directory specified");
        };
        Ok(cache_dir)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum GraphOptimizationLevel {
    Disabled,
    Basic,
    Extended,
    Level3,
}

impl GraphOptimizationLevel {
    #[cfg(feature = "stt-vad")]
    pub fn to_ort_level(&self) -> ort::session::builder::GraphOptimizationLevel {
        use ort::session::builder::GraphOptimizationLevel as OrtLevel;
        match self {
            Self::Disabled => OrtLevel::Disable,
            Self::Basic => OrtLevel::Level1,
            Self::Extended => OrtLevel::Level2,
            Self::Level3 => OrtLevel::Level3,
        }
    }
}
