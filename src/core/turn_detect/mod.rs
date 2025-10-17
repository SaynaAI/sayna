#[cfg(feature = "turn-detect")]
pub mod assets;
pub mod config;
#[cfg(feature = "turn-detect")]
pub mod detector;
#[cfg(feature = "turn-detect")]
pub mod model_manager;
#[cfg(feature = "turn-detect")]
pub mod tokenizer;

#[cfg(not(feature = "turn-detect"))]
mod stub;

pub use config::TurnDetectorConfig;

#[cfg(feature = "turn-detect")]
pub use detector::{TurnDetector, TurnDetectorBuilder};

#[cfg(not(feature = "turn-detect"))]
pub use stub::{TurnDetector, TurnDetectorBuilder, create_turn_detector};

#[cfg(feature = "turn-detect")]
use anyhow::Result;
#[cfg(feature = "turn-detect")]
use std::path::Path;

#[cfg(feature = "turn-detect")]
pub async fn create_turn_detector(model_path: Option<&Path>) -> Result<TurnDetector> {
    TurnDetector::new(model_path).await
}
