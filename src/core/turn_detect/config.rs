use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnDetectorConfig {
    pub threshold: f32,
    pub max_context_turns: usize,
    pub max_sequence_length: usize,
    pub model_path: Option<PathBuf>,
    pub model_url: Option<String>,
    pub tokenizer_path: Option<PathBuf>,
    pub tokenizer_url: Option<String>,
    pub cache_path: Option<PathBuf>,
    pub use_quantized: bool,
    pub num_threads: Option<usize>,
    pub graph_optimization_level: GraphOptimizationLevel,
}

impl Default for TurnDetectorConfig {
    fn default() -> Self {
        Self {
            threshold: 0.7,
            max_context_turns: 4,
            max_sequence_length: 512,
            model_path: None,
            // Use the LiveKit model - it outputs language model logits
            model_url: Some(
                "https://huggingface.co/livekit/turn-detector/resolve/main/model_quantized.onnx"
                    .to_string(),
            ),
            tokenizer_path: None,
            tokenizer_url: Some(
                "https://huggingface.co/livekit/turn-detector/resolve/main/tokenizer.json"
                    .to_string(),
            ),
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
    #[cfg(feature = "turn-detect")]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub threshold: f32,
    pub max_context_turns: usize,
    pub max_sequence_length: usize,
    pub model_revision: String,
    pub tokenizer_revision: String,
    pub language_thresholds: std::collections::HashMap<String, f32>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        let mut language_thresholds = std::collections::HashMap::new();
        language_thresholds.insert("en".to_string(), 0.7);
        language_thresholds.insert("multilingual".to_string(), 0.7);

        Self {
            threshold: 0.7,
            max_context_turns: 4,
            max_sequence_length: 512,
            model_revision: "main".to_string(),
            tokenizer_revision: "main".to_string(),
            language_thresholds,
        }
    }
}
