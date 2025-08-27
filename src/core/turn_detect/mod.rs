pub mod config;
pub mod detector;
pub mod model_manager;
pub mod tokenizer;

pub use config::TurnDetectorConfig;
pub use detector::TurnDetector;

use anyhow::Result;
use std::path::Path;

pub async fn create_turn_detector(model_path: Option<&Path>) -> Result<TurnDetector> {
    TurnDetector::new(model_path).await
}
